use super::*;
use image::{Rgb, RgbImage};
use maru_core::{Point, Rect};
use maru_ocr::{OcrEngine, OcrLine, OcrOutput};
use maru_platform::{ScreenGrabber, app_window::AppWindow};
use std::path::Path;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct GameScreen {
    window: Mutex<Option<AppWindow>>,
    helper: Mutex<Option<AppWindow>>,
    foreground: Mutex<Option<AppInfo>>,
    page: Mutex<u8>,
    requests: Mutex<Vec<Rect>>,
}
impl GameScreen {
    fn new() -> Self {
        let app = AppInfo {
            exe: "hollow_knight.exe".into(),
            title: "Hollow Knight".into(),
        };
        Self {
            window: Mutex::new(Some(AppWindow {
                app: app.clone(),
                rect: Rect::new(100.0, 100.0, 1000.0, 600.0),
                identity: 1,
            })),
            helper: Mutex::new(None),
            foreground: Mutex::new(Some(app)),
            page: Mutex::new(1),
            requests: Mutex::new(Vec::new()),
        }
    }
    fn page(&self, n: u8) {
        *self.page.lock().unwrap() = n;
    }
    fn foreground(&self, exe: &str) {
        *self.foreground.lock().unwrap() = Some(AppInfo {
            exe: exe.into(),
            title: exe.into(),
        });
    }
    fn show_helper(&self) {
        *self.helper.lock().unwrap() = Some(AppWindow {
            app: AppInfo {
                exe: "hollow_knight.exe".into(),
                title: "Game helper".into(),
            },
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            identity: 2,
        });
    }
}
impl ScreenGrabber for GameScreen {
    fn grab(&self, rect: Rect) -> maru_platform::Result<RgbImage> {
        self.requests.lock().unwrap().push(rect);
        let page = *self.page.lock().unwrap();
        let mut image = RgbImage::from_pixel(32, 16, Rgb([page.saturating_mul(60), 0, 0]));
        image.put_pixel(0, 0, Rgb([page, 0, 0]));
        Ok(image)
    }
    fn cursor(&self) -> Option<Point> {
        None
    }
    fn foreground_app(&self) -> Option<AppInfo> {
        self.foreground.lock().unwrap().clone()
    }
    fn screen_bounds(&self) -> Rect {
        Rect::new(-2000.0, -1000.0, 6000.0, 4000.0)
    }
    fn app_window(&self, exe: &str) -> Option<AppWindow> {
        if let Some(helper) = self
            .helper
            .lock()
            .unwrap()
            .as_ref()
            .filter(|w| w.app.exe.eq_ignore_ascii_case(exe))
        {
            return Some(helper.clone());
        }
        self.window
            .lock()
            .unwrap()
            .clone()
            .filter(|w| w.app.exe.eq_ignore_ascii_case(exe))
    }
    fn app_window_by_id(&self, exe: &str, identity: u64) -> Option<AppWindow> {
        self.window
            .lock()
            .unwrap()
            .clone()
            .filter(|w| w.identity == identity)
            .or_else(|| {
                self.helper
                    .lock()
                    .unwrap()
                    .clone()
                    .filter(|w| w.identity == identity)
            })
            .filter(|w| w.identity == identity && w.app.exe.eq_ignore_ascii_case(exe))
    }
}
struct ReadingOcr {
    calls: AtomicUsize,
    switch_foreground: AtomicBool,
    show_helper: AtomicBool,
    screen: Arc<GameScreen>,
}
impl OcrEngine for ReadingOcr {
    fn name(&self) -> &'static str {
        "scripted paragraph OCR"
    }
    fn recognize(&self, image: &RgbImage) -> maru_ocr::Result<OcrOutput> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.switch_foreground.swap(false, Ordering::Relaxed) {
            self.screen.foreground("other_game.exe");
        }
        if self.show_helper.swap(false, Ordering::Relaxed) {
            self.screen.show_helper();
            self.screen.foreground("maru-app.exe");
        }
        let text = match image.get_pixel(0, 0)[0] {
            1 => SENTENCE,
            2 => "아직 여기 있어요",
            3 => "카메라 설정",
            _ => "",
        };
        Ok(OcrOutput {
            lines: if text.is_empty() {
                vec![]
            } else {
                vec![OcrLine {
                    text: text.into(),
                    rect: Rect::new(0.0, 0.0, 200.0, 30.0),
                    words: vec![],
                    confidence: 0.99,
                }]
            },
        })
    }
}
struct ReadingHarness {
    engine: Engine,
    screen: Arc<GameScreen>,
    ocr: Arc<ReadingOcr>,
    server: MockServer,
}
async fn reading_harness(path: &Path) -> ReadingHarness {
    let server = spawn_mock(model).await;
    let cfg = Config::default();
    let screen = Arc::new(GameScreen::new());
    let ocr = Arc::new(ReadingOcr {
        calls: AtomicUsize::new(0),
        switch_foreground: AtomicBool::new(false),
        show_helper: AtomicBool::new(false),
        screen: screen.clone(),
    });
    let dict = Dictionary::open_in_memory().unwrap();
    dict.import_kaikki(Cursor::new(KAIKKI), |_| {}).unwrap();
    let engine = Engine::new(EngineParts {
        config: cfg.clone(),
        analyzer: Analyzer::heuristic(),
        dict: Some(Arc::new(dict)),
        vocab: Arc::new(Vocab::open_in_memory(SrsConfig::from_learning(&cfg.learning)).unwrap()),
        llm: Some(LlmClient::new(&server.url, &cfg.llm)),
        llm_status: None,
        platform: Platform {
            selection: Arc::new(FakeSelection::default()),
            screen: screen.clone(),
            os_ocr: None,
        },
        ocr: Some(ocr.clone()),
        scan_ocr: None,
        live_ocr: None,
        live_llm: None,
        labels: Some(Arc::new(
            maru_engine::labels::LabelCache::open_in_memory().unwrap(),
        )),
    });
    engine.open_reading_areas(path).unwrap();
    engine.reading_context();
    ReadingHarness {
        engine,
        screen,
        ocr,
        server,
    }
}
fn save(h: &ReadingHarness, name: &str) -> i64 {
    h.engine.begin_reading_pick(name, None).unwrap();
    h.engine
        .finish_reading_pick(Some(Rect::new(200.0, 160.0, 600.0, 150.0)))
        .unwrap()
        .unwrap()
        .id
}
async fn settle(h: &ReadingHarness) {
    h.engine.observe_reading().await;
    tokio::time::sleep(Duration::from_millis(710)).await;
    h.engine.observe_reading().await;
    tokio::time::sleep(Duration::from_millis(710)).await;
    h.engine.observe_reading().await;
}

