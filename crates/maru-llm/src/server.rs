//! Spawns and supervises `llama-server`.
//!
//! Flags were checked against the current llama.cpp server documentation:
//! `-ngl all`, `-fa on`, `--cache-reuse`, `--reasoning off`, `-np`, and the
//! speculative-decoding pair `-md` / `--spec-draft-n-max`. The port is always
//! passed explicitly because llama-server's default has changed between releases.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use maru_core::Paths;
use maru_core::config::LlmConfig;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::client::LlmClient;
use crate::{LlmError, Result};

/// Children must not outlive us.
///
/// `kill_on_drop` only helps when the destructor runs. Quitting the tray app calls
/// `process::exit`, a crash aborts, and Task Manager "End task" stops us cold: in all three the
/// `llama-server` processes used to stay behind, each holding a model in memory. A Windows job
/// object with "kill on close" is enforced by the OS itself: when our process ends for any
/// reason its handle closes and every process in the job is terminated.
#[cfg(windows)]
mod job {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    use tokio::process::Child;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows::core::PCWSTR;

    struct Job(HANDLE);
    // A job handle is just a kernel object reference; it is never closed on purpose.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    static JOB: OnceLock<Option<Job>> = OnceLock::new();

    fn job() -> Option<&'static Job> {
        JOB.get_or_init(|| unsafe {
            let handle = CreateJobObjectW(None, PCWSTR::null()).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast::<c_void>(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .ok()?;
            Some(Job(handle))
        })
        .as_ref()
    }

    /// Is `child` in our job (and so ended when we end)?
    #[cfg(test)]
    pub fn contains(child: &Child) -> bool {
        let (Some(job), Some(process)) = (job(), child.raw_handle()) else { return false };
        let mut inside = windows::core::BOOL::default();
        unsafe { windows::Win32::System::JobObjects::IsProcessInJob(HANDLE(process), Some(job.0), &mut inside).is_ok() && inside.as_bool() }
    }

    /// Tie `child`'s life to ours. False if the OS refused (it then still dies with `kill_on_drop`).
    pub fn adopt(child: &Child) -> bool {
        let (Some(job), Some(process)) = (job(), child.raw_handle()) else { return false };
        unsafe { AssignProcessToJobObject(job.0, HANDLE(process)).is_ok() }
    }
}

/// Where the weights come from.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSource {
    File(PathBuf),
    /// `user/model[:quant]`, downloaded by llama-server itself.
    HuggingFace(String),
}

impl ModelSource {
    pub fn from_config(cfg: &LlmConfig) -> Option<Self> {
        match (&cfg.model_path, &cfg.hf_repo) {
            (Some(p), _) => Some(Self::File(p.clone())),
            (None, Some(repo)) if !repo.trim().is_empty() => Some(Self::HuggingFace(repo.trim().to_string())),
            _ => None,
        }
    }
}

/// The command line for `llama-server` (without the executable).
pub fn build_args(cfg: &LlmConfig, model: &ModelSource) -> Vec<String> {
    let mut a: Vec<String> = match model {
        ModelSource::File(p) => vec!["-m".into(), p.display().to_string()],
        // The vision projector some repos ship is useless for translation and costs VRAM.
        ModelSource::HuggingFace(repo) if cfg.vision => vec!["-hf".into(), repo.clone()],
        ModelSource::HuggingFace(repo) => vec!["-hf".into(), repo.clone(), "--no-mmproj".into()],
    };
    a.extend([
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        cfg.port.to_string(),
        // `-c` is shared between slots, so give every slot the configured context.
        "-c".into(),
        (cfg.ctx_size * cfg.parallel.max(1)).to_string(),
        "-np".into(),
        cfg.parallel.max(1).to_string(),
        "-ngl".into(),
        cfg.gpu_layers.clone(),
        "-fa".into(),
        if cfg.flash_attn { "on" } else { "off" }.into(),
        // Re-use KV-cache chunks of the shared system prompt even after small prompt edits.
        "--cache-reuse".into(),
        "256".into(),
        // Translation must not "think" out loud: it is pure latency.
        "--reasoning".into(),
        "off".into(),
    ]);
    if let Some(draft) = &cfg.draft_model_path {
        a.extend([
            "-md".into(),
            draft.display().to_string(),
            "--spec-draft-n-max".into(),
            "4".into(),
            "-ngld".into(),
            cfg.gpu_layers.clone(),
        ]);
    }
    a.extend(cfg.extra_args.iter().cloned());
    a
}

