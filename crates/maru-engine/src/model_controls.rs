//! Explicit preparation and release of the models owned by an interactive session.

use crate::{Services, engine::lock};
use maru_core::live_model::{LiveDevice, LiveModel};
use maru_llm::{LiveRequest, LlmStatus, PromptContext, model_runtime::ModelRole};
use maru_ocr::OcrEngine;
use serde::Serialize;
use std::sync::{Mutex, atomic::Ordering};

#[derive(Default, Clone, Copy, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Translator,
    Ocr,
    Ready,
    Failed,
    Releasing,
}

#[derive(Default, Clone)]
struct ControlState {
    phase: Phase,
    message: Option<String>,
    prepared_process: Option<(ModelRole, u32)>,
    prepared_reader: Option<u32>,
}

#[derive(Default)]
pub(crate) struct ModelControls {
    operation: tokio::sync::Mutex<()>,
    state: Mutex<ControlState>,
}

struct Operation<'a>(&'a ModelControls);
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.0.state);
        if matches!(
            state.phase,
            Phase::Translator | Phase::Ocr | Phase::Releasing
        ) {
            state.phase = Phase::Idle;
            state.message =
                Some("Model action stopped; the next request can load its model.".into());
        }
    }
}

#[derive(Serialize)]
pub struct LiveSetupStatus {
    pub model: &'static str,
    pub device: String,
    pub model_status: String,
    pub ocr: String,
    pub ocr_status: String,
    pub preparation: &'static str,
    pub message: Option<String>,
    pub busy: bool,
    pub idle_reader_loaded: bool,
}

fn status_text(status: LlmStatus) -> String {
    match status {
        LlmStatus::Ready => "Ready".into(),
        LlmStatus::Starting => "Loading".into(),
        LlmStatus::Disabled => "Loads on demand".into(),
        LlmStatus::Failed(error) => format!("Error: {error}"),
    }
}

impl Services {
    pub fn live_setup_status(&self) -> LiveSetupStatus {
        let cfg = self.engine.config();
        let selected = cfg.continuous.translation.model;
        let (model, device) = match selected {
            LiveModel::Main => (
                "Gemma / main model",
                if cfg.llm.endpoint.is_some() {
                    "External server".into()
                } else {
                    "Local · configured offload".into()
                },
            ),
            LiveModel::HyMt2 => (
                "HY-MT2 7B Q4",
                device_name(cfg.continuous.translation.device),
            ),
            LiveModel::Milmmt => (
                "MiLMMT 4B Q6",
                device_name(cfg.continuous.translation.device),
            ),
            LiveModel::HyMt2Small => (
                "HY-MT2 1.8B Q4",
                device_name(cfg.continuous.translation.device),
            ),
            LiveModel::Lfm => (
                "LFM2 KoEn 1.2B Q5",
                device_name(cfg.continuous.translation.device),
            ),
        };
        let wanted = if selected == LiveModel::Main {
            ModelRole::Main
        } else {
            ModelRole::Live
        };
        let client = if selected == LiveModel::Main {
            &self.engine.inner.llm
        } else {
            &self.engine.inner.live_llm
        };
        let mut model_status = if selected == LiveModel::Main {
            self.llm_status()
        } else {
            self.live_llm_status()
        };
        if client.is_none() {
            model_status = LlmStatus::Failed("No translation model configured".into());
        } else if client.as_ref().is_some_and(|c| c.has_runtime())
            && self
                .model_runtime
                .as_ref()
                .is_some_and(|r| r.current_role() != Some(wanted))
            && model_status == LlmStatus::Ready
        {
            model_status = LlmStatus::Disabled;
        }
        let ocr_engine = if cfg.continuous.cpu_ocr {
            &self.engine.inner.live_ocr
        } else {
            &self.engine.inner.ocr
        };
        let uses_reader = !cfg.continuous.cpu_ocr && self.deferred_vision.is_some();
        let ocr = if cfg.continuous.cpu_ocr {
            "CPU PP-OCR".into()
        } else if uses_reader {
            format!(
                "Hybrid · {} detector + PaddleOCR-VL",
                cfg.ocr.device.to_uppercase()
            )
        } else {
            ocr_engine
                .as_ref()
                .map(|o| format!("{} OCR", o.name()))
                .unwrap_or_else(|| "Unavailable".into())
        };
        let ocr_status = if ocr_engine.is_none() {
            "Unavailable".into()
        } else if uses_reader {
            status_text(self.vision_status())
        } else {
            "Ready".into()
        };
        let state = lock(&self.model_controls.state);
        let prepared = model_status == LlmStatus::Ready
            && ocr_status == "Ready"
            && state.prepared_process
                == self.model_runtime.as_ref().and_then(|r| r.owned_process())
            && (!uses_reader
                || state.prepared_reader
                    == self
                        .vision_server
                        .lock()
                        .ok()
                        .and_then(|s| s.as_ref().and_then(|s| s.process_id())));
        let preparation = match state.phase {
            Phase::Translator => "Preparing translator…",
            Phase::Ocr => "Preparing OCR…",
            Phase::Releasing => "Releasing idle models…",
            Phase::Ready if prepared => "Prepared",
            Phase::Failed => "Preparation failed",
            _ => "Not prepared",
        };
        LiveSetupStatus {
            model,
            device,
            model_status: status_text(model_status),
            ocr,
            ocr_status,
            preparation,
            message: state.message.clone(),
            busy: matches!(
                state.phase,
                Phase::Translator | Phase::Ocr | Phase::Releasing
            ),
            idle_reader_loaded: cfg.continuous.cpu_ocr && self.vision_server_running(),
        }
    }

