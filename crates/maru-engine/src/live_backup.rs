//! A local backup for live lines when the translation server does not answer: a crashed or reloading Ollama runner,
//! a cloud rate limit, a dropped connection. The server is asked first. If it fails before any text, or has not
//! started answering in time, the line goes to the backup model, and the next lines skip the server for a short rest.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use maru_core::live_model::LiveModel;
use maru_llm::{LiveRequest, LlmChunk, LlmClient, Result};

use crate::engine::{Inner, lock};

/// After a failure, lines go straight to the backup this long before the server is asked again.
const SERVER_REST: Duration = Duration::from_secs(20);

pub(crate) struct Backup {
    pub client: LlmClient,
    pub model: LiveModel,
}

/// The configured backup, when live lines go to the main (server) model.
pub(crate) fn backup(inner: &Inner) -> Option<Backup> {
    let model = inner.cfg.continuous.translation.backup_model.filter(|m| *m != LiveModel::Main)?;
    Some(Backup { client: inner.live_llm.clone()?, model })
}

pub(crate) enum Answer {
    Server(Result<()>),
    Backup(Result<()>),
}

/// Translate one line with the server, or the backup when the server fails or is late. `started` is set by `sink`
/// once text arrives: a server that has begun answering is never interrupted.
pub(crate) async fn translate(
    inner: &Inner,
    server: &LlmClient,
    backup: &Backup,
    req: &LiveRequest<'_>,
    backup_req: &LiveRequest<'_>,
    started: &AtomicBool,
    sink: &mut (dyn FnMut(LlmChunk) + Send),
) -> Answer {
    let resting = lock(&inner.server_rest).is_some_and(|until| Instant::now() < until);
    if !resting {
        let wait = Duration::from_millis(inner.cfg.continuous.translation.backup_after_ms.max(100));
        let outcome = {
            let deadline = tokio::time::sleep(wait);
            let call = server.translate_selected_live(LiveModel::Main, req, &mut *sink);
            tokio::pin!(deadline, call);
            let mut armed = true;
            loop {
                tokio::select! {
                    result = &mut call => break Some(result),
                    _ = &mut deadline, if armed => {
                        if started.load(Ordering::Relaxed) { armed = false } else { break None }
                    }
                }
            }
        };
        match outcome {
            Some(Ok(())) => {
                lock(&inner.server_rest).take();
                return Answer::Server(Ok(()));
            }
            // Text is already on screen: switching translators now would mix two answers.
            Some(Err(e)) if started.load(Ordering::Relaxed) => return Answer::Server(Err(e)),
            Some(Err(e)) => tracing::warn!("live translation server failed ({e}); this line uses the backup translator"),
            None => tracing::warn!("live translation server did not answer within {} ms; this line uses the backup translator", wait.as_millis()),
        }
        *lock(&inner.server_rest) = Some(Instant::now() + SERVER_REST);
        // Dropping the late request also stops Ollama from loading the model; let it finish in the background,
        // so the server is ready when its rest ends. A hosted API has nothing to load.
        if !server.is_hosted() {
            let server = server.clone();
            inner.runtime.spawn(async move {
                if let Err(e) = server.hold_server_model().await {
                    tracing::warn!("the translation server is still not answering: {e}");
                }
            });
        }
    } else {
        tracing::info!("live translation server is resting after a failure; this line uses the backup translator");
    }
    match backup.client.translate_selected_live(backup.model, backup_req, &mut *sink).await {
        Ok(()) => Answer::Backup(Ok(())),
        // A backup that cannot answer (missing file, failed load) must not cost the line: wait for the server after all.
        Err(e) if !started.load(Ordering::Relaxed) => {
            tracing::warn!("the backup translator failed ({e}); waiting for the server");
            lock(&inner.server_rest).take();
            Answer::Server(server.translate_selected_live(LiveModel::Main, req, sink).await)
        }
        Err(e) => Answer::Backup(Err(e)),
    }
}
