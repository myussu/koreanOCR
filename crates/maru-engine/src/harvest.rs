//! "Save what you read": keep the exact picture the live-subtitle watcher gave the reader, with what it read, so a person can
//! later write down what the subtitle really says and score OCR changes on their own games (`maru eval-ocr`).
//!
//! Layout, under the folder the engine is given (`<data>/evalset/raw`):
//!
//! ```text
//! <game>/<unix seconds>-<n>.png    the picture as the reader saw it (not enlarged)
//! <game>/<unix seconds>-<n>.json   the reader's draft, the program, window title, confidence, timings
//! ```
//!
//! The draft is the reader's own output, so it is a starting point for a label and never the answer. Nothing leaves the machine,
//! and nothing is kept unless `[continuous] save_reads` is on.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use image::RgbImage;
use maru_core::AppInfo;
use serde::Serialize;

/// What goes beside a saved picture.
#[derive(Debug, Serialize)]
struct Sidecar<'a> {
    /// What the reader made of the picture.
    draft: &'a str,
    /// The program in front (executable name without `.exe`).
    game: &'a str,
    window_title: &'a str,
    saved_at: u64,
    confidence: Option<f32>,
    width: u32,
    height: u32,
    /// From the line appearing until it was read, and what the reading itself took.
    ready_ms: u32,
    ocr_ms: u32,
}

/// Saves read lines under one folder.
pub struct Harvest {
    root: PathBuf,
    next: AtomicU32,
}

/// The facts about one read line that are saved with its picture.
pub struct Read<'a> {
    pub draft: &'a str,
    pub app: Option<&'a AppInfo>,
    pub confidence: Option<f32>,
    pub ready_ms: u32,
    pub ocr_ms: u32,
}

impl Harvest {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            next: AtomicU32::new(1),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write the picture and its sidecar; returns the picture's path.
    pub fn save(&self, frame: &RgbImage, read: &Read<'_>) -> io::Result<PathBuf> {
        let game = game_folder(read.app);
        let dir = self.root.join(&game);
        std::fs::create_dir_all(&dir)?;
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // A name nobody has used: the counter restarts with each session, so two sessions in the same second could meet.
        let (png, json) = loop {
            let name = format!("{secs}-{:03}", self.next.fetch_add(1, Ordering::Relaxed));
            let (png, json) = (
                dir.join(format!("{name}.png")),
                dir.join(format!("{name}.json")),
            );
            if !png.exists() {
                break (png, json);
            }
        };
        frame.save(&png).map_err(io::Error::other)?;
        let side = Sidecar {
            draft: read.draft,
            game: &game,
            window_title: read.app.map_or("", |a| a.title.as_str()),
            saved_at: secs,
            confidence: read.confidence,
            width: frame.width(),
            height: frame.height(),
            ready_ms: read.ready_ms,
            ocr_ms: read.ocr_ms,
        };
        let text = serde_json::to_string_pretty(&side).map_err(io::Error::other)?;
        std::fs::write(&json, text)?;
        Ok(png)
    }
}

/// A folder name for the program in front: its executable without `.exe`, lower-case, safe in a path.
pub fn game_folder(app: Option<&AppInfo>) -> String {
    let exe = app.map_or("", |a| a.exe.as_str()).to_lowercase();
    let exe = exe.strip_suffix(".exe").unwrap_or(&exe);
    let clean: String = exe
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.trim_matches('_').is_empty() {
        "unknown".into()
    } else {
        clean
    }
}

#[cfg(test)]
mod tests {
    use image::Rgb;

    use super::*;

    fn app(exe: &str) -> AppInfo {
        AppInfo {
            exe: exe.into(),
            title: "Xenoblade Chronicles 2".into(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("maru-harvest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_program_becomes_a_safe_folder_name() {
        assert_eq!(game_folder(Some(&app("Yuzu.exe"))), "yuzu");
        assert_eq!(
            game_folder(Some(&app("..\\..\\evil game.exe"))),
            "______evil_game"
        );
        assert_eq!(game_folder(Some(&app(""))), "unknown");
        assert_eq!(game_folder(Some(&app("...exe"))), "unknown");
        assert_eq!(game_folder(None), "unknown");
    }

    #[test]
    fn a_read_line_is_saved_as_a_picture_with_its_draft_beside_it() {
        let root = scratch("save");
        let harvest = Harvest::new(root.clone());
        let frame = RgbImage::from_pixel(40, 12, Rgb([200, 10, 10]));
        let a = app("yuzu.exe");
        let read = Read {
            draft: "안녕하세요",
            app: Some(&a),
            confidence: Some(0.93),
            ready_ms: 410,
            ocr_ms: 120,
        };
        let png = harvest.save(&frame, &read).unwrap();

        assert_eq!(png.parent().unwrap(), root.join("yuzu"));
        let back = image::open(&png).unwrap().to_rgb8();
        assert_eq!(
            (back.width(), back.height()),
            (40, 12),
            "the picture is saved as it was, not enlarged"
        );
        let side: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap())
                .unwrap();
        assert_eq!(side["draft"], "안녕하세요");
        assert_eq!(side["game"], "yuzu");
        assert_eq!(side["window_title"], "Xenoblade Chronicles 2");
        assert_eq!(side["ready_ms"], 410);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_lines_saved_in_the_same_second_get_different_names() {
        let root = scratch("names");
        let harvest = Harvest::new(root.clone());
        let frame = RgbImage::new(8, 8);
        let read = Read {
            draft: "가",
            app: None,
            confidence: None,
            ready_ms: 0,
            ocr_ms: 0,
        };
        let first = harvest.save(&frame, &read).unwrap();
        let second = harvest.save(&frame, &read).unwrap();
        assert_ne!(first, second);
        // a second session starts its counter again but must not overwrite the first one's pictures
        let again = Harvest::new(root.clone()).save(&frame, &read).unwrap();
        assert!(again != first && again != second);
        assert_eq!(first.parent().unwrap(), root.join("unknown"));
        let _ = std::fs::remove_dir_all(root);
    }
}
