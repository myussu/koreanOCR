//! One model per task. `[tasks]` says which connection (and model) each job uses: live subtitles, captures
//! (snapshot, hover, scan), card meanings, the card's sentence translation, and the two voices. A connection is
//! `local` (the model this PC runs, `[llm]`) or the name of a saved connection (`[llm].servers`).
//!
//! Files written before this had no `[tasks]`: their routing lived in `[llm] endpoint` and
//! `[continuous.translation]`. [`derive`] reads that older routing as a task table, [`migrate`] writes it into
//! the file once (`version = 1`), and [`apply`] turns a task table back into the older fields, which is what
//! the engine's capture and live paths still read. The older keys are never renamed and keep parsing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, LlmConfig};
use crate::error::{Error, Result};
use crate::external_servers::{SavedServer, ServerKey, ServerProvider};
use crate::live_model::LiveModel;

/// The connection name of the model this PC runs itself.
pub const LOCAL: &str = "local";
/// The version [`migrate`] writes. 0 (or no `[tasks]`) means the older settings decide.
pub const VERSION: u32 = 1;

/// Which connection answers a task, and with which model.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskRoute {
    /// `local` or the name of a saved connection. Empty: the task makes no request of its own.
    pub connection: String,
    /// The model to ask for there. Empty: the connection's own. For `local` live subtitles it names one of the
    /// translation-only models (`hy_mt2`, `milmmt`, `hy_mt2_small`, `lfm`).
    pub model: String,
}

/// Captures can use another connection while live subtitles run (when theirs is this PC's model, which live
/// subtitles may have replaced with a translation-only one).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureRoute {
    pub connection: String,
    pub model: String,
    /// A saved connection to use instead while live subtitles are on. Empty: the same one.
    pub while_live: String,
    pub while_live_model: String,
}

/// Which voice speaks. Empty: the one in `[tts]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceRoute {
    pub voice: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TasksConfig {
    pub version: u32,
    pub live: TaskRoute,
    pub capture: CaptureRoute,
    pub card_meanings: TaskRoute,
    /// Empty connection: a card keeps the translation the line already has.
    pub card_sentence: TaskRoute,
    pub voice_reading: VoiceRoute,
    pub voice_cards: VoiceRoute,
}

/// The jobs a model is chosen for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    Live,
    Capture,
    CardMeanings,
    CardSentence,
}

impl Task {
    pub const ALL: [Task; 4] = [Task::Live, Task::Capture, Task::CardMeanings, Task::CardSentence];

    /// What the settings call it.
    pub fn label(self) -> &'static str {
        match self {
            Task::Live => "Live subtitles",
            Task::Capture => "Captures",
            Task::CardMeanings => "Card meanings",
            Task::CardSentence => "Card sentence translation",
        }
    }
}

impl TasksConfig {
    pub fn route(&self, task: Task) -> TaskRoute {
        match task {
            Task::Live => self.live.clone(),
            Task::Capture => TaskRoute { connection: self.capture.connection.clone(), model: self.capture.model.clone() },
            Task::CardMeanings => self.card_meanings.clone(),
            Task::CardSentence => self.card_sentence.clone(),
        }
    }
}

pub fn live_model_name(model: LiveModel) -> &'static str {
    match model {
        LiveModel::Main => "main",
        LiveModel::HyMt2 => "hy_mt2",
        LiveModel::Milmmt => "milmmt",
        LiveModel::HyMt2Small => "hy_mt2_small",
        LiveModel::Lfm => "lfm",
    }
}

/// A translation-only local model named in a live route; `None` for the main model or an unknown name.
pub fn live_model_named(name: &str) -> Option<LiveModel> {
    [LiveModel::HyMt2, LiveModel::Milmmt, LiveModel::HyMt2Small, LiveModel::Lfm].into_iter().find(|m| live_model_name(*m) == name.trim())
}

/// Where a connection name leads.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// The model this PC runs.
    Local,
    /// A saved connection, as the settings to talk to it (the rest of `[llm]` is kept).
    Server { name: String, llm: Box<LlmConfig> },
    /// No saved connection has that name any more.
    Unknown,
}

fn is_local(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(LOCAL)
}