/// Where to find the executable: explicit path, then our bin dir, then `PATH`.
pub fn resolve_binary(cfg: &LlmConfig, paths: &Paths) -> PathBuf {
    if let Some(p) = &cfg.server_path {
        return p.clone();
    }
    let name = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    let local = paths.bin_dir.join(name);
    if local.exists() { local } else { PathBuf::from(name) }
}

/// A running `llama-server`. Killed when dropped.
pub struct LlamaServer {
    child: Child,
    client: LlmClient,
    log: Arc<Mutex<VecDeque<String>>>,
}

impl LlamaServer {
    /// Start the server and return once it answers `/health` (model loaded).
    pub async fn start(cfg: &LlmConfig, paths: &Paths, ready_timeout: Duration) -> Result<Self> {
        let model = ModelSource::from_config(cfg)
            .ok_or_else(|| LlmError::Spawn("no model configured: set [llm] model_path or hf_repo in config.toml".into()))?;
        if let ModelSource::File(p) = &model
            && !p.exists()
        {
            return Err(LlmError::Spawn(format!("model file not found: {}", p.display())));
        }
        let binary = resolve_binary(cfg, paths);
        let args = build_args(cfg, &model);
        tracing::info!("starting {} {}", binary.display(), args.join(" "));

        // Something already answering on our private port is almost always a server left behind by
        // an earlier run. Starting another would fail to bind and exit, which looks like a
        // mysterious "llama-server exited": say what is actually wrong.
        if LlmClient::from_config(cfg).health().await {
            return Err(LlmError::Spawn(format!(
                "port {} is already answered by another server (most likely a llama-server left over from an earlier run). \
                 End it in Task Manager, or run `taskkill /F /IM llama-server.exe`, then start Maru again.",
                cfg.port
            )));
        }

        let mut cmd = Command::new(&binary);
        cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd
            .spawn()
            .map_err(|e| LlmError::Spawn(format!("{}: {e} (set [llm] server_path or put llama-server on PATH)", binary.display())))?;

        #[cfg(windows)]
        if !job::adopt(&child) {
            tracing::warn!("could not tie llama-server to this process; it may outlive a crash");
        }

        let log = Arc::new(Mutex::new(VecDeque::with_capacity(64)));
        if let Some(out) = child.stdout.take() {
            spawn_log_reader(out, log.clone());
        }
        if let Some(err) = child.stderr.take() {
            spawn_log_reader(err, log.clone());
        }

        let client = LlmClient::from_config(cfg);
        let mut server = Self { child, client, log };
        server.wait_until_ready(ready_timeout).await?;
        Ok(server)
    }

