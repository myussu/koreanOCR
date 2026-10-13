//! A single owned server, switching models only after all active requests finish.

use crate::{LlamaServer, LlmError, LlmStatus, Result};
use maru_core::{Paths, config::LlmConfig};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::{OwnedRwLockReadGuard, RwLock, watch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRole {
    Main,
    Live,
}

#[derive(Default)]
struct Running {
    role: Option<ModelRole>,
    server: Option<LlamaServer>,
}

pub struct ModelRuntime {
    main: Option<LlmConfig>,
    live: LlmConfig,
    paths: Paths,
    running: Arc<RwLock<Running>>,
    status: watch::Sender<LlmStatus>,
    ready: AtomicBool,
    role: std::sync::atomic::AtomicU8,
    process: std::sync::Mutex<Option<(ModelRole, u32)>>,
}

impl std::fmt::Debug for ModelRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRuntime").finish_non_exhaustive()
    }
}

pub struct ModelLease {
    _running: OwnedRwLockReadGuard<Running>,
}

struct LoadingStatus<'a>(&'a watch::Sender<LlmStatus>);

impl Drop for LoadingStatus<'_> {
    fn drop(&mut self) {
        if matches!(*self.0.borrow(), LlmStatus::Starting) {
            self.0.send_replace(LlmStatus::Disabled);
        }
    }
}

impl ModelRuntime {
    pub fn new(main: Option<LlmConfig>, live: LlmConfig, paths: Paths, status: watch::Sender<LlmStatus>) -> Arc<Self> {
        Arc::new(Self {
            main,
            live,
            paths,
            running: Arc::new(RwLock::new(Running::default())),
            status,
            ready: AtomicBool::new(false),
            role: std::sync::atomic::AtomicU8::new(0),
            process: std::sync::Mutex::new(None),
        })
    }

