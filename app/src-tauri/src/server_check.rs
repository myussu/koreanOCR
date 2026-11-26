//! Settings draft checks do not alter configuration or the running services.

#[tauri::command]
pub async fn test_model_connection(endpoint: String, model_name: String, reasoning_effort: String, provider: Option<maru_core::external_servers::ServerProvider>, api_key: Option<maru_core::external_servers::ServerKey>) -> Result<maru_engine::ConnectionReply, String> {
    maru_engine::test_authenticated_connection(&endpoint, &model_name, &reasoning_effort, provider.unwrap_or_default(), api_key).await
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
