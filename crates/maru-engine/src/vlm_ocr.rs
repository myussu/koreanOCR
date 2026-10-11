//! OCR through a vision-language model served by `llama-server` (or any OpenAI-compatible
//! endpoint): PaddleOCR-VL, Gemma 4, …
//!
//! This is the "second opinion" engine. It reads stylised, shadowed or handwritten text far
//! better than a small recogniser, but it returns *text only*: there are no per-word
//! positions, so line and word boxes here are spread evenly over the image. Use it for text
//! quality, and keep PP-OCR (or a detector) for geometry.

use std::time::Duration;

use image::imageops::FilterType;
use image::{ImageEncoder, RgbImage};
use maru_core::Rect;
use maru_core::config::LlmConfig;
use maru_llm::{ChatOpts, LlmClient, Message};
use maru_ocr::{OcrEngine, OcrError, OcrLine, OcrOutput, Result, words_from_chars};

/// PaddleOCR-VL is trained on this exact instruction.
pub const PADDLE_VL_PROMPT: &str = "OCR:";

/// For general chat models: make them behave like an OCR engine.
pub const GENERAL_VLM_PROMPT: &str = "Transcribe all the text in this image exactly as written, in natural reading order. \
Write one line per line of text. Do not translate, explain, correct or add anything, and do not use markdown.";

/// Larger images are shrunk first: more pixels only cost time (and the model downsamples anyway).
const MAX_SIDE: u32 = 1600;

pub struct VlmOcr {
    root: String,
    cfg: LlmConfig,
    prompt: String,
}

impl VlmOcr {
    /// `root` is the server address, e.g. `http://127.0.0.1:18090`.
    pub fn new(root: &str, cfg: &LlmConfig, prompt: &str) -> Self {
        let mut cfg = cfg.clone();
        cfg.temperature = 0.0;
        cfg.max_tokens = cfg.max_tokens.max(1024);
        cfg.request_timeout_secs = cfg.request_timeout_secs.max(120);
        Self { root: root.to_string(), cfg, prompt: prompt.to_string() }
    }
}

impl OcrEngine for VlmOcr {
    fn name(&self) -> &'static str {
        "vlm"
    }

    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
        let url = data_url(image).map_err(OcrError::Run)?;
        let messages = [Message::user_with_image(self.prompt.clone(), url)];
        let opts = ChatOpts { max_tokens: Some(self.cfg.max_tokens), temperature: Some(0.0), stop: Vec::new(), log_timings: false };
        let (root, cfg) = (self.root.clone(), self.cfg.clone());
        // The engine trait is synchronous and may be called from inside a Tokio runtime, so the
        // request gets its own thread and a private current-thread runtime (and a fresh client,
        // whose connection pool must not outlive that runtime).
        let text = std::thread::scope(|scope| {
            scope
                .spawn(move || -> std::result::Result<String, String> {
                    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
                    let client = LlmClient::new(&root, &cfg);
                    rt.block_on(client.stream_chat(&messages, &opts, &mut |_| {})).map_err(|e| e.to_string())
                })
                .join()
                .unwrap_or_else(|_| Err("the OCR request thread panicked".into()))
        })
        .map_err(OcrError::Run)?;
        Ok(output_from_text(&text, image.width() as f32, image.height() as f32))
    }
}

/// The model's reply as OCR lines, with boxes spread evenly down the image.
pub fn output_from_text(text: &str, width: f32, height: f32) -> OcrOutput {
    let lines: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("```"))
        .map(str::to_string)
        .collect();
    let n = lines.len().max(1) as f32;
    let lines = lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let rect = Rect::new(0.0, i as f32 * height / n, width, height / n);
            // No real confidence from a VLM; 1.0 keeps it above any threshold.
            OcrLine { words: words_from_chars(&text, rect, &[]), text, rect, confidence: 1.0 }
        })
        .collect();
    OcrOutput { lines }
}

fn data_url(image: &RgbImage) -> std::result::Result<String, String> {
    let (w, h) = image.dimensions();
    let longest = w.max(h);
    let resized;
    let image = if longest > MAX_SIDE {
        let scale = MAX_SIDE as f32 / longest as f32;
        resized = image::imageops::resize(image, (w as f32 * scale).round() as u32, (h as f32 * scale).round() as u32, FilterType::Triangle);
        &resized
    } else {
        image
    };
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(format!("data:image/png;base64,{}", base64(&png)))
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// How long to wait for a vision model to download and load before giving up.
pub const SERVER_READY: Duration = Duration::from_secs(900);

#[cfg(test)]
mod tests {
    use maru_llm::testing::{MockResponse, sse_answer, spawn_mock};

    use super::*;

    #[test]
    fn base64_matches_the_standard_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn reply_text_becomes_lines_without_markdown_fences() {
        let out = output_from_text("```\n좋은 사람들과\n\n  좋은 시간을 \n```", 400.0, 100.0);
        let texts: Vec<_> = out.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["좋은 사람들과", "좋은 시간을"]);
        assert!(out.lines[1].rect.y > out.lines[0].rect.y && out.lines[1].rect.bottom() <= 100.01);
        assert_eq!(out.lines[0].words.len(), 2);
        assert!(output_from_text("", 10.0, 10.0).lines.is_empty());
    }

    #[test]
    fn large_images_are_shrunk_before_sending() {
        let url = data_url(&RgbImage::new(3200, 800)).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        let png = url.trim_start_matches("data:image/png;base64,");
        // a PNG header, 8 bytes, base64-encodes to this prefix
        assert!(png.starts_with("iVBORw0KGgo"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn recognises_through_a_server_and_sends_the_image_and_prompt() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["전금도\n", "충분히 잘하고 있어요."]), delay_ms: 0 }).await;
        let ocr = VlmOcr::new(&server.url, &LlmConfig::default(), PADDLE_VL_PROMPT);
        // called from inside a runtime, exactly like the overlay's blocking tasks
        let out = tokio::task::spawn_blocking(move || ocr.recognize(&RgbImage::new(64, 32))).await.unwrap().unwrap();
        assert_eq!(out.lines.len(), 2);
        assert_eq!(out.lines[0].text, "전금도");
        let sent = server.requests.lock().unwrap()[0].1.clone();
        assert!(sent.contains("image_url") && sent.contains("data:image/png;base64,") && sent.contains("OCR:"), "{sent}");
        assert!(sent.contains("\"temperature\":0.0"), "{sent}");
    }
}
