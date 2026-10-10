//! Editing `config.toml` from the settings page.
//!
//! The page sends only the settings the user changed, as `{"ui.font_size_px": 20, "hotkeys.scan":
//! "F7"}`. [`patch`] applies them to the file's text with `toml_edit`, so comments, ordering and
//! everything the page does not know about survive; the result is parsed as a [`Config`] and checked
//! with [`validate`] before anything is written. A `null` removes the key, which puts the default
//! back in charge.

use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item, Table};

use crate::config::Config;
use crate::error::{Error, Result};

/// The text of the new file and the settings whose values differ from before.
#[derive(Debug, PartialEq)]
pub struct Patched {
    pub text: String,
    /// The new text, parsed.
    pub config: Config,
    pub changed: Vec<String>,
}

/// Apply `changes` (dotted path → JSON value) to the config file's `text`.
pub fn patch(text: &str, changes: &Map<String, Value>) -> Result<Patched> {
    let before: Config = toml::from_str(text).map_err(|e| Error::Config(e.to_string()))?;
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| Error::Config(e.to_string()))?;
    for (path, value) in changes {
        set(&mut doc, path, value)?;
    }
    let text = doc.to_string();
    let after: Config = toml::from_str(&text).map_err(|e| Error::Config(format!("that combination is not valid: {e}")))?;
    validate(&after)?;
    let changed = changed_paths(&before, &after);
    Ok(Patched { text, config: after, changed })
}

fn set(doc: &mut DocumentMut, path: &str, value: &Value) -> Result<()> {
    let parts: Vec<&str> = path.split('.').collect();
    let (Some((leaf, parents)), false) = (parts.split_last(), path.is_empty()) else {
        return Err(Error::Config("empty setting name".into()));
    };
    let mut table: &mut Table = doc.as_table_mut();
    for part in parents {
        if !table.contains_key(part) {
            if value.is_null() {
                return Ok(()); // nothing to remove
            }
            let mut fresh = Table::new();
            fresh.set_implicit(true);
            table.insert(part, Item::Table(fresh));
        }
        table = table
            .get_mut(part)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| Error::Config(format!("{path}: {part} is not a section")))?;
    }
    if value.is_null() {
        table.remove(leaf);
        return Ok(());
    }
    let mut new = to_toml(value).ok_or_else(|| Error::Config(format!("{path}: unsupported value {value}")))?;
    // Replace in place: the key keeps the comments above it, and the value the one on its line.
    match table.get_mut(leaf) {
        Some(item) => {
            if let Some(old) = item.as_value() {
                *new.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(new);
        }
        None => {
            table.insert(leaf, Item::Value(new));
        }
    }
    Ok(())
}

fn to_toml(value: &Value) -> Option<toml_edit::Value> {
    Some(match value {
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64()?.into(),
        },
        Value::String(s) => s.as_str().into(),
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(to_toml(item)?);
            }
            array.into()
        }
        Value::Null | Value::Object(_) => return None,
    })
}

/// The config as JSON, for the settings page.
pub fn snapshot(cfg: &Config) -> Value {
    serde_json::to_value(cfg).unwrap_or(Value::Null)
}

/// Dotted paths of every setting whose value differs between `old` and `new`.
pub fn changed_paths(old: &Config, new: &Config) -> Vec<String> {
    let mut out = Vec::new();
    diff("", &snapshot(old), &snapshot(new), &mut out);
    out
}

fn diff(prefix: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<_> = x.keys().chain(y.keys()).collect();
            for key in keys {
                let path = if prefix.is_empty() { key.to_string() } else { format!("{prefix}.{key}") };
                diff(&path, x.get(key).unwrap_or(&Value::Null), y.get(key).unwrap_or(&Value::Null), out);
            }
        }
        _ if a != b => out.push(prefix.to_string()),
        _ => {}
    }
}

/// Settings the running app picks up at once. Everything else is read when Maru starts.
pub fn applies_live(path: &str) -> bool {
    path.starts_with("hotkeys.") || path.starts_with("ui.")
}

