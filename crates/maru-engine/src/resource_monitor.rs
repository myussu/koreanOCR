//! Demand-driven, one-second resource samples and opt-in bounded local recording.
use crate::Services;
use maru_core::{
    Event,
    resources::{ResourceSample, now_ms},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

const SESSION_LIMIT: u64 = 16 * 1024 * 1024;
const FOLDER_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Clone, Default, Serialize)]
pub struct ResourceStatus {
    pub enabled: bool,
    pub recording: bool,
    pub path: Option<String>,
    pub error: Option<String>,
    pub dropped_events: u64,
    pub queued: usize,
    pub active: usize,
    pub sample: Option<ResourceSample>,
}

struct Shared {
    status: ResourceStatus,
    interest: Instant,
}
enum Command {
    Record(bool, tokio::sync::oneshot::Sender<Result<(), String>>),
    Enable(bool, tokio::sync::oneshot::Sender<Result<(), String>>),
    Event(Value),
}

pub struct ResourceService {
    shared: Arc<Mutex<Shared>>,
    sender: mpsc::SyncSender<Command>,
    stopping: Arc<std::sync::atomic::AtomicBool>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl ResourceService {
    pub fn start(services: Arc<Services>) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            status: ResourceStatus {
                enabled: true,
                ..Default::default()
            },
            interest: Instant::now() - Duration::from_secs(10),
        }));
        let (sender, receiver) = mpsc::sync_channel(128);
        let thread_shared = shared.clone();
        let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stopping = stopping.clone();
        let thread = match std::thread::Builder::new()
            .name("maru-resources".into())
            .spawn(move || worker(services, thread_shared, receiver, thread_stopping))
        {
            Ok(thread) => Some(thread),
            Err(error) => {
                let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
                state.status.enabled = false;
                state.status.error = Some(format!("Resource monitor could not start: {error}"));
                None
            }
        };
        Self {
            shared,
            sender,
            stopping,
            thread: Mutex::new(thread),
        }
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, std::sync::atomic::Ordering::Release);
        if let Some(thread) = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = thread.join();
        }
    }
    pub fn status(&self) -> ResourceStatus {
        let mut shared = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        shared.interest = Instant::now();
        shared.status.clone()
    }
    pub fn cached_status(&self) -> ResourceStatus {
        self.shared.lock().unwrap_or_else(|p| p.into_inner()).status.clone()
    }
    pub async fn record(&self, on: bool) -> Result<(), String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.sender
            .try_send(Command::Record(on, tx))
            .map_err(|_| "Resource monitor is busy or unavailable; try again.".to_string())?;
        rx.await.map_err(|_| "Resource monitor stopped.".to_string())?
    }
    pub async fn enable(&self, on: bool) -> Result<(), String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.sender
            .try_send(Command::Enable(on, tx))
            .map_err(|_| "Resource monitor is busy or unavailable; try again.".to_string())?;
        rx.await.map_err(|_| "Resource monitor stopped.".to_string())?
    }
    pub fn event(&self, event: &Event, source: &str) {
        if !matches!(event, Event::Timing { .. } | Event::Error { .. } | Event::Done { .. }) {
            return;
        }
        let mut shared = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if !shared.status.recording {
            return;
        }
        let row = match event {
            Event::Timing { id, stage, ms } => json!({"type":"timing","at_ms":now_ms(),"source":source,"id":id,"stage":stage,"ms":ms}),
            Event::Error { id, stage, .. } => json!({"type":"request_error","at_ms":now_ms(),"source":source,"id":id,"stage":stage}),
            Event::Done { id } => json!({"type":"request_done","at_ms":now_ms(),"source":source,"id":id}),
            _ => return,
        };
        if self.sender.try_send(Command::Event(row)).is_err() {
            shared.status.dropped_events += 1;
        }
    }
}
impl Drop for ResourceService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn needed(enabled: bool, recording: bool, age: Duration) -> bool {
    enabled && (recording || age < Duration::from_secs(3))
}
fn worker(services: Arc<Services>, shared: Arc<Mutex<Shared>>, receiver: mpsc::Receiver<Command>, stopping: Arc<std::sync::atomic::AtomicBool>) {
    let mut monitor = None;
    let mut recorder: Option<Recorder> = None;
    let mut last_sample = Instant::now() - Duration::from_secs(2);
    while !stopping.load(std::sync::atomic::Ordering::Acquire) {
        match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(command) => {
                let (result, reply) = match command {
                    Command::Record(on, reply) => {
                        let enabled = shared.lock().unwrap_or_else(|p| p.into_inner()).status.enabled;
                        let result = if on && !enabled {
                            Err("Resume resource monitoring before recording.".into())
                        } else if on && recorder.is_none() {
                            Recorder::start(&services.paths.data_dir.join("resource-logs")).map(|r| recorder = Some(r))
                        } else if !on {
                            finish(&mut recorder)
                        } else {
                            Ok(())
                        };
                        (result, Some(reply))
                    }
                    Command::Enable(on, reply) => {
                        let result = if !on { finish(&mut recorder) } else { Ok(()) };
                        let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
                        state.status.enabled = on;
                        state.interest = Instant::now();
                        if !on {
                            state.status.sample = None;
                            monitor = None;
                        }
                        (result, Some(reply))
                    }
                    Command::Event(row) => (if let Some(r) = &mut recorder { r.write(&row) } else { Ok(()) }, None),
                };
                let response = result.clone();
                let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
                if let Err(error) = result {
                    state.status.error = Some(error);
                    let _ = finish(&mut recorder);
                } else if recorder.is_some() {
                    state.status.error = None;
                }
                state.status.recording = recorder.is_some();
                if let Some(r) = &recorder {
                    state.status.path = Some(r.path.display().to_string());
                }
                drop(state);
                if let Some(reply) = reply {
                    let _ = reply.send(response);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = finish(&mut recorder);
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let should_sample = {
            let state = shared.lock().unwrap_or_else(|p| p.into_inner());
            needed(state.status.enabled, recorder.is_some(), state.interest.elapsed())
        };
        if !should_sample {
            monitor = None;
            continue;
        }
        if last_sample.elapsed() < Duration::from_secs(1) {
            continue;
        }
        let sampler = monitor.get_or_insert_with(maru_platform::resources::native);
        let sample = sampler.sample(services.resource_targets());
        let (queued, active) = services.engine.live_work();
        last_sample = Instant::now();
        let dropped = shared.lock().unwrap_or_else(|p| p.into_inner()).status.dropped_events;
        let row = json!({"type":"sample","queued":queued,"active":active,"dropped_events":dropped,"sample":sample});
        let error = recorder.as_mut().and_then(|r| r.write(&row).err());
        let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
        state.status.queued = queued;
        state.status.active = active;
        state.status.sample = Some(sample);
        if let Some(error) = error {
            state.status.error = Some(error);
            let _ = finish(&mut recorder);
            state.status.recording = false;
        }
    }
    // Flush already accepted timing records when the app exits normally.
    for command in receiver.try_iter().take(128) {
        match command {
            Command::Event(row) => {
                if let Some(r) = &mut recorder {
                    let _ = r.write(&row);
                }
            }
            Command::Record(_, reply) | Command::Enable(_, reply) => {
                let _ = reply.send(Err("Resource monitor stopped.".into()));
            }
        }
    }
    let _ = finish(&mut recorder);
}

fn finish(recorder: &mut Option<Recorder>) -> Result<(), String> {
    if let Some(mut r) = recorder.take() {
        r.file.flush().map_err(|e| format!("Could not finish the resource log: {e}"))?;
    }
    Ok(())
}
struct Recorder {
    file: BufWriter<File>,
    path: PathBuf,
    bytes: u64,
    limit: u64,
    flushed: Instant,
}
impl Recorder {
    fn start(folder: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(folder).map_err(|e| format!("Could not create the resource log folder: {e}"))?;
        let mut used = 0u64;
        for item in std::fs::read_dir(folder).map_err(|e| e.to_string())? {
            let entry = item.map_err(|e| e.to_string())?;
            if entry.file_name().to_string_lossy().starts_with("resources-") && entry.path().extension().is_some_and(|x| x == "jsonl") {
                used = used.saturating_add(entry.metadata().map_err(|e| e.to_string())?.len());
            }
        }
        if used >= FOLDER_LIMIT {
            return Err("Resource logs have reached 64 MiB. Move or remove old logs before recording again.".into());
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = folder.join(format!(
            "resources-{}-{}-{}.jsonl",
            now_ms(),
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("Could not create a resource log: {e}"))?;
        let mut out = Self {
            file: BufWriter::new(file),
            path,
            bytes: 0,
            limit: SESSION_LIMIT.min(FOLDER_LIMIT - used),
            flushed: Instant::now(),
        };
        out.write(&json!({"type":"session","version":1,"at_ms":now_ms(),"sample_interval_ms":1000,"cpu":"percent of all logical CPUs","gpu":"busiest engine; process percentages are not additive","ram":"working set","memory":"bytes; shared allocations may overlap","text_recorded":false}))?;
        out.file.flush().map_err(|e| e.to_string())?;
        Ok(out)
    }
    fn write(&mut self, row: &Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(row).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        if self.bytes.saturating_add(bytes.len() as u64) > self.limit {
            return Err("Resource recording stopped at its size limit. The completed log is retained.".into());
        }
        self.file.write_all(&bytes).map_err(|e| format!("Could not write the resource log: {e}"))?;
        self.bytes += bytes.len() as u64;
        if self.flushed.elapsed() >= Duration::from_secs(5) {
            self.file.flush().map_err(|e| e.to_string())?;
            self.flushed = Instant::now();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sampling_stops_without_a_viewer_unless_recording_and_pause_always_wins() {
        assert!(!needed(true, false, Duration::from_secs(4)));
        assert!(needed(true, false, Duration::from_secs(1)));
        assert!(needed(true, true, Duration::from_secs(100)));
        assert!(!needed(false, true, Duration::ZERO));
    }
    #[test]
    fn a_bounded_log_stops_without_truncating_or_deleting_earlier_records() {
        let folder = std::env::temp_dir().join(format!("maru-resources-{}-{}", std::process::id(), now_ms()));
        let mut recorder = Recorder::start(&folder).unwrap();
        let path = recorder.path.clone();
        let row = json!({"type":"timing","id":1,"ms":123});
        recorder.write(&row).unwrap();
        recorder.limit = recorder.bytes;
        assert!(recorder.write(&row).is_err());
        finish(&mut Some(recorder)).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let rows: Vec<Value> = content.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], row);
        assert_eq!(rows[0]["text_recorded"], false);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
    #[tokio::test]
    async fn pause_flushes_recording_and_request_logs_do_not_copy_subtitle_or_error_text() {
        let folder = std::env::temp_dir().join(format!("maru-resource-service-{}-{}", std::process::id(), now_ms()));
        let services = Arc::new(
            crate::boot(
                maru_core::Paths::under(&folder),
                maru_core::Config::default(),
                crate::BootOptions { start_llm: false, ..Default::default() },
            )
            .await,
        );
        let service = ResourceService::start(services);
        service.cached_status();
        assert!(service.shared.lock().unwrap().interest.elapsed() >= Duration::from_secs(3));
        service.record(true).await.unwrap();
        let recording = service.status();
        assert!(recording.recording);
        service.event(
            &Event::Note {
                id: 1,
                text: "private subtitle".into(),
            },
            "live",
        );
        service.event(
            &Event::Error {
                id: 1,
                stage: maru_core::Stage::Llm,
                message: "private error".into(),
            },
            "live",
        );
        service.event(
            &Event::Timing {
                id: 1,
                stage: maru_core::Stage::FirstToken,
                ms: 123,
            },
            "live",
        );
        service.enable(false).await.unwrap();
        let paused = service.status();
        assert!(!paused.enabled);
        assert!(!paused.recording);
        assert!(paused.sample.is_none());
        assert!(service.record(true).await.is_err());
        let text = std::fs::read_to_string(recording.path.unwrap()).unwrap();
        assert!(text.contains("first_token"));
        assert!(text.contains("request_error"));
        assert!(!text.contains("private"));
        service.enable(true).await.unwrap();
        assert!(service.status().enabled);
        drop(service);
        // These are test-created files only; leave database cleanup to the temp directory.
    }
}
