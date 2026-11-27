//! How each task reads the screen. `[reading]` says, for live subtitles and for captures (selection fallback, region,
//! hover; the screen scan uses the captures' line finder and letter reader), which line finder (detector), which letter
//! reader (recogniser), whether the reading model (PaddleOCR-VL) checks the letters, and which reading model.
//!
//! Run 4 measured, on the owner's labelled game subtitles, that PP-OCR alone with the PP-OCRv6 small detector reads
//! subtitles better than the hybrid and in a fifth of the time, so that is the live default; captures keep the reading
//! model, which helps with stylised and small lettering. A `[[profile]]` can override either task for one program.
//!
//! Files written before this had no `[reading]`: the older `[ocr]` keys decided for every task, and
//! `[continuous] cpu_ocr` took the reading model off live subtitles. [`derive`] reads that as a table (keeping any
//! detector, recogniser or reading model the file names, and giving everything left at its default the new defaults),
//! [`migrate`] writes it into the file once (`version = 1`) and [`resolve`] turns a task's route into the `[ocr]`
//! settings the OCR engines are built from. The older keys are never renamed and keep parsing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, OcrConfig};
use crate::error::{Error, Result};
use crate::paths::Paths;

/// The version [`migrate`] writes. 0 (no `[reading]`) means [`derive`] decides from the older keys.
pub const VERSION: u32 = 1;

/// The PP-OCRv6 small detector: the live default.
pub const SMALL_DETECTOR: &str = "pp-ocrv6-small";
/// What the detector and recogniser fields call the files `maru setup ocr` put in `models/ocr/`.
pub const STANDARD: &str = "standard";
pub const READER_15: &str = "paddleocr-vl-1.5";
pub const READER_16: &str = "paddleocr-vl-1.6";
/// The reading model captures use when the file names none. Run 4: on 73 full game screens 1.6 was not worse than 1.5
/// (15 lines better, 12 worse), and on 468 subtitles better (38 to 12).
pub const CAPTURE_READER: &str = READER_16;

/// The tasks that read the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadTask {
    Live,
    /// Captures and, for the line finder and letter reader, the screen scan.
    Capture,
}

/// How one task reads. Empty fields (and `reader = None`) follow `[ocr]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadingRoute {
    /// The line finder: `standard`, `pp-ocrv6-small`, `pp-ocrv6-medium`, `pp-ocrv5-mobile`, or a path to an `.onnx` file.
    pub detector: String,
    /// The letter reader: `standard`, `qota-ocr` (a community model), or a path to an `.onnx` file (its character list
    /// is `[ocr] dict_path`).
    pub recogniser: String,
    /// The reading model checks the letters (costs about 2 GB of video memory while loaded).
    pub reader: Option<bool>,
    /// `paddleocr-vl-1.5`, `paddleocr-vl-1.6`, or a folder with another reading model (as `[ocr] reader_dir`).
    pub reader_model: String,
}