    async fn wait_until_ready(&mut self, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| LlmError::NotReady(e.to_string()))? {
                return Err(LlmError::NotReady(format!("llama-server exited ({status}). Last output:\n{}", self.log_tail())));
            }
            if self.client.health().await {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(LlmError::NotReady(format!("still loading after {timeout:?}. Last output:\n{}", self.log_tail())));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub fn client(&self) -> LlmClient {
        self.client.clone()
    }

    /// The last lines the server printed, for error messages.
    pub fn log_tail(&self) -> String {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).iter().rev().take(12).rev().cloned().collect::<Vec<_>>().join("\n")
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// Keep the last 64 lines of a child's output for error messages.
fn spawn_log_reader<R>(stream: R, log: Arc<Mutex<VecDeque<String>>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::trace!(target: "llama", "{line}");
            let mut l = log.lock().unwrap_or_else(|p| p.into_inner());
            if l.len() == 64 {
                l.pop_front();
            }
            l.push_back(line);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use super::*;

    fn cfg() -> LlmConfig {
        LlmConfig { port: 19001, ctx_size: 4096, parallel: 2, ..Default::default() }
    }

    fn pair<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
    }

    #[tokio::test]
    async fn a_server_already_on_our_port_is_reported_instead_of_racing_it() {
        use crate::client::testing::{MockResponse, spawn_mock};
        let mock = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![b"{}".to_vec()], delay_ms: 0 }).await;
        let port: u16 = mock.url.rsplit(':').next().unwrap().parse().unwrap();
        let cfg = LlmConfig { port, hf_repo: Some("a/b".into()), ..Default::default() };
        let err = LlamaServer::start(&cfg, &Paths::under(&std::env::temp_dir()), Duration::from_secs(1)).await.err().expect("must refuse").to_string();
        assert!(err.contains(&port.to_string()) && err.contains("already") && err.contains("taskkill"), "{err}");
    }

    /// The kill-on-close job is what ends llama-server when Maru crashes or is quit from the tray;
    /// check on real Windows that a child really ends up inside it.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_spawned_child_joins_the_kill_on_close_job() {
        let mut child = Command::new("cmd").args(["/C", "ping -n 30 127.0.0.1 >NUL"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).spawn().expect("cmd.exe starts");
        assert!(!job::contains(&child), "not in the job until adopted");
        assert!(job::adopt(&child), "the OS accepted the child");
        assert!(job::contains(&child), "and it is now in the job");
        child.kill().await.ok();
    }

    #[test]
    fn args_cover_gpu_cache_and_slots() {
        let a = build_args(&cfg(), &ModelSource::File("/m/model.gguf".into()));
        assert_eq!(pair(&a, "-m"), Some("/m/model.gguf"));
        assert_eq!(pair(&a, "--port"), Some("19001"));
        assert_eq!(pair(&a, "--host"), Some("127.0.0.1"));
        assert_eq!(pair(&a, "-ngl"), Some("all"));
        assert_eq!(pair(&a, "-fa"), Some("on"));
        assert_eq!(pair(&a, "-np"), Some("2"));
        assert_eq!(pair(&a, "-c"), Some("8192"), "context is multiplied by slot count");
        assert_eq!(pair(&a, "--reasoning"), Some("off"));
        assert!(a.contains(&"--cache-reuse".to_string()));
        assert!(!a.contains(&"-md".to_string()));
    }

    #[test]
    fn hugging_face_models_use_the_builtin_downloader_without_the_vision_projector() {
        let a = build_args(&cfg(), &ModelSource::HuggingFace("unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL".into()));
        assert_eq!(pair(&a, "-hf"), Some("unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL"));
        assert!(a.contains(&"--no-mmproj".to_string()) && !a.contains(&"-m".to_string()));

        let vision = LlmConfig { vision: true, ..cfg() };
        let a = build_args(&vision, &ModelSource::HuggingFace("a/b".into()));
        assert!(!a.contains(&"--no-mmproj".to_string()), "vision models keep their projector");

        let mut c = cfg();
        assert_eq!(ModelSource::from_config(&c), None);
        c.hf_repo = Some(" a/b:Q4_K_M ".into());
        assert_eq!(ModelSource::from_config(&c), Some(ModelSource::HuggingFace("a/b:Q4_K_M".into())));
        c.model_path = Some("/x.gguf".into());
        assert_eq!(ModelSource::from_config(&c), Some(ModelSource::File("/x.gguf".into())), "an explicit file wins");
    }

    #[test]
    fn draft_model_enables_speculative_decoding() {
        let mut c = cfg();
        c.draft_model_path = Some("/m/draft.gguf".into());
        c.extra_args = vec!["--no-warmup".into()];
        let a = build_args(&c, &ModelSource::File("m.gguf".into()));
        assert_eq!(pair(&a, "-md"), Some("/m/draft.gguf"));
        assert_eq!(pair(&a, "--spec-draft-n-max"), Some("4"));
        assert_eq!(a.last().map(String::as_str), Some("--no-warmup"), "extra args come last so they can override");
    }

    #[test]
    fn binary_resolution_prefers_explicit_then_local_then_path() {
        let paths = Paths::under(Path::new("/nonexistent-maru"));
        let mut c = cfg();
        assert!(resolve_binary(&c, &paths).file_name().unwrap().to_string_lossy().starts_with("llama-server"));
        c.server_path = Some("/opt/llama/llama-server".into());
        assert_eq!(resolve_binary(&c, &paths), PathBuf::from("/opt/llama/llama-server"));
    }

    #[tokio::test]
    async fn missing_model_is_a_clear_error() {
        let paths = Paths::under(Path::new("/nonexistent-maru"));
        let Err(e) = LlamaServer::start(&cfg(), &paths, Duration::from_secs(1)).await else { panic!("expected an error") };
        assert!(e.to_string().contains("model_path or hf_repo"), "{e}");
        let mut c = cfg();
        c.model_path = Some("/nope.gguf".into());
        let Err(e) = LlamaServer::start(&c, &paths, Duration::from_secs(1)).await else { panic!("expected an error") };
        assert!(e.to_string().contains("not found"), "{e}");
    }

    /// Supervise a stand-in "llama-server" (a Python script that records its argv and
    /// serves /health after a short delay) to prove spawn → ready → kill works.
    // The stand-in server is a `#!` script, which Windows cannot launch; Linux CI covers this path.
    #[cfg(unix)]
    #[tokio::test]
    async fn supervises_a_child_process() {
        if std::process::Command::new("python3").arg("--version").output().is_err() {
            eprintln!("python3 missing; skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("maru-llm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-llama-server");
        std::fs::write(
            &script,
            r#"#!/usr/bin/env python3
import sys, time, http.server, json
args = sys.argv[1:]
open(sys.argv[0] + ".args", "w").write(" ".join(args))
port = int(args[args.index("--port") + 1])
time.sleep(0.4)
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'{"status":"ok"}')
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let model = dir.join("model.gguf");
        std::fs::write(&model, b"x").unwrap();

        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let c = LlmConfig { port, model_path: Some(model), server_path: Some(script.clone()), ..Default::default() };
        let mut server = LlamaServer::start(&c, &Paths::under(&dir), Duration::from_secs(10)).await.unwrap();
        assert!(server.is_running());
        assert!(server.client().health().await);
        let args = std::fs::read_to_string(dir.join("fake-llama-server.args")).unwrap();
        assert!(args.contains(&format!("--port {port}")) && args.contains("--reasoning off"), "{args}");

        drop(server); // kill_on_drop
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!LlmClient::new(&format!("http://127.0.0.1:{port}"), &c).health().await, "process should be gone");
        std::fs::remove_dir_all(dir).ok();
    }

    // The stand-in server is a `#!` script, which Windows cannot launch; Linux CI covers this path.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_server_that_dies_on_start_reports_its_output() {
        if std::process::Command::new("python3").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("maru-llm-test-dead-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("dying-llama-server");
        std::fs::write(&script, "#!/usr/bin/env python3\nimport sys\nprint('error: unsupported model architecture', file=sys.stderr)\nsys.exit(3)\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let model = dir.join("model.gguf");
        std::fs::write(&model, b"x").unwrap();
        let c = LlmConfig { port: 1, model_path: Some(model), server_path: Some(script), ..Default::default() };
        let Err(e) = LlamaServer::start(&c, &Paths::under(&dir), Duration::from_secs(5)).await else { panic!("expected failure") };
        let msg = e.to_string();
        assert!(msg.contains("exited") && msg.contains("unsupported model architecture"), "{msg}");
        std::fs::remove_dir_all(dir).ok();
    }
}
