use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::types::{Rect, Register};

/// Top-level configuration. Every section has working defaults, so an empty
/// (or missing) `config.toml` is valid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub hotkeys: Hotkeys,
    pub llm: LlmConfig,
    pub ocr: OcrConfig,
    pub learning: LearningConfig,
    pub ui: UiConfig,
    pub continuous: ContinuousConfig,
    pub capture: CaptureConfig,
    #[serde(rename = "profile")]
    pub profiles: Vec<AppProfile>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// The profile whose `exe` list matches, if any.
    pub fn profile_for(&self, exe: &str) -> Option<&AppProfile> {
        let exe = exe.to_ascii_lowercase();
        self.profiles.iter().find(|p| p.exe.iter().any(|e| e.eq_ignore_ascii_case(&exe)))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    /// Translate the current selection (falls back to OCR under the cursor).
    pub translate: String,
    /// Drag a rectangle anywhere and translate what is inside.
    pub region: String,
    /// Toggle hover mode (gloss the word under the cursor).
    pub hover: String,
    /// Toggle continuous mode on the configured region (subtitles/dialogue).
    pub continuous: String,
    /// Show/hide the history of recent lines.
    pub history: String,
    /// Dismiss the panel.
    pub dismiss: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            translate: "Ctrl+Alt+T".into(),
            region: "Ctrl+Alt+R".into(),
            hover: "Ctrl+Alt+H".into(),
            continuous: "Ctrl+Alt+C".into(),
            history: "Ctrl+Alt+L".into(),
            dismiss: "Ctrl+Alt+Escape".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// Use an already running OpenAI-compatible server instead of spawning one
    /// (LM Studio, Ollama's `/v1`, a remote box on your LAN…).
    pub endpoint: Option<String>,
    /// Spawn and supervise `llama-server` ourselves.
    pub autostart: bool,
    /// Path to `llama-server(.exe)`; defaults to `<bin_dir>/llama-server`.
    pub server_path: Option<PathBuf>,
    pub model_path: Option<PathBuf>,
    /// Load the model's vision projector (costs VRAM; only OCR experiments need it).
    pub vision: bool,
    /// Let llama-server download (and cache) a model from Hugging Face instead:
    /// `unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL`. Ignored if `model_path` is set.
    pub hf_repo: Option<String>,
    /// How long to wait for the model to finish loading (first run may also download it).
    pub ready_timeout_secs: u64,
    /// Small draft model for speculative decoding (same vocabulary as the main one).
    pub draft_model_path: Option<PathBuf>,
    pub port: u16,
    pub ctx_size: u32,
    /// Parallel slots: translation and glossing run concurrently.
    pub parallel: u32,
    /// `all`, `auto` or a number.
    pub gpu_layers: String,
    pub flash_attn: bool,
    /// Extra raw arguments appended to the `llama-server` command line.
    pub extra_args: Vec<String>,
    /// Model name sent in requests (ignored by llama-server, needed by others).
    pub model_name: String,
    pub temperature: f32,
    /// Hard cap on generated tokens per request.
    pub max_tokens: u32,
    /// How many recent lines to give the model as context.
    pub context_lines: usize,
    pub request_timeout_secs: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            autostart: true,
            server_path: None,
            model_path: None,
            vision: false,
            hf_repo: None,
            ready_timeout_secs: 180,
            draft_model_path: None,
            // llama-server's own default port changed between releases, so we
            // always pass one explicitly.
            port: 18080,
            ctx_size: 4096,
            parallel: 2,
            gpu_layers: "all".into(),
            flash_attn: true,
            extra_args: Vec::new(),
            model_name: "maru".into(),
            temperature: 0.2,
            max_tokens: 512,
            context_lines: 4,
            request_timeout_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OcrBackend {
    /// Windows built-in OCR when available, else PP-OCR.
    #[default]
    Auto,
    /// PP-OCRv5 Korean through ONNX Runtime (needs the model files).
    Paddle,
    /// `Windows.Media.Ocr` — no model download, Windows only.
    Windows,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OcrConfig {
    pub backend: OcrBackend,
    pub det_model: Option<PathBuf>,
    pub rec_model: Option<PathBuf>,
    pub dict_path: Option<PathBuf>,
    /// `cpu`, `cuda:0` or `directml:0`.
    pub device: String,
    /// Below this mean confidence the text is flagged as uncertain.
    pub min_confidence: f32,
    /// Ask a vision-language model to re-read low-confidence captures.
    pub vlm_rescue: bool,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            backend: OcrBackend::Auto,
            det_model: None,
            rec_model: None,
            dict_path: None,
            device: "cpu".into(),
            min_confidence: 0.80,
            vlm_rescue: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GlossMode {
    /// Gloss every content word.
    All,
    /// Words you know are dimmed and shortened; unknown ones get full glosses.
    #[default]
    FadeKnown,
    /// Hide glosses for known words entirely.
    HideKnown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnkiConfig {
    pub enabled: bool,
    /// AnkiConnect endpoint.
    pub url: String,
    pub deck: String,
    pub note_type: String,
}

impl Default for AnkiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: "http://127.0.0.1:8765".into(),
            deck: "Maru".into(),
            note_type: "Basic".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LearningConfig {
    pub gloss_mode: GlossMode,
    /// A word counts as "known" once its memory stability reaches this many days.
    pub known_stability_days: f32,
    /// FSRS target recall probability at review time.
    pub desired_retention: f32,
    /// Every lookup is recorded (and counted) automatically.
    pub track_lookups: bool,
    /// Add a word to the review queue the 2nd time you look it up.
    pub auto_save_after_lookups: u32,
    pub save_screenshots: bool,
    pub anki: AnkiConfig,
    /// Default politeness for English → Korean.
    pub register: Register,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            gloss_mode: GlossMode::FadeKnown,
            known_stability_days: 21.0,
            desired_retention: 0.90,
            track_lookups: true,
            auto_save_after_lookups: 2,
            save_screenshots: false,
            anki: AnkiConfig::default(),
            register: Register::Polite,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    /// Below the selection unless there is no room.
    #[default]
    Auto,
    Below,
    Above,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub placement: Placement,
    pub font_size_px: u32,
    pub show_romanization: bool,
    pub show_pronunciation: bool,
    pub show_grammar_formula: bool,
    pub opacity: f32,
    pub max_width_px: u32,
    /// Seconds before an idle panel disappears (0 = stay until dismissed).
    pub auto_hide_secs: u32,
    pub theme: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            placement: Placement::Auto,
            font_size_px: 18,
            show_romanization: true,
            show_pronunciation: true,
            show_grammar_formula: true,
            opacity: 0.96,
            max_width_px: 760,
            auto_hide_secs: 0,
            theme: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContinuousConfig {
    /// Screen region to watch (set it with the region hotkey while in continuous mode).
    pub region: Option<Rect>,
    /// How often to grab the region.
    pub interval_ms: u64,
    /// Text must stay unchanged this long before it is translated.
    pub stable_ms: u64,
    /// Fraction of sampled pixels that must change to count as a new frame (0..1).
    pub diff_threshold: f32,
    pub history_len: usize,
}

impl Default for ContinuousConfig {
    fn default() -> Self {
        Self { region: None, interval_ms: 250, stable_ms: 350, diff_threshold: 0.01, history_len: 50 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    /// Try reading the selection through the accessibility API first.
    pub use_accessibility: bool,
    /// Fall back to Ctrl+C (clipboard is saved and restored).
    pub use_clipboard_fallback: bool,
    /// Fall back to OCR of the area around the cursor.
    pub use_ocr_fallback: bool,
    pub ocr_radius_px: u32,
    pub clipboard_timeout_ms: u64,
    /// Longest selection we process; longer text is cut at a sentence boundary.
    pub max_chars: usize,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            use_accessibility: true,
            use_clipboard_fallback: true,
            use_ocr_fallback: true,
            ocr_radius_px: 320,
            clipboard_timeout_ms: 250,
            max_chars: 500,
        }
    }
}

/// Per-application behaviour: a game wants different hints than a chat app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppProfile {
    pub name: String,
    /// Executable file names this profile applies to (case-insensitive).
    pub exe: Vec<String>,
    pub register: Option<Register>,
    /// Free-text hint given to the model ("dialogue from a fantasy RPG").
    pub hint: Option<String>,
    /// Fixed translations for names/terms: `{"이도현" = "Lee Do-hyun"}`.
    pub glossary: BTreeMap<String, String>,
    pub gloss_mode: Option<GlossMode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_toml_is_valid_and_defaults() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c, Config::default());
    }

    #[test]
    fn partial_toml_overrides_only_what_it_names() {
        let c: Config = toml::from_str(
            r#"
            [llm]
            port = 9000
            [[profile]]
            name = "game"
            exe = ["Game.exe"]
            register = "casual"
            [profile.glossary]
            "이도현" = "Lee Do-hyun"
            "#,
        )
        .unwrap();
        assert_eq!(c.llm.port, 9000);
        assert_eq!(c.llm.ctx_size, LlmConfig::default().ctx_size);
        let p = c.profile_for("game.exe").expect("case-insensitive match");
        assert_eq!(p.register, Some(Register::Casual));
        assert_eq!(p.glossary["이도현"], "Lee Do-hyun");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = std::env::temp_dir().join(format!("maru-cfg-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut c = Config::default();
        c.ui.font_size_px = 22;
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The documented example must stay valid, and say what the docs claim.
    #[test]
    fn the_example_config_in_the_docs_parses() {
        let c: Config = toml::from_str(include_str!("../../../docs/config.example.toml")).unwrap();
        assert_eq!(c.hotkeys.dismiss, "Ctrl+Alt+Escape");
        assert_eq!(c.llm.hf_repo.as_deref(), Some("unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL"));
        assert_eq!(c.profile_for("mygame.exe").unwrap().glossary["이도현"], "Lee Do-hyun");
        assert_eq!(c.profiles[0].gloss_mode, Some(GlossMode::All));
    }

    #[test]
    fn missing_file_gives_defaults() {
        let c = Config::load(Path::new("/definitely/not/here/config.toml")).unwrap();
        assert_eq!(c, Config::default());
    }
}