impl ReadingRoute {
    /// `self` with every field `over` sets replacing it.
    fn overlaid(&self, over: &ReadingRoute) -> ReadingRoute {
        let pick = |a: &String, b: &String| if b.trim().is_empty() { a.clone() } else { b.clone() };
        ReadingRoute {
            detector: pick(&self.detector, &over.detector),
            recogniser: pick(&self.recogniser, &over.recogniser),
            reader: over.reader.or(self.reader),
            reader_model: pick(&self.reader_model, &over.reader_model),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadingConfig {
    pub version: u32,
    pub live: ReadingRoute,
    pub capture: ReadingRoute,
}

impl ReadingConfig {
    pub fn route(&self, task: ReadTask) -> &ReadingRoute {
        match task {
            ReadTask::Live => &self.live,
            ReadTask::Capture => &self.capture,
        }
    }
}

/// A `[[profile]]`'s override for one program: `[profile.reading.live]`, `[profile.reading.capture]`. Empty = the task's.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileReading {
    pub live: ReadingRoute,
    pub capture: ReadingRoute,
}

impl ProfileReading {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A model file Maru knows by name: where it may be (the first that exists wins) and what Settings calls it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Named {
    pub id: &'static str,
    /// For Settings: plain words, no file names.
    pub label: &'static str,
    /// Below the models folder. `maru setup` installs into the first; the second is where run 3 left the candidates.
    pub files: &'static [&'static str],
    /// The recogniser's character list, beside `files`.
    pub dict: &'static [&'static str],
}

pub const DETECTORS: [Named; 4] = [
    Named { id: STANDARD, label: "Standard", files: &["ocr/det.onnx"], dict: &[] },
    Named { id: SMALL_DETECTOR, label: "Small and quick", files: &["ocr/pp-ocrv6-small/det.onnx", "ocr-candidates/pp-ocrv6-small-det/det.onnx"], dict: &[] },
    Named { id: "pp-ocrv6-medium", label: "Medium", files: &["ocr/pp-ocrv6-medium/det.onnx", "ocr-candidates/pp-ocrv6-medium-det/det.onnx"], dict: &[] },
    Named { id: "pp-ocrv5-mobile", label: "Older small", files: &["ocr/pp-ocrv5-mobile/det.onnx", "ocr-candidates/pp-ocrv5-mobile-det/det.onnx"], dict: &[] },
];

pub const RECOGNISERS: [Named; 2] = [
    Named { id: STANDARD, label: "Standard", files: &["ocr/rec.onnx"], dict: &["ocr/dict.txt"] },
    Named { id: "qota-ocr", label: "Qota-OCR (community model)", files: &["ocr/qota-ocr/rec.onnx", "ocr-candidates/qota-ocr/rec.onnx"], dict: &["ocr/qota-ocr/dict.txt", "ocr-candidates/qota-ocr/dict.txt"] },
];

/// Folders below the models folder; their files are found by `vision::reader_in`.
pub const READERS: [Named; 2] = [
    Named { id: READER_15, label: "PaddleOCR-VL 1.5", files: &["vlm/paddleocr-vl-1.5"], dict: &[] },
    Named { id: READER_16, label: "PaddleOCR-VL 1.6", files: &["vlm/paddleocr-vl-1.6"], dict: &[] },
];

fn named<'a>(list: &'a [Named], id: &str) -> Option<&'a Named> {
    list.iter().find(|n| n.id.eq_ignore_ascii_case(id.trim()))
}

fn first_there(paths: &Paths, rel: &[&str], folder: bool) -> Option<PathBuf> {
    rel.iter().map(|r| paths.models_dir.join(r)).find(|p| if folder { has_reader(p) } else { p.metadata().is_ok_and(|m| m.is_file() && m.len() > 0) })
}

/// A folder holds a reading model: a `.gguf` with `mmproj` in its name and another `.gguf`.
fn has_reader(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    let names: Vec<String> = entries.filter_map(|e| e.ok()).filter(|e| e.metadata().is_ok_and(|m| m.len() > 0)).map(|e| e.file_name().to_string_lossy().to_lowercase()).filter(|n| n.ends_with(".gguf")).collect();
    names.iter().any(|n| n.contains("mmproj")) && names.iter().any(|n| !n.contains("mmproj"))
}

/// The table the engine uses: the file's when it has one, else what the older keys amount to, with any `[reading]`
/// key the file already names on top (a setting saved before the file was migrated).
pub fn table(cfg: &Config) -> ReadingConfig {
    if cfg.reading.version >= VERSION {
        return cfg.reading.clone();
    }
    let d = derive(cfg);
    ReadingConfig { version: 0, live: d.live.overlaid(&cfg.reading.live), capture: d.capture.overlaid(&cfg.reading.capture) }
}

/// The older keys as a table. Whatever the file names stays (a detector, a recogniser or a reading model of its own,
/// the reading model switched off); what was left at its default gets the new defaults: live subtitles read with PP-OCR
/// alone and the small detector, captures with the reading model.
pub fn derive(cfg: &Config) -> ReadingConfig {
    let o = &cfg.ocr;
    let own_detector = o.det_model.is_some();
    let own_recogniser = o.rec_model.is_some() || o.dict_path.is_some();
    let route = |detector: &str, reader: bool, reader_model: &str| ReadingRoute {
        detector: if own_detector { String::new() } else { detector.to_string() },
        recogniser: if own_recogniser { String::new() } else { STANDARD.to_string() },
        reader: Some(reader),
        reader_model: if o.reader_dir.is_some() { String::new() } else { reader_model.to_string() },
    };
    ReadingConfig {
        version: 0,
        live: route(SMALL_DETECTOR, false, CAPTURE_READER),
        capture: route(STANDARD, o.vision_reader, CAPTURE_READER),
    }
}

