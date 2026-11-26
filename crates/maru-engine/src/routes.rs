//! One model per task (`maru_core::tasks`): the clients each task asks, in order, and what happened to them.
//!
//! Captures and live subtitles keep their own paths (`pipeline.rs`); this module adds the live route when it is
//! not the capture connection, and the card tasks, which try the chosen connection, then this PC's model, and say
//! which one answered. A local model is never loaded for a card while live subtitles run: the sheet is told, and
//! offers another connection now or the same request once live subtitles stop.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use maru_core::Config;
use maru_core::external_servers::ServerProvider;
use maru_core::tasks::{LOCAL, Target, Task, TasksConfig};
use maru_llm::model_runtime::ModelRole;
use maru_llm::problem::{Problem, classify, sentence};
use maru_llm::{ChatOpts, LlmClient, LlmError, Message};
use serde::{Deserialize, Serialize};

use crate::engine::{Engine, Inner, lock};

/// Where a connection runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Place {
    ThisPc,
    /// Another computer on the home network (a NAS, a second PC).
    Network,
    /// A paid or free service on the internet.
    Hosted,
}

/// Where the server at `endpoint` is, from its address alone.
pub fn place_of(endpoint: &str, provider: ServerProvider) -> Place {
    if provider != ServerProvider::Openai {
        return Place::Hosted;
    }
    let rest = endpoint.split("://").nth(1).unwrap_or(endpoint);
    let host = rest.split(['/', ':']).next().unwrap_or_default().to_ascii_lowercase();
    let octets: Vec<u8> = host.split('.').filter_map(|p| p.parse::<u8>().ok()).collect();
    let is_ip = octets.len() == 4 && host.split('.').count() == 4;
    if host == "localhost" || host == "::1" || (is_ip && octets[0] == 127) {
        Place::ThisPc
    } else if (is_ip && (octets[0] == 10 || (octets[0] == 172 && (16..32).contains(&octets[1])) || (octets[0] == 192 && octets[1] == 168) || (octets[0] == 169 && octets[1] == 254)))
        || !host.contains('.')
        || host.ends_with(".local")
        || host.ends_with(".lan")
        || host.ends_with(".home")
    {
        Place::Network
    } else {
        Place::Hosted
    }
}

/// A connection ready to be asked.
#[derive(Clone)]
pub struct Route {
    /// `local`, or the name the connection was saved under.
    pub connection: String,
    pub model: String,
    pub place: Place,
    pub client: LlmClient,
}

impl Route {
    pub fn is_local(&self) -> bool {
        self.connection == LOCAL
    }

    /// How the connection is called in a sentence for a person.
    pub fn name(&self) -> &str {
        if self.is_local() { "This PC's model" } else { &self.connection }
    }

    /// A local model that is in memory now (or needs no loading): asking it loads nothing.
    fn loaded(&self) -> bool {
        !self.client.has_runtime() || self.client.loaded_role() == Some(ModelRole::Main)
    }
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} · {}", self.connection, self.model)
    }
}

/// The routes of the tasks that have their own.
#[derive(Debug, Clone, Default)]
pub struct Routes {
    /// What translates live lines, when that is neither the capture connection nor a translation-only local model.
    pub live: Option<Route>,
    /// Card meanings: the chosen connection, then this PC's model. Empty: the capture model, as before `[tasks]`.
    pub card_meanings: Vec<Route>,
    /// The card's sentence translation. Empty: the card keeps the translation the line has.
    pub card_sentence: Vec<Route>,
    /// Every connection a card can be re-asked with: this PC's model first, then the saved ones.
    pub all: Vec<Route>,
    pub tasks: TasksConfig,
}

/// This PC's model as a route; `llm` is its settings.
pub fn local_route(llm: &maru_core::config::LlmConfig, client: LlmClient) -> Route {
    let model = llm.model_path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).or_else(|| llm.hf_repo.clone().filter(|r| !r.trim().is_empty())).unwrap_or_else(|| "local model".into());
    Route { connection: LOCAL.into(), model, place: Place::ThisPc, client }
}