#[tokio::test]
async fn a_saved_area_reads_the_full_paragraph_with_meanings_and_its_original_game() {
    let h = reading_harness(Path::new(":memory:")).await;
    save(&h, "Dialogue");
    h.engine.request_reading().await.unwrap();
    let job = h.engine.take_reading_job().unwrap();
    assert_eq!(job.capture.text, SENTENCE);
    assert_eq!(job.capture.app.as_ref().unwrap().exe, "hollow_knight.exe");
    assert!(!h.engine.reading_status().automatic);
    let events = collect(h.engine.translate(job.capture).1).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Event::Breakdown {eojeols,..} if eojeols.len()>1))
    );
    assert!(events.iter().any(|e| matches!(e, Event::Gloss { .. })));
    assert!(
        events
            .iter()
            .any(|e| matches!(e,Event::TranslationDone {text,..} if text=="I studied at school."))
    );
    h.engine.finish_reading(job.serial, None);
    h.engine.request_reading().await.unwrap();
    let repeated = h.engine.take_reading_job().unwrap();
    let cached = collect(h.engine.translate(repeated.capture).1).await;
    assert!(
        cached
            .iter()
            .any(|e| matches!(e, Event::Started { cached: true, .. }))
    );
    assert_eq!(
        h.server.requests.lock().unwrap().len(),
        2,
        "translation plus meanings; repeat uses the normal cache"
    );
    let eojeols = events
        .iter()
        .find_map(|event| {
            if let Event::Breakdown { eojeols, .. } = event {
                Some(eojeols)
            } else {
                None
            }
        })
        .unwrap();
    for word in eojeols.iter().filter(|word| word.content) {
        let deep = collect(
            h.engine
                .deep_dive(DeepDiveRequest {
                    eojeol: word.clone(),
                    sentence: SENTENCE.into(),
                    translation: Some("I studied at school.".into()),
                    meaning: None,
                    app: Some(AppInfo {
                        exe: "hollow_knight.exe".into(),
                        title: "Hollow Knight".into(),
                    }),
                })
                .1,
        )
        .await;
        assert!(
            deep.iter()
                .any(|event| matches!(event, Event::DeepDelta { .. })),
            "every content word can receive an explanation: {}",
            word.surface
        );
    }
}

