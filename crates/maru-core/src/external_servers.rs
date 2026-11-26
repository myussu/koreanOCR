//! Named external connections and their credentials. Choosing one copies ordinary settings.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerProvider {
    #[default]
    Openai,
    Azure,
    /// Google's Gemini API through its OpenAI-compatible route (`/v1beta/openai`), with an API key.
    Gemini,
    /// Anthropic's own Messages API (`/v1/messages`), with an API key: the system prompt is cached between requests.
    /// (A connection saved as `openai` with Anthropic's address keeps using its OpenAI-compatible endpoint.)
    Anthropic,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServerKey(String);
impl ServerKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl From<String> for ServerKey {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl std::fmt::Debug for ServerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedServer {
    pub name: String,
    pub endpoint: String,
    pub model_name: String,
    #[serde(default)]
    pub provider: ServerProvider,
    #[serde(default)]
    pub api_key: Option<ServerKey>,
}

pub fn validate_connection(
    endpoint: &str,
    model: &str,
    provider: ServerProvider,
    key: Option<&ServerKey>,
) -> Result<(), String> {
    if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
        return Err("Enter a complete external server address, or use Detect server first.".into());
    }
    if endpoint.contains(['\r', '\n', '?', '#'])
        || endpoint.split("://").nth(1).is_some_and(|v| {
            v.split('/')
                .next()
                .is_none_or(|h| h.is_empty() || h.contains('@'))
        })
    {
        return Err(
            "Use a server base URL without credentials, query parameters or fragments.".into(),
        );
    }
    if model.trim().is_empty() {
        return Err("Enter the server model or Azure deployment name.".into());
    }
    if key.is_some_and(|key| key.as_str().contains(['\r', '\n'])) {
        return Err("The API key must fit on one line.".into());
    }
    if provider == ServerProvider::Azure {
        if key.is_none_or(|key| key.as_str().trim().is_empty()) {
            return Err("Enter the Azure OpenAI resource API key.".into());
        }
        if let Some((resource, rest)) = endpoint.split_once("/openai/deployments/") {
            let deployment = rest.split(['/', '?']).next().unwrap_or_default();
            return Err(format!("For Azure, enter {resource} as the address and {deployment} as the deployment name."));
        }
        if endpoint.contains("/deployments/") || endpoint.ends_with("/chat/completions") {
            return Err("For Azure, use the resource URL or its /openai/v1 base URL; enter the deployment name separately.".into());
        }
    }
    if provider == ServerProvider::Gemini && key.is_none_or(|key| key.as_str().trim().is_empty()) {
        return Err("Enter the Gemini API key (from Google AI Studio).".into());
    }
    if provider == ServerProvider::Anthropic && key.is_none_or(|key| key.as_str().trim().is_empty()) {
        return Err("Enter the Anthropic API key (from the Claude Console).".into());
    }
    Ok(())
}

pub fn validate_saved(servers: &[SavedServer]) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for server in servers {
        if server.name.trim().is_empty() || server.name.len() > 128 {
            return Err("Give each saved server a name (up to 128 bytes).".into());
        }
        if !names.insert(server.name.trim().to_lowercase()) {
            return Err("Saved server names must be different.".into());
        }
        validate_connection(
            &server.endpoint,
            &server.model_name,
            server.provider,
            server.api_key.as_ref(),
        )?;
    }
    Ok(())
}