/// A task's reading, ready to build engines from: `[ocr]` with the task's pieces in place, and what could not be
/// honoured (a named model that is not installed falls back to what is), in plain words.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub ocr: OcrConfig,
    pub notes: Vec<String>,
    /// What Settings shows.
    pub detector_label: String,
    pub recogniser_label: String,
    /// `None` when the reading model is off for this task.
    pub reader_label: Option<String>,
}

/// The route of `task` for the program `exe` (its `[[profile]]` override on top of the table).
pub fn route_for(cfg: &Config, task: ReadTask, exe: Option<&str>) -> ReadingRoute {
    let base = table(cfg).route(task).clone();
    match exe.and_then(|e| cfg.profile_for(e)) {
        Some(p) => base.overlaid(p.reading.route(task)),
        None => base,
    }
}

impl ProfileReading {
    pub fn route(&self, task: ReadTask) -> &ReadingRoute {
        match task {
            ReadTask::Live => &self.live,
            ReadTask::Capture => &self.capture,
        }
    }
}

/// `[ocr]` as `task` reads in `exe`.
pub fn resolve(cfg: &Config, paths: &Paths, task: ReadTask, exe: Option<&str>) -> Resolved {
    resolve_route(&cfg.ocr, paths, &route_for(cfg, task, exe), task)
}

fn custom_label(path: &str) -> String {
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    format!("Your own file ({name})")
}

pub fn resolve_route(base: &OcrConfig, paths: &Paths, route: &ReadingRoute, task: ReadTask) -> Resolved {
    let mut ocr = base.clone();
    let mut notes = Vec::new();
    let who = match task {
        ReadTask::Live => "live subtitles",
        ReadTask::Capture => "captures",
    };
    let own = |p: &Option<PathBuf>| p.as_ref().map(|p| custom_label(&p.display().to_string()));

    // The line finder.
    let mut detector_label = own(&base.det_model).unwrap_or_else(|| DETECTORS[0].label.to_string());
    match route.detector.trim() {
        "" => {}
        id if named(&DETECTORS, id).is_some() => {
            let n = named(&DETECTORS, id).copied().unwrap_or(DETECTORS[0]);
            match first_there(paths, n.files, false) {
                Some(file) => {
                    let det = (n.id != STANDARD || base.det_model.is_some()).then_some(file);
                    if det != base.det_model {
                        // The tuning in `[ocr]` was made for the detector named there.
                        (ocr.det_score_threshold, ocr.det_box_threshold, ocr.det_unclip_ratio) = (None, None, None);
                    }
                    ocr.det_model = det;
                    detector_label = n.label.to_string();
                }
                None if n.id == STANDARD => {}
                None => notes.push(format!("The {} line finder for {who} is not installed (run `maru setup ocr`); the standard one reads instead.", n.label.to_lowercase())),
            }
        }
        path => {
            if base.det_model.as_deref() != Some(Path::new(path)) {
                (ocr.det_score_threshold, ocr.det_box_threshold, ocr.det_unclip_ratio) = (None, None, None);
            }
            ocr.det_model = Some(PathBuf::from(path));
            detector_label = custom_label(path);
        }
    }

    // The letter reader.
    let mut recogniser_label = own(&base.rec_model).unwrap_or_else(|| RECOGNISERS[0].label.to_string());
    match route.recogniser.trim() {
        "" => {}
        id if named(&RECOGNISERS, id).is_some() => {
            let n = named(&RECOGNISERS, id).copied().unwrap_or(RECOGNISERS[0]);
            match (first_there(paths, n.files, false), first_there(paths, n.dict, false)) {
                (Some(rec), Some(dict)) => {
                    let standard = n.id == STANDARD && base.rec_model.is_none() && base.dict_path.is_none();
                    (ocr.rec_model, ocr.dict_path) = if standard { (None, None) } else { (Some(rec), Some(dict)) };
                    recogniser_label = n.label.to_string();
                }
                _ if n.id == STANDARD => {}
                _ => notes.push(format!("The {} letter reader for {who} is not on this PC; the standard one reads instead.", n.label)),
            }
        }
        path => {
            ocr.rec_model = Some(PathBuf::from(path));
            recogniser_label = custom_label(path);
        }
    }

    // The reading model.
    ocr.vision_reader = route.reader.unwrap_or(base.vision_reader);
    let mut reader_label = base.reader_dir.as_ref().map(|d| custom_label(&d.display().to_string())).unwrap_or_else(|| READERS[0].label.to_string());
    match route.reader_model.trim() {
        "" => {}
        id if named(&READERS, id).is_some() => {
            let n = named(&READERS, id).copied().unwrap_or(READERS[0]);
            match first_there(paths, n.files, true) {
                Some(dir) => {
                    // 1.5 is the one `maru setup ocr-vl` always installed, where `vision::reader` looks without a folder.
                    ocr.reader_dir = (n.id != READER_15).then_some(dir);
                    (ocr.reader_prompt, ocr.reader_args, ocr.reader_html) = (None, Vec::new(), false);
                    reader_label = n.label.to_string();
                }
                None if ocr.vision_reader => notes.push(format!("{} is not installed (run `maru setup ocr-vl`); {who} use the reading model that is.", n.label)),
                None => {}
            }
        }
        dir => {
            ocr.reader_dir = Some(PathBuf::from(dir));
            reader_label = custom_label(dir);
        }
    }
    Resolved { ocr, notes, detector_label, recogniser_label, reader_label: route.reader.unwrap_or(base.vision_reader).then_some(reader_label) }
}