    pub fn is_running(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn status(&self) -> LlmStatus { self.status.borrow().clone() }
    /// Never wait behind a model switch just to collect diagnostics.
    pub fn owned_process(&self) -> Option<(ModelRole, u32)> {
        *self.process.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn current_role(&self) -> Option<ModelRole> {
        match self.role.load(Ordering::Acquire) {
            1 => Some(ModelRole::Main),
            2 => Some(ModelRole::Live),
            _ => None,
        }
    }

    /// Release only our idle child. An active stream or model load keeps its lease.
    pub async fn release_idle(&self) -> Result<Option<bool>> {
        let Ok(mut running) = self.running.try_write() else { return Ok(None) };
        let mut owned = running.server.take();
        let had_server = owned.is_some();
        running.role = None;
        self.ready.store(false, Ordering::Release);
        self.role.store(0, Ordering::Release);
        *self.process.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.status.send_replace(LlmStatus::Disabled);
        if let Some(server) = owned.as_mut() { server.stop().await?; }
        Ok(Some(had_server))
    }

    pub async fn acquire(&self, role: ModelRole) -> Result<ModelLease> {
        if role == ModelRole::Live && self.live.model_path.is_none() {
            return Err(LlmError::Spawn("choose a local GGUF file for live subtitles in Settings".into()));
        }
        let running = self.running.clone().read_owned().await;
        if running.role == Some(role) && running.server.is_some() {
            return Ok(ModelLease { _running: running });
        }
        drop(running);
        let mut running = self.running.clone().write_owned().await;
        if running.role != Some(role) || running.server.is_none() {
            let cfg = match role {
                ModelRole::Main => self
                    .main
                    .as_ref()
                    .ok_or_else(|| LlmError::Spawn("the main model is not managed by this live session".into()))?,
                ModelRole::Live => &self.live,
            };
            self.ready.store(false, Ordering::Release);
            self.role.store(0, Ordering::Release);
            *self.process.lock().unwrap_or_else(|p| p.into_inner()) = None;
            self.status.send_replace(LlmStatus::Starting);
            // Cancelling a preload must not leave the controls showing "Loading" forever.
            let _loading = LoadingStatus(&self.status);
            running.role = None;
            if let Some(mut old) = running.server.take()
                && let Err(error) = old.stop().await
            {
                self.status.send_replace(LlmStatus::Failed(error.to_string()));
                return Err(error);
            }
            tracing::info!(?role, "loading translation model; previous owned model is unloaded");
            match LlamaServer::start(cfg, &self.paths, Duration::from_secs(cfg.ready_timeout_secs)).await {
                Ok(server) => {
                    *self.process.lock().unwrap_or_else(|p| p.into_inner()) = server.process_id().map(|pid| (role, pid));
                    running.server = Some(server);
                    running.role = Some(role);
                    self.role.store(if role == ModelRole::Main { 1 } else { 2 }, Ordering::Release);
                    self.ready.store(true, Ordering::Release);
                    self.status.send_replace(LlmStatus::Ready);
                }
                Err(error) => {
                    self.status.send_replace(LlmStatus::Failed(error.to_string()));
                    return Err(error);
                }
            }
        }
        Ok(ModelLease {
            _running: tokio::sync::OwnedRwLockWriteGuard::downgrade(running),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ChildServer;
    use std::path::Path;

    async fn running() -> Arc<ModelRuntime> {
        let (tx, _) = watch::channel(LlmStatus::Ready);
        let cfg = LlmConfig::default();
        let live = LlmConfig { model_path: Some("scripted-live.gguf".into()), ..cfg.clone() };
        let runtime = ModelRuntime::new(Some(cfg), live, Paths::under(&std::env::temp_dir()), tx);
        #[cfg(windows)]
        let child = ChildServer::spawn("test model", Path::new("cmd"), &["/C".into(), "ping -n 30 127.0.0.1 >NUL".into()], &[]).unwrap();
        #[cfg(not(windows))]
        let child = ChildServer::spawn("test model", Path::new("sh"), &["-c".into(), "sleep 30".into()], &[]).unwrap();
        let mut slot = runtime.running.write().await;
        slot.role = Some(ModelRole::Live);
        slot.server = Some(LlamaServer::test_owned(child));
        *runtime.process.lock().unwrap() = slot.server.as_ref().and_then(|s| s.process_id()).map(|pid| (ModelRole::Live, pid));
        runtime.ready.store(true, Ordering::Release);
        drop(slot);
        runtime
    }

    #[tokio::test]
    async fn switching_waits_for_a_stream_lease_and_a_failed_start_is_reported() {
        let runtime = running().await;
        let lease = runtime.acquire(ModelRole::Live).await.unwrap();
        let switch = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.acquire(ModelRole::Main).await })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!switch.is_finished());
        assert!(runtime.owned_process().is_some());
        assert!(runtime.is_running());
        drop(lease);
        assert!(switch.await.unwrap().is_err(), "the test main config has no weights");
        assert!(!runtime.is_running());
        assert!(matches!(*runtime.status.borrow(), LlmStatus::Failed(_)));
    }

    #[tokio::test]
    async fn cancelling_a_waiting_switch_does_not_unload_the_active_model() {
        let runtime = running().await;
        let lease = runtime.acquire(ModelRole::Live).await.unwrap();
        let switch = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.acquire(ModelRole::Main).await })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        switch.abort();
        let _ = switch.await;
        let another = tokio::time::timeout(Duration::from_secs(1), runtime.acquire(ModelRole::Live))
            .await
            .unwrap()
            .unwrap();
        assert!(runtime.is_running());
        drop(lease);
        drop(another);
    }

    #[tokio::test]
    async fn releasing_an_idle_model_preserves_active_streams_and_clears_its_owned_process() {
        let runtime = running().await;
        let lease = runtime.acquire(ModelRole::Live).await.unwrap();
        let pid = runtime.owned_process();
        assert_eq!(runtime.release_idle().await.unwrap(), None);
        assert_eq!(runtime.owned_process(), pid);
        drop(lease);
        assert_eq!(runtime.release_idle().await.unwrap(), Some(true));
        assert!(!runtime.is_running());
        assert_eq!(runtime.current_role(), None);
        assert_eq!(runtime.owned_process(), None);
        assert_eq!(runtime.status(), LlmStatus::Disabled);
        assert_eq!(runtime.release_idle().await.unwrap(), Some(false));
    }

