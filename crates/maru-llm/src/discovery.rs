//! Metadata-only discovery of chat servers; never loads or downloads a model.

use std::time::Duration;
use reqwest::Url;
use serde::Serialize;
use serde_json::Value;

pub const COMMON_PORTS: [u16; 3] = [11434, 1234, 8080];

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FoundServer {
    pub endpoint: String,
    pub api: String,
    pub models: Vec<String>,
}

pub fn normalize(input: &str) -> Result<String, String> {
    let input = input.trim();
    let address = if input.contains("://") { input.to_string() } else { format!("http://{input}") };
    let mut url = Url::parse(&address).map_err(|_| "Enter an IP address, hostname, or complete server URL.".to_string())?;
    if input.is_empty() || !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Enter an IP address, hostname, or http(s) server URL.".into());
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err("Use the server base URL without credentials, a query, or a fragment.".into());
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    for suffix in ["/v1/chat/completions", "/v1/models", "/api/tags", "/api", "/v1"] {
        if let Some(base) = path.strip_suffix(suffix) { path = base.to_string(); break; }
    }
    url.set_path(&path);
    let authority = input.split_once("://").map_or(input, |(_, rest)| rest).split('/').next().unwrap_or_default();
    if let Some(port) = authority.rsplit_once(':').and_then(|(_, port)| port.parse::<u16>().ok())
        .filter(|port| url.port().is_none() && Some(*port) == url.port_or_known_default()) {
        let rendered = url.to_string();
        let (scheme, rest) = rendered.split_once("://").ok_or("Invalid server URL.")?;
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        return Ok(format!("{scheme}://{host}:{port}{path}").trim_end_matches('/').into());
    }
    Ok(url.to_string().trim_end_matches('/').into())
}

pub fn candidates(input: &str) -> Result<Vec<String>, String> {
    let base = normalize(input)?;
    let url = Url::parse(&base).map_err(|e| e.to_string())?;
    // Inspect the original authority: URL parsing removes explicit default ports (80/443).
    let original = input.trim().split_once("://").map_or(input.trim(), |(_, rest)| rest);
    let authority = original.split('/').next().unwrap_or_default();
    let explicit_port = if authority.starts_with('[') { authority.contains("]:") } else { authority.contains(':') };
    let explicit_path = original.split_once('/').is_some_and(|(_, path)| !path.trim_matches('/').is_empty());
    if explicit_port || explicit_path || url.scheme() == "https" {
        return Ok(vec![base]);
    }
    COMMON_PORTS.into_iter().map(|port| {
        let mut candidate = url.clone();
        candidate.set_port(Some(port)).map_err(|_| "The server address cannot have a port.".to_string())?;
        Ok(candidate.to_string().trim_end_matches('/').into())
    }).collect()
}

#[derive(Clone)]
pub struct ServerProbe { http: reqwest::Client }

impl ServerProbe {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder().connect_timeout(Duration::from_millis(900))
            .timeout(Duration::from_secs(2)).redirect(reqwest::redirect::Policy::none())
            .build().map_err(|e| e.to_string())?;
        Ok(Self { http })
    }

    pub async fn check(&self, endpoint: &str) -> Result<FoundServer, String> {
        let endpoint = normalize(endpoint)?;
        tokio::time::timeout(Duration::from_secs(3), self.metadata(&endpoint)).await
            .map_err(|_| format!("{endpoint}: metadata request timed out"))?
    }

    async fn metadata(&self, endpoint: &str) -> Result<FoundServer, String> {
        for (path, api, list, key) in [("/v1/models", "OpenAI-compatible", "data", "id"), ("/api/tags", "Ollama", "models", "name")] {
            let mut response = match self.http.get(format!("{endpoint}{path}")).send().await {
                Ok(response) => response,
                Err(error) if error.is_connect() => return Err(format!("{endpoint}: {error}")),
                Err(_) => continue,
            };
            let status = response.status();
            if matches!(status.as_u16(), 401 | 403) { return Err(format!("{endpoint}: server requires authentication (HTTP {status})")); }
            if !status.is_success() { continue; }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
                if bytes.len() + chunk.len() > 262_144 { return Err(format!("{endpoint}: model list is too large")); }
                bytes.extend_from_slice(&chunk);
            }
            let Ok(json) = serde_json::from_slice::<Value>(&bytes) else { continue; };
            let Some(items) = json.get(list).and_then(Value::as_array) else { continue; };
            let mut models = Vec::new();
            for item in items {
                let Some(name) = item.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty() && s.len() <= 1024) else {
                    return Err(format!("{endpoint}: invalid model list"));
                };
                models.push(name.into());
            }
            models.sort(); models.dedup();
            return Ok(FoundServer { endpoint: endpoint.into(), api: api.into(), models });
        }
        Err(format!("{endpoint}: no supported model-list API found"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MockResponse, spawn_mock};

    #[test]
    fn bare_addresses_try_common_ports_but_explicit_ports_and_routes_are_preserved() {
        assert_eq!(candidates("192.168.18.217").unwrap(), COMMON_PORTS.map(|p| format!("http://192.168.18.217:{p}")));
        assert_eq!(candidates("nas.local:80").unwrap(), ["http://nas.local:80"]);
        assert_eq!(candidates("http://nas.local/").unwrap().len(), 3);
        assert_eq!(candidates("http://nas:9191/proxy/v1/models").unwrap(), ["http://nas:9191/proxy"]);
        assert_eq!(candidates("https://nas").unwrap(), ["https://nas"]);
        assert_eq!(normalize("nas:11434/api/tags").unwrap(), "http://nas:11434");
        assert!(normalize("file:///model").is_err());
        assert!(normalize("http://nas?token=x").is_err());
    }

    #[tokio::test]
    async fn model_discovery_only_reads_metadata_and_deduplicates_model_names() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![br#"{"data":[{"id":"gemma"},{"id":"gemma"},{"id":"hy-mt2"}]}"#.to_vec()], delay_ms: 0 }).await;
        let found = ServerProbe::new().unwrap().check(&format!("{}/v1", server.url)).await.unwrap();
        assert_eq!(found.models, ["gemma", "hy-mt2"]);
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0], ("/v1/models".into(), "".into()));
    }

    #[tokio::test]
    async fn ollama_metadata_is_used_when_the_compatible_route_is_missing() {
        let server = spawn_mock(|path, _| MockResponse { status: if path == "/api/tags" {200} else {404}, chunks: vec![br#"{"models":[{"name":"gemma:12b"}]}"#.to_vec()], delay_ms: 0 }).await;
        let found = ServerProbe::new().unwrap().check(&server.url).await.unwrap();
        assert_eq!(found.api, "Ollama"); assert_eq!(found.models, ["gemma:12b"]);
    }

    #[tokio::test]
    async fn html_and_authentication_errors_do_not_count_as_model_servers() {
        let html = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![b"<html>NAS dashboard</html>".to_vec()], delay_ms: 0 }).await;
        assert!(ServerProbe::new().unwrap().check(&html.url).await.is_err());
        let auth = spawn_mock(|_, _| MockResponse { status: 401, chunks: vec![], delay_ms: 0 }).await;
        assert!(ServerProbe::new().unwrap().check(&auth.url).await.unwrap_err().contains("authentication"));
    }
}
