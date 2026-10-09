//! `Windows.Media.Ocr`: built into Windows 10/11, no model download, returns
//! words with bounding boxes. Needs the Korean OCR language capability
//! (Settings → Time & language → Language & region → Korean → Language options).

use std::time::Duration;

use image::RgbImage;
use maru_core::{Rect, WordBox};
use maru_ocr::{OcrEngine, OcrError, OcrLine, OcrOutput};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine as WinOcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::core::HSTRING;

use super::com::ComWorker;

/// The API reports no confidence; this value passes the default threshold.
const ASSUMED_CONFIDENCE: f32 = 0.85;
const TIMEOUT: Duration = Duration::from_secs(10);

pub struct WindowsOcr {
    worker: ComWorker<WinOcrEngine>,
}

impl WindowsOcr {
    pub fn new(language_tag: &str) -> Result<Self, OcrError> {
        let tag = HSTRING::from(language_tag);
        let worker = ComWorker::start("maru-winocr", move || {
            let lang = Language::CreateLanguage(&tag)?;
            WinOcrEngine::TryCreateFromLanguage(&lang)
        })
        .map_err(|e| {
            OcrError::Init(format!(
                "{e}. Install the OCR language pack: Settings → Time & language → Language & region → Korean → Language options → Optical character recognition"
            ))
        })?;
        Ok(Self { worker })
    }
}

fn to_bitmap(image: &RgbImage) -> windows::core::Result<SoftwareBitmap> {
    let mut bgra = Vec::with_capacity(image.width() as usize * image.height() as usize * 4);
    for p in image.pixels() {
        bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let buffer = writer.DetachBuffer()?;
    SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, image.width() as i32, image.height() as i32)
}

impl OcrEngine for WindowsOcr {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn recognize(&self, image: &RgbImage) -> maru_ocr::Result<OcrOutput> {
        let image = image.clone();
        self.worker
            .call(TIMEOUT, move |engine| -> windows::core::Result<OcrOutput> {
                let bitmap = to_bitmap(&image)?;
                let result = engine.RecognizeAsync(&bitmap)?.join()?;
                let mut lines = Vec::new();
                for line in result.Lines()? {
                    let mut words = Vec::new();
                    let mut bounds: Option<Rect> = None;
                    for word in line.Words()? {
                        let r = word.BoundingRect()?;
                        let rect = Rect::new(r.X, r.Y, r.Width, r.Height);
                        bounds = Some(bounds.map_or(rect, |b| b.union(&rect)));
                        words.push(WordBox { text: word.Text()?.to_string(), rect });
                    }
                    if let Some(rect) = bounds {
                        lines.push(OcrLine { text: line.Text()?.to_string(), rect, confidence: ASSUMED_CONFIDENCE, words });
                    }
                }
                Ok(OcrOutput { lines })
            })
            .map_err(|e| OcrError::Run(e.to_string()))?
            .map_err(|e| OcrError::Run(e.to_string()))
    }
}