impl crate::config::LlmConfig {
    /// Command-line destination overrides must not carry another server's credentials.
    pub fn override_endpoint(&mut self, endpoint: String) {
        if self.endpoint.as_deref() != Some(endpoint.as_str()) {
            self.provider = ServerProvider::Openai;
            self.api_key = None;
        }
        self.endpoint = Some(endpoint);
    }
    pub fn for_snapshot_server(&self, live: &crate::live_model::LiveTranslationConfig) -> Self {
        Self {
            endpoint: live.snapshot_endpoint.clone(),
            model_name: live.snapshot_model_name.clone(),
            provider: live.snapshot_provider,
            api_key: live.snapshot_api_key.clone(),
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_endpoint_override_preserves_the_same_connection_but_clears_another_servers_key() {
        let mut cfg = crate::config::LlmConfig {
            endpoint: Some("https://azure.example".into()),
            provider: ServerProvider::Azure,
            api_key: Some("private".to_string().into()),
            ..Default::default()
        };
        cfg.override_endpoint("https://azure.example".into());
        assert!(cfg.api_key.is_some());
        cfg.override_endpoint("http://nas:11434".into());
        assert!(cfg.api_key.is_none());
        assert_eq!(cfg.provider, ServerProvider::Openai);
    }
    #[test]
    fn saved_connections_round_trip_without_changing_active_routes_or_unrelated_comments() {
        let before = "# keep my setup\n[llm]\nendpoint='http://nas:11434'\nmodel_name='gemma'\n# keep reasoning\nreasoning_effort='none'\n";
        let changes = serde_json::json!({"llm.servers":[{"name":"NAS","endpoint":"http://nas:11434","model_name":"gemma","provider":"openai","api_key":null},{"name":"Azure","endpoint":"https://example.openai.azure.com/openai/v1","model_name":"my-deployment","provider":"azure","api_key":"private-test-key"}]});
        let patched = crate::settings::patch(before, changes.as_object().unwrap()).unwrap();
        assert_eq!(patched.config.llm.servers.len(), 2);
        assert_eq!(
            patched.config.llm.endpoint.as_deref(),
            Some("http://nas:11434")
        );
        assert!(patched.text.contains("# keep my setup"));
        assert!(patched.text.contains("# keep reasoning"));
        assert_eq!(patched.changed, ["llm.servers"]);
        assert!(crate::settings::applies_live("llm.servers"));
        let reloaded: crate::Config = toml::from_str(&patched.text).unwrap();
        assert_eq!(reloaded, patched.config);
    }
    #[test]
    fn older_configs_have_no_saved_connections_or_authentication() {
        let cfg: crate::Config = toml::from_str("[llm]\nendpoint='http://nas:11434'").unwrap();
        assert!(cfg.llm.servers.is_empty());
        assert!(cfg.llm.api_key.is_none());
        assert_eq!(cfg.llm.provider, ServerProvider::Openai);
    }
    #[test]
    fn snapshot_connections_never_inherit_the_main_servers_credentials() {
        let main = crate::config::LlmConfig {
            provider: ServerProvider::Azure,
            api_key: Some("main-secret".to_string().into()),
            ..Default::default()
        };
        let live = crate::live_model::LiveTranslationConfig {
            snapshot_endpoint: Some("http://nas:11434".into()),
            ..Default::default()
        };
        let snapshot = main.for_snapshot_server(&live);
        assert_eq!(snapshot.provider, ServerProvider::Openai);
        assert!(snapshot.api_key.is_none());
        assert!(!format!("{main:?}").contains("main-secret"));
    }
    #[test]
    fn a_pasted_azure_deployment_url_is_explained_as_its_address_and_deployment_and_gemini_needs_a_key() {
        let key: ServerKey = "k".to_string().into();
        let error = validate_connection(
            "https://contoso.openai.azure.com/openai/deployments/gpt-4.1-mini/chat/completions",
            "x",
            ServerProvider::Azure,
            Some(&key),
        )
        .unwrap_err();
        assert!(error.contains("https://contoso.openai.azure.com as the address") && error.contains("gpt-4.1-mini as the deployment"), "{error}");
        let gemini = "https://generativelanguage.googleapis.com/v1beta/openai";
        assert!(validate_connection(gemini, "gemini-2.5-flash", ServerProvider::Gemini, None).is_err());
        validate_connection(gemini, "gemini-2.5-flash", ServerProvider::Gemini, Some(&key)).unwrap();
        let parsed: crate::Config = toml::from_str("[llm]\nendpoint='https://generativelanguage.googleapis.com/v1beta/openai'\nprovider='gemini'").unwrap();
        assert_eq!(parsed.llm.provider, ServerProvider::Gemini);
    }

    #[test]
    fn malformed_credentials_and_duplicate_names_are_refused_without_echoing_keys() {
        let key: ServerKey = "private\nkey".to_string().into();
        let error = validate_connection(
            "https://azure.example",
            "deployment",
            ServerProvider::Azure,
            Some(&key),
        )
        .unwrap_err();
        assert!(!error.contains(key.as_str()));
        assert!(
            validate_connection(
                "https://azure.example",
                "deployment",
                ServerProvider::Azure,
                None
            )
            .is_err()
        );
        let profile = SavedServer {
            name: "NAS".into(),
            endpoint: "http://nas:11434".into(),
            model_name: "gemma".into(),
            provider: ServerProvider::Openai,
            api_key: None,
        };
        assert!(validate_saved(&[profile.clone(), profile]).is_err());
    }
}