fn saved<'a>(cfg: &'a Config, name: &str) -> Option<&'a SavedServer> {
    let wanted = name.trim().to_lowercase();
    cfg.llm.servers.iter().find(|s| s.name.trim().to_lowercase() == wanted)
}

/// The settings for `connection` (asking for `model` when it is not empty).
pub fn target(cfg: &Config, connection: &str, model: &str) -> Target {
    if is_local(connection) {
        return Target::Local;
    }
    match saved(cfg, connection) {
        Some(server) => {
            let mut llm = cfg.llm.clone();
            llm.endpoint = Some(server.endpoint.clone());
            llm.provider = server.provider;
            llm.api_key = server.api_key.clone();
            llm.model_name = if model.trim().is_empty() { server.model_name.clone() } else { model.trim().to_string() };
            Target::Server { name: server.name.clone(), llm: Box::new(llm) }
        }
        None => Target::Unknown,
    }
}

/// `[llm]` as the settings of the model this PC runs: no server address or key. `None` when no local model is
/// configured (neither a file nor a Hugging Face name).
pub fn local_llm(cfg: &Config) -> Option<LlmConfig> {
    let has_model = cfg.llm.model_path.is_some() || cfg.llm.hf_repo.as_deref().is_some_and(|r| !r.trim().is_empty());
    has_model.then(|| {
        let mut llm = cfg.llm.clone();
        if llm.endpoint.take().is_some() {
            llm.model_name = LlmConfig::default().model_name;
        }
        llm.provider = ServerProvider::Openai;
        llm.api_key = None;
        llm
    })
}

/// The older routing read as a task table, and the connections that had to be given a name for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Derived {
    pub tasks: TasksConfig,
    /// Servers that were only an address in the older settings: saved under their host name so a task can name them.
    pub new_servers: Vec<SavedServer>,
}

fn same_address(a: &str, b: &str) -> bool {
    a.trim().trim_end_matches('/').eq_ignore_ascii_case(b.trim().trim_end_matches('/'))
}

fn host_of(endpoint: &str) -> &str {
    let rest = endpoint.split("://").nth(1).unwrap_or(endpoint);
    rest.split(['/', ':']).next().filter(|h| !h.is_empty()).unwrap_or("Server")
}

/// The saved connection for an address written directly in the older settings, and the model override it needs.
fn name_for(known: &[SavedServer], added: &mut Vec<SavedServer>, endpoint: &str, model: &str, provider: ServerProvider, key: Option<&ServerKey>) -> (String, String) {
    let all = || known.iter().chain(added.iter());
    let matches = |s: &&SavedServer| same_address(&s.endpoint, endpoint) && s.provider == provider;
    if let Some(exact) = all().filter(matches).find(|s| s.model_name == model) {
        return (exact.name.clone(), String::new());
    }
    if let Some(other_model) = all().find(matches) {
        return (other_model.name.clone(), model.to_string());
    }
    let host = host_of(endpoint);
    let taken = |name: &str| all().any(|s| s.name.trim().eq_ignore_ascii_case(name)) || is_local(name);
    let name = std::iter::once(host.to_string()).chain((2..).map(|n| format!("{host} {n}"))).find(|n| !taken(n)).unwrap_or_else(|| host.to_string());
    added.push(SavedServer { name: name.clone(), endpoint: endpoint.to_string(), model_name: model.to_string(), provider, api_key: key.cloned() });
    (name, String::new())
}

