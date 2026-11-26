//! Settings for captions sent by the browser extension (`[captions]`): a video's own Korean caption track arrives
//! as text on a local port instead of being read from the screen. The receiver is `maru-engine::captions`.

use serde::{Deserialize, Serialize};

use crate::external_servers::ServerKey;

/// The shortest code the receiver accepts from the settings; the ones Maru makes are 32 characters.
pub const MIN_TOKEN_CHARS: usize = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptionsConfig {
    /// Listen for caption lines from the extension. Off by default: nothing listens.
    pub enabled: bool,
    /// The port on this PC (127.0.0.1 only) the extension sends to.
    pub port: u16,
    /// The code every request must carry. Maru writes one the first time the feature is switched on; it is never logged.
    pub token: Option<ServerKey>,
    /// Only this extension may send, e.g. `chrome-extension://abcdefghijklmnopabcdefghijklmnop` (the extension's
    /// popup shows it). Empty accepts any browser extension that knows the code; web pages are always refused.
    pub allowed_origin: String,
}

impl Default for CaptionsConfig {
    fn default() -> Self {
        Self { enabled: false, port: 18096, token: None, allowed_origin: String::new() }
    }
}

impl CaptionsConfig {
    /// The code, when one is set.
    pub fn token(&self) -> Option<&str> {
        self.token.as_ref().map(ServerKey::as_str).filter(|t| !t.is_empty())
    }

    /// What is wrong with these settings, in words for the settings page.
    pub fn problems(&self) -> Vec<&'static str> {
        let mut problems = Vec::new();
        if !(1024..=65535).contains(&self.port) {
            problems.push("the port for YouTube captions must be between 1024 and 65535");
        }
        if let Some(token) = self.token()
            && (token.chars().count() < MIN_TOKEN_CHARS || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        {
            problems.push("the code for the browser extension must be at least 16 letters or digits (empty lets Maru make one)");
        }
        let origin = self.allowed_origin.trim();
        if !origin.is_empty() && !is_extension_origin(origin) {
            problems.push("the extension allowed to send captions must look like chrome-extension://… (empty accepts any extension that knows the code)");
        }
        problems
    }
}

/// Is this the `Origin` a browser extension sends (never a web page's)?
pub fn is_extension_origin(origin: &str) -> bool {
    ["chrome-extension://", "moz-extension://"].iter().any(|scheme| {
        origin.strip_prefix(scheme).is_some_and(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    #[test]
    fn a_config_written_before_captions_existed_parses_with_the_listener_off() {
        let old: Config = toml::from_str("[continuous]\ncontext_lines = 3").unwrap();
        assert_eq!(old.captions, CaptionsConfig::default());
        assert!(!old.captions.enabled && old.captions.token().is_none());
        let on: Config = toml::from_str("[captions]\nenabled = true").unwrap();
        assert!(on.captions.enabled);
        assert_eq!(on.captions.port, 18096, "the other keys keep their defaults");
    }

    #[test]
    fn the_documented_example_leaves_captions_off_with_the_real_default_port() {
        let example: Config = toml::from_str(include_str!("../../../docs/config.example.toml")).unwrap();
        assert_eq!(example.captions, CaptionsConfig::default());
    }

    #[test]
    fn the_code_is_never_printed_with_the_settings() {
        let cfg: Config = toml::from_str("[captions]\ntoken = \"0123456789abcdef0123456789abcdef\"").unwrap();
        assert_eq!(cfg.captions.token(), Some("0123456789abcdef0123456789abcdef"));
        assert!(!format!("{cfg:?}").contains("0123456789abcdef"), "Debug output goes to logs");
    }

    #[test]
    fn a_short_code_a_low_port_and_a_web_page_as_the_sender_are_refused_in_words() {
        assert!(CaptionsConfig::default().problems().is_empty());
        let short = CaptionsConfig { token: Some("abc".to_string().into()), ..Default::default() };
        assert!(short.problems()[0].contains("at least 16"));
        let low = CaptionsConfig { port: 80, ..Default::default() };
        assert!(low.problems()[0].contains("1024"));
        let page = CaptionsConfig { allowed_origin: "https://www.youtube.com".into(), ..Default::default() };
        assert!(page.problems()[0].contains("chrome-extension://"));
        let pinned = CaptionsConfig { allowed_origin: "chrome-extension://abcdefghijklmnopabcdefghijklmnop".into(), ..Default::default() };
        assert!(pinned.problems().is_empty());
        // An empty code in the file is the same as none: Maru makes one.
        let empty = CaptionsConfig { token: Some(String::new().into()), ..Default::default() };
        assert!(empty.problems().is_empty() && empty.token().is_none());
    }

    #[test]
    fn only_extension_origins_count_as_an_extension() {
        assert!(is_extension_origin("chrome-extension://abcdefghijklmnopabcdefghijklmnop"));
        assert!(is_extension_origin("moz-extension://1b2c3d4e-0000-4000-8000-123456789abc"));
        for not in ["https://www.youtube.com", "http://127.0.0.1:18096", "null", "chrome-extension://", "chrome-extension://abc/evil", ""] {
            assert!(!is_extension_origin(not), "{not:?}");
        }
    }
}