/// The comment written above the table.
const HEADER: &str = "\n# How each task reads the screen (see docs/config.example.toml). Live subtitles: PP-OCR alone with the small line\n# finder; captures: PP-OCR with the reading model. Written once from the older [ocr] settings, which still parse.\n";

/// The file's text with `[reading]` written in, or `None` when it already has it (or there is nothing to change).
pub fn migrate(text: &str) -> Result<Option<String>> {
    let cfg: Config = toml::from_str(text).map_err(|e| Error::Config(e.to_string()))?;
    if cfg.reading.version >= VERSION {
        return Ok(None);
    }
    let t = table(&cfg);
    let mut changes = serde_json::Map::new();
    for (name, route) in [("live", &t.live), ("capture", &t.capture)] {
        changes.insert(format!("reading.{name}.detector"), route.detector.clone().into());
        changes.insert(format!("reading.{name}.recogniser"), route.recogniser.clone().into());
        changes.insert(format!("reading.{name}.reader"), route.reader.unwrap_or(cfg.ocr.vision_reader).into());
        changes.insert(format!("reading.{name}.reader_model"), route.reader_model.clone().into());
    }
    changes.insert("reading.version".into(), VERSION.into());
    let patched = crate::settings::patch(text, &changes)?;
    let mut doc: toml_edit::DocumentMut = patched.text.parse().map_err(|e: toml_edit::TomlError| Error::Config(e.to_string()))?;
    if let Some(table) = doc.get_mut("reading").and_then(toml_edit::Item::as_table_mut) {
        table.set_implicit(false);
        if table.decor().prefix().and_then(|p| p.as_str()).is_none_or(|p| p.trim().is_empty()) {
            table.decor_mut().set_prefix(HEADER);
        }
    }
    Ok(Some(doc.to_string()))
}

/// What [`migrate_file`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Migrated {
    /// The copy of the file as it was.
    pub backup: PathBuf,
    pub reading: ReadingConfig,
}