/// What a file without `[tasks]` does today, as a task table. Card meanings go to this PC's model when it has
/// one (the choice made after the blind review of run 1); everything else is exactly the older behaviour.
pub fn derive(cfg: &Config) -> Derived {
    let mut added = Vec::new();
    let known = &cfg.llm.servers;
    let live_cfg = &cfg.continuous.translation;
    let (connection, model) = match &cfg.llm.endpoint {
        None => (LOCAL.to_string(), String::new()),
        Some(endpoint) => name_for(known, &mut added, endpoint, &cfg.llm.model_name, cfg.llm.provider, cfg.llm.api_key.as_ref()),
    };
    let (while_live, while_live_model) = match &live_cfg.snapshot_endpoint {
        None => Default::default(),
        Some(endpoint) => name_for(known, &mut added, endpoint, &live_cfg.snapshot_model_name, live_cfg.snapshot_provider, live_cfg.snapshot_api_key.as_ref()),
    };
    let live = match live_cfg.model {
        LiveModel::Main => TaskRoute { connection: connection.clone(), model: model.clone() },
        other => TaskRoute { connection: LOCAL.into(), model: live_model_name(other).into() },
    };
    let card_meanings = if local_llm(cfg).is_some() || is_local(&connection) {
        TaskRoute { connection: LOCAL.into(), model: String::new() }
    } else {
        TaskRoute { connection: connection.clone(), model: model.clone() }
    };
    let tasks = TasksConfig {
        version: VERSION,
        live,
        capture: CaptureRoute { connection, model, while_live, while_live_model },
        card_meanings,
        card_sentence: TaskRoute::default(),
        voice_reading: cfg.tasks.voice_reading.clone(),
        voice_cards: cfg.tasks.voice_cards.clone(),
    };
    Derived { tasks, new_servers: added }
}

/// The task table in force: the file's own once it has one, else the older settings read as one.
pub fn effective(cfg: &Config) -> TasksConfig {
    if cfg.tasks.version >= VERSION { cfg.tasks.clone() } else { derive(cfg).tasks }
}

/// A live route that is not the capture route: what translates live lines then.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveRoute {
    pub connection: String,
    /// `None`: this PC's main model.
    pub server: Option<Box<LlmConfig>>,
}

/// A configuration with its task table written back into the fields the engine's capture and live paths read.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub config: Config,
    pub tasks: TasksConfig,
    /// The settings of the model this PC runs, whatever captures use.
    pub local: Option<LlmConfig>,
    /// Set when live subtitles use another connection than captures (and not a translation-only local model).
    pub live: Option<LiveRoute>,
    /// Plain-language notes: a task naming a connection that no longer exists.
    pub problems: Vec<String>,
}

pub fn apply(cfg: &Config) -> Applied {
    let derived = (cfg.tasks.version < VERSION).then(|| derive(cfg));
    // The configuration with every server a task may name, including those the older settings only had as an address.
    let mut named = cfg.clone();
    if let Some(d) = &derived {
        named.llm.servers.extend(d.new_servers.iter().cloned());
    }
    let tasks = derived.map(|d| d.tasks).unwrap_or_else(|| cfg.tasks.clone());
    let local = local_llm(cfg);
    let mut problems = Vec::new();
    let mut config = named.clone();

    // Captures: the older `[llm] endpoint` fields.
    match target(&named, &tasks.capture.connection, &tasks.capture.model) {
        Target::Server { llm, .. } => config.llm = *llm,
        found => {
            if found == Target::Unknown {
                problems.push(format!("Captures: there is no saved connection called \"{}\"; this PC's model is used instead. Choose one in Settings.", tasks.capture.connection));
            }
            if config.llm.endpoint.take().is_some() {
                config.llm.model_name = LlmConfig::default().model_name;
            }
            config.llm.provider = ServerProvider::Openai;
            config.llm.api_key = None;
        }
    }

    // Captures while live subtitles run: the older snapshot server.
    let live_cfg = &mut config.continuous.translation;
    match target(&named, &tasks.capture.while_live, &tasks.capture.while_live_model) {
        Target::Server { llm, .. } => {
            live_cfg.snapshot_endpoint = llm.endpoint.clone();
            live_cfg.snapshot_model_name = llm.model_name.clone();
            live_cfg.snapshot_provider = llm.provider;
            live_cfg.snapshot_api_key = llm.api_key.clone();
        }
        _ => {
            if live_cfg.snapshot_endpoint.take().is_some() {
                live_cfg.snapshot_provider = ServerProvider::Openai;
                live_cfg.snapshot_api_key = None;
            }
        }
    }

    // Live subtitles: a translation-only local model is the older `model`; anything else that is not the capture
    // connection is handed to the engine as its own route.
    let specialised = is_local(&tasks.live.connection).then(|| live_model_named(&tasks.live.model)).flatten();
    live_cfg.model = specialised.unwrap_or(LiveModel::Main);
    let same_as_capture = tasks.live.connection.trim().eq_ignore_ascii_case(tasks.capture.connection.trim()) && tasks.live.model.trim() == tasks.capture.model.trim();
    let live = if specialised.is_some() || same_as_capture {
        None
    } else {
        match target(&named, &tasks.live.connection, &tasks.live.model) {
            Target::Local => Some(LiveRoute { connection: LOCAL.into(), server: None }),
            Target::Server { name, llm } => Some(LiveRoute { connection: name, server: Some(llm) }),
            Target::Unknown => {
                problems.push(format!("Live subtitles: there is no saved connection called \"{}\"; the captures' connection is used instead. Choose one in Settings.", tasks.live.connection));
                None
            }
        }
    };
    config.tasks = tasks.clone();
    Applied { config, tasks, local, live, problems }
}