/// The card routes for `cfg` (already passed through `tasks::apply`). `local` is this PC's model, when it has one.
pub fn build(cfg: &Config, local: Option<Route>) -> Routes {
    let tasks = cfg.tasks.clone();
    let mut all: Vec<Route> = local.iter().cloned().collect();
    for server in &cfg.llm.servers {
        if let Target::Server { name, llm } = maru_core::tasks::target(cfg, &server.name, "") {
            all.push(Route { connection: name, model: llm.model_name.clone(), place: place_of(&server.endpoint, server.provider), client: LlmClient::from_config(&llm) });
        }
    }
    let order = |connection: &str, model: &str| -> Vec<Route> {
        let chosen = match maru_core::tasks::target(cfg, connection, model) {
            Target::Local => local.clone(),
            Target::Server { name, llm } => Some(Route {
                place: llm.endpoint.as_deref().map_or(Place::Hosted, |e| place_of(e, llm.provider)),
                connection: name,
                model: llm.model_name.clone(),
                client: LlmClient::from_config(&llm),
            }),
            Target::Unknown => None,
        };
        let mut order: Vec<Route> = chosen.into_iter().collect();
        if let Some(local) = &local
            && !order.iter().any(Route::is_local)
        {
            order.push(local.clone());
        }
        order
    };
    let card_meanings = order(&tasks.card_meanings.connection, &tasks.card_meanings.model);
    let card_sentence = if tasks.card_sentence.connection.trim().is_empty() { Vec::new() } else { order(&tasks.card_sentence.connection, &tasks.card_sentence.model) };
    Routes { live: None, card_meanings, card_sentence, all, tasks }
}

/// Why a connection was passed over for a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skipped {
    NoKey,
    Quota,
    Unreachable,
    Failed,
    /// This PC's model is not in memory and live subtitles are running: it is not loaded beside them.
    LiveRunning,
}

impl From<Problem> for Skipped {
    fn from(p: Problem) -> Self {
        match p {
            Problem::NoKey => Skipped::NoKey,
            Problem::Quota => Skipped::Quota,
            Problem::Unreachable => Skipped::Unreachable,
            Problem::Failed => Skipped::Failed,
        }
    }
}

/// A connection that was asked (or passed over) and did not answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Attempt {
    pub connection: String,
    pub model: String,
    pub problem: Skipped,
    /// One plain sentence for the person.
    pub message: String,
}

/// The connection that answered.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Answered {
    pub connection: String,
    pub model: String,
    pub place: Place,
}

/// A connection the sheet can re-ask a line with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetryOption {
    pub connection: String,
    pub model: String,
    pub place: Place,
    /// Asking it now would load a model beside live subtitles, so it is offered for when they stop.
    pub after_live: bool,
}

/// How to ask for a card's meanings or sentence.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct CardAsk {
    /// Ask this connection only ("Retry with …"); `None`: the task's own order.
    pub connection: Option<String>,
    /// Wait until live subtitles stop before asking (this PC's model is then free to load).
    pub after_live: bool,
}

pub(crate) struct Outcome<T> {
    pub value: Option<T>,
    pub answered: Option<Answered>,
    pub attempts: Vec<Attempt>,
}

impl<T> Outcome<T> {
    /// What to tell the person: who failed and why, then who answered (or `otherwise`).
    pub fn note(&self, otherwise: &str) -> Option<String> {
        if self.attempts.is_empty() {
            return None;
        }
        let mut text: Vec<String> = self.attempts.iter().map(|a| a.message.clone()).collect();
        text.push(match &self.answered {
            Some(a) if a.connection == LOCAL => "This PC's model answered instead.".into(),
            Some(a) => format!("{} answered instead.", a.connection),
            None => otherwise.to_string(),
        });
        Some(text.join(" "))
    }
}

/// What is known about a connection from the last time it was asked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Health {
    pub problem: Option<Problem>,
    pub at: Instant,
}

pub(crate) type HealthMap = HashMap<String, Health>;

fn health_key(route: &Route) -> String {
    format!("{}\u{1f}{}", route.connection.to_lowercase(), route.model)
}

pub(crate) fn remember(inner: &Inner, route: &Route, problem: Option<Problem>) {
    lock(&inner.route_health).insert(health_key(route), Health { problem, at: Instant::now() });
}

impl Inner {
    pub(crate) fn routes(&self) -> Arc<Routes> {
        lock(&self.routes).clone()
    }
}

/// The routes to ask for a card task, as `ask` wants them.
pub(crate) fn candidates(routes: &Routes, task_order: &[Route], ask: &CardAsk) -> Vec<Route> {
    match ask.connection.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        None => task_order.to_vec(),
        Some(wanted) => routes.all.iter().filter(|r| r.connection.eq_ignore_ascii_case(wanted)).cloned().collect(),
    }
}

