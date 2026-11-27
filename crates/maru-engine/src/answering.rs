//! Which connection and model answers a request, said to the front end as `Event::Answering`.
//!
//! The capture and deep-dive paths hold an `LlmClient`, not a route: this finds the route that client belongs to
//! (this PC's model, or a saved connection by its address and model), so the panel can name it the way Settings does.

use maru_core::Event;
use maru_core::external_servers::ServerProvider;
use maru_core::live_model::LiveModel;
use maru_core::tasks::LOCAL;
use maru_llm::LlmClient;

use crate::engine::Inner;
use crate::routes::{Place, Route, place_of};

/// What the panel shows: the connection's name, its model and where it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Who {
    pub connection: String,
    pub model: String,
    pub place: Place,
}

/// A translation-only model on this PC, by the name Settings gives it (its file when one is set).
fn live_model_name(inner: &Inner) -> String {
    let cfg = &inner.cfg.continuous.translation;
    if let Some(file) = cfg.selected_path().and_then(|p| p.file_name()) {
        return file.to_string_lossy().into_owned();
    }
    match cfg.model {
        LiveModel::Main => "this PC's model",
        LiveModel::HyMt2 => "HY-MT2 7B",
        LiveModel::Milmmt => "MiLMMT 4B",
        LiveModel::HyMt2Small => "HY-MT2 1.8B",
        LiveModel::Lfm => "LFM2 KoEn 1.2B",
    }
    .into()
}

/// The route `client` belongs to among `routes`, else a description from its address alone.
pub(crate) fn who(routes: &[Route], client: &LlmClient, translation_only: Option<String>) -> Who {
    if let Some(model) = translation_only {
        return Who { connection: LOCAL.into(), model, place: Place::ThisPc };
    }
    if client.has_runtime() {
        let model = routes.iter().find(|r| r.is_local()).map_or_else(|| client.model().to_string(), |r| r.model.clone());
        return Who { connection: LOCAL.into(), model, place: Place::ThisPc };
    }
    if let Some(route) = routes.iter().find(|r| !r.is_local() && r.client.same_server_model(client)) {
        return Who { connection: route.connection.clone(), model: route.model.clone(), place: route.place };
    }
    let host = client.root().split("://").nth(1).unwrap_or(client.root()).split(['/', ':']).next().unwrap_or_default().to_string();
    let place = if client.is_hosted() { Place::Hosted } else { place_of(client.root(), ServerProvider::Openai) };
    Who { connection: if host.is_empty() { "a server".into() } else { host }, model: client.model().to_string(), place }
}

fn place_word(place: Place) -> &'static str {
    match place {
        Place::ThisPc => "this_pc",
        Place::Network => "network",
        Place::Hosted => "hosted",
    }
}

/// The event that names who answers request `id`. `translation_only` is set when the live translation model answers.
pub(crate) fn event(inner: &Inner, id: u64, client: &LlmClient, translation_only: bool) -> Event {
    let routes = inner.routes();
    let named = translation_only.then(|| live_model_name(inner));
    let w = who(&routes.all, client, named);
    Event::Answering { id, connection: w.connection, model: w.model, place: place_word(w.place).into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maru_core::config::LlmConfig;

    fn server(name: &str, endpoint: &str, model: &str) -> Route {
        let cfg = LlmConfig { endpoint: Some(endpoint.into()), model_name: model.into(), ..LlmConfig::default() };
        Route { connection: name.into(), model: model.into(), place: place_of(endpoint, ServerProvider::Openai), client: LlmClient::from_config(&cfg) }
    }

    #[test]
    fn a_client_is_named_after_the_saved_connection_with_its_address_and_model() {
        let routes = [server("NAS", "http://192.168.1.20:11434", "gemma4-12b"), server("Other", "http://192.168.1.20:11434", "qwen3")];
        let asked = LlmClient::from_config(&LlmConfig { endpoint: Some("http://192.168.1.20:11434".into()), model_name: "gemma4-12b".into(), ..LlmConfig::default() });
        assert_eq!(who(&routes, &asked, None), Who { connection: "NAS".into(), model: "gemma4-12b".into(), place: Place::Network });
    }

    #[test]
    fn a_server_that_is_not_saved_is_named_by_its_host_and_the_live_model_by_its_own_name() {
        let asked = LlmClient::from_config(&LlmConfig { endpoint: Some("http://truenas:8080/v1".into()), model_name: "maru".into(), ..LlmConfig::default() });
        assert_eq!(who(&[], &asked, None), Who { connection: "truenas".into(), model: "maru".into(), place: Place::Network });
        assert_eq!(who(&[], &asked, Some("HY-MT2 1.8B".into())), Who { connection: LOCAL.into(), model: "HY-MT2 1.8B".into(), place: Place::ThisPc });
    }
}
