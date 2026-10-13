//! Model preparation uses synthetic data and does not submit normal pipeline work.
use crate::{BootOptions, Services};
use maru_core::{Config, Paths, config::OcrBackend};
use maru_llm::LlmStatus;
use maru_llm::testing::{MockResponse, spawn_mock, sse_answer};
use maru_ocr::OcrEngine;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Home(std::path::PathBuf);
impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
                "maru-controls-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Ocr(Arc<AtomicUsize>);
impl OcrEngine for Ocr {
    fn name(&self) -> &'static str {
        "scripted"
    }
    fn recognize(&self, _: &image::RgbImage) -> maru_ocr::Result<maru_ocr::OcrOutput> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(maru_ocr::OcrOutput::default())
    }
}
async fn services(home: &Home, endpoint: &str, reads: Arc<AtomicUsize>) -> Services {
    let mut cfg = Config::default();
    cfg.llm.endpoint = Some(endpoint.into());
    cfg.ocr.backend = OcrBackend::Windows;
    cfg.ocr.vision_reader = false;
    cfg.tts.enabled = false;
    let platform = maru_platform::Platform {
        selection: Arc::new(maru_platform::testing::FakeSelection::default()),
        screen: Arc::new(maru_platform::testing::FakeScreen::new(vec![])),
        os_ocr: Some(Arc::new(Ocr(reads))),
    };
    crate::bootstrap::boot_with_platform(
        Paths::under(&home.0),
        cfg,
        BootOptions {
            defer_models: true,
            ..Default::default()
        },
        Some(platform),
    )
    .await
}
fn response(_: &str, _: &str) -> MockResponse {
    MockResponse {
        status: 200,
        chunks: sse_answer(&["Hello."]),
        delay_ms: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparing_uses_only_fixed_input_and_does_not_toggle_live_or_add_history() {
    let home = Home::new();
    let server = spawn_mock(response).await;
    let reads = Arc::new(AtomicUsize::new(0));
    let service = services(&home, &server.url, reads.clone()).await;
    service.prepare_live_session().await.unwrap();
    assert_eq!(service.live_setup_status().preparation, "Prepared");
    assert_eq!(service.live_setup_status().device, "External server");
    assert!(!service.engine.inner.live_active.load(Ordering::Acquire));
    assert_eq!(service.engine.live_work(), (0, 0));
    assert!(service.engine.history().is_empty());
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].1.contains("안녕하세요."));
    assert!(!requests[0].1.contains("Word:"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparation_failure_can_be_retried_and_release_does_not_contact_an_external_server() {
    let failed = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = failed.clone();
    let server = spawn_mock(move |path, body| {
        if flag.load(Ordering::Acquire) {
            MockResponse {
                status: 503,
                chunks: vec![b"unavailable".to_vec()],
                delay_ms: 0,
            }
        } else {
            response(path, body)
        }
    })
    .await;
    let home = Home::new();
    let service = services(&home, &server.url, Arc::new(AtomicUsize::new(0))).await;
    assert!(
        service
            .prepare_live_session()
            .await
            .unwrap_err()
            .contains("503")
    );
    assert_eq!(
        service.live_setup_status().preparation,
        "Preparation failed"
    );
    service.release_idle_models().await.unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    assert_eq!(service.llm_status(), LlmStatus::Ready);
    failed.store(false, Ordering::Release);
    service.prepare_live_session().await.unwrap();
    assert_eq!(service.live_setup_status().preparation, "Prepared");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparation_rejects_live_capture_and_duplicate_actions_without_starting_another_request() {
    let home = Home::new();
    let server = spawn_mock(|_, _| MockResponse {
        status: 200,
        chunks: sse_answer(&["Hello."]),
        delay_ms: 50,
    })
    .await;
    let service = Arc::new(services(&home, &server.url, Arc::new(AtomicUsize::new(0))).await);
    service.engine.set_live_active(true);
    assert!(service.prepare_live_session().await.is_err());
    assert!(server.requests.lock().unwrap().is_empty());
    service.engine.set_live_active(false);
    let prepare = {
        let service = service.clone();
        tokio::spawn(async move { service.prepare_live_session().await })
    };
    while server.requests.lock().unwrap().is_empty() {
        tokio::task::yield_now().await;
    }
    assert!(service.live_setup_status().busy);
    assert!(
        service
            .prepare_live_session()
            .await
            .unwrap_err()
            .contains("still running")
    );
    assert!(
        service
            .release_idle_models()
            .await
            .unwrap_err()
            .contains("still running")
    );
    prepare.await.unwrap().unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}
