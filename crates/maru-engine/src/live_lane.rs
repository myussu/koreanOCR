//! Optional live-only scheduling and timings. Pending lines are kept, never superseded.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::continuous::LiveLine;
use crate::engine::{EventTx, Inner};

#[derive(Default)]
pub(crate) struct Counters {
    waiting: AtomicUsize,
    active: AtomicUsize,
}
impl Counters {
    pub(crate) fn counts(&self) -> (usize, usize) { (self.waiting.load(Ordering::Relaxed), self.active.load(Ordering::Relaxed)) }
}

pub(crate) struct Timing {
    counts: Arc<Counters>,
    id: u64,
    submitted: Instant,
    appeared: Instant,
    request: Option<Instant>,
    first: Option<Instant>,
    finished: bool,
}

impl Timing {
    pub(crate) fn new(counts: Arc<Counters>, id: u64, line: &LiveLine) -> Self {
        let waiting = counts.waiting.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(id, phase = "submitted", appeared_ms = ms(line.appeared), read_ms = line.ready_ms,
            ocr_ms = line.ocr_ms, waiting, active = counts.active.load(Ordering::Relaxed), "live request");
        Self { counts, id, submitted: Instant::now(), appeared: line.appeared, request: None, first: None, finished: false }
    }

    pub(crate) fn admitted(&self) {
        tracing::info!(id = self.id, phase = "admitted", submit_ms = ms(self.submitted), "live request");
    }

    pub(crate) fn request(&mut self, turns: usize) {
        self.request = Some(Instant::now());
        self.counts.waiting.fetch_sub(1, Ordering::Relaxed);
        let active = self.counts.active.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(id = self.id, phase = "model_start", submit_ms = ms(self.submitted),
            appeared_ms = ms(self.appeared), turns, active, waiting = self.counts.waiting.load(Ordering::Relaxed), "live request");
    }

    pub(crate) fn first(&mut self) {
        if self.first.is_none() {
            self.first = Some(Instant::now());
            tracing::info!(id = self.id, phase = "first_text", request_ms = self.request.map(ms),
                appeared_ms = ms(self.appeared), "live request");
        }
    }

    pub(crate) fn finish(&mut self, outcome: &str) {
        if self.finished {
            return;
        }
        self.finished = true;
        if self.request.is_some() {
            self.counts.active.fetch_sub(1, Ordering::Relaxed);
        } else {
            self.counts.waiting.fetch_sub(1, Ordering::Relaxed);
        }
        tracing::info!(id = self.id, phase = "finished", outcome, submit_ms = ms(self.submitted),
            request_ms = self.request.map(ms), generation_ms = self.first.map(ms), appeared_ms = ms(self.appeared),
            active = self.counts.active.load(Ordering::Relaxed), waiting = self.counts.waiting.load(Ordering::Relaxed), "live request");
    }
}

impl Drop for Timing {
    fn drop(&mut self) {
        self.finish("unfinished");
    }
}

fn ms(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub(crate) struct Job {
    pub id: u64,
    pub line: LiveLine,
    pub tx: EventTx,
    pub timing: Timing,
}

pub(crate) fn queue(runtime: &tokio::runtime::Handle, inner: Weak<Inner>, limit: usize) -> mpsc::UnboundedSender<Job> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
    runtime.spawn(async move {
        let mut running = JoinSet::new();
        loop {
            if running.len() >= limit {
                let _ = running.join_next().await;
                continue;
            }
            tokio::select! {
                job = rx.recv() => {
                    let Some(job) = job else { break };
                    let Some(inner) = inner.upgrade() else { break };
                    running.spawn(crate::pipeline::live(inner, job.id, job.line, job.tx, job.timing));
                }
                _ = running.join_next(), if !running.is_empty() => {}
            }
        }
        while running.join_next().await.is_some() {}
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use maru_core::Capture;

    #[test]
    fn a_failed_or_dropped_live_request_releases_its_diagnostic_counts() {
        let counts = Arc::new(Counters::default());
        let line = LiveLine { capture: Capture::typed("안녕"), appeared: Instant::now(), ready_ms: 0, ocr_ms: 0 };
        {
            let mut timing = Timing::new(counts.clone(), 1, &line);
            assert_eq!(counts.waiting.load(Ordering::Relaxed), 1);
            timing.request(0);
            assert_eq!(counts.waiting.load(Ordering::Relaxed), 0);
            assert_eq!(counts.active.load(Ordering::Relaxed), 1);
            timing.finish("error");
            timing.finish("error");
        }
        assert_eq!(counts.active.load(Ordering::Relaxed), 0);
        drop(Timing::new(counts.clone(), 2, &line));
        assert_eq!(counts.waiting.load(Ordering::Relaxed), 0);
    }
}