    #[tokio::test]
    async fn cancelling_a_model_start_restores_the_unloaded_status() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let cfg = LlmConfig {
            port: listener.local_addr().unwrap().port(),
            hf_repo: Some("scripted/model".into()),
            server_path: Some("missing-test-server".into()),
            ..Default::default()
        };
        let (status, mut observed) = watch::channel(LlmStatus::Disabled);
        let runtime = ModelRuntime::new(Some(cfg.clone()), cfg, Paths::under(&std::env::temp_dir()), status);
        let load = { let runtime = runtime.clone(); tokio::spawn(async move { runtime.acquire(ModelRole::Main).await }) };
        tokio::time::timeout(Duration::from_secs(2), observed.wait_for(|s| matches!(s, LlmStatus::Starting))).await.unwrap().unwrap();
        load.abort();
        let _ = load.await;
        assert_eq!(*observed.borrow(), LlmStatus::Disabled);
        assert!(!runtime.is_running());
        assert!(runtime.owned_process().is_none());
        drop(listener);
    }

    #[tokio::test]
    async fn a_missing_live_path_leaves_the_running_main_model_available() {
        let mut runtime = running().await;
        Arc::get_mut(&mut runtime).unwrap().live.model_path = None;
        runtime.running.write().await.role = Some(ModelRole::Main);
        assert!(runtime.acquire(ModelRole::Live).await.is_err());
        assert!(runtime.is_running());
        let lease = runtime.acquire(ModelRole::Main).await.unwrap();
        drop(lease);
    }

    /// No downloads: explicit local weights and a separately bound loopback port.
    #[tokio::test]
    #[ignore = "requires MARU_TEST_SERVER, MARU_TEST_MAIN, MARU_TEST_LIVE and MARU_TEST_HOME"]
    async fn approved_local_models_switch_and_stream_using_one_owned_server() {
        use crate::{KoEnRequest, LiveRequest, LlmChunk, LlmClient, PromptContext};
        use maru_core::{event::Variant, live_model::LiveModel};
        let local = |name: &str| std::path::PathBuf::from(std::env::var(name).unwrap());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let paths = Paths::under(&local("MARU_TEST_HOME"));
        paths.ensure().unwrap();
        let main = LlmConfig {
            model_path: Some(local("MARU_TEST_MAIN")),
            hf_repo: None,
            server_path: Some(local("MARU_TEST_SERVER")),
            port,
            ready_timeout_secs: 180,
            ..Default::default()
        };
        let live = LlmConfig {
            model_path: Some(local("MARU_TEST_LIVE")),
            ctx_size: 4096,
            parallel: 2,
            ..main.clone()
        };
        let family = if std::env::var("MARU_TEST_FAMILY").unwrap() == "hy_mt2" {
            LiveModel::HyMt2
        } else {
            LiveModel::Milmmt
        };
        let (tx, _) = watch::channel(LlmStatus::Disabled);
        let runtime = ModelRuntime::new(Some(main.clone()), live.clone(), paths, tx);
        let client = LlmClient::from_config(&live).with_runtime(runtime.clone(), ModelRole::Live);
        let ctx = PromptContext::default();
        let mut translation = String::new();
        client
            .translate_selected_live(
                family,
                &LiveRequest {
                    text: "폐하, 어제 제47구가 봉쇄됐습니다.",
                    ctx: &ctx,
                    turns: &[],
                },
                &mut |c| {
                    if let LlmChunk::TextDone { text, .. } = c {
                        translation = text;
                    }
                },
            )
            .await
            .unwrap();
        assert!(!translation.is_empty());
        println!("live {family:?}: {translation}");
        drop(client);
        let client = LlmClient::from_config(&main).with_runtime(runtime.clone(), ModelRole::Main);
        let mut translation = String::new();
        client
            .translate_ko_en(
                &KoEnRequest {
                    text: "폐하, 어제 제47구가 봉쇄됐습니다.",
                    ctx: &ctx,
                },
                &mut |c| {
                    if let LlmChunk::TextDone {
                        variant: Variant::Primary,
                        text,
                    } = c
                    {
                        translation = text;
                    }
                },
            )
            .await
            .unwrap();
        assert!(!translation.is_empty());
        println!("restored main: {translation}");
        assert_eq!(runtime.running.read().await.role, Some(ModelRole::Main));
        drop(client);
        drop(runtime);
        let probe = LlmClient::from_config(&main);
        assert!(crate::server::port_frees_up(Duration::from_secs(5), || probe.health()).await);
    }
}