/// Wait until live subtitles are off (the sheet's "ask when live stops").
pub(crate) async fn until_live_stops(inner: &Inner) {
    while inner.live_active.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

/// Ask each route in turn until one gives a usable answer. `call` returns `Ok(None)` for a reply with nothing usable.
pub(crate) async fn ask_in_order<T, F, Fut>(inner: &Inner, order: Vec<Route>, call: F) -> Outcome<T>
where
    F: Fn(LlmClient) -> Fut,
    Fut: Future<Output = Result<Option<T>, LlmError>>,
{
    let mut attempts = Vec::new();
    for route in order {
        let skip = |problem: Skipped, message: String| Attempt { connection: route.connection.clone(), model: route.model.clone(), problem, message };
        if route.is_local() && inner.live_active.load(Ordering::Acquire) && !route.loaded() {
            attempts.push(skip(Skipped::LiveRunning, "This PC's model is not loaded while live subtitles run.".into()));
            continue;
        }
        match call(route.client.clone()).await {
            Ok(Some(value)) => {
                remember(inner, &route, None);
                let answered = Answered { connection: route.connection.clone(), model: route.model.clone(), place: route.place };
                return Outcome { value: Some(value), answered: Some(answered), attempts };
            }
            Ok(None) => {
                remember(inner, &route, Some(Problem::Failed));
                attempts.push(skip(Skipped::Failed, sentence(route.name(), Problem::Failed)));
            }
            Err(error) => {
                let problem = classify(&error);
                tracing::warn!("{} ({}) did not answer a card request: {error}", route.connection, route.model);
                remember(inner, &route, Some(problem));
                attempts.push(skip(problem.into(), sentence(route.name(), problem)));
            }
        }
    }
    Outcome { value: None, answered: None, attempts }
}

/// What the settings show for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// It answered the last time it was asked.
    Working,
    NoKey,
    Quota,
    Unreachable,
    Failed,
    /// Not asked yet this session.
    Unknown,
    /// This PC's model, not in memory: it loads when first used.
    NotLoaded,
    /// The task makes no request (a card keeps the translation the line has), or no model is set up for it.
    Off,
}

