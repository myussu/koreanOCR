//! What went wrong with a connection, as the few kinds a person can act on. The settings show one of these per
//! task, and a card request that fails with one moves on to the next connection in its order.

use serde::{Deserialize, Serialize};

use crate::LlmError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    /// No API key, or the service refused it.
    NoKey,
    /// The account has no quota or credit left, or is being rate limited.
    Quota,
    /// Nothing answered: the server is off, the address is wrong, the model did not load, or it stopped mid-way.
    Unreachable,
    /// It answered, but not with something usable (an unknown model name, a refused request, an empty reply).
    Failed,
}

fn mentions_quota(text: &str) -> bool {
    let text = text.to_lowercase();
    ["quota", "resource_exhausted", "rate limit", "rate_limit", "credit balance", "billing", "insufficient_quota", "too many requests"].iter().any(|w| text.contains(w))
}

fn mentions_key(text: &str) -> bool {
    let text = text.to_lowercase();
    ["api key", "api_key", "apikey", "unauthorized", "authentication", "permission_denied"].iter().any(|w| text.contains(w))
}

pub fn classify(error: &LlmError) -> Problem {
    match error {
        LlmError::Http { status, body } => match status {
            429 => Problem::Quota,
            _ if mentions_quota(body) => Problem::Quota,
            401 | 403 => Problem::NoKey,
            400 if mentions_key(body) => Problem::NoKey,
            500..=599 => Problem::Unreachable,
            _ => Problem::Failed,
        },
        // The client refuses to send a request a hosted service needs a key for ("Enter the … API key in Settings").
        LlmError::Api(message) if mentions_key(message) => Problem::NoKey,
        LlmError::Api(message) if mentions_quota(message) => Problem::Quota,
        LlmError::Api(_) | LlmError::Stream(_) => Problem::Failed,
        LlmError::Unreachable { .. } | LlmError::Stalled(_) | LlmError::Spawn(_) | LlmError::NotReady(_) => Problem::Unreachable,
    }
}

/// One plain sentence about `name` for a person: no status codes, no server text.
pub fn sentence(name: &str, problem: Problem) -> String {
    match problem {
        Problem::NoKey => format!("{name} has no API key, or the key was refused."),
        Problem::Quota => format!("{name} has no quota left."),
        Problem::Unreachable => format!("{name} cannot be reached."),
        Problem::Failed => format!("{name} did not give a usable answer."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, body: &str) -> LlmError {
        LlmError::Http { status, body: body.into() }
    }

    #[test]
    fn a_rate_limit_or_an_exhausted_account_is_a_quota_problem_whatever_the_status() {
        assert_eq!(classify(&http(429, "")), Problem::Quota);
        assert_eq!(classify(&http(429, r#"{"error":{"status":"RESOURCE_EXHAUSTED"}}"#)), Problem::Quota);
        assert_eq!(classify(&http(400, "Your credit balance is too low to access the API")), Problem::Quota);
        assert_eq!(classify(&http(403, "You exceeded your current quota")), Problem::Quota);
    }

    #[test]
    fn a_missing_or_refused_key_is_told_apart_from_a_server_that_is_down_and_a_bad_request() {
        assert_eq!(classify(&http(401, "invalid x-api-key")), Problem::NoKey);
        assert_eq!(classify(&http(403, "")), Problem::NoKey);
        assert_eq!(classify(&http(400, "API key not valid. Please pass a valid API key.")), Problem::NoKey);
        assert_eq!(classify(&LlmError::Api("Enter the Gemini API key in Settings.".into())), Problem::NoKey);
        assert_eq!(classify(&http(503, "high demand")), Problem::Unreachable);
        assert_eq!(classify(&LlmError::Stalled(60)), Problem::Unreachable);
        assert_eq!(classify(&LlmError::Spawn("no model file".into())), Problem::Unreachable);
        assert_eq!(classify(&http(404, "model not found")), Problem::Failed);
        assert_eq!(classify(&LlmError::Stream("cut".into())), Problem::Failed);
    }

    #[test]
    fn the_sentence_for_a_person_names_the_connection_and_nothing_technical() {
        assert_eq!(sentence("Gemini", Problem::Quota), "Gemini has no quota left.");
        for p in [Problem::NoKey, Problem::Quota, Problem::Unreachable, Problem::Failed] {
            let text = sentence("NAS", p);
            assert!(text.starts_with("NAS ") && !text.contains("HTTP") && !text.contains('{'), "{text}");
        }
    }
}