impl Config {
    /// For a command-line override (`--endpoint`, `--no-llm`): bring the older `[llm]` and
    /// `[continuous.translation]` fields up to date with `[tasks]` and let them decide again, so that changing
    /// one of them changes where requests go.
    pub fn route_by_older_fields(&mut self) {
        *self = apply(self).config;
        self.tasks.version = 0;
    }
}

/// Every connection a task names must exist; a `local` live model must be one Maru knows.
pub fn validate(cfg: &Config) -> std::result::Result<(), String> {
    if cfg.tasks.version < VERSION {
        return Ok(());
    }
    let t = &cfg.tasks;
    let named = [
        (Task::Live.label(), &t.live.connection),
        (Task::Capture.label(), &t.capture.connection),
        ("Captures while live subtitles are on", &t.capture.while_live),
        (Task::CardMeanings.label(), &t.card_meanings.connection),
        (Task::CardSentence.label(), &t.card_sentence.connection),
    ];
    for (label, name) in named {
        if !name.trim().is_empty() && !is_local(name) && saved(cfg, name).is_none() {
            return Err(format!("{label} uses the connection \"{name}\", which is not saved. Choose another connection for it first."));
        }
    }
    for (label, name) in [(Task::Live.label(), &t.live.connection), (Task::Capture.label(), &t.capture.connection), (Task::CardMeanings.label(), &t.card_meanings.connection)] {
        if name.trim().is_empty() {
            return Err(format!("{label} needs a connection: this PC's model (\"local\") or a saved one."));
        }
    }
    if is_local(&t.capture.while_live) {
        return Err("Captures while live subtitles are on: choose a saved connection, or leave it empty to use the same one.".into());
    }
    if is_local(&t.live.connection) && !t.live.model.trim().is_empty() && t.live.model.trim() != "main" && live_model_named(&t.live.model).is_none() {
        return Err(format!("Live subtitles: \"{}\" is not a model this PC can run for them.", t.live.model));
    }
    Ok(())
}

const HEADER: &str = "\n# One model per task. \"local\" is the model this PC runs ([llm]); any other name is a connection saved\n# under [llm] servers. Written by Maru when it updated this file; the older [llm] endpoint and\n# [continuous.translation] snapshot_* keys are no longer read once this table has a version.\n";

/// The file's text with its routing written as `[tasks]`, or `None` when it already has one. Nothing else in
/// the text changes; comments and order survive.
pub fn migrate(text: &str) -> Result<Option<String>> {
    let cfg: Config = toml::from_str(text).map_err(|e| Error::Config(e.to_string()))?;
    if cfg.tasks.version >= VERSION {
        return Ok(None);
    }
    let derived = derive(&cfg);
    let t = &derived.tasks;
    let mut changes = serde_json::Map::new();
    let mut put = |path: &str, value: &str| {
        changes.insert(path.to_string(), serde_json::Value::String(value.to_string()));
    };
    put("tasks.live.connection", &t.live.connection);
    put("tasks.live.model", &t.live.model);
    put("tasks.capture.connection", &t.capture.connection);
    put("tasks.capture.model", &t.capture.model);
    put("tasks.capture.while_live", &t.capture.while_live);
    put("tasks.capture.while_live_model", &t.capture.while_live_model);
    put("tasks.card_meanings.connection", &t.card_meanings.connection);
    put("tasks.card_meanings.model", &t.card_meanings.model);
    put("tasks.card_sentence.connection", &t.card_sentence.connection);
    put("tasks.card_sentence.model", &t.card_sentence.model);
    put("tasks.voice_reading.voice", &t.voice_reading.voice);
    put("tasks.voice_cards.voice", &t.voice_cards.voice);
    changes.insert("tasks.version".into(), serde_json::Value::from(VERSION));
    if !derived.new_servers.is_empty() {
        let all: Vec<&SavedServer> = cfg.llm.servers.iter().chain(&derived.new_servers).collect();
        changes.insert("llm.servers".into(), serde_json::to_value(all).map_err(|e| Error::Config(e.to_string()))?);
    }
    let patched = crate::settings::patch(text, &changes)?;
    let mut doc: toml_edit::DocumentMut = patched.text.parse().map_err(|e: toml_edit::TomlError| Error::Config(e.to_string()))?;
    if let Some(table) = doc.get_mut("tasks").and_then(toml_edit::Item::as_table_mut) {
        table.set_implicit(false);
        if table.decor().prefix().and_then(|p| p.as_str()).is_none_or(|p| p.trim().is_empty()) {
            table.decor_mut().set_prefix(HEADER);
        }
    }
    Ok(Some(doc.to_string()))
}