    /// Fixed synthetic input only: no capture, context, history, speech or snapshot routing.
    pub async fn prepare_live_session(&self) -> Result<(), String> {
        let _operation = self
            .model_controls
            .operation
            .try_lock()
            .map_err(|_| "Another model action is still running.".to_string())?;
        if self.engine.inner.live_active.load(Ordering::Acquire) || self.model_work_pending() {
            return Err("Turn live subtitles off and let pending requests finish before preparing a session.".into());
        }
        *lock(&self.model_controls.state) = ControlState {
            phase: Phase::Translator,
            ..Default::default()
        };
        let _reset = Operation(&self.model_controls);
        let result = self.warm_live_models().await;
        let mut state = lock(&self.model_controls.state);
        match result {
            Ok(()) => {
                state.phase = Phase::Ready;
                state.prepared_process =
                    self.model_runtime.as_ref().and_then(|r| r.owned_process());
                state.prepared_reader = self
                    .vision_server
                    .lock()
                    .ok()
                    .and_then(|s| s.as_ref().and_then(|s| s.process_id()));
                state.message =
                    Some("Live models prepared. You can now turn live subtitles on.".into());
                Ok(())
            }
            Err(error) => {
                state.phase = Phase::Failed;
                state.message = Some(error.clone());
                Err(error)
            }
        }
    }

    async fn warm_live_models(&self) -> Result<(), String> {
        let cfg = self.engine.config();
        let selected = cfg.continuous.translation.model;
        let client = if selected == LiveModel::Main {
            &self.engine.inner.llm
        } else {
            &self.engine.inner.live_llm
        };
        let client = client
            .as_ref()
            .ok_or("Choose a translation model in Settings before preparing live subtitles.")?;
        let ctx = PromptContext::default();
        let mut answer = String::new();
        client
            .translate_selected_live(
                selected,
                &LiveRequest {
                    text: "안녕하세요.",
                    ctx: &ctx,
                    turns: &[],
                },
                &mut |chunk| {
                    if let maru_llm::LlmChunk::Text { delta, .. } = chunk {
                        answer.push_str(&delta);
                    }
                },
            )
            .await
            .map_err(|e| format!("Live translator preparation failed: {e}"))?;
        if answer.trim().is_empty() {
            return Err("The live translator returned no text during preparation.".into());
        }
        lock(&self.model_controls.state).phase = Phase::Ocr;
        let image = image::load_from_memory(include_bytes!("../assets/live-warmup.png"))
            .map_err(|e| e.to_string())?
            .to_rgb8();
        if !cfg.continuous.cpu_ocr
            && let Some(model) = &self.deferred_vision
        {
            let reader_lease = model.acquire().await?;
            // Test the reader directly: hybrid fallback must not hide a failed warm-up request.
            let reader = crate::vlm_ocr::VlmOcr::new(
                &format!("http://127.0.0.1:{}", crate::vision::PORT),
                &crate::vision::base_config(&cfg.llm),
                crate::vlm_ocr::PADDLE_VL_PROMPT,
            );
            let sample = image.clone();
            tokio::task::spawn_blocking(move || {
                let _reader = reader_lease;
                reader.recognize(&sample)
            })
            .await
            .map_err(|e| format!("OCR preparation stopped: {e}"))?
            .map_err(|e| format!("OCR reader preparation failed: {e}"))?;
        }
        let ocr = if cfg.continuous.cpu_ocr { self.engine.inner.live_ocr.clone() } else { self.engine.inner.ocr.clone() }
            .ok_or("The selected live OCR is unavailable. Check Reading the screen and Live subtitles in Settings.")?;
        tokio::task::spawn_blocking(move || ocr.recognize(&image))
            .await
            .map_err(|e| format!("OCR preparation stopped: {e}"))?
            .map_err(|e| format!("OCR preparation failed: {e}"))?;
        Ok(())
    }