#[tokio::test]
async fn moving_the_game_moves_the_saved_crop_and_resizing_requires_an_explicit_check() {
    let h = reading_harness(Path::new(":memory:")).await;
    save(&h, "Dialogue");
    h.screen.window.lock().unwrap().as_mut().unwrap().rect.x = 400.0;
    h.engine.request_reading().await.unwrap();
    assert_eq!(h.screen.requests.lock().unwrap().last().unwrap().x, 500.0);
    let job = h.engine.take_reading_job().unwrap();
    h.engine.finish_reading(job.serial, None);
    h.screen.window.lock().unwrap().as_mut().unwrap().rect = Rect::new(0.0, 0.0, 2000.0, 1200.0);
    let before = h.screen.requests.lock().unwrap().len();
    assert!(
        h.engine
            .request_reading()
            .await
            .unwrap_err()
            .to_string()
            .contains("changed size")
    );
    assert!(h.engine.reading_status().position_check);
    assert_eq!(h.screen.requests.lock().unwrap().len(), before);
    h.engine.confirm_reading_position().unwrap();
    h.engine.request_reading().await.unwrap();
    assert_eq!(
        *h.screen.requests.lock().unwrap().last().unwrap(),
        Rect::new(200.0, 120.0, 1200.0, 300.0)
    );
}

#[tokio::test]
async fn opening_the_area_picker_from_maru_uses_the_games_current_borderless_rectangle() {
    let h = reading_harness(Path::new(":memory:")).await;
    h.screen.foreground("maru-app.exe");
    let borderless = Rect::new(0.0, 0.0, 1920.0, 1080.0);
    h.screen.window.lock().unwrap().as_mut().unwrap().rect = borderless;

    h.engine.begin_reading_pick("Dialogue", None).unwrap();
    h.engine.reading_context();
    let area = h
        .engine
        .finish_reading_pick(Some(Rect::new(300.0, 120.0, 1200.0, 300.0)))
        .unwrap()
        .unwrap();

    assert_eq!(
        area.rectangle(borderless),
        Rect::new(300.0, 120.0, 1200.0, 300.0)
    );
    assert_eq!(area.exe, "hollow_knight.exe");
    assert_eq!(area.reference_width, 1920.0);
    assert!(!h.engine.reading_picker_open());
}

#[tokio::test]
async fn a_game_that_disappeared_while_the_manager_was_open_is_refused_before_showing_the_picker() {
    let h = reading_harness(Path::new(":memory:")).await;
    h.screen.foreground("maru-app.exe");
    *h.screen.window.lock().unwrap() = None;

    assert!(h.engine.begin_reading_pick("Dialogue", None).is_err());
    assert!(!h.engine.reading_picker_open());
    assert!(h.engine.reading_status().areas.is_empty());
}

#[tokio::test]
async fn a_window_that_moves_after_the_picker_opens_is_still_refused() {
    let h = reading_harness(Path::new(":memory:")).await;
    h.engine.begin_reading_pick("Dialogue", None).unwrap();
    h.screen.foreground("maru-app.exe");
    h.screen.window.lock().unwrap().as_mut().unwrap().rect.x += 20.0;

    assert!(
        h.engine
            .finish_reading_pick(Some(Rect::new(200.0, 160.0, 600.0, 150.0)))
            .unwrap_err()
            .to_string()
            .contains("moved or changed size")
    );
    assert!(h.engine.reading_status().areas.is_empty());
}

#[tokio::test]
async fn a_same_executable_helper_after_picker_focus_cannot_replace_the_selected_game_window() {
    let h = reading_harness(Path::new(":memory:")).await;
    h.engine.begin_reading_pick("Dialogue", None).unwrap();
    h.screen.foreground("maru-app.exe");
    h.screen.show_helper();
    assert_eq!(
        h.screen.app_window("hollow_knight.exe").unwrap().identity,
        2
    );

    let area = h
        .engine
        .finish_reading_pick(Some(Rect::new(200.0, 160.0, 600.0, 150.0)))
        .unwrap()
        .unwrap();
    assert_eq!(area.reference_width, 1000.0);
    assert_eq!(area.reference_height, 600.0);
    assert_eq!(h.engine.reading_status().areas.len(), 1);
}

