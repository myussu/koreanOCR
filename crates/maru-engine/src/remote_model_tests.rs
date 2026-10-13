//! Remote analysis stays independent of the optional local live translator.

use crate::{BootOptions, DeepDiveRequest, EventRx, Services};
use maru_core::{config::OcrBackend, live_model::LiveModel, Capture, Config, Event, Paths};
use maru_lang::Analyzer;
use maru_llm::{
    testing::{spawn_mock, sse_answer, MockResponse},
    LlmStatus,
};

async fn boot(paths: Paths, cfg: Config, opts: BootOptions) -> Services {
    use maru_platform::{
        testing::{FakeScreen, FakeSelection},
        Platform,
    };
    use std::sync::Arc;
    let platform = Platform {
        selection: Arc::new(FakeSelection::default()),
        screen: Arc::new(FakeScreen::new(vec![])),
        os_ocr: Some(Arc::new(FixedOcr)),
    };
    crate::bootstrap::boot_with_platform(paths, cfg, opts, Some(platform)).await
}

struct FixedOcr;

impl maru_ocr::OcrEngine for FixedOcr {
    fn name(&self) -> &'static str {
        "scripted"
    }
    fn recognize(&self, _: &image::RgbImage) -> maru_ocr::Result<maru_ocr::OcrOutput> {
        Ok(maru_ocr::OcrOutput {
            lines: vec![maru_ocr::OcrLine {
                text: "학교".into(),
                rect: maru_core::Rect::new(10.0, 10.0, 60.0, 20.0),
                confidence: 1.0,
                words: vec![],
            }],
        })
    }
}

struct Home(std::path::PathBuf);

impl Home {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!("maru-remote-test-{}-{nonce}", std::process::id())))
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn collect(mut rx: EventRx) -> Vec<Event> {
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        let done = matches!(event, Event::Done { .. });
        events.push(event);
        if done {
            break;
        }
    }
    events
}

fn request() -> DeepDiveRequest {
    DeepDiveRequest {
        eojeol: Analyzer::heuristic().analyze("학교").eojeols.remove(0),
        sentence: "학교에 갑니다.".into(),
        translation: None,
        meaning: None,
        app: None,
    }
}

fn config(endpoint: &str) -> Config {
    let mut cfg = Config::default();
    cfg.llm.endpoint = Some(endpoint.into());
    cfg.llm.model_name = "nas-gemma".into();
    cfg.llm.server_path = Some("missing-test-llama-server".into());
    cfg.continuous.translation.model = LiveModel::Milmmt;
    cfg.continuous.translation.milmmt_path = Some("missing-test-live.gguf".into());
    cfg.continuous.translation.snapshot_endpoint = Some(endpoint.into());
    cfg.continuous.translation.snapshot_model_name = "nas-gemma".into();
    cfg.ocr.backend = OcrBackend::Windows;
    cfg.ocr.vision_reader = false;
    cfg.tts.enabled = false;
    cfg.learning.track_lookups = false;
    cfg
}