    /// Keep the voice alive for the entire generation, including Anki and Settings tests.
    pub async fn speech_lease(
        &self,
    ) -> Result<Option<tokio::sync::OwnedRwLockReadGuard<()>>, String> {
        if let Some(model) = &self.deferred_speech {
            return model.acquire().await.map(Some);
        }
        self.ensure_speech().await?;
        Ok(None)
    }

    fn model_work_pending(&self) -> bool {
        let inner = &self.engine.inner;
        let (waiting, active) = self.engine.live_work();
        waiting > 0
            || active > 0
            || [
                &inner.translate_task,
                &inner.deep_task,
                &inner.scan_task,
                &inner.live_load_task,
            ]
            .iter()
            .any(|slot| lock(slot).as_ref().is_some_and(|task| !task.is_finished()))
    }

    pub async fn release_idle_models(&self) -> Result<String, String> {
        let _operation = self
            .model_controls
            .operation
            .try_lock()
            .map_err(|_| "Another model action is still running.".to_string())?;
        let previous = lock(&self.model_controls.state).clone();
        lock(&self.model_controls.state).phase = Phase::Releasing;
        let _reset = Operation(&self.model_controls);
        let live = self.engine.inner.live_active.load(Ordering::Acquire);
        let mut released = Vec::new();
        let mut kept = Vec::new();
        let mut errors = Vec::new();
        if let Some(model) = &self.model_runtime {
            if live || self.model_work_pending() {
                kept.push("translator (live or pending requests)");
            } else {
                record_release(
                    "translator",
                    model.release_idle().await.map_err(|e| e.to_string()),
                    &mut released,
                    &mut kept,
                    &mut errors,
                );
            }
        }
        if let Some(model) = &self.deferred_vision {
            if live && !self.engine.config().continuous.cpu_ocr {
                kept.push("OCR reader (live subtitles on)");
            } else {
                record_release(
                    "OCR reader",
                    model.release_idle().await,
                    &mut released,
                    &mut kept,
                    &mut errors,
                );
            }
        }
        if let Some(model) = &self.deferred_speech {
            record_release(
                "speech",
                model.release_idle().await,
                &mut released,
                &mut kept,
                &mut errors,
            );
        }
        let mut message = if released.is_empty() {
            "No idle local models were loaded.".into()
        } else {
            format!("Released {}.", released.join(", "))
        };
        if !kept.is_empty() {
            message.push_str(&format!(" Kept {}.", kept.join(", ")));
        }
        if !errors.is_empty() {
            message.push_str(&format!(" {}", errors.join(" ")));
        }
        *lock(&self.model_controls.state) = ControlState {
            phase: if previous.phase == Phase::Ready {
                Phase::Ready
            } else {
                Phase::Idle
            },
            message: Some(message.clone()),
            prepared_process: previous.prepared_process,
            prepared_reader: previous.prepared_reader,
        };
        tracing::info!(%message, "explicit local model release");
        if errors.is_empty() {
            Ok(message)
        } else {
            Err(message)
        }
    }
}

fn device_name(device: LiveDevice) -> String {
    if device == LiveDevice::Cpu {
        "CPU"
    } else {
        "GPU"
    }
    .into()
}

fn record_release(
    name: &'static str,
    result: Result<Option<bool>, String>,
    released: &mut Vec<&'static str>,
    kept: &mut Vec<&'static str>,
    errors: &mut Vec<String>,
) {
    match result {
        Ok(Some(true)) => released.push(name),
        Ok(Some(false)) => {}
        Ok(None) => kept.push(name),
        Err(error) => errors.push(format!("Could not release {name}: {error}")),
    }
}
