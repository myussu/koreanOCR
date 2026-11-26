//! Models on another computer stay loaded only while they are wanted (`[llm] server_keep_minutes`): Maru loads
//! them when it opens and when live subtitles start, every request restarts the server's countdown, and they are
//! unloaded when live subtitles stop and when Maru quits, so that computer's GPU is free for its own work.

use std::time::Duration;

use maru_core::live_model::LiveModel;
use maru_llm::LlmClient;

use crate::engine::{Engine, Inner, lock};

/// Choosing a new subtitle area stops and restarts live mode: that must not unload and reload the model.
#[cfg(not(test))]
const RELEASE_AFTER: Duration = Duration::from_secs(60);
#[cfg(test)]
const RELEASE_AFTER: Duration = Duration::from_millis(200);

/// The server models a session would use: the main model when it translates live lines (or always, outside
/// live mode), and the snapshot server that live mode sends snapshots to.
fn server_clients(inner: &Inner, for_live: bool) -> Vec<LlmClient> {
    let live = &inner.cfg.continuous.translation;
    // Live subtitles on their own connection hold that server instead of the captures' one.
    let own_live = inner.routes().live.as_ref().map(|r| r.client.clone());
    let mut clients: Vec<LlmClient> = inner
        .llm
        .iter()
        .filter(|c| c.holds_server_model() && (!for_live || (live.model == LiveModel::Main && own_live.is_none())))
        .cloned()
        .collect();
    if let Some(client) = own_live.filter(|c| c.holds_server_model() && live.model == LiveModel::Main)
        && !clients.iter().any(|c| c.same_server_model(&client))
    {
        clients.push(client);
    }
    if live.snapshot_endpoint.is_some() {
        let cfg = inner.cfg.llm.for_snapshot_server(live);
        let snapshot = LlmClient::from_config(&cfg);
        if snapshot.holds_server_model() && !clients.iter().any(|c| c.same_server_model(&snapshot)) {
            clients.push(snapshot);
        }
    }
    clients
}

async fn release(clients: Vec<LlmClient>) {
    for client in clients {
        match client.release_server_model().await {
            Ok(()) => tracing::info!("released the model on {}", client.root()),
            Err(e) => tracing::warn!("could not release the model on {}: {e}", client.root()),
        }
    }
}

impl Engine {
    /// Load the server models now (Maru opened, or live subtitles started) and cancel a pending release.
    pub fn hold_server_models(&self, for_live: bool) {
        if let Some(task) = lock(&self.inner.server_release).take() {
            task.abort();
        }
        for client in server_clients(&self.inner, for_live) {
            self.inner.runtime.spawn(async move {
                match client.hold_server_model().await {
                    Ok(()) => tracing::info!("loaded the model on {}", client.root()),
                    Err(e) => tracing::warn!("could not load the model on {}: {e}", client.root()),
                }
            });
        }
    }

    /// Unload the server models after a short grace period, unless something holds them again first.
    pub fn release_server_models_soon(&self) {
        let clients = server_clients(&self.inner, false);
        if clients.is_empty() {
            return;
        }
        let task = self.inner.runtime.spawn(async move {
            tokio::time::sleep(RELEASE_AFTER).await;
            release(clients).await;
        });
        if let Some(previous) = lock(&self.inner.server_release).replace(task) {
            previous.abort();
        }
    }

    /// Unload the server models now (Maru is quitting).
    pub async fn release_server_models(&self) {
        if let Some(task) = lock(&self.inner.server_release).take() {
            task.abort();
        }
        release(server_clients(&self.inner, false)).await;
    }
}
