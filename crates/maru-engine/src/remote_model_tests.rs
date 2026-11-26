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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_snapshot_analysis_uses_only_its_own_credentials_when_the_main_server_is_different() {
    use maru_core::external_servers::ServerProvider;
    for azure_main in [false, true] {
        let main = spawn_mock(model).await;
        let snapshot = spawn_mock(model).await;
        let home = Home::new();
        let mut cfg = config(&main.url);
        cfg.llm.provider = if azure_main { ServerProvider::Azure } else { ServerProvider::Openai };
        cfg.llm.api_key = Some("main-test-key".to_string().into());
        cfg.continuous.translation.snapshot_endpoint = Some(snapshot.url.clone());
        cfg.continuous.translation.snapshot_provider = if azure_main { ServerProvider::Openai } else { ServerProvider::Azure };
        cfg.continuous.translation.snapshot_api_key = (!azure_main).then(|| "snapshot-test-key".to_string().into());
        let services = boot(Paths::under(home.path()), cfg, BootOptions { defer_models: true, ..Default::default() }).await;
        services.engine.set_live_active(true);
        let events = collect(services.engine.deep_dive(request()).1).await;
        assert!(events.iter().any(|event| matches!(event, Event::DeepDelta { .. })), "{events:?}");
        assert!(!events.iter().any(|event| matches!(event, Event::Error { .. })), "{events:?}");
        let headers = snapshot.headers.lock().unwrap();
        assert!(!headers.is_empty());
        assert!(headers.iter().all(|header| !header.contains("main-test-key")));
        if azure_main {
            assert!(headers.iter().all(|header| !header.contains("api-key:") && !header.contains("authorization:")));
        } else {
            assert!(headers.iter().all(|header| header.contains("api-key: snapshot-test-key")));
            assert!(snapshot.requests.lock().unwrap().iter().all(|(path, _)| path == "/openai/v1/chat/completions"));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_subtitles_hold_the_server_model_and_release_it_after_they_stop_but_not_between_areas() {
    let server = spawn_mock(|path, body| {
        if path == "/api/generate" {
            MockResponse { status: 200, chunks: vec![br#"{"done":true}"#.to_vec()], delay_ms: 0 }
        } else {
            model(path, body)
        }
    })
    .await;
    let home = Home::new();
    let mut cfg = config(&server.url);
    cfg.continuous.translation.model = LiveModel::Main;
    cfg.llm.server_keep_minutes = 15;
    let services = boot(Paths::under(home.path()), cfg, BootOptions { defer_models: true, ..Default::default() }).await;
    let releases = || server.requests.lock().unwrap().iter().filter(|(p, _)| p == "/api/generate").count();
    let holds = || server.requests.lock().unwrap().iter().filter(|(_, b)| b.contains(r#""keep_alive":"15m""#)).count();

    services.engine.set_live_active(true);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(holds(), 1, "the same model is loaded once, though it is both the live and the snapshot model");
    // Choosing a new area stops and restarts live mode: no unload in between.
    services.engine.set_live_active(false);
    services.engine.set_live_active(true);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(releases(), 0);
    services.engine.set_live_active(false);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(releases(), 1);
    let unload: serde_json::Value = serde_json::from_str(&server.requests.lock().unwrap().iter().find(|(p, _)| p == "/api/generate").unwrap().1).unwrap();
    assert_eq!(unload, serde_json::json!({ "model": "nas-gemma", "keep_alive": 0 }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_keep_time_live_subtitles_leave_the_server_alone() {
    let server = spawn_mock(model).await;
    let home = Home::new();
    let mut cfg = config(&server.url);
    cfg.continuous.translation.model = LiveModel::Main;
    let services = boot(Paths::under(home.path()), cfg, BootOptions { defer_models: true, ..Default::default() }).await;
    services.engine.set_live_active(true);
    services.engine.set_live_active(false);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(server.requests.lock().unwrap().is_empty(), "nothing is loaded or unloaded unless asked");
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
    assert!(events.iter().any(|e| matches!(e, Event::Error { message, .. } if message.contains("not answering") && !message.contains("503") && !message.contains("setup llm"))), "the server's failure in plain words, without its status code: {events:?}");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires MARU_TEST_NAS_ENDPOINT, MARU_TEST_NAS_MODEL, MARU_TEST_BACKUP_GGUF and MARU_TEST_LLAMA_SERVER; unloads that server's model"]
async fn real_nas_backup_takes_live_lines_while_the_server_reloads() {
    let endpoint = std::env::var("MARU_TEST_NAS_ENDPOINT").unwrap();
    let model_name = std::env::var("MARU_TEST_NAS_MODEL").unwrap();
    let home = Home::new();
    let mut cfg = config(&endpoint);
    cfg.llm.model_name = model_name.clone();
    cfg.continuous.translation.model = LiveModel::Main;
    cfg.continuous.translation.snapshot_endpoint = None;
    cfg.continuous.translation.backup_model = Some(LiveModel::HyMt2Small);
    cfg.continuous.translation.hy_mt2_small_path = Some(std::env::var("MARU_TEST_BACKUP_GGUF").unwrap().into());
    // The backup runs on the CPU here so the test leaves this PC's GPU alone.
    cfg.continuous.translation.device = maru_core::live_model::LiveDevice::Cpu;
    cfg.llm.server_path = Some(std::env::var("MARU_TEST_LLAMA_SERVER").unwrap().into());
    let services = boot(Paths::under(home.path()), cfg, BootOptions { defer_models: true, ..Default::default() }).await;
    let line = |text: &str| crate::LiveLine { capture: Capture::typed(text), appeared: std::time::Instant::now(), ready_ms: 0, ocr_ms: 0 };
    let translate = |text: &'static str| {
        let engine = services.engine.clone();
        let line = line(text);
        async move {
            let start = std::time::Instant::now();
            let events = collect(engine.translate_live(line).1).await;
            let answer = events.iter().find_map(|e| match e { Event::TranslationDone { text, .. } => Some(text.clone()), _ => None });
            let error = events.iter().find_map(|e| match e { Event::Error { message, .. } => Some(message.clone()), _ => None });
            eprintln!("{text} -> {answer:?} {error:?} in {} ms", start.elapsed().as_millis());
            (answer, start.elapsed())
        }
    };
    services.engine.set_live_active(true);
    for _ in 0..240 {
        if matches!(services.live_llm_status(), LlmStatus::Ready | LlmStatus::Failed(_)) { break; }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    eprintln!("backup status: {:?}", services.live_llm_status());
    let server = maru_llm::LlmClient::from_config(&maru_core::config::LlmConfig { endpoint: Some(endpoint.clone()), model_name: model_name.clone(), ..Default::default() });
    server.hold_server_model().await.unwrap();
    let (answer, took) = translate("이걸 타면 위로 갈 수 있어요.").await;
    assert!(answer.is_some() && took < std::time::Duration::from_millis(2500), "the warm server answers itself: {took:?}");

    // Make the server reload its model, as after a crashed runner.
    server.release_server_model().await.unwrap();
    let (answer, took) = translate("왜 아직도 여기 있는 거야?").await;
    assert!(answer.is_some_and(|a| !a.is_empty()), "the backup answered while the server reloaded");
    assert!(took < std::time::Duration::from_secs(8), "{took:?}");
    let (answer, took) = translate("축복을 잃은 자여, 돌아가라.").await;
    assert!(answer.is_some(), "a line during the rest goes straight to the backup");
    assert!(took < std::time::Duration::from_secs(4), "{took:?}");
    // The server finished loading in the background; after its rest it answers again, warm.
    tokio::time::sleep(std::time::Duration::from_secs(21)).await;
    let (answer, took) = translate("그 사람은 이미 떠났어.").await;
    assert!(answer.is_some() && took < std::time::Duration::from_millis(2500), "the server is back and warm: {took:?}");
    services.engine.set_live_active(false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires MARU_TEST_OCR_DIR (det.onnx, rec.onnx, dict.txt) and MARU_TEST_OCR_IMAGE (a picture of a Korean line)"]
async fn real_pp_ocr_reads_on_directml_and_a_device_that_cannot_load_falls_back_to_the_cpu() {
    let models = std::path::PathBuf::from(std::env::var("MARU_TEST_OCR_DIR").unwrap());
    let picture = image::open(std::env::var("MARU_TEST_OCR_IMAGE").unwrap()).unwrap().to_rgb8();
    for device in ["directml:0", "npu:0"] {
        let home = Home::new();
        let paths = Paths::under(home.path());
        let ocr_dir = paths.models_dir.join("ocr");
        std::fs::create_dir_all(&ocr_dir).unwrap();
        for file in ["det.onnx", "rec.onnx", "dict.txt"] {
            std::fs::copy(models.join(file), ocr_dir.join(file)).unwrap();
        }
        let mut cfg = config("http://127.0.0.1:9");
        cfg.ocr.backend = OcrBackend::Paddle;
        cfg.ocr.device = device.into();
        let services = boot(paths, cfg, BootOptions { defer_models: true, ..Default::default() }).await;
        let started = std::time::Instant::now();
        let events = collect(services.engine.scan_image(picture.clone()).1).await;
        let texts: Vec<String> = events.iter().filter_map(|e| match e {
            Event::ScanItems { items, .. } => Some(items.iter().map(|i| i.text.clone()).collect::<Vec<_>>().join(" ")),
            _ => None,
        }).collect();
        eprintln!("{device}: {texts:?} in {} ms (including loading)", started.elapsed().as_millis());
        assert!(texts.join(" ").contains("여기"), "{device} read {texts:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_older_config_keeps_its_capture_server_and_gets_card_meanings_on_this_pcs_model_without_loading_it() {
    use crate::routes::{Place, State};
    use maru_core::tasks::Task;
    let server = spawn_mock(model).await;
    let home = Home::new();
    let mut cfg = config(&server.url);
    cfg.continuous.translation.model = LiveModel::Main;
    cfg.llm.hf_repo = Some("someone/gemma-GGUF:Q4".into());
    let services = boot(Paths::under(home.path()), cfg, BootOptions { defer_models: true, ..Default::default() }).await;
    let status = services.engine.task_status();
    let of = |task: Task| status.iter().find(|s| s.task == task).unwrap();

    // Captures and live subtitles: the server the file named, now under its host name.
    assert_eq!((of(Task::Capture).connection.as_str(), of(Task::Capture).model.as_str()), ("127.0.0.1", "nas-gemma"));
    assert_eq!(of(Task::Capture).place, Some(Place::ThisPc), "the scripted server is on this machine");
    assert_eq!(of(Task::Live).connection, "127.0.0.1");
    assert_eq!(services.engine.config().llm.endpoint.as_deref(), Some(server.url.as_str()));

    // Card meanings: this PC's model, known but not started; then the dictionary.
    let cards = of(Task::CardMeanings);
    assert_eq!((cards.connection.as_str(), cards.model.as_str(), cards.state), ("local", "someone/gemma-GGUF:Q4", State::NotLoaded));
    assert_eq!(cards.fallbacks, ["the dictionary"]);
    assert_eq!(of(Task::CardSentence).state, State::Off);
    assert!(!services.llm_server_running(), "naming this PC's model for cards loads nothing at start-up");
    assert!(services.notes.iter().any(|n| n.starts_with("Card meanings: This PC's model")), "{:?}", services.notes);

    // A capture still goes to the server.
    let events = collect(services.engine.deep_dive(request()).1).await;
    assert!(events.iter().any(|e| matches!(e, Event::DeepDelta { .. })), "{events:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_table_can_put_live_subtitles_on_a_server_and_captures_on_this_pcs_model() {
    use maru_core::external_servers::SavedServer;
    use maru_core::tasks::{CaptureRoute, Task, TaskRoute, TasksConfig, VERSION};
    let server = spawn_mock(model).await;
    let home = Home::new();
    let mut cfg = config(&server.url);
    cfg.continuous.translation.model = LiveModel::Main;
    cfg.continuous.translation.snapshot_endpoint = None;
    cfg.llm.hf_repo = Some("someone/gemma-GGUF:Q4".into());
    cfg.llm.servers = vec![SavedServer { name: "NAS".into(), endpoint: server.url.clone(), model_name: "nas-gemma".into(), provider: Default::default(), api_key: None }];
    cfg.tasks = TasksConfig {
        version: VERSION,
        live: TaskRoute { connection: "NAS".into(), model: String::new() },
        capture: CaptureRoute { connection: "local".into(), ..Default::default() },
        card_meanings: TaskRoute { connection: "NAS".into(), model: String::new() },
        ..Default::default()
    };
    let services = boot(Paths::under(home.path()), cfg, BootOptions { defer_models: true, ..Default::default() }).await;
    let status = services.engine.task_status();
    let of = |task: Task| status.iter().find(|s| s.task == task).unwrap();
    assert_eq!(of(Task::Capture).connection, "local");
    assert_eq!(services.engine.config().llm.endpoint, None, "the older address in the file no longer decides");
    assert_eq!((of(Task::Live).connection.as_str(), of(Task::Live).model.as_str()), ("NAS", "nas-gemma"));
    assert_eq!(of(Task::CardMeanings).connection, "NAS");
    assert_eq!(of(Task::CardMeanings).fallbacks, ["This PC's model", "the dictionary"]);

    // A live line is translated by the server although captures are local.
    let mut capture = Capture::typed("학교에 갑니다.");
    capture.source = maru_core::CaptureSource::Continuous;
    let line = crate::LiveLine { capture, appeared: std::time::Instant::now(), ready_ms: 0, ocr_ms: 0 };
    let events = collect(services.engine.translate_live(line).1).await;
    assert!(events.iter().any(|e| matches!(e, Event::TranslationDone { .. })), "{events:?}");
    assert!(!services.llm_server_running(), "this PC's model was not started for it");
}
