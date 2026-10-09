//! Spawns and supervises `llama-server`.
//!
//! Flags were checked against the current llama.cpp server documentation:
//! `-ngl all`, `-fa on`, `--cache-reuse`, `--reasoning off`, `-np`, and the
//! speculative-decoding pair `-md` / `--spec-draft-n-max`. The port is always
//! passed explicitly because llama-server's default has changed between releases.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use maru_core::Paths;
use maru_core::config::LlmConfig;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::client::LlmClient;
use crate::{LlmError, Result};

/// The command line for `llama-server` (without the executable).
pub fn build_args(cfg: &LlmConfig, model: &Path) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-m".into(),
        model.display().to_string(),
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
    ];
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
        let model = cfg
            .model_path
            .clone()
            .ok_or_else(|| LlmError::Spawn("no model configured: set [llm] model_path in config.toml".into()))?;
        if !model.exists() {
            return Err(LlmError::Spawn(format!("model file not found: {}", model.display())));
        }
        let binary = resolve_binary(cfg, paths);
        let args = build_args(cfg, &model);
        tracing::info!("starting {} {}", binary.display(), args.join(" "));

        let mut cmd = Command::new(&binary);
        cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd
            .spawn()
            .map_err(|e| LlmError::Spawn(format!("{}: {e} (set [llm] server_path or put llama-server on PATH)", binary.display())))?;

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
    use super::*;

    fn cfg() -> LlmConfig {
        LlmConfig { port: 19001, ctx_size: 4096, parallel: 2, ..Default::default() }
    }

    fn pair<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
    }

    #[test]
    fn args_cover_gpu_cache_and_slots() {
        let a = build_args(&cfg(), Path::new("/m/model.gguf"));
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
    fn draft_model_enables_speculative_decoding() {
        let mut c = cfg();
        c.draft_model_path = Some("/m/draft.gguf".into());
        c.extra_args = vec!["--no-warmup".into()];
        let a = build_args(&c, Path::new("m.gguf"));
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
        assert!(e.to_string().contains("model_path"), "{e}");
        let mut c = cfg();
        c.model_path = Some("/nope.gguf".into());
        let Err(e) = LlamaServer::start(&c, &paths, Duration::from_secs(1)).await else { panic!("expected an error") };
        assert!(e.to_string().contains("not found"), "{e}");
    }

    /// Supervise a stand-in "llama-server" (a Python script that records its argv and
    /// serves /health after a short delay) to prove spawn → ready → kill works.
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