impl From<Problem> for State {
    fn from(p: Problem) -> Self {
        match p {
            Problem::NoKey => State::NoKey,
            Problem::Quota => State::Quota,
            Problem::Unreachable => State::Unreachable,
            Problem::Failed => State::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskStatus {
    pub task: Task,
    pub label: &'static str,
    pub connection: String,
    pub model: String,
    pub place: Option<Place>,
    pub state: State,
    /// One plain sentence.
    pub message: String,
    /// The connections tried after the chosen one, in order; card meanings end with the dictionary.
    pub fallbacks: Vec<String>,
    /// Seconds since the state was last observed; `None` when it never was.
    pub checked_secs_ago: Option<u64>,
}

fn fallbacks_of(task: Task, order: &[Route]) -> Vec<String> {
    let mut names: Vec<String> = order.iter().skip(1).map(|r| r.name().to_string()).collect();
    if task == Task::CardMeanings {
        names.push("the dictionary".into());
    }
    names
}

impl Engine {
    /// Replace the task routes (at start-up, and when the card connections are changed in Settings).
    pub fn set_routes(&self, routes: Routes) {
        *lock(&self.inner.routes) = Arc::new(routes);
    }

    /// The card connections were changed in Settings (or a saved connection was edited): rebuild the card routes
    /// from the file's settings. Captures and live subtitles keep what they started with until a restart.
    pub fn reroute_cards(&self, file: &Config) {
        let old = self.inner.routes();
        let local = old.all.iter().find(|r| r.is_local()).cloned();
        let mut new = build(&maru_core::tasks::apply(file).config, local);
        new.live = old.live.clone();
        new.tasks.live = old.tasks.live.clone();
        new.tasks.capture = old.tasks.capture.clone();
        self.set_routes(new);
    }

    /// The connections a card can be re-asked with. This PC's model is marked for later when asking it now would
    /// load it beside live subtitles.
    pub fn card_retry_options(&self) -> Vec<RetryOption> {
        let live = self.inner.live_active.load(Ordering::Acquire);
        self.inner.routes().all.iter().map(|r| RetryOption { connection: r.connection.clone(), model: r.model.clone(), place: r.place, after_live: r.is_local() && live && !r.loaded() }).collect()
    }

    /// The route a task asks first, with the rest of its order.
    fn task_order(&self, task: Task) -> Vec<Route> {
        let inner = &self.inner;
        let routes = inner.routes();
        let tasks = &routes.tasks;
        let named = |connection: &str, model: &str, client: Option<LlmClient>| -> Vec<Route> {
            let Some(client) = client else { return Vec::new() };
            let known = routes.all.iter().find(|r| r.connection.eq_ignore_ascii_case(connection.trim()));
            vec![Route {
                connection: known.map_or_else(|| if connection.trim().is_empty() { LOCAL.to_string() } else { connection.trim().to_string() }, |r| r.connection.clone()),
                model: if model.trim().is_empty() { known.map(|r| r.model.clone()).unwrap_or_else(|| client.model().to_string()) } else { model.trim().to_string() },
                place: known.map_or(Place::ThisPc, |r| r.place),
                client,
            }]
        };
        match task {
            Task::Capture => named(&tasks.capture.connection, &tasks.capture.model, inner.llm.clone()),
            Task::Live => match &routes.live {
                Some(route) => vec![route.clone()],
                None if inner.cfg.continuous.translation.model != maru_core::live_model::LiveModel::Main => named(LOCAL, &tasks.live.model, inner.live_llm.clone()),
                None => named(&tasks.live.connection, &tasks.live.model, inner.llm.clone()),
            },
            Task::CardMeanings if routes.card_meanings.is_empty() => named(&tasks.capture.connection, &tasks.capture.model, inner.llm.clone()),
            Task::CardMeanings => routes.card_meanings.clone(),
            Task::CardSentence => routes.card_sentence.clone(),
        }
    }

    fn status_of(&self, task: Task, order: &[Route]) -> TaskStatus {
        let Some(first) = order.first() else {
            let message = if task == Task::CardSentence { "Cards keep the translation the line already has." } else { "No model is set up for this." };
            return TaskStatus { task, label: task.label(), connection: String::new(), model: String::new(), place: None, state: State::Off, message: message.into(), fallbacks: Vec::new(), checked_secs_ago: None };
        };
        let health = lock(&self.inner.route_health).get(&health_key(first)).copied();
        let needs_key = first.client.is_hosted() && !first.client.has_key();
        let (state, message) = match health {
            _ if needs_key => (State::NoKey, sentence(first.name(), Problem::NoKey)),
            Some(Health { problem: Some(p), .. }) => (p.into(), sentence(first.name(), p)),
            Some(Health { problem: None, .. }) => (State::Working, format!("{} answered.", first.name())),
            None if first.is_local() && first.client.has_runtime() && !first.loaded() => (State::NotLoaded, "This PC's model loads when it is first used.".into()),
            None => (State::Unknown, "Not asked yet since Maru started. Press Test to check it.".into()),
        };
        TaskStatus {
            task,
            label: task.label(),
            connection: first.connection.clone(),
            model: first.model.clone(),
            place: Some(first.place),
            state,
            message,
            fallbacks: fallbacks_of(task, order),
            checked_secs_ago: health.map(|h| h.at.elapsed().as_secs()),
        }
    }

    /// Each task's connection and what is known about it, without asking anything.
    pub fn task_status(&self) -> Vec<TaskStatus> {
        Task::ALL.into_iter().map(|task| self.status_of(task, &self.task_order(task))).collect()
    }

    /// Ask a task's connection one short fixed question and report what happened (the settings' Test button).
    /// This PC's model is loaded by it, except beside live subtitles.
    pub async fn test_task(&self, task: Task) -> TaskStatus {
        let order = self.task_order(task);
        let Some(first) = order.first().cloned() else { return self.status_of(task, &order) };
        if first.is_local() && self.inner.live_active.load(Ordering::Acquire) && !first.loaded() {
            let mut status = self.status_of(task, &order);
            status.state = State::NotLoaded;
            status.message = "This PC's model is not loaded while live subtitles run. Test it after they stop.".into();
            return status;
        }
        let started = Instant::now();
        let opts = ChatOpts { max_tokens: Some(16), ..Default::default() };
        let asked = tokio::time::timeout(Duration::from_secs(90), first.client.stream_chat_reply(&[Message::user("안녕")], &opts, &mut |_| {})).await;
        let problem = match asked {
            Ok(Ok(_)) => None,
            Ok(Err(error)) => {
                tracing::warn!("testing {} ({}): {error}", first.connection, first.model);
                Some(classify(&error))
            }
            Err(_) => Some(Problem::Unreachable),
        };
        remember(&self.inner, &first, problem);
        let mut status = self.status_of(task, &order);
        if problem.is_none() {
            status.message = format!("{} answered in {:.1} s.", first.name(), started.elapsed().as_secs_f32());
        }
        status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_is_placed_on_this_pc_the_home_network_or_a_hosted_service_by_its_address() {
        let open = ServerProvider::Openai;
        assert_eq!(place_of("http://127.0.0.1:18080", open), Place::ThisPc);
        assert_eq!(place_of("http://localhost:1234/v1", open), Place::ThisPc);
        assert_eq!(place_of("http://192.168.18.217:11434", open), Place::Network);
        assert_eq!(place_of("http://10.0.0.5", open), Place::Network);
        assert_eq!(place_of("http://172.20.1.1:8080", open), Place::Network);
        assert_eq!(place_of("http://truenas:11434", open), Place::Network);
        assert_eq!(place_of("http://nas.local:11434", open), Place::Network);
        assert_eq!(place_of("https://api.anthropic.com", open), Place::Hosted);
        assert_eq!(place_of("http://172.200.1.1", open), Place::Hosted, "not a private range");
        assert_eq!(place_of("http://192.168.1.2", ServerProvider::Azure), Place::Hosted, "a hosted provider is hosted whatever its address");
    }

    #[test]
    fn card_meanings_ask_the_chosen_connection_then_this_pcs_model_and_the_sentence_only_when_one_is_chosen() {
        use maru_core::external_servers::SavedServer;
        let mut cfg = Config::default();
        cfg.llm.hf_repo = Some("a/b:Q4".into());
        cfg.llm.servers = vec![
            SavedServer { name: "NAS".into(), endpoint: "http://192.168.1.20:11434".into(), model_name: "gemma".into(), provider: ServerProvider::Openai, api_key: None },
            SavedServer { name: "Claude".into(), endpoint: "https://api.anthropic.com".into(), model_name: "claude-haiku".into(), provider: ServerProvider::Openai, api_key: Some("k".to_string().into()) },
        ];
        cfg.tasks = maru_core::tasks::derive(&cfg).tasks;
        let local = local_route(&cfg.llm, LlmClient::from_config(&cfg.llm));
        let routes = build(&cfg, Some(local.clone()));
        assert_eq!(routes.card_meanings.iter().map(|r| r.connection.as_str()).collect::<Vec<_>>(), [LOCAL], "this PC's model, with nothing before it");
        assert!(routes.card_sentence.is_empty());
        assert_eq!(routes.all.iter().map(|r| (r.connection.as_str(), r.place)).collect::<Vec<_>>(), [(LOCAL, Place::ThisPc), ("NAS", Place::Network), ("Claude", Place::Hosted)]);
        assert_eq!(local.model, "a/b:Q4");

        cfg.tasks.card_meanings = maru_core::tasks::TaskRoute { connection: "claude".into(), model: "claude-sonnet".into() };
        cfg.tasks.card_sentence = maru_core::tasks::TaskRoute { connection: "NAS".into(), model: String::new() };
        let routes = build(&cfg, Some(local));
        assert_eq!(routes.card_meanings.iter().map(|r| (r.connection.as_str(), r.model.as_str())).collect::<Vec<_>>(), [("Claude", "claude-sonnet"), (LOCAL, "a/b:Q4")]);
        assert_eq!(routes.card_sentence.iter().map(|r| r.connection.as_str()).collect::<Vec<_>>(), ["NAS", LOCAL]);
        assert_eq!(fallbacks_of(Task::CardMeanings, &routes.card_meanings), ["This PC's model", "the dictionary"]);

        // Without a model on this PC there is nothing to fall back to but the dictionary.
        let routes = build(&cfg, None);
        assert_eq!(routes.card_meanings.iter().map(|r| r.connection.as_str()).collect::<Vec<_>>(), ["Claude"]);
        let only = candidates(&routes, &routes.card_meanings, &CardAsk { connection: Some("nas".into()), after_live: false });
        assert_eq!(only.iter().map(|r| r.connection.as_str()).collect::<Vec<_>>(), ["NAS"], "a retry asks the named connection only");
    }
}
