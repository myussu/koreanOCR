//! The local speech server: qwentts.cpp's `tts-server` running Qwen3-TTS.
//!
//! Maru starts it like it starts `llama-server`: tied to this process so it cannot outlive a crash,
//! one fixed port, and ready when `/health` answers.

use std::path::PathBuf;
use std::time::Duration;

use maru_core::Paths;
use maru_core::config::TtsConfig;
use maru_llm::server::{ChildServer, PORT_GRACE, port_frees_up};

use crate::client::TtsClient;
use crate::{Result, TtsError};

/// The model `maru setup tts` downloads: the 1.7B "customvoice" model, which has the named speakers
/// (including the Korean `sohee`), quantised to 8 bits (about 2 GB).
pub const REPO: &str = "Serveurperso/Qwen3-TTS-GGUF";
pub const TALKER_FILE: &str = "qwen-talker-1.7b-customvoice-Q8_0.gguf";
pub const CODEC_FILE: &str = "qwen-tokenizer-12hz-Q8_0.gguf";

pub fn server_binary(cfg: &TtsConfig, paths: &Paths) -> PathBuf {
    cfg.server_path.clone().unwrap_or_else(|| paths.bin_dir.join(if cfg!(windows) { "tts-server.exe" } else { "tts-server" }))
}

pub fn talker_path(cfg: &TtsConfig, paths: &Paths) -> PathBuf {
    cfg.model_path.clone().unwrap_or_else(|| paths.tts_models_dir().join(TALKER_FILE))
}

pub fn codec_path(cfg: &TtsConfig, paths: &Paths) -> PathBuf {
    cfg.codec_path.clone().unwrap_or_else(|| paths.tts_models_dir().join(CODEC_FILE))
}

/// Everything the local server needs is on disk.
pub fn installed(cfg: &TtsConfig, paths: &Paths) -> bool {
    server_binary(cfg, paths).is_file() && talker_path(cfg, paths).is_file() && codec_path(cfg, paths).is_file()
}

/// What is missing, in words (empty when [`installed`]).
pub fn missing(cfg: &TtsConfig, paths: &Paths) -> Vec<String> {
    [("speech server", server_binary(cfg, paths)), ("speech model", talker_path(cfg, paths)), ("speech codec", codec_path(cfg, paths))]
        .into_iter()
        .filter(|(_, p)| !p.is_file())
        .map(|(what, p)| format!("{what}: {}", p.display()))
        .collect()
}

pub fn build_args(cfg: &TtsConfig, paths: &Paths) -> Vec<String> {
    let language = if cfg.language.trim().is_empty() { "Korean" } else { cfg.language.trim() };
    vec![
        "--model".into(),
        talker_path(cfg, paths).display().to_string(),
        "--codec".into(),
        codec_path(cfg, paths).display().to_string(),
        "--alias".into(),
        cfg.model.clone(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        cfg.port.to_string(),
        "--lang".into(),
        language.to_string(),
    ]
}

/// `GGML_BACKEND` picks the device; unset lets the server choose the best one.
pub fn environment(cfg: &TtsConfig) -> Vec<(String, String)> {
    match cfg.device.trim() {
        "" | "auto" => Vec::new(),
        device => vec![("GGML_BACKEND".into(), device.to_string())],
    }
}

/// A running local speech server. Ended when dropped (and by the OS if Maru dies).
pub struct LocalTts {
    process: ChildServer,
}

impl LocalTts {
    pub async fn start(cfg: &TtsConfig, paths: &Paths) -> Result<Self> {
        let missing = missing(cfg, paths);
        if !missing.is_empty() {
            return Err(TtsError::NotConfigured(format!("not installed (run `maru setup tts`): {}", missing.join("; "))));
        }
        let binary = server_binary(cfg, paths);
        let args = build_args(cfg, paths);
        tracing::info!("starting {} {}", binary.display(), args.join(" "));
        let client = TtsClient::new(&format!("http://127.0.0.1:{}", cfg.port), cfg);
        if !port_frees_up(PORT_GRACE, || async { client.check().await.is_ok() }).await {
            return Err(TtsError::Spawn(format!(
                "port {} is already answered by another server (probably a tts-server left over from an earlier run). \
                 End it in Task Manager, or run `taskkill /F /IM tts-server.exe`, then try again, or change [tts] port.",
                cfg.port
            )));
        }
        let mut process = ChildServer::spawn("tts-server", &binary, &args, &environment(cfg)).map_err(|e| TtsError::Spawn(e.to_string()))?;
        process
            .wait_ready(Duration::from_secs(cfg.ready_timeout_secs.max(10)), || async { client.check().await.is_ok() })
            .await
            .map_err(|e| TtsError::Spawn(e.to_string()))?;
        Ok(Self { process })
    }

    pub fn is_running(&mut self) -> bool {
        self.process.is_running()
    }

    pub fn log_tail(&self) -> String {
        self.process.log_tail()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_line_has_the_flags_the_server_documents() {
        let paths = Paths::under(std::path::Path::new("/m"));
        let args = build_args(&TtsConfig::default(), &paths);
        let pair = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].as_str());
        assert!(pair("--model").unwrap().ends_with(TALKER_FILE) && pair("--codec").unwrap().ends_with(CODEC_FILE));
        assert_eq!((pair("--host"), pair("--port"), pair("--lang")), (Some("127.0.0.1"), Some("18095"), Some("Korean")));
        assert_eq!(pair("--alias"), Some("qwen3-tts"));
    }

    #[test]
    fn the_device_is_only_forced_when_asked() {
        assert!(environment(&TtsConfig::default()).is_empty());
        let cfg = TtsConfig { device: "Vulkan0".into(), ..Default::default() };
        assert_eq!(environment(&cfg), [("GGML_BACKEND".to_string(), "Vulkan0".to_string())]);
    }

    #[tokio::test]
    async fn a_missing_install_says_what_to_run() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        assert!(!installed(&TtsConfig::default(), &paths));
        let Err(err) = LocalTts::start(&TtsConfig::default(), &paths).await else { panic!("must not start") };
        let msg = err.to_string();
        assert!(msg.contains("maru setup tts") && msg.contains("tts-server"), "{msg}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn starts_a_server_and_ends_it_when_dropped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(paths.tts_models_dir()).unwrap();
        std::fs::create_dir_all(&paths.bin_dir).unwrap();
        std::fs::write(talker_path(&TtsConfig::default(), &paths), b"x").unwrap();
        std::fs::write(codec_path(&TtsConfig::default(), &paths), b"x").unwrap();
        // a stand-in server: answers /health on the configured port with python's http.server
        let script = "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = \"--port\" ] && port=$2; shift; done\nmkdir -p /tmp/maru-tts-test-$port && cd /tmp/maru-tts-test-$port && echo ok > health && exec python3 -m http.server $port --bind 127.0.0.1\n";
        let bin = server_binary(&TtsConfig::default(), &paths);
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let cfg = TtsConfig { port, ready_timeout_secs: 20, ..Default::default() };
        let mut server = LocalTts::start(&cfg, &paths).await.expect("starts");
        assert!(server.is_running());
        let client = TtsClient::new(&format!("http://127.0.0.1:{port}"), &cfg);
        assert!(client.check().await.is_ok());
        drop(server);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(client.check().await.is_err(), "the server ends with its handle");
    }
}
