//! Optional local translation models for the live-subtitle lane.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveModel {
    #[default]
    Main,
    HyMt2,
    Milmmt,
    HyMt2Small,
    Lfm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LiveDevice {
    #[default]
    Gpu,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveTranslationConfig {
    pub model: LiveModel,
    pub hy_mt2_path: Option<PathBuf>,
    pub milmmt_path: Option<PathBuf>,
    pub hy_mt2_small_path: Option<PathBuf>,
    pub lfm_path: Option<PathBuf>,
    pub device: LiveDevice,
    /// Off uses the tested native single-source prompt. Earlier answers are experimental.
    pub dialogue_context: bool,
    pub cpu_threads: u32,
    /// Optional quality model for snapshots while live subtitles are active.
    pub snapshot_endpoint: Option<String>,
    pub snapshot_model_name: String,
}

impl Default for LiveTranslationConfig {
    fn default() -> Self {
        Self {
            model: LiveModel::Main,
            hy_mt2_path: None,
            milmmt_path: None,
            hy_mt2_small_path: None,
            lfm_path: None,
            device: LiveDevice::Gpu,
            dialogue_context: false,
            cpu_threads: 4,
            snapshot_endpoint: None,
            snapshot_model_name: "maru".into(),
        }
    }
}

impl LiveTranslationConfig {
    pub fn selected_path(&self) -> Option<&std::path::Path> {
        match self.model {
            LiveModel::Main => None,
            LiveModel::HyMt2 => self.hy_mt2_path.as_deref(),
            LiveModel::Milmmt => self.milmmt_path.as_deref(),
            LiveModel::HyMt2Small => self.hy_mt2_small_path.as_deref(),
            LiveModel::Lfm => self.lfm_path.as_deref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_configs_keep_the_main_model_and_new_settings_round_trip() {
        let old: crate::Config = toml::from_str("[continuous]\ncontext_lines=3").unwrap();
        assert_eq!(old.continuous.translation, LiveTranslationConfig::default());
        let cfg: crate::Config = toml::from_str("[continuous.translation]\nmodel='hy_mt2'\nhy_mt2_path='D:/model.gguf'\ndevice='cpu'\nsnapshot_endpoint='http://nas:11434'\nsnapshot_model_name='gemma'\n").unwrap();
        assert_eq!(cfg.continuous.translation.model, LiveModel::HyMt2);
        assert_eq!(cfg.continuous.translation.device, LiveDevice::Cpu);
        assert_eq!(
            cfg.continuous.translation.selected_path(),
            Some(std::path::Path::new("D:/model.gguf"))
        );
        assert_eq!(
            toml::from_str::<crate::Config>(&toml::to_string(&cfg).unwrap()).unwrap(),
            cfg
        );
    }

    #[test]
    fn small_model_paths_round_trip_without_replacing_the_larger_model_paths() {
        for (name, expected, path) in [
            ("hy_mt2_small", LiveModel::HyMt2Small, "D:/small-hy.gguf"),
            ("lfm", LiveModel::Lfm, "D:/lfm.gguf"),
        ] {
            let cfg: crate::Config = toml::from_str(&format!("[continuous.translation]\nmodel='{name}'\nhy_mt2_path='D:/large-hy.gguf'\nmilmmt_path='D:/mi.gguf'\nhy_mt2_small_path='D:/small-hy.gguf'\nlfm_path='D:/lfm.gguf'\n")).unwrap();
            assert_eq!(cfg.continuous.translation.model, expected);
            assert_eq!(
                cfg.continuous.translation.selected_path(),
                Some(std::path::Path::new(path))
            );
            let read: crate::Config = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
            assert_eq!(read, cfg);
            assert_eq!(
                read.continuous.translation.hy_mt2_path,
                Some("D:/large-hy.gguf".into())
            );
        }
    }
}
