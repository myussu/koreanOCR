//! Local model discovery and configuration; no downloads or installed settings changes.

use maru_core::{
    Config, Paths,
    config::LlmConfig,
    live_model::{LiveDevice, LiveModel},
};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct ModelFile {
    pub model: &'static str,
    pub path: Option<PathBuf>,
}

pub fn validate_file(path: &Path) -> Result<(), String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if &magic != b"GGUF" {
        return Err("Choose a GGUF model file, not a folder or download receipt.".into());
    }
    Ok(())
}

pub fn available(paths: &Paths) -> Vec<ModelFile> {
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/live-diagnostics/benchmark-expanded-2026-10-12/models");
    let small = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/live-diagnostics/low-vram-2026-10-12/models");
    [
        ("hy_mt2", "Hy-MT2-7B-Q4_K_M.gguf", development.join("mt2-7-q4")),
        ("milmmt", "MiLMMT-46-4B-v1.0.Q6_K.gguf", development.join("milmmt4-q6")),
        ("hy_mt2_small", "Hy-MT2-1.8B-Q4_K_M.gguf", small.join("mt2-q4")),
        ("lfm", "lfm2-1.2b-koen-mt-v8-rl-10k-merged-Q5_K_M.gguf", small.join("lfm2-v8")),
    ]
    .into_iter()
    .map(|(model, name, folder)| {
        let path = [paths.models_dir.join("live").join(name), folder.join(name)]
            .into_iter()
            .find(|p| validate_file(p).is_ok());
        ModelFile { model, path }
    })
    .collect()
}

pub(crate) fn local_config(cfg: &Config) -> Result<LlmConfig, String> {
    let live = &cfg.continuous.translation;
    // A broken live path must not disable the independently configured main model.
    let path = live.selected_path().map(Path::to_path_buf);
    let mut local = LlmConfig {
        model_path: path,
        hf_repo: None,
        endpoint: None,
        server_path: cfg.llm.server_path.clone(),
        port: if cfg.llm.endpoint.is_some() { 18096 } else { cfg.llm.port },
        ctx_size: 4096,
        parallel: 2,
        flash_attn: true,
        model_name: match live.model {
            LiveModel::HyMt2 => "hy-mt2",
            LiveModel::HyMt2Small => "hy-mt2-small",
            LiveModel::Lfm => "lfm",
            _ => "milmmt",
        }
        .into(),
        ready_timeout_secs: cfg.llm.ready_timeout_secs,
        ..Default::default()
    };
    if matches!(live.model, LiveModel::HyMt2Small | LiveModel::Lfm) {
        local.ctx_size = if live.model == LiveModel::HyMt2Small { 1024 } else { 2048 };
        local.parallel = 1;
        local.extra_args = vec!["-b".into(), "128".into(), "-ub".into(), "128".into()];
    }
    if live.device == LiveDevice::Cpu {
        local.gpu_layers = "0".into();
        local.extra_args.extend([
            "-t".into(),
            live.cpu_threads.to_string(),
            "--device".into(),
            "none".into(),
            "--no-kv-offload".into(),
            "--no-op-offload".into(),
        ]);
    }
    Ok(local)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alternate_models_do_not_inherit_main_downloads_or_gpu_overrides() {
        let dir = std::env::temp_dir().join(format!("maru-live-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.gguf");
        std::fs::write(&path, b"GGUFtest").unwrap();
        let mut cfg = Config::default();
        cfg.continuous.translation.model = LiveModel::Milmmt;
        let missing = local_config(&cfg).unwrap();
        assert!(missing.model_path.is_none() && missing.hf_repo.is_none(), "a missing live path cannot trigger a download or replace main settings");
        cfg.continuous.translation.milmmt_path = Some(path.clone());
        cfg.continuous.translation.device = LiveDevice::Cpu;
        cfg.llm.extra_args = vec!["--model".into(), "wrong.gguf".into()];
        let local = local_config(&cfg).unwrap();
        assert!(local.hf_repo.is_none() && local.endpoint.is_none());
        assert_eq!(local.model_path, Some(path.clone()));
        assert_eq!(local.gpu_layers, "0");
        assert!(local.extra_args.contains(&"--no-op-offload".into()));
        assert!(!local.extra_args.contains(&"wrong.gguf".into()));
        std::fs::write(&path, b"html").unwrap();
        assert!(validate_file(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn lightweight_choices_limit_context_slots_and_buffers_without_changing_existing_models() {
        let mut cfg = Config::default();
        for (model, context, slots) in [(LiveModel::Milmmt, 4096, 2), (LiveModel::HyMt2, 4096, 2), (LiveModel::HyMt2Small, 1024, 1), (LiveModel::Lfm, 2048, 1)] {
            cfg.continuous.translation.model = model;
            let local = local_config(&cfg).unwrap();
            assert_eq!((local.ctx_size, local.parallel), (context, slots));
            assert_eq!(local.extra_args.contains(&"-ub".into()), slots == 1);
        }
        cfg.continuous.translation.device = LiveDevice::Cpu;
        let local = local_config(&cfg).unwrap();
        assert!(local.extra_args.contains(&"-ub".into()) && local.extra_args.contains(&"--no-kv-offload".into()));
    }
}