/// Range checks for what the page lets people type. Every problem is reported, not only the first.
pub fn validate(cfg: &Config) -> Result<()> {
    let mut problems = Vec::new();
    let mut check = |ok: bool, what: &str| {
        if !ok {
            problems.push(what.to_string());
        }
    };
    check((1024..=65535).contains(&cfg.llm.port), "model server port must be between 1024 and 65535");
    check((1..=8).contains(&cfg.llm.parallel), "model slots must be between 1 and 8");
    check(cfg.llm.ctx_size >= 512, "context size must be at least 512");
    check(cfg.llm.temperature >= 0.0 && cfg.llm.temperature <= 2.0, "temperature must be between 0 and 2");
    check((1..=1024).contains(&cfg.lite_profile.parallel), "lite model slots must be at least 1");
    check(cfg.lite_profile.ctx_size >= 512, "lite context size must be at least 512");
    check((0.0..=1.0).contains(&cfg.ocr.min_confidence), "OCR confidence must be between 0 and 1");
    check((8..=64).contains(&cfg.ui.font_size_px), "font size must be between 8 and 64");
    check((240..=2400).contains(&cfg.ui.max_width_px), "panel width must be between 240 and 2400");
    check(cfg.learning.desired_retention > 0.5 && cfg.learning.desired_retention < 1.0, "desired retention must be between 0.5 and 1");
    check(cfg.learning.known_stability_days >= 1.0, "a known word needs at least 1 day of stability");
    check(cfg.continuous.interval_ms >= 50, "live mode must look at least every 50 ms");
    check((0.0..=1.0).contains(&cfg.continuous.diff_threshold), "change threshold must be between 0 and 1");
    check(cfg.continuous.linger_ms <= 10_000, "a live translation can stay at most 10 seconds after its line is gone");
    check(cfg.continuous.context_lines <= 20, "live mode can show the model at most 20 earlier lines");
    check((1024..=65535).contains(&cfg.tts.port), "speech server port must be between 1024 and 65535");
    check(cfg.tts.speed >= 0.25 && cfg.tts.speed <= 4.0, "speech speed must be between 0.25 and 4");
    check(cfg.tts.timeout_secs >= 5, "speech timeout must be at least 5 seconds");
    check(cfg.tts.cache_mb <= 20_000, "speech cache is limited to 20 GB");
    check(
        cfg.tts.provider != crate::config::TtsProvider::Endpoint || !cfg.tts.enabled || cfg.tts.endpoint.as_deref().is_some_and(|e| e.trim().starts_with("http")),
        "speech from a server needs its address (starting with http:// or https://)",
    );
    check(!cfg.learning.anki.enabled || cfg.learning.anki.url.trim().starts_with("http"), "the AnkiConnect address must start with http://");
    check(!cfg.learning.anki.enabled || !cfg.learning.anki.deck.trim().is_empty(), "choose a default Anki deck");
    check(cfg.learning.anki.refresh_minutes <= 1440, "reading progress back from Anki is limited to once a day at most");
    check(cfg.capture.max_chars >= 20, "the longest selection must allow at least 20 characters");
    if problems.is_empty() { Ok(()) } else { Err(Error::Config(problems.join("; "))) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn changes(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    const FILE: &str = r#"# my notes
lite = false

[hotkeys]
# keep my favourite
translate = "Ctrl+Alt+T"   # the main one
scan = "Ctrl+Alt+S"

[ui]
font_size_px = 18
"#;

    #[test]
    fn edits_values_and_keeps_comments_and_neighbours() {
        let p = patch(FILE, &changes(json!({"hotkeys.translate": "Ctrl+Alt+T | F6", "ui.font_size_px": 22}))).unwrap();
        assert!(p.text.contains("# my notes") && p.text.contains("# keep my favourite"), "{}", p.text);
        assert!(p.text.contains(r#"translate = "Ctrl+Alt+T | F6"   # the main one"#), "same-line comment kept:\n{}", p.text);
        assert!(p.text.contains(r#"scan = "Ctrl+Alt+S""#));
        let cfg: Config = toml::from_str(&p.text).unwrap();
        assert_eq!((cfg.ui.font_size_px, cfg.hotkeys.translate.as_str()), (22, "Ctrl+Alt+T | F6"));
        assert_eq!(p.changed, ["hotkeys.translate", "ui.font_size_px"]);
    }

    #[test]
    fn creates_sections_and_removes_keys_with_null() {
        let p = patch(FILE, &changes(json!({"llm.ctx_size": 8192, "ocr.vision_reader": false, "ui.font_size_px": null}))).unwrap();
        let cfg: Config = toml::from_str(&p.text).unwrap();
        assert_eq!(cfg.llm.ctx_size, 8192);
        assert!(!cfg.ocr.vision_reader);
        assert_eq!(cfg.ui.font_size_px, Config::default().ui.font_size_px, "null puts the default back");
        assert!(!p.text.contains("font_size_px"));
        // removing something that was never there is fine
        assert!(patch("", &changes(json!({"llm.port": null}))).unwrap().changed.is_empty());
    }

    #[test]
    fn a_model_pinned_by_the_user_can_be_set_and_cleared() {
        let set = patch("", &changes(json!({"llm.model_path": "D:/m/x.gguf", "llm.extra_args": ["--no-mmap"]}))).unwrap();
        let cfg: Config = toml::from_str(&set.text).unwrap();
        assert_eq!(cfg.llm.model_path.as_deref(), Some(std::path::Path::new("D:/m/x.gguf")));
        assert_eq!(cfg.llm.extra_args, ["--no-mmap"]);
        let cleared = patch(&set.text, &changes(json!({"llm.model_path": null}))).unwrap();
        assert_eq!(toml::from_str::<Config>(&cleared.text).unwrap().llm.model_path, None);
    }

    #[test]
    fn rejects_wrong_types_unknown_shapes_and_out_of_range_values_without_changing_anything() {
        for (bad, why) in [
            (json!({"ui.font_size_px": "big"}), "wrong type"),
            (json!({"ui.font_size_px": 200}), "range"),
            (json!({"llm.port": 80}), "port"),
            (json!({"ui.placement": "sideways"}), "unknown enum value"),
            (json!({"ui.font_size_px": {"a": 1}}), "object"),
            (json!({"hotkeys.translate.x": "F6"}), "not a section"),
        ] {
            assert!(patch(FILE, &changes(bad)).is_err(), "{why} should be refused");
        }
        assert!(patch("not = [valid", &changes(json!({}))).is_err(), "a broken file is reported, not overwritten");
        let msg = patch(FILE, &changes(json!({"ui.font_size_px": 200, "llm.port": 80}))).unwrap_err().to_string();
        assert!(msg.contains("font size") && msg.contains("port"), "every problem is listed: {msg}");
    }

    #[test]
    fn only_appearance_and_shortcuts_apply_without_a_restart() {
        for live in ["hotkeys.scan", "ui.theme", "ui.max_width_px"] {
            assert!(applies_live(live), "{live}");
        }
        for restart in ["llm.hf_repo", "lite", "ocr.vision_reader", "learning.gloss_mode", "continuous.interval_ms", "capture.use_ocr_fallback"] {
            assert!(!applies_live(restart), "{restart}");
        }
    }

    #[test]
    fn the_example_config_passes_validation() {
        let cfg: Config = toml::from_str(include_str!("../../../docs/config.example.toml")).unwrap();
        validate(&cfg).unwrap();
        validate(&Config::default()).unwrap();
    }
}