/// What [`migrate_file`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Migrated {
    /// The copy of the file as it was.
    pub backup: PathBuf,
    pub tasks: TasksConfig,
    /// Names given to servers that were only an address.
    pub named: Vec<String>,
}

/// Write `[tasks]` into the config file at `path` if it has none, after copying the file beside itself as
/// `config.toml.before-tasks-<stamp>.bak`. A missing file, or one that already has the table, is left alone.
pub fn migrate_file(path: &Path, now_secs: i64) -> Result<Option<Migrated>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let Some(new_text) = migrate(&text)? else { return Ok(None) };
    let before: Config = toml::from_str(&text).map_err(|e| Error::Config(e.to_string()))?;
    let derived = derive(&before);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("config.toml");
    let stamp = utc_stamp(now_secs);
    let backup = (0..)
        .map(|n| path.with_file_name(if n == 0 { format!("{name}.before-tasks-{stamp}.bak") } else { format!("{name}.before-tasks-{stamp}-{n}.bak") }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| path.with_file_name(format!("{name}.before-tasks.bak")));
    std::fs::write(&backup, &text)?;
    std::fs::write(path, new_text)?;
    Ok(Some(Migrated { backup, tasks: derived.tasks, named: derived.new_servers.into_iter().map(|s| s.name).collect() }))
}

/// `20261126-153012` (UTC) for a Unix time.
pub fn utc_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}-{:02}{:02}{:02}", rem / 3_600, rem % 3_600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = r#"# my setup
lite = false

[llm]
# the hosted one I was trying
endpoint = "https://generativelanguage.googleapis.com/v1beta/openai"
provider = "gemini"
api_key = "k-gemini"
model_name = "gemini-flash"
hf_repo = "unsloth/gemma:Q4"
servers = [{ name = "NAS", endpoint = "http://192.168.1.20:11434", model_name = "gemma-nas" }, { name = "Gemini", endpoint = "https://generativelanguage.googleapis.com/v1beta/openai", model_name = "gemini-flash", provider = "gemini", api_key = "k-gemini" }, { name = "Azure", endpoint = "https://contoso.openai.azure.com", model_name = "gpt-mini", provider = "azure", api_key = "k-azure" }]

[continuous.translation]
snapshot_endpoint = "https://contoso.openai.azure.com"
snapshot_model_name = "gpt-mini"
snapshot_provider = "azure"
snapshot_api_key = "k-azure"
backup_model = "lfm"

