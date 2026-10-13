//! Load optional local services on the first request, sharing one start across callers.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use image::RgbImage;
use maru_llm::LlmStatus;
use maru_ocr::{OcrEngine, OcrError, OcrOutput};
use tokio::sync::{watch, OwnedRwLockReadGuard, RwLock};

type Start = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send>;
type Factory = Arc<dyn Fn() -> Start + Send + Sync>;
type Stop = Box<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

pub(crate) struct DeferredModel {
    start: Mutex<Option<Start>>,
    result: watch::Receiver<Option<Result<(), String>>>,
    complete: watch::Sender<Option<Result<(), String>>>,
    status: watch::Sender<LlmStatus>,
    runtime: tokio::runtime::Handle,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    factory: Option<Factory>,
    stop: Option<Stop>,
    requests: Arc<RwLock<()>>,
}

impl DeferredModel {
    #[cfg(test)]
    pub(crate) fn new<F, Fut>(status: watch::Sender<LlmStatus>, start: F) -> Arc<Self>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        Self::build(status, Box::new(move || Box::pin(start())), None, None)
    }

    pub(crate) fn restartable<F, Fut, S, Stopped>(
        status: watch::Sender<LlmStatus>,
        start: F,
        stop: S,
    ) -> Arc<Self>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
        S: Fn() -> Stopped + Send + Sync + 'static,
        Stopped: Future<Output = Result<(), String>> + Send + 'static,
    {
        let start = Arc::new(start);
        let factory: Factory = Arc::new(move || {
            let start = start.clone();
            Box::new(move || Box::pin(start()))
        });
        Self::build(
            status,
            factory(),
            Some(factory),
            Some(Box::new(move || Box::pin(stop()))),
        )
    }

    fn build(
        status: watch::Sender<LlmStatus>,
        start: Start,
        factory: Option<Factory>,
        stop: Option<Stop>,
    ) -> Arc<Self> {
        let (complete, result) = watch::channel(None);
        Arc::new(Self {
            start: Mutex::new(Some(start)),
            result,
            complete,
            status,
            runtime: tokio::runtime::Handle::current(),
            task: Mutex::new(None),
            factory,
            stop,
            requests: Arc::new(RwLock::new(())),
        })
    }

    pub(crate) async fn ensure_loaded(&self) -> Result<(), String> {
        self.acquire().await.map(|_| ())
    }

    pub(crate) async fn acquire(&self) -> Result<OwnedRwLockReadGuard<()>, String> {
        let request = self.requests.clone().read_owned().await;
        // The shared worker also holds a lease after a waiting caller is cancelled.
        let loading = self.requests.clone().read_owned().await;
        // The worker belongs to this service, so cancelling one caller cannot start a duplicate.
        if let Some(start) = self.start.lock().unwrap_or_else(|p| p.into_inner()).take() {
            self.status.send_replace(LlmStatus::Starting);
            let (complete, status) = (self.complete.clone(), self.status.clone());
            let task = self.runtime.spawn(async move {
                let _loading = loading;
                let load =
                    tokio_util::task::AbortOnDropHandle::new(tokio::spawn(
                        async move { start().await },
                    ));
                let outcome = load
                    .await
                    .unwrap_or_else(|e| Err(format!("The model loader stopped unexpectedly: {e}")));
                status.send_replace(match &outcome {
                    Ok(()) => LlmStatus::Ready,
                    Err(error) => LlmStatus::Failed(error.clone()),
                });
                complete.send_replace(Some(outcome));
            });
            *self.task.lock().unwrap_or_else(|p| p.into_inner()) = Some(task);
        }
        let mut result = self.result.clone();
        let finished = result
            .wait_for(|r| r.is_some())
            .await
            .map_err(|_| "The model loader stopped before it was ready.".to_string())?;
        finished
            .as_ref()
            .cloned()
            .unwrap_or_else(|| Err("The model loader did not report a result.".into()))?;
        Ok(request)
    }

    pub(crate) async fn release_idle(&self) -> Result<Option<bool>, String> {
        let Ok(_idle) = self.requests.try_write() else {
            return Ok(None);
        };
        let Some(factory) = &self.factory else {
            return Err("This model cannot reload without a restart.".into());
        };
        let loaded = matches!(*self.status.borrow(), LlmStatus::Ready);
        self.task.lock().unwrap_or_else(|p| p.into_inner()).take();
        *self.start.lock().unwrap_or_else(|p| p.into_inner()) = Some(factory());
        self.complete.send_replace(None);
        self.status.send_replace(LlmStatus::Disabled);
        if let Some(stop) = &self.stop {
            stop().await?;
        }
        Ok(Some(loaded))
    }
}