fn model(_: &str, body: &str) -> MockResponse {
    let answer = if body.contains("Word:") {
        "USAGE: School.\nEXAMPLES: I go to school.\nNUANCE: An educational institution.\n"
    } else {
        "0|School\n"
    };
    MockResponse {
        status: 200,
        chunks: sse_answer(&[answer]),
        delay_ms: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restarting_with_a_remote_main_model_and_an_unloaded_live_model_keeps_deep_dive_available()
{
    let server = spawn_mock(model).await;
    for _ in 0..2 {
        let home = Home::new();
        let services = boot(
            Paths::under(home.path()),
            config(&server.url),
            BootOptions {
                defer_models: true,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(services.llm_status(), LlmStatus::Ready);
        assert_eq!(services.live_llm_status(), LlmStatus::Disabled);
        assert_eq!(services.engine.llm_status(), LlmStatus::Ready);
        assert!(!services.llm_server_running());
        let scan = collect(services.engine.scan_image(image::RgbImage::new(128, 64)).1).await;
        assert!(
            scan.iter().any(|e| matches!(e, Event::ScanLabel { .. })),
            "{scan:?}"
        );
        let events = collect(services.engine.deep_dive(request()).1).await;
        assert!(
            events.iter().any(|e| matches!(e, Event::DeepDelta { .. })),
            "{events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, Event::Error { .. })),
            "{events:?}"
        );
    }
    assert_eq!(server.requests.lock().unwrap().len(), 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_local_live_model_does_not_disable_remote_analysis_when_live_is_turned_off() {
    let server = spawn_mock(model).await;
    let home = Home::new();
    let services = boot(
        Paths::under(home.path()),
        config(&server.url),
        BootOptions {
            defer_models: true,
            ..Default::default()
        },
    )
    .await;
    services.engine.set_live_active(true);
    let line = crate::LiveLine {
        capture: Capture::typed("안녕하세요."),
        appeared: std::time::Instant::now(),
        ready_ms: 0,
        ocr_ms: 0,
    };
    let events = collect(services.engine.translate_live(line).1).await;
    assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
    assert_eq!(services.llm_status(), LlmStatus::Ready);
    assert!(matches!(services.live_llm_status(), LlmStatus::Failed(_)));
    for active in [true, false] {
        services.engine.set_live_active(active);
        let events = collect(services.engine.deep_dive(request()).1).await;
        assert!(
            events.iter().any(|e| matches!(e, Event::DeepDelta { .. })),
            "{events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, Event::Error { .. })),
            "{events:?}"
        );
    }
    assert!(!services.llm_server_running());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_request_errors_preserve_offline_entries_and_a_later_deep_dive_can_recover() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let failed = Arc::new(AtomicBool::new(true));
    let flag = failed.clone();
    let server = spawn_mock(move |path, body| {
        if flag.load(Ordering::Acquire) {
            MockResponse {
                status: 503,
                chunks: vec![b"temporarily unavailable".to_vec()],
                delay_ms: 0,
            }
        } else {
            model(path, body)
        }
    })
    .await;
    let home = Home::new();
    let services = boot(
        Paths::under(home.path()),
        config(&server.url),
        BootOptions {
            defer_models: true,
            ..Default::default()
        },
    )
    .await;
    let events = collect(services.engine.deep_dive(request()).1).await;
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::DeepEntries { .. })));
    assert!(events.iter().any(|e| matches!(e, Event::Error { message, .. } if message.contains("503") && !message.contains("setup llm"))), "{events:?}");
    failed.store(false, Ordering::Release);
    let events = collect(services.engine.deep_dive(request()).1).await;
    assert!(
        events.iter().any(|e| matches!(e, Event::DeepDelta { .. })),
        "{events:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit MARU_TEST_NAS_ENDPOINT and MARU_TEST_NAS_MODEL; sends synthetic Korean to that server"]
async fn real_nas_scan_and_deep_dive_survive_restarts_and_live_toggle_changes() {
    let endpoint = std::env::var("MARU_TEST_NAS_ENDPOINT").unwrap();
    let model_name = std::env::var("MARU_TEST_NAS_MODEL").unwrap();
    for (label, active, suffix, wrong_model) in [
        ("cold_boot", false, "", false),
        ("restart", false, "/v1", false),
        ("live_on_failed_local", true, "", false),
        ("wrong_model", false, "", true),
        ("recovery", false, "", false),
    ] {
        let home = Home::new();
        let mut cfg = config(&format!("{}{suffix}", endpoint.trim_end_matches('/')));
        cfg.llm.model_name = if wrong_model {
            "maru-test-missing-model-do-not-pull".into()
        } else {
            model_name.clone()
        };
        cfg.continuous.translation.snapshot_model_name = model_name.clone();
        let services = boot(
            Paths::under(home.path()),
            cfg,
            BootOptions {
                defer_models: true,
                ..Default::default()
            },
        )
        .await;
        services.engine.set_live_active(active);
        if active {
            let line = crate::LiveLine {
                capture: Capture::typed("안녕하세요."),
                appeared: std::time::Instant::now(),
                ready_ms: 0,
                ocr_ms: 0,
            };
            let events = collect(services.engine.translate_live(line).1).await;
            assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
            assert!(matches!(services.live_llm_status(), LlmStatus::Failed(_)));
            assert_eq!(services.llm_status(), LlmStatus::Ready);
        }
        let start = std::time::Instant::now();
        let scan = collect(services.engine.scan_image(image::RgbImage::new(128, 64)).1).await;
        let deep = collect(services.engine.deep_dive(request()).1).await;
        let errors: Vec<_> = scan
            .iter()
            .chain(deep.iter())
            .filter_map(|e| {
                if let Event::Error { message, .. } = e {
                    Some(message)
                } else {
                    None
                }
            })
            .collect();
        let labels = scan
            .iter()
            .filter(|e| matches!(e, Event::ScanLabel { .. }))
            .count();
        let chunks = deep
            .iter()
            .filter(|e| matches!(e, Event::DeepDelta { .. }))
            .count();
        println!(
            "{}",
            serde_json::json!({"condition":label,"took_ms":start.elapsed().as_millis(),"scan_labels":labels,"deep_chunks":chunks,"errors":errors,"deep_entries":deep.iter().any(|e|matches!(e,Event::DeepEntries{..})),"local_server":services.llm_server_running()})
        );
        assert!(
            errors.iter().all(|e| !e.contains("setup llm")),
            "{errors:?}"
        );
        if wrong_model {
            assert!(errors.iter().any(|e| e.contains("404")), "{errors:?}");
        } else {
            assert!(
                errors.is_empty() && labels > 0 && chunks > 0,
                "{label}: {errors:?}"
            );
        }
        assert!(!services.llm_server_running());
    }
}