[learning.anki]
deck = "Coreano::2 Media" # keep
"#;

    fn parse(text: &str) -> Config {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn an_older_file_is_read_as_the_routing_it_already_had_with_card_meanings_on_this_pcs_model() {
        let d = derive(&parse(OLD));
        assert!(d.new_servers.is_empty(), "every address was already a saved connection");
        assert_eq!(d.tasks.version, VERSION);
        assert_eq!(d.tasks.capture, CaptureRoute { connection: "Gemini".into(), model: String::new(), while_live: "Azure".into(), while_live_model: String::new() });
        assert_eq!(d.tasks.live, TaskRoute { connection: "Gemini".into(), model: String::new() }, "live lines went to the main model");
        assert_eq!(d.tasks.card_meanings, TaskRoute { connection: LOCAL.into(), model: String::new() });
        assert_eq!(d.tasks.card_sentence, TaskRoute::default(), "cards keep the translation the line has");
    }

    #[test]
    fn a_plain_local_setup_and_a_translation_only_live_model_are_read_as_such() {
        let plain = derive(&parse("[llm]\nhf_repo = 'a/b:Q4'"));
        assert_eq!((plain.tasks.capture.connection.as_str(), plain.tasks.live.connection.as_str(), plain.tasks.card_meanings.connection.as_str()), (LOCAL, LOCAL, LOCAL));
        assert!(plain.tasks.capture.while_live.is_empty() && plain.tasks.live.model.is_empty());

        let special = derive(&parse("[llm]\nhf_repo = 'a/b:Q4'\n[continuous.translation]\nmodel = 'hy_mt2_small'\nhy_mt2_small_path = 'D:/m.gguf'"));
        assert_eq!(special.tasks.live, TaskRoute { connection: LOCAL.into(), model: "hy_mt2_small".into() });
        assert_eq!(special.tasks.capture.connection, LOCAL);
        assert_eq!(live_model_named("hy_mt2_small"), Some(LiveModel::HyMt2Small));
        assert_eq!(live_model_named("main"), None);
    }

    #[test]
    fn a_server_known_only_by_its_address_is_saved_under_its_host_name_and_another_model_there_is_an_override() {
        let d = derive(&parse("[llm]\nendpoint = 'http://nas.lan:11434'\nmodel_name = 'gemma'\n[continuous.translation]\nsnapshot_endpoint = 'http://nas.lan:11434/'\nsnapshot_model_name = 'bigger'"));
        assert_eq!(d.new_servers.len(), 1);
        assert_eq!((d.new_servers[0].name.as_str(), d.new_servers[0].model_name.as_str()), ("nas.lan", "gemma"));
        assert_eq!(d.tasks.capture.connection, "nas.lan");
        assert_eq!((d.tasks.capture.while_live.as_str(), d.tasks.capture.while_live_model.as_str()), ("nas.lan", "bigger"), "the same server with another model");
        assert_eq!(d.tasks.card_meanings.connection, "nas.lan", "no model on this PC: card meanings stay where captures are");

        let clash = derive(&parse("[llm]\nendpoint = 'http://nas.lan:11434'\nmodel_name = 'gemma'\nservers = [{ name = 'nas.lan', endpoint = 'http://other:1', model_name = 'x' }]"));
        assert_eq!(clash.new_servers[0].name, "nas.lan 2", "an existing name is not reused for another server");
    }

    /// The promise of the migration: nothing but card meanings changes.
    #[test]
    fn applying_the_derived_table_gives_back_the_older_settings() {
        for text in [OLD, "", "[llm]\nhf_repo = 'a/b:Q4'", "[llm]\nhf_repo = 'a/b:Q4'\n[continuous.translation]\nmodel = 'milmmt'\nsnapshot_endpoint = 'http://nas:1'\nsnapshot_model_name = 'g'", "[llm]\nendpoint = 'http://nas:1'\nmodel_name = 'g'\nserver_keep_minutes = 15"] {
            let before = parse(text);
            let applied = apply(&before);
            assert!(applied.problems.is_empty(), "{:?}", applied.problems);
            assert!(applied.live.is_none(), "live lines keep using what they used: {text}");
            let mut after = applied.config.clone();
            after.tasks = before.tasks.clone();
            after.llm.servers.truncate(before.llm.servers.len()); // a server that was only an address gains a name
            assert_eq!(after, before, "the capture and live fields are the ones the file had: {text}");
        }
    }

    #[test]
    fn a_task_table_decides_the_capture_fields_and_gives_live_subtitles_their_own_route() {
        let mut cfg = parse(OLD);
        cfg.tasks = TasksConfig {
            version: VERSION,
            live: TaskRoute { connection: "nas".into(), model: String::new() },
            capture: CaptureRoute { connection: LOCAL.into(), ..Default::default() },
            card_meanings: TaskRoute { connection: "Azure".into(), model: "gpt-big".into() },
            ..Default::default()
        };
        let applied = apply(&cfg);
        assert_eq!(applied.config.llm.endpoint, None, "captures on this PC's model");
        assert!(applied.config.llm.api_key.is_none() && applied.config.llm.provider == ServerProvider::Openai, "and without the hosted key");
        assert_eq!(applied.config.llm.model_name, "maru");
        assert_eq!(applied.config.continuous.translation.snapshot_endpoint, None, "no other connection while live is on");
        assert!(applied.config.continuous.translation.snapshot_api_key.is_none());
        let live = applied.live.expect("live subtitles are not on the capture connection");
        assert_eq!(live.connection, "NAS", "found whatever the capitals");
        assert_eq!(live.server.as_ref().unwrap().endpoint.as_deref(), Some("http://192.168.1.20:11434"));
        assert_eq!(applied.local.as_ref().and_then(|l| l.hf_repo.as_deref()), Some("unsloth/gemma:Q4"));
        assert!(applied.local.as_ref().is_some_and(|l| l.endpoint.is_none() && l.api_key.is_none()));
        match target(&applied.config, "azure", "gpt-big") {
            Target::Server { name, llm } => assert_eq!((name.as_str(), llm.model_name.as_str(), llm.provider), ("Azure", "gpt-big", ServerProvider::Azure)),
            other => panic!("{other:?}"),
        }

        // Live on this PC's main model while captures go to a server.
        cfg.tasks.live = TaskRoute { connection: LOCAL.into(), model: String::new() };
        cfg.tasks.capture.connection = "Gemini".into();
        let applied = apply(&cfg);
        assert_eq!(applied.live, Some(LiveRoute { connection: LOCAL.into(), server: None }));
        assert_eq!(applied.config.llm.model_name, "gemini-flash");
        assert_eq!(applied.config.continuous.translation.model, LiveModel::Main);

        // A translation-only local model is the older setting, not a separate route.
        cfg.tasks.live = TaskRoute { connection: LOCAL.into(), model: "lfm".into() };
        let applied = apply(&cfg);
        assert!(applied.live.is_none());
        assert_eq!(applied.config.continuous.translation.model, LiveModel::Lfm);
    }

    #[test]
    fn a_task_naming_a_connection_that_is_gone_falls_back_and_says_so_and_saving_it_is_refused() {
        let mut cfg = parse(OLD);
        cfg.tasks = derive(&cfg).tasks;
        cfg.tasks.capture.connection = "Removed".into();
        cfg.tasks.live.connection = "Removed".into();
        cfg.tasks.live.model = "x".into();
        let applied = apply(&cfg);
        assert_eq!(applied.config.llm.endpoint, None, "this PC's model stands in");
        assert_eq!(applied.problems.len(), 2, "{:?}", applied.problems);
        assert!(applied.problems.iter().all(|p| p.contains("\"Removed\"")));
        let error = validate(&cfg).unwrap_err();
        assert!(error.contains("Captures") || error.contains("Live subtitles"), "{error}");

        cfg.tasks = derive(&parse(OLD)).tasks;
        validate(&cfg).unwrap();
        cfg.tasks.card_meanings.connection = String::new();
        assert!(validate(&cfg).unwrap_err().contains("Card meanings needs a connection"));
        cfg.tasks.card_meanings.connection = LOCAL.into();
        cfg.tasks.live = TaskRoute { connection: LOCAL.into(), model: "gpt".into() };
        assert!(validate(&cfg).is_err(), "an unknown local live model");
        validate(&parse("")).unwrap();
    }

    #[test]
    fn migrating_writes_the_table_once_keeps_every_other_line_and_never_changes_the_meaning() {
        let new = migrate(OLD).unwrap().expect("an older file is migrated");
        for line in OLD.lines().filter(|l| !l.trim().is_empty()) {
            assert!(new.contains(line), "kept: {line}\n{new}");
        }
        assert!(new.starts_with(OLD.trim_end()), "the table is added after what was there:\n{new}");
        assert!(new.contains("# One model per task"), "{new}");
        let cfg = parse(&new);
        assert_eq!(cfg.tasks, derive(&parse(OLD)).tasks);
        assert_eq!(cfg.tasks.version, VERSION);
        let mut same = cfg.clone();
        same.tasks = Default::default();
        assert_eq!(same, parse(OLD), "nothing else changed");
        validate(&cfg).unwrap();
        assert_eq!(migrate(&new).unwrap(), None, "a second run changes nothing");
        // What the engine is given is the same before and after, card meanings apart.
        assert_eq!(apply(&cfg).config.llm, apply(&parse(OLD)).config.llm);
        assert_eq!(apply(&cfg).config.continuous, apply(&parse(OLD)).config.continuous);
        assert_eq!(migrate("").unwrap().map(|t| parse(&t).tasks.capture.connection), Some(LOCAL.to_string()), "an empty file too");
    }

    #[test]
    fn migrating_saves_an_unnamed_server_as_a_connection_with_its_key() {
        let old = "[llm]\nendpoint = 'https://contoso.openai.azure.com'\nprovider = 'azure'\napi_key = 'k'\nmodel_name = 'dep'\n";
        let new = migrate(old).unwrap().unwrap();
        let cfg = parse(&new);
        assert_eq!(cfg.llm.servers.len(), 1);
        assert_eq!(cfg.llm.servers[0].name, "contoso.openai.azure.com");
        assert_eq!(cfg.llm.servers[0].api_key.as_ref().map(ServerKey::as_str), Some("k"));
        assert_eq!(cfg.tasks.capture.connection, "contoso.openai.azure.com");
        assert_eq!(cfg.llm.endpoint.as_deref(), Some("https://contoso.openai.azure.com"), "the older keys stay in the file");
        assert_eq!(migrate(&new).unwrap(), None);
    }

    #[test]
    fn a_file_rewritten_whole_by_an_older_save_is_still_migrated() {
        // `Config::save` writes an empty `[tasks]` (version 0): that is still an older file.
        let dir = std::env::temp_dir().join(format!("maru-tasks-save-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut cfg = parse("[llm]\nhf_repo = 'a/b:Q4'");
        cfg.tasks = Default::default();
        cfg.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let new = migrate(&text).unwrap().expect("version 0 is migrated");
        assert_eq!(parse(&new).tasks.card_meanings.connection, LOCAL);
        assert_eq!(parse(&new).tasks.version, VERSION);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_file_is_backed_up_beside_itself_before_it_is_changed_and_only_once() {
        let dir = std::env::temp_dir().join(format!("maru-tasks-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        assert_eq!(migrate_file(&path, 0).unwrap(), None, "no file, nothing to do");
        std::fs::write(&path, OLD).unwrap();
        let done = migrate_file(&path, 1_795_793_412).unwrap().expect("migrated");
        assert_eq!(done.backup.file_name().unwrap().to_str().unwrap(), "config.toml.before-tasks-20261127-153012.bak");
        assert_eq!(std::fs::read_to_string(&done.backup).unwrap(), OLD, "the backup is the file as it was");
        assert_eq!(done.tasks.card_meanings.connection, LOCAL);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(parse(&after).tasks.version, VERSION);
        assert_eq!(migrate_file(&path, 1_795_793_999).unwrap(), None, "already migrated: no second backup");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_command_line_override_of_the_address_still_decides_where_requests_go_after_migration() {
        let mut cfg = parse(&migrate(OLD).unwrap().unwrap());
        cfg.tasks.capture.connection = "NAS".into();
        cfg.tasks.live.connection = "NAS".into();
        cfg.route_by_older_fields();
        assert_eq!(cfg.llm.endpoint.as_deref(), Some("http://192.168.1.20:11434"), "the table's choice is in the older field");
        cfg.llm.override_endpoint("http://other:1".into());
        assert_eq!(apply(&cfg).config.llm.endpoint.as_deref(), Some("http://other:1"));
        cfg.llm.endpoint = None;
        assert_eq!(apply(&cfg).config.llm.endpoint, None);
    }

    #[test]
    fn the_backup_stamp_is_a_utc_date_and_time() {
        assert_eq!(utc_stamp(0), "19700101-000000");
        assert_eq!(utc_stamp(951_782_400), "20000229-000000");
        assert_eq!(utc_stamp(1_795_793_412), "20261127-153012");
    }
}
