//! Settings draft checks do not alter configuration or the running services.

#[tauri::command]
pub async fn test_model_connection(endpoint: String, model_name: String, reasoning_effort: String, provider: Option<maru_core::external_servers::ServerProvider>, api_key: Option<maru_core::external_servers::ServerKey>) -> Result<maru_engine::ConnectionReply, String> {
    maru_engine::test_authenticated_connection(&endpoint, &model_name, &reasoning_effort, provider.unwrap_or_default(), api_key).await
}

/// What a connection check found, for the Connections page: a state the page turns into a dot, and one plain sentence.
#[derive(serde::Serialize)]
pub struct ConnectionCheck {
    /// `working`, `no_key`, `quota`, `unreachable`, `failed`, or `input` (what was typed cannot be asked).
    state: &'static str,
    message: String,
    /// The address as it was understood (a missing scheme or port filled in).
    endpoint: Option<String>,
}

/// Ask a connection as typed (saved or not) one short fixed question. Nothing is saved and no local model starts.
#[tauri::command]
pub async fn connection_check(name: String, endpoint: String, model_name: String, reasoning_effort: Option<String>, provider: Option<maru_core::external_servers::ServerProvider>, api_key: Option<maru_core::external_servers::ServerKey>) -> ConnectionCheck {
    use maru_llm::problem::Problem;
    let name = if name.trim().is_empty() { "The server" } else { name.trim() };
    match maru_engine::check_connection(&endpoint, &model_name, reasoning_effort.as_deref().unwrap_or("none"), provider.unwrap_or_default(), api_key).await {
        Ok(reply) => ConnectionCheck { state: "working", message: format!("{name} answered in {:.1} s.", reply.took_ms as f32 / 1000.0), endpoint: Some(reply.endpoint) },
        Err(failure) => {
            let (problem, message) = failure.plain(name);
            let state = match problem {
                None => "input",
                Some(Problem::NoKey) => "no_key",
                Some(Problem::Quota) => "quota",
                Some(Problem::Unreachable) => "unreachable",
                Some(Problem::Failed) => "failed",
            };
            ConnectionCheck { state, message, endpoint: None }
        }
    }
}

#[tauri::command]
pub async fn discovery_networks() -> Result<Vec<maru_engine::server_discovery::NetworkChoice>, String> {
    maru_engine::server_discovery::local_networks().await
}

#[tauri::command]
pub async fn discovery_start(service: tauri::State<'_, maru_engine::server_discovery::DiscoveryService>, kind: String, target: String) -> Result<u32, String> {
    service.start(&kind, &target)
}

#[tauri::command]
pub fn discovery_status(service: tauri::State<maru_engine::server_discovery::DiscoveryService>, job_id: u32) -> Result<maru_engine::server_discovery::DiscoveryProgress, String> {
    service.status(job_id)
}

#[tauri::command]
pub fn discovery_cancel(service: tauri::State<maru_engine::server_discovery::DiscoveryService>, job_id: u32) -> Result<(), String> {
    service.cancel(job_id)
}