/// Write `[reading]` into the config file at `path` if it has none, after copying the file beside itself as
/// `config.toml.before-reading-<stamp>.bak`. A missing file, or one that already has the table, is left alone.
pub fn migrate_file(path: &Path, now_secs: i64) -> Result<Option<Migrated>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let Some(new_text) = migrate(&text)? else { return Ok(None) };
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("config.toml");
    let stamp = crate::tasks::utc_stamp(now_secs);
    let backup = (0..)
        .map(|n| path.with_file_name(if n == 0 { format!("{name}.before-reading-{stamp}.bak") } else { format!("{name}.before-reading-{stamp}-{n}.bak") }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| path.with_file_name(format!("{name}.before-reading.bak")));
    std::fs::write(&backup, &text)?;
    std::fs::write(path, &new_text)?;
    let after: Config = toml::from_str(&new_text).map_err(|e| Error::Config(e.to_string()))?;
    Ok(Some(Migrated { backup, reading: after.reading }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Config {
        toml::from_str(text).unwrap()
    }

    /// A models folder with the files `maru setup ocr` and `ocr-vl` install, plus the ones named in `extra`.
    fn models(extra: &[&str]) -> (tempdir::Dir, Paths) {
        let dir = tempdir::Dir::new();
        let paths = Paths::under(&dir.0);
        for rel in ["ocr/det.onnx", "ocr/rec.onnx", "ocr/dict.txt", "vlm/paddleocr-vl-1.5/PaddleOCR-VL-1.5.gguf", "vlm/paddleocr-vl-1.5/PaddleOCR-VL-1.5-mmproj.gguf"].iter().chain(extra) {
            let file = paths.models_dir.join(rel);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"x").unwrap();
        }
        (dir, paths)
    }

    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                let p = std::env::temp_dir().join(format!("maru-reading-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
                std::fs::create_dir_all(&p).unwrap();
                Self(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).ok();
            }
        }
    }

    const SMALL: &str = "ocr/pp-ocrv6-small/det.onnx";
    const VL16: [&str; 2] = ["vlm/paddleocr-vl-1.6/PaddleOCR-VL-1.6.gguf", "vlm/paddleocr-vl-1.6/PaddleOCR-VL-1.6-mmproj.gguf"];

    #[test]
    fn a_file_with_nothing_of_its_own_reads_live_subtitles_with_the_small_finder_alone_and_captures_with_the_reading_model() {
        let (_d, paths) = models(&[SMALL, VL16[0], VL16[1]]);
        let cfg = Config::default();
        let live = resolve(&cfg, &paths, ReadTask::Live, None);
        assert_eq!(live.ocr.det_model, Some(paths.models_dir.join(SMALL)));
        assert!(!live.ocr.vision_reader, "live subtitles do not use the reading model");
        assert_eq!(live.reader_label, None);
        assert_eq!(live.detector_label, "Small and quick");
        let capture = resolve(&cfg, &paths, ReadTask::Capture, None);
        assert_eq!(capture.ocr.det_model, None, "the installed finder, where it always was");
        assert!(capture.ocr.vision_reader);
        assert_eq!(capture.ocr.reader_dir, Some(paths.models_dir.join("vlm/paddleocr-vl-1.6")));
        assert_eq!(capture.reader_label.as_deref(), Some("PaddleOCR-VL 1.6"));
        assert!(live.notes.is_empty() && capture.notes.is_empty());
    }

    #[test]
    fn captures_without_paddleocr_vl_1_6_installed_keep_1_5_and_say_how_to_get_it() {
        let (_d, paths) = models(&[]);
        let capture = resolve(&Config::default(), &paths, ReadTask::Capture, None);
        assert_eq!((capture.ocr.reader_dir.clone(), capture.reader_label.as_deref()), (None, Some("PaddleOCR-VL 1.5")));
        assert!(capture.notes.iter().any(|n| n.contains("maru setup ocr-vl")), "{:?}", capture.notes);
    }

    #[test]
    fn a_small_finder_that_is_not_installed_falls_back_to_the_standard_one_and_says_how_to_get_it() {
        let (_d, paths) = models(&[]);
        let live = resolve(&Config::default(), &paths, ReadTask::Live, None);
        assert_eq!(live.ocr.det_model, None);
        assert_eq!(live.detector_label, "Standard");
        assert!(live.notes[0].contains("maru setup ocr"), "{:?}", live.notes);
    }

    #[test]
    fn the_small_finder_left_by_run_3_is_used_until_setup_installs_its_own_copy() {
        let (_d, paths) = models(&["ocr-candidates/pp-ocrv6-small-det/det.onnx"]);
        let live = resolve(&Config::default(), &paths, ReadTask::Live, None);
        assert_eq!(live.ocr.det_model, Some(paths.models_dir.join("ocr-candidates/pp-ocrv6-small-det/det.onnx")));
    }

    #[test]
    fn the_files_own_choices_stay_for_every_task_and_only_defaults_change() {
        let cfg = parse("[ocr]\ndet_model = 'C:/mine/det.onnx'\nrec_model = 'C:/mine/rec.onnx'\ndict_path = 'C:/mine/dict.txt'\nreader_dir = 'C:/mine/reader'\n");
        let t = derive(&cfg);
        for route in [&t.live, &t.capture] {
            assert_eq!((route.detector.as_str(), route.recogniser.as_str(), route.reader_model.as_str()), ("", "", ""), "follow [ocr]");
        }
        assert_eq!(t.live.reader, Some(false), "live subtitles still lose the reading model: true was the default");
        let (_d, paths) = models(&[SMALL]);
        let live = resolve(&cfg, &paths, ReadTask::Live, None);
        assert_eq!(live.ocr.det_model.as_deref(), Some(Path::new("C:/mine/det.onnx")));
        assert_eq!(live.ocr.rec_model.as_deref(), Some(Path::new("C:/mine/rec.onnx")));
        let capture = resolve(&cfg, &paths, ReadTask::Capture, None);
        assert_eq!(capture.ocr.reader_dir.as_deref(), Some(Path::new("C:/mine/reader")));
        assert_eq!(capture.reader_label.as_deref(), Some("Your own file (reader)"));
    }

    #[test]
    fn a_reading_model_switched_off_stays_off_for_captures() {
        let t = derive(&parse("[ocr]\nvision_reader = false\n"));
        assert_eq!(t.capture.reader, Some(false));
        assert_eq!(t.live.reader, Some(false));
    }

    #[test]
    fn the_older_quick_reader_switch_means_what_live_subtitles_now_do_anyway() {
        assert_eq!(derive(&parse("[continuous]\ncpu_ocr = true\n")).live, derive(&Config::default()).live);
    }

    #[test]
    fn a_profile_overrides_only_what_it_names_for_its_own_program() {
        let cfg = parse("[[profile]]\nname = 'Xenoblade'\nexe = ['yuzu.exe']\n[profile.reading.live]\nreader = true\n");
        let (_d, paths) = models(&[SMALL]);
        let there = resolve(&cfg, &paths, ReadTask::Live, Some("Yuzu.exe"));
        assert!(there.ocr.vision_reader, "this game keeps the reading model for live subtitles");
        assert_eq!(there.ocr.det_model, Some(paths.models_dir.join(SMALL)), "and the task's finder");
        assert!(!resolve(&cfg, &paths, ReadTask::Live, Some("eldenring.exe")).ocr.vision_reader);
        assert!(!resolve(&cfg, &paths, ReadTask::Live, None).ocr.vision_reader);
    }

    #[test]
    fn a_named_reading_model_points_the_reader_at_its_folder_and_drops_settings_made_for_another_model() {
        let (_d, paths) = models(&VL16);
        let mut cfg = parse("[ocr]\nreader_prompt = 'Text Recognition:'\nreader_html = true\n");
        cfg.reading = ReadingConfig { version: 1, live: derive(&cfg).live, capture: ReadingRoute { reader_model: READER_16.into(), ..derive(&cfg).capture } };
        let capture = resolve(&cfg, &paths, ReadTask::Capture, None);
        assert_eq!(capture.ocr.reader_dir, Some(paths.models_dir.join("vlm/paddleocr-vl-1.6")));
        assert_eq!((capture.ocr.reader_prompt.clone(), capture.ocr.reader_html), (None, false));
        assert_eq!(capture.reader_label.as_deref(), Some("PaddleOCR-VL 1.6"));
    }

    #[test]
    fn a_named_reading_model_that_is_not_installed_leaves_the_installed_one_and_says_so() {
        let (_d, paths) = models(&[]);
        let cfg = Config { reading: ReadingConfig { version: 1, capture: ReadingRoute { reader: Some(true), reader_model: READER_16.into(), ..Default::default() }, ..Default::default() }, ..Default::default() };
        let capture = resolve(&cfg, &paths, ReadTask::Capture, None);
        assert_eq!(capture.ocr.reader_dir, None);
        assert!(capture.notes[0].contains("maru setup ocr-vl"), "{:?}", capture.notes);
    }

    #[test]
    fn a_detector_other_than_the_one_the_tuning_was_made_for_drops_the_tuning() {
        let (_d, paths) = models(&[SMALL]);
        let cfg = parse("[ocr]\ndet_score_threshold = 0.2\n");
        assert_eq!(resolve(&cfg, &paths, ReadTask::Live, None).ocr.det_score_threshold, None);
        assert_eq!(resolve(&cfg, &paths, ReadTask::Capture, None).ocr.det_score_threshold, Some(0.2), "made for the standard finder, which captures keep");
    }

    #[test]
    fn the_community_recogniser_needs_its_character_list_and_otherwise_stays_standard() {
        let (_d, paths) = models(&["ocr-candidates/qota-ocr/rec.onnx"]);
        let route = ReadingRoute { recogniser: "qota-ocr".into(), ..Default::default() };
        let r = resolve_route(&OcrConfig::default(), &paths, &route, ReadTask::Live);
        assert_eq!(r.ocr.rec_model, None);
        assert!(!r.notes.is_empty());
        let (_d2, paths) = models(&["ocr-candidates/qota-ocr/rec.onnx", "ocr-candidates/qota-ocr/dict.txt"]);
        let r = resolve_route(&OcrConfig::default(), &paths, &route, ReadTask::Live);
        assert_eq!(r.ocr.dict_path, Some(paths.models_dir.join("ocr-candidates/qota-ocr/dict.txt")));
        assert_eq!(r.recogniser_label, "Qota-OCR (community model)");
    }

    const OLD: &str = "# mine\n[ocr]\ndevice = \"cpu\"   # keep\nvision_reader = true\n\n[continuous]\ncpu_ocr = false\n";

    #[test]
    fn migration_writes_the_table_once_keeps_every_older_key_and_comment_and_reads_the_same() {
        let text = migrate(OLD).unwrap().unwrap();
        assert!(text.contains("# mine") && text.contains("# keep") && text.contains("cpu_ocr = false"), "{text}");
        assert!(text.contains("[reading.live]") && text.contains("# How each task reads the screen"), "{text}");
        let after = parse(&text);
        assert_eq!(after.reading.version, VERSION);
        let derived = derive(&parse(OLD));
        assert_eq!((after.reading.live.clone(), after.reading.capture.clone()), (derived.live, derived.capture), "the written table reads as the derived one");
        assert_eq!(migrate(&text).unwrap(), None, "a second run changes nothing");
    }

    #[test]
    fn migrating_a_file_leaves_a_copy_of_it_beside_it_and_does_nothing_the_second_time() {
        let dir = tempdir::Dir::new();
        let path = dir.0.join("config.toml");
        std::fs::write(&path, OLD).unwrap();
        let done = migrate_file(&path, 1_795_793_412).unwrap().unwrap();
        assert_eq!(done.backup.file_name().unwrap().to_str().unwrap(), "config.toml.before-reading-20261127-153012.bak");
        assert_eq!(std::fs::read_to_string(&done.backup).unwrap(), OLD);
        assert_eq!(done.reading.live.detector, SMALL_DETECTOR);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(migrate_file(&path, 1_795_793_999).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2, "one backup only");
        assert_eq!(migrate_file(&dir.0.join("missing.toml"), 0).unwrap(), None);
    }

    #[test]
    fn a_file_migrated_by_hand_to_other_values_is_read_as_written() {
        let cfg = parse("[reading]\nversion = 1\n[reading.live]\ndetector = 'standard'\nreader = true\n");
        let (_d, paths) = models(&[SMALL]);
        let live = resolve(&cfg, &paths, ReadTask::Live, None);
        assert_eq!(live.ocr.det_model, None);
        assert!(live.ocr.vision_reader);
    }

    #[test]
    fn a_reading_setting_saved_before_the_file_was_migrated_counts_and_migration_keeps_it() {
        let text = "[reading.live]
reader = true
";
        let cfg = parse(text);
        assert_eq!(table(&cfg).live.reader, Some(true));
        assert_eq!(table(&cfg).live.detector, SMALL_DETECTOR, "the rest from the defaults");
        let after = parse(&migrate(text).unwrap().unwrap());
        assert_eq!((after.reading.version, after.reading.live.reader), (VERSION, Some(true)));
    }

    #[test]
    fn the_documented_table_is_what_a_file_without_one_reads_as() {
        let example: Config = toml::from_str(include_str!("../../../docs/config.example.toml")).unwrap();
        let derived = derive(&Config::default());
        assert_eq!((example.reading.live, example.reading.capture), (derived.live, derived.capture));
    }

    #[test]
    fn the_lite_profile_takes_the_reading_model_off_every_task_even_when_the_table_names_it() {
        let mut cfg = parse(&migrate(OLD).unwrap().unwrap());
        cfg.profiles.push(crate::config::AppProfile { exe: vec!["g.exe".into()], reading: ProfileReading { live: ReadingRoute { reader: Some(true), ..Default::default() }, ..Default::default() }, ..Default::default() });
        cfg.lite = true;
        let (cfg, lite) = cfg.effective();
        assert!(lite);
        let (_d, paths) = models(&[SMALL]);
        for (task, exe) in [(ReadTask::Capture, None), (ReadTask::Live, None), (ReadTask::Live, Some("g.exe"))] {
            assert!(!resolve(&cfg, &paths, task, exe).ocr.vision_reader, "{task:?} {exe:?}");
        }
    }
}
