//! PP-OCR detection + recognition through ONNX Runtime (via `oar-ocr`).
//!
//! The recogniser's CTC output gives a time-step per character, which is how
//! `oar-ocr` can report per-character boxes; [`words_from_chars`] folds those
//! into the space-delimited words the overlay positions glosses over.

use std::path::{Path, PathBuf};

use image::RgbImage;
use maru_core::config::OcrConfig;
use maru_core::{Paths, Rect};
use oar_ocr::core::config::{OrtExecutionProvider, OrtSessionConfig};
use oar_ocr::domain::tasks::TextDetectionConfig;
use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};
use oar_ocr::processors::LimitType;

use crate::{OcrEngine, OcrError, OcrLine, OcrOutput, Result, geometry, prep};

/// Where the three model files live.
#[derive(Debug, Clone, PartialEq)]
pub struct PaddleConfig {
    pub det_model: PathBuf,
    pub rec_model: PathBuf,
    /// One character per line, as shipped with PP-OCR recognisers.
    pub dict_path: PathBuf,
    /// `cpu`, `cuda[:N]`, `directml[:N]`
    pub device: String,
    /// Longest image side the detector may look at without shrinking it. `None` keeps the library's
    /// default (960 px), which suits selections; reading a whole 1080p screen needs about 2048 or
    /// small interface text shrinks out of existence.
    pub detect_side: Option<u32>,
}

impl PaddleConfig {
    /// Explicit config paths win; otherwise `<models>/ocr/{det.onnx,rec.onnx,dict.txt}`.
    pub fn resolve(cfg: &OcrConfig, paths: &Paths) -> Self {
        let dir = paths.models_dir.join("ocr");
        Self {
            det_model: cfg.det_model.clone().unwrap_or_else(|| dir.join("det.onnx")),
            rec_model: cfg.rec_model.clone().unwrap_or_else(|| dir.join("rec.onnx")),
            dict_path: cfg.dict_path.clone().unwrap_or_else(|| dir.join("dict.txt")),
            device: cfg.device.clone(),
            detect_side: None,
        }
    }

    fn check(&self) -> Result<()> {
        let missing: Vec<String> = [&self.det_model, &self.rec_model, &self.dict_path]
            .into_iter()
            .filter(|p| !p.exists())
            .map(|p| p.display().to_string())
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(OcrError::MissingModels(format!("{} (run `maru setup ocr` to download them)", missing.join(", "))))
        }
    }
}

/// `cpu` → default session; `cuda:0` / `directml:0` → that execution provider.
pub fn session_config(device: &str) -> Result<Option<OrtSessionConfig>> {
    let d = device.trim().to_ascii_lowercase();
    let (kind, id) = d.split_once(':').unwrap_or((d.as_str(), "0"));
    let device_id: i32 = id.parse().map_err(|_| OcrError::Init(format!("bad device id in {device:?}")))?;
    let provider = match kind {
        "cpu" | "" => return Ok(None),
        "cuda" => OrtExecutionProvider::CUDA {
            device_id: Some(device_id),
            gpu_mem_limit: None,
            arena_extend_strategy: None,
            cudnn_conv_algo_search: None,
            cudnn_conv_use_max_workspace: None,
        },
        "directml" | "dml" => OrtExecutionProvider::DirectML { device_id: Some(device_id) },
        other => return Err(OcrError::Init(format!("unknown OCR device {other:?}; use cpu, cuda[:N] or directml[:N]"))),
    };
    // DirectML requires sequential execution; we never enable parallel execution.
    Ok(Some(OrtSessionConfig::new().with_execution_providers(vec![provider])))
}

pub struct PaddleOcr {
    ocr: OAROCR,
}

impl PaddleOcr {
    pub fn load(cfg: &PaddleConfig) -> Result<Self> {
        cfg.check()?;
        let dict = std::fs::read_to_string(&cfg.dict_path).map_err(|e| OcrError::Init(format!("{}: {e}", cfg.dict_path.display())))?;
        let mut builder = OAROCRBuilder::new(cfg.det_model.clone(), cfg.rec_model.clone(), Path::new("in-memory-dict"))
            .character_dict_content(dict)
            .return_word_box(true);
        if let Some(session) = session_config(&cfg.device)? {
            builder = builder.ort_session(session);
        }
        if let Some(side) = cfg.detect_side {
            builder = builder.text_detection_config(TextDetectionConfig {
                limit_side_len: Some(side),
                limit_type: Some(LimitType::Max),
                max_side_len: Some(side.max(4000)),
                ..Default::default()
            });
        }
        let ocr = builder.build().map_err(|e| OcrError::Init(e.to_string()))?;
        Ok(Self { ocr })
    }
}

fn rect_of(b: &oar_ocr::processors::BoundingBox, p: &prep::Prepared) -> Rect {
    let (x0, y0) = (p.to_original(b.x_min()), p.to_original(b.y_min()));
    let (x1, y1) = (p.to_original(b.x_max()), p.to_original(b.y_max()));
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

impl OcrEngine for PaddleOcr {
    fn name(&self) -> &'static str {
        "paddle"
    }

    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
        if image.width() == 0 || image.height() == 0 {
            return Ok(OcrOutput::default());
        }
        let (prepared, p) = prep::prepare(image);
        let mut results = self.ocr.predict(vec![prepared]).map_err(|e| OcrError::Run(e.to_string()))?;
        let Some(result) = results.pop() else { return Ok(OcrOutput::default()) };

        let mut lines: Vec<OcrLine> = result
            .text_regions
            .iter()
            .filter_map(|region| {
                let text = region.text.as_deref()?.trim().to_string();
                if text.is_empty() {
                    return None;
                }
                let rect = rect_of(&region.bounding_box, &p);
                let boxes: Vec<Rect> = region.word_boxes.iter().flatten().map(|b| rect_of(b, &p)).collect();
                let words = geometry::words_from_chars(&text, rect, &boxes);
                // Rebuilt from the words so repaired spacing reaches the analyser and the model too.
                let text = if words.is_empty() { text } else { words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ") };
                Some(OcrLine {
                    words,
                    text,
                    rect,
                    confidence: region.confidence.unwrap_or(0.0),
                })
            })
            .collect();
        geometry::reading_order(&mut lines);
        Ok(OcrOutput { lines })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_strings() {
        assert!(session_config("cpu").unwrap().is_none());
        assert!(session_config("CUDA:1").unwrap().is_some());
        assert!(session_config("directml").unwrap().is_some());
        assert!(session_config("dml:0").unwrap().is_some());
        assert!(session_config("tpu").is_err());
        assert!(session_config("cuda:x").is_err());
    }

    #[test]
    fn missing_models_are_reported_with_the_fix() {
        let cfg = PaddleConfig::resolve(&OcrConfig::default(), &Paths::under(Path::new("/nonexistent-maru")));
        let Err(err) = PaddleOcr::load(&cfg) else { panic!("expected an error") };
        let msg = err.to_string();
        assert!(msg.contains("det.onnx") && msg.contains("rec.onnx") && msg.contains("maru setup"), "{msg}");
    }

    #[test]
    fn default_paths_live_under_the_models_dir() {
        let cfg = PaddleConfig::resolve(&OcrConfig::default(), &Paths::under(Path::new("/m")));
        assert_eq!(cfg.det_model, Path::new("/m/models/ocr/det.onnx"));
    }
}
