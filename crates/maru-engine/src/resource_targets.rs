//! Attribute only servers owned by these services. OCR inside the app cannot be split into a second process.
use crate::Services;
use maru_core::{live_model::LiveModel, resources::ProcessTarget};
use maru_llm::model_runtime::ModelRole;

fn target(role: &str, name: &str, state: &str, pid: Option<u32>) -> ProcessTarget {
    ProcessTarget {
        role: role.into(),
        name: name.into(),
        state: state.into(),
        pid,
    }
}

impl Services {
    pub fn resource_targets(&self) -> Vec<ProcessTarget> {
        let cfg = self.engine.config();
        let owned = self.model_runtime.as_ref().and_then(|r| r.owned_process());
        let main_pid = if let Some((ModelRole::Main, pid)) = owned {
            Some(pid)
        } else {
            self.server.lock().ok().and_then(|s| s.as_ref().and_then(|s| s.process_id()))
        };
        let live_pid = owned.and_then(|(role, pid)| (role == ModelRole::Live).then_some(pid));
        let main_name = cfg
            .llm
            .model_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("Gemma / main model");
        let mut out = vec![
            target("app", "Maru + in-process OCR", "running", Some(std::process::id())),
            target(
                "main",
                main_name,
                if cfg.llm.endpoint.is_some() {
                    "external server; local resource readings unavailable"
                } else if main_pid.is_some() {
                    "loaded"
                } else {
                    "unloaded or loading; no owned PID yet"
                },
                main_pid,
            ),
        ];
        if cfg.continuous.translation.model != LiveModel::Main {
            let name = match cfg.continuous.translation.model {
                LiveModel::HyMt2 => "HY-MT2 7B Q4",
                LiveModel::HyMt2Small => "HY-MT2 1.8B",
                LiveModel::Lfm => "LFM2 KoEn",
                _ => "MiLMMT 4B Q6",
            };
            out.push(target(
                "live",
                name,
                if live_pid.is_some() {
                    "loaded"
                } else {
                    "unloaded or loading; no owned PID yet"
                },
                live_pid,
            ));
        }
        out.push(target("ocr", "PP-OCR / Windows OCR", "inside Maru; included in the Maru row", None));
        if cfg.continuous.translation.snapshot_endpoint.is_some() {
            out.push(target(
                "snapshot",
                "Snapshot Gemma",
                "external server; local resource readings unavailable",
                None,
            ));
        }
        let vision_pid = self.vision_server.lock().ok().and_then(|s| s.as_ref().and_then(|s| s.process_id()));
        out.push(target(
            "ocr_reader",
            "PaddleOCR-VL reader",
            if vision_pid.is_some() { "loaded" } else if self.vision_on_demand() && matches!(self.vision_status(), maru_llm::LlmStatus::Disabled) { "loads on demand" } else { "not running" },
            vision_pid,
        ));
        let speech_pid = self.tts_server.lock().ok().and_then(|s| s.as_ref().and_then(|s| s.process_id()));
        out.push(target(
            "speech",
            "Speech model",
            if speech_pid.is_some() {
                "loaded"
            } else if cfg.tts.enabled && cfg.tts.provider == maru_core::config::TtsProvider::Endpoint {
                "external server"
            } else if self.speech_on_demand() && matches!(self.tts_status(), maru_llm::LlmStatus::Disabled) {
                "loads on demand"
            } else {
                "not running"
            },
            speech_pid,
        ));
        out
    }
}