impl Drop for DeferredModel {
    fn drop(&mut self) {
        if let Some(task) = self.task.lock().unwrap_or_else(|p| p.into_inner()).take() {
            task.abort();
        }
    }
}

pub(crate) struct DeferredOcr {
    pub(crate) inner: Arc<dyn OcrEngine>,
    pub(crate) model: Arc<DeferredModel>,
}

impl OcrEngine for DeferredOcr {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn recognize(&self, image: &RgbImage) -> maru_ocr::Result<OcrOutput> {
        // OCR is synchronous. Wait on a separate thread while the owning runtime loads the model.
        let _request = std::thread::scope(|scope| {
            scope
                .spawn(|| self.model.runtime.block_on(self.model.acquire()))
                .join()
                .unwrap_or_else(|_| Err("The OCR model loader thread stopped unexpectedly.".into()))
        })
        .map_err(OcrError::Run)?;
        self.inner.recognize(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn an_idle_model_reloads_once_after_release_and_active_requests_are_preserved() {
        let calls = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let (status, rx) = watch::channel(LlmStatus::Disabled);
        let model = DeferredModel::restartable(
            status,
            {
                let calls = calls.clone();
                move || {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                }
            },
            {
                let stops = stops.clone();
                move || {
                    let stops = stops.clone();
                    async move {
                        stops.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                }
            },
        );
        let request = model.acquire().await.unwrap();
        assert_eq!(model.release_idle().await.unwrap(), None);
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        drop(request);
        assert_eq!(model.release_idle().await.unwrap(), Some(true));
        assert_eq!(*rx.borrow(), LlmStatus::Disabled);
        let (a, b) = tokio::join!(model.acquire(), model.acquire());
        assert!(a.is_ok() && b.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn releasing_during_a_shared_load_keeps_the_worker_even_after_its_caller_is_cancelled() {
        let release = Arc::new(tokio::sync::Notify::new());
        let (status, mut rx) = watch::channel(LlmStatus::Disabled);
        let model = DeferredModel::restartable(
            status,
            {
                let release = release.clone();
                move || {
                    let release = release.clone();
                    async move {
                        release.notified().await;
                        Ok(())
                    }
                }
            },
            || async { Ok(()) },
        );
        let waiter = {
            let model = model.clone();
            tokio::spawn(async move { model.ensure_loaded().await })
        };
        rx.wait_for(|s| *s == LlmStatus::Starting).await.unwrap();
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(model.release_idle().await.unwrap(), None);
        release.notify_one();
        model.ensure_loaded().await.unwrap();
        assert_eq!(model.release_idle().await.unwrap(), Some(true));
    }

    #[tokio::test]
    async fn cancelling_a_release_clears_stale_readiness_and_allows_a_new_load() {
        let (status, rx) = watch::channel(LlmStatus::Disabled);
        let stopping = Arc::new(tokio::sync::Notify::new());
        let model = DeferredModel::restartable(status, || async { Ok(()) }, {
            let stopping = stopping.clone();
            move || {
                let stopping = stopping.clone();
                async move {
                    stopping.notify_one();
                    std::future::pending::<()>().await;
                    Ok(())
                }
            }
        });
        model.ensure_loaded().await.unwrap();
        let release = {
            let model = model.clone();
            tokio::spawn(async move { model.release_idle().await })
        };
        stopping.notified().await;
        release.abort();
        let _ = release.await;
        assert_eq!(*rx.borrow(), LlmStatus::Disabled);
        model.ensure_loaded().await.unwrap();
        assert_eq!(*rx.borrow(), LlmStatus::Ready);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_ocr_request_holds_its_reader_lease_until_recognition_finishes() {
        struct Reader {
            started: std::sync::mpsc::Sender<()>,
            finish: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl OcrEngine for Reader {
            fn name(&self) -> &'static str {
                "scripted"
            }
            fn recognize(&self, _: &RgbImage) -> maru_ocr::Result<OcrOutput> {
                self.started.send(()).unwrap();
                self.finish.lock().unwrap().recv().unwrap();
                Ok(OcrOutput::default())
            }
        }
        let (started, observed) = std::sync::mpsc::channel();
        let (finish, wait) = std::sync::mpsc::channel();
        let (status, _) = watch::channel(LlmStatus::Disabled);
        let model = DeferredModel::restartable(status, || async { Ok(()) }, || async { Ok(()) });
        let ocr = DeferredOcr {
            inner: Arc::new(Reader {
                started,
                finish: Mutex::new(wait),
            }),
            model: model.clone(),
        };
        let request = tokio::task::spawn_blocking(move || ocr.recognize(&RgbImage::new(1, 1)));
        tokio::task::spawn_blocking(move || observed.recv().unwrap())
            .await
            .unwrap();
        assert_eq!(model.release_idle().await.unwrap(), None);
        finish.send(()).unwrap();
        assert!(request.await.unwrap().is_ok());
        assert_eq!(model.release_idle().await.unwrap(), Some(true));
    }

    #[tokio::test]
    async fn construction_and_status_reads_do_not_load_a_model_and_concurrent_requests_share_one_start(
    ) {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (status, rx) = watch::channel(LlmStatus::Disabled);
        let model = DeferredModel::new(status, move || async move {
            count.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            Ok(())
        });
        assert_eq!(*rx.borrow(), LlmStatus::Disabled);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let (a, b) = tokio::join!(model.ensure_loaded(), model.ensure_loaded());
        assert!(a.is_ok() && b.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*rx.borrow(), LlmStatus::Ready);
    }

    #[tokio::test]
    async fn cancelling_a_waiter_keeps_the_shared_load_and_a_failed_load_does_not_restart_on_every_request(
    ) {
        let (release, wait) = tokio::sync::oneshot::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (status, rx) = watch::channel(LlmStatus::Disabled);
        let model = DeferredModel::new(status, move || async move {
            count.fetch_add(1, Ordering::SeqCst);
            let _ = wait.await;
            Err("scripted model failure".into())
        });
        let waiter = {
            let model = model.clone();
            tokio::spawn(async move { model.ensure_loaded().await })
        };
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        waiter.abort();
        let _ = waiter.await;
        release.send(()).unwrap();
        assert_eq!(
            model.ensure_loaded().await.unwrap_err(),
            "scripted model failure"
        );
        assert!(model.ensure_loaded().await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(matches!(*rx.borrow(), LlmStatus::Failed(_)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_ocr_waits_for_its_reader_and_does_not_replace_it_with_the_fast_reader() {
        struct Reader;
        impl OcrEngine for Reader {
            fn name(&self) -> &'static str {
                "scripted-quality-reader"
            }
            fn recognize(&self, _: &RgbImage) -> maru_ocr::Result<OcrOutput> {
                Ok(OcrOutput::default())
            }
        }
        let (status, rx) = watch::channel(LlmStatus::Disabled);
        let model = DeferredModel::new(status, || async {
            tokio::task::yield_now().await;
            Ok(())
        });
        let reader = DeferredOcr {
            inner: Arc::new(Reader),
            model,
        };
        assert_eq!(*rx.borrow(), LlmStatus::Disabled);
        assert_eq!(reader.name(), "scripted-quality-reader");
        assert!(
            tokio::task::spawn_blocking(move || reader.recognize(&RgbImage::new(1, 1)))
                .await
                .unwrap()
                .is_ok()
        );
        assert_eq!(*rx.borrow(), LlmStatus::Ready);
    }

    #[tokio::test]
    async fn a_loader_panic_is_reported_instead_of_leaving_every_caller_waiting() {
        let (status, rx) = watch::channel(LlmStatus::Disabled);
        let model = DeferredModel::new(status, || async { panic!("scripted loader panic") });
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), model.ensure_loaded())
            .await
            .unwrap();
        assert!(result.unwrap_err().contains("unexpectedly"));
        assert!(matches!(*rx.borrow(), LlmStatus::Failed(_)));
    }
}