#[tokio::test]
async fn a_new_pick_from_the_manager_keeps_the_known_game_when_a_helper_has_higher_window_order() {
    let h = reading_harness(Path::new(":memory:")).await;
    h.screen.foreground("maru-app.exe");
    h.screen.show_helper();
    h.engine.begin_reading_pick("Dialogue", None).unwrap();
    assert!(
        h.engine
            .finish_reading_pick(Some(Rect::new(200.0, 160.0, 600.0, 150.0)))
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_disappeared_selected_window_cannot_be_replaced_by_a_same_executable_helper() {
    let h = reading_harness(Path::new(":memory:")).await;
    h.engine.begin_reading_pick("Dialogue", None).unwrap();
    h.screen.foreground("maru-app.exe");
    h.screen.show_helper();
    *h.screen.window.lock().unwrap() = None;
    assert!(
        h.engine
            .finish_reading_pick(Some(Rect::new(200.0, 160.0, 600.0, 150.0)))
            .is_err()
    );
    assert!(h.engine.reading_status().areas.is_empty());
}

#[tokio::test]
async fn a_helper_window_after_manual_ocr_does_not_discard_the_original_games_paragraph() {
    let h = reading_harness(Path::new(":memory:")).await;
    save(&h, "Dialogue");
    h.ocr.show_helper.store(true, Ordering::Relaxed);
    h.engine.request_reading().await.unwrap();
    let job = h.engine.take_reading_job().unwrap();
    assert_eq!(job.capture.text, SENTENCE);
    assert_eq!(job.capture.app.as_ref().unwrap().title, "Hollow Knight");
}

#[tokio::test]
async fn a_same_executable_helper_cannot_change_the_saved_areas_status_reposition_or_size_check() {
    let h = reading_harness(Path::new(":memory:")).await;
    let id = save(&h, "Dialogue");
    h.screen.foreground("maru-app.exe");
    h.screen.show_helper();

    assert!(!h.engine.reading_status().position_check);
    h.engine.select_reading_area(id).unwrap();
    assert!(!h.engine.reading_status().position_check);
    h.engine.request_reading().await.unwrap();
    let job = h.engine.take_reading_job().unwrap();
    h.engine.finish_reading(job.serial, None);

    h.engine.begin_reading_pick("Dialogue", Some(id)).unwrap();
    assert_eq!(
        h.engine.reading_picker_hint().unwrap().1,
        Some(Rect::new(200.0, 160.0, 600.0, 150.0))
    );
    let area = h
        .engine
        .finish_reading_pick(Some(Rect::new(250.0, 200.0, 500.0, 120.0)))
        .unwrap()
        .unwrap();
    assert_eq!(area.id, id);
    assert_eq!(area.reference_width, 1000.0);

    h.screen.window.lock().unwrap().as_mut().unwrap().rect.w = 1200.0;
    assert!(h.engine.reading_status().position_check);
    h.engine.confirm_reading_position().unwrap();
    let status = h.engine.reading_status();
    assert!(!status.position_check);
    assert_eq!(status.areas[0].reference_width, 1200.0);
    assert_eq!(status.areas[0].reference_height, 600.0);
}

#[tokio::test]
async fn reopening_the_game_keeps_the_existing_area_available_for_manual_reposition() {
    let h = reading_harness(Path::new(":memory:")).await;
    let id = save(&h, "Dialogue");
    h.screen.foreground("maru-app.exe");
    h.screen.window.lock().unwrap().as_mut().unwrap().identity = 3;
    h.engine.begin_reading_pick("Dialogue", Some(id)).unwrap();
    let area = h
        .engine
        .finish_reading_pick(Some(Rect::new(200.0, 160.0, 600.0, 150.0)))
        .unwrap()
        .unwrap();
    assert_eq!(area.id, id);
    assert_eq!(h.engine.reading_status().areas.len(), 1);
}

#[tokio::test]
async fn a_moved_window_after_capture_cannot_receive_a_stale_answer_at_its_old_position() {
    let h = reading_harness(Path::new(":memory:")).await;
    save(&h, "Dialogue");
    h.engine.request_reading().await.unwrap();
    h.screen.window.lock().unwrap().as_mut().unwrap().rect.x += 20.0;
    assert!(h.engine.take_reading_job().is_none());
    assert_eq!(h.engine.reading_status().phase, "paused");
    assert!(h.server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn repeated_requests_keep_only_the_latest_pending_paragraph_and_ignore_older_failures() {
    let h = reading_harness(Path::new(":memory:")).await;
    save(&h, "Dialogue");
    h.engine.request_reading().await.unwrap();
    let first = h.engine.take_reading_job().unwrap();
    h.screen.page(2);
    h.engine.request_reading().await.unwrap();
    h.screen.page(3);
    h.engine.request_reading().await.unwrap();
    assert!(h.engine.take_reading_job().is_none());
    assert!(!h.engine.accepts_reading(first.serial));
    h.engine
        .finish_reading(first.serial, Some("old failure".into()));
    let last = h.engine.take_reading_job().unwrap();
    assert_eq!(last.capture.text, "카메라 설정");
    assert!(h.engine.accepts_reading(last.serial));
    assert_eq!(h.engine.reading_status().message, None);
}

#[tokio::test]
async fn automatic_reading_confirms_text_ignores_duplicates_and_treats_blank_dialogue_as_normal() {
    let h = reading_harness(Path::new(":memory:")).await;
    save(&h, "Dialogue");
    h.engine.set_reading_auto(true).unwrap();
    h.engine.observe_reading().await;
    assert!(h.engine.take_reading_job().is_none());
    tokio::time::sleep(Duration::from_millis(710)).await;
    h.engine.observe_reading().await;
    assert!(h.engine.take_reading_job().is_none());
    tokio::time::sleep(Duration::from_millis(710)).await;
    h.engine.observe_reading().await;
    let first = h.engine.take_reading_job().unwrap();
    h.engine.finish_reading(first.serial, None);
    let calls = h.ocr.calls.load(Ordering::Relaxed);
    h.engine.observe_reading().await;
    assert_eq!(h.ocr.calls.load(Ordering::Relaxed), calls);
    assert!(h.engine.take_reading_job().is_none());
    h.screen.page(0);
    settle(&h).await;
    assert_eq!(h.engine.reading_status().phase, "empty");
    assert!(h.engine.take_reading_job().is_none());
    assert!(h.engine.reading_status().message.is_none());
    h.screen.page(1);
    settle(&h).await;
    assert!(
        h.engine.take_reading_job().is_some(),
        "the same paragraph can return after a confirmed blank box"
    );
}

#[tokio::test]
async fn switching_games_and_holding_an_answer_stop_capture_without_forgetting_the_saved_area() {
    let h = reading_harness(Path::new(":memory:")).await;
    let id = save(&h, "Dialogue");
    h.engine.set_reading_auto(true).unwrap();
    h.screen.foreground("other_game.exe");
    h.engine.observe_reading().await;
    assert!(h.screen.requests.lock().unwrap().is_empty());
    assert_eq!(h.engine.reading_status().phase, "paused");
    assert_eq!(h.engine.reading_status().selected, Some(id));
    h.screen.foreground("hollow_knight.exe");
    settle(&h).await;
    let first = h.engine.take_reading_job().unwrap();
    h.engine.hold_reading_for_interaction();
    assert!(
        h.engine.accepts_reading(first.serial),
        "holding keeps the current paragraph's remaining output readable"
    );
    h.engine.finish_reading(first.serial, None);
    let before = h.screen.requests.lock().unwrap().len();
    h.screen.page(2);
    h.engine.observe_reading().await;
    assert_eq!(h.screen.requests.lock().unwrap().len(), before);
    h.engine.set_reading_hold(false);
    settle(&h).await;
    assert_eq!(
        h.engine.take_reading_job().unwrap().capture.text,
        "아직 여기 있어요"
    );
}

#[tokio::test]
async fn a_foreground_change_during_ocr_is_refused_before_any_model_request() {
    let h = reading_harness(Path::new(":memory:")).await;
    save(&h, "Dialogue");
    h.ocr.switch_foreground.store(true, Ordering::Relaxed);
    assert!(h.engine.request_reading().await.is_err());
    assert!(h.engine.take_reading_job().is_none());
    assert!(h.server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn holding_or_stopping_auto_keeps_the_current_answer_finishing_but_another_snapshot_supersedes_it()
 {
    let h = reading_harness(Path::new(":memory:")).await;
    save(&h, "Dialogue");
    h.engine.set_reading_auto(true).unwrap();
    h.engine.request_reading().await.unwrap();
    let first = h.engine.take_reading_job().unwrap();
    h.screen.page(2);
    h.engine.request_reading().await.unwrap();
    assert!(!h.engine.accepts_reading(first.serial));
    h.engine.hold_reading_for_interaction();
    assert!(h.engine.accepts_reading(first.serial));
    assert!(h.engine.reading_status().held);
    h.engine.set_reading_auto(false).unwrap();
    assert!(h.engine.accepts_reading(first.serial));
    h.screen.foreground("other_game.exe");
    h.engine.reading_context();
    assert!(
        h.engine.accepts_reading(first.serial),
        "an already requested manual paragraph retains its game after switching apps"
    );
    h.engine.interrupt_reading();
    assert!(!h.engine.accepts_reading(first.serial));
    h.engine
        .finish_reading(first.serial, Some("superseded".into()));
    assert_eq!(h.engine.reading_status().message, None);
}

#[tokio::test]
async fn invalid_or_moved_selections_and_duplicate_names_leave_saved_areas_intact() {
    let h = reading_harness(Path::new(":memory:")).await;
    let first = save(&h, "Dialogue");
    assert!(h.engine.begin_reading_pick(" ", None).is_err());
    for rect in [
        Rect::new(0.0, 0.0, 20.0, 20.0),
        Rect::new(200.0, 160.0, 1.0, 20.0),
        Rect::new(f32::NAN, 160.0, 20.0, 20.0),
    ] {
        h.engine.begin_reading_pick("Other", None).unwrap();
        assert!(h.engine.finish_reading_pick(Some(rect)).is_err());
    }
    h.engine.begin_reading_pick("dialogue", None).unwrap();
    assert!(
        h.engine
            .finish_reading_pick(Some(Rect::new(200.0, 160.0, 600.0, 150.0)))
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    h.engine.begin_reading_pick("Other", None).unwrap();
    h.screen.window.lock().unwrap().as_mut().unwrap().identity = 2;
    assert!(
        h.engine
            .finish_reading_pick(Some(Rect::new(200.0, 160.0, 600.0, 150.0)))
            .is_err()
    );
    assert_eq!(h.engine.reading_status().areas.len(), 1);
    assert_eq!(h.engine.reading_status().selected, Some(first));
}

#[tokio::test]
async fn named_areas_and_each_games_selection_survive_reopening_while_automatic_reading_starts_off()
{
    let path = std::env::temp_dir().join(format!(
        "maru-reading-{}-{}.db",
        std::process::id(),
        maru_srs::now_secs()
    ));
    let h = reading_harness(&path).await;
    let first = save(&h, "Dialogue");
    let second = save(&h, "Journal");
    h.engine.select_reading_area(first).unwrap();
    h.engine.rename_reading_area(second, "Items").unwrap();
    assert_eq!(h.engine.reading_status().selected, Some(first));
    let original = h.screen.window.lock().unwrap().clone().unwrap();
    h.screen.window.lock().unwrap().as_mut().unwrap().app = AppInfo {
        exe: "other_game.exe".into(),
        title: "Other game".into(),
    };
    h.screen.foreground("other_game.exe");
    let other = save(&h, "Dialogue");
    assert_ne!(other, first);
    *h.screen.window.lock().unwrap() = Some(original);
    h.screen.foreground("hollow_knight.exe");
    h.engine.reading_context();
    assert_eq!(h.engine.reading_status().selected, Some(first));
    h.engine.set_reading_auto(true).unwrap();
    drop(h);
    let reopened = reading_harness(&path).await;
    let status = reopened.engine.reading_status();
    assert_eq!(status.selected, Some(first));
    assert!(!status.automatic);
    assert!(status.areas.iter().any(|a| a.name == "Items"));
    reopened.engine.remove_reading_area(first).unwrap();
    assert_eq!(reopened.engine.reading_status().selected, None);
    assert!(reopened.engine.select_reading_area(first).is_err());
    assert_eq!(reopened.engine.reading_status().areas.len(), 2);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}
