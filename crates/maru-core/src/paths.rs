use std::path::{Path, PathBuf};

use directories::ProjectDirs;

/// Every on-disk location Maru uses, resolved once.
///
/// `MARU_HOME` overrides everything (handy for portable installs and tests):
/// the layout is then `$MARU_HOME/{config.toml,data,models,bin}`.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub models_dir: PathBuf,
    pub bin_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Self {
        if let Some(home) = std::env::var_os("MARU_HOME") {
            return Self::under(Path::new(&home));
        }
        match ProjectDirs::from("dev", "maru", "Maru") {
            Some(dirs) => Self {
                config_file: dirs.config_dir().join("config.toml"),
                data_dir: dirs.data_dir().to_path_buf(),
                models_dir: dirs.data_dir().join("models"),
                bin_dir: dirs.data_dir().join("bin"),
            },
            None => Self::under(Path::new(".maru")),
        }
    }

    pub fn under(root: &Path) -> Self {
        Self {
            config_file: root.join("config.toml"),
            data_dir: root.join("data"),
            models_dir: root.join("models"),
            bin_dir: root.join("bin"),
        }
    }

    pub fn vocab_db(&self) -> PathBuf {
        self.data_dir.join("vocab.db")
    }
    pub fn dict_db(&self) -> PathBuf {
        self.data_dir.join("dict.db")
    }
    pub fn cache_db(&self) -> PathBuf {
        self.data_dir.join("cache.db")
    }
    /// Remembered screen-scan labels (see `maru-engine`'s `labels` module).
    pub fn labels_db(&self) -> PathBuf {
        self.data_dir.join("labels.db")
    }
    /// The text-to-speech models (`maru setup tts`).
    pub fn tts_models_dir(&self) -> PathBuf {
        self.models_dir.join("tts")
    }
    /// Spoken sentences, kept so hearing one again is free.
    pub fn tts_cache_dir(&self) -> PathBuf {
        self.data_dir.join("tts-cache")
    }
    pub fn screenshots_dir(&self) -> PathBuf {
        self.data_dir.join("screenshots")
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [&self.data_dir, &self.models_dir, &self.bin_dir] {
            std::fs::create_dir_all(dir)?;
        }
        if let Some(parent) = self.config_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}
