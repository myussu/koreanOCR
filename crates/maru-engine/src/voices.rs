//! A voice per task (`[tasks.voice_reading]`, `[tasks.voice_cards]`): the one that reads aloud and the one on cards.
//!
//! Each is `[tts]` as it is, `[tts]` with another speaker, or another speech server altogether. When reading goes
//! to another server, the voice model on this PC is needed only to make card sound: it loads when cards are made
//! and [`Services::release_card_voice`] gives the graphics card back afterwards.

use maru_core::tasks::{VoiceRoute, voice_config};
use maru_core::{Config, Paths};
use maru_tts::Tts;

use crate::Services;

#[derive(Clone)]
pub struct Voice {
    pub tts: Tts,
    /// Another server than `[tts]` speaks: the voice model on this PC is not needed for it.
    pub elsewhere: bool,
}

/// `None` where speech is off or that voice could not be set up.
#[derive(Clone, Default)]
pub struct Voices {
    pub reading: Option<Voice>,
    pub cards: Option<Voice>,
}

impl Voices {
    /// The voice on this PC only makes card sound; reading goes to another server.
    pub fn local_voice_is_only_for_cards(&self) -> bool {
        self.reading.as_ref().is_some_and(|v| v.elsewhere) && self.cards.as_ref().is_some_and(|v| !v.elsewhere)
    }
}

/// `base` is the voice of `[tts]` when it could be set up.
pub(crate) fn build(cfg: &Config, paths: &Paths, base: Option<&Tts>, notes: &mut Vec<String>) -> Voices {
    if !cfg.tts.enabled {
        return Voices::default();
    }
    let mut one = |route: &VoiceRoute, what: &str| -> Option<Voice> {
        if !route.elsewhere() && route.voice.trim().is_empty() {
            return base.cloned().map(|tts| Voice { tts, elsewhere: false });
        }
        // Another speaker of `[tts]` still needs `[tts]` itself.
        if !route.elsewhere() && base.is_none() {
            return None;
        }
        match Tts::new(&voice_config(&cfg.tts, route), paths) {
            Ok(tts) => {
                if route.elsewhere() {
                    notes.push(format!("Speech {what}: from {}", tts.client().root()));
                }
                Some(Voice { tts, elsewhere: route.elsewhere() })
            }
            Err(e) => {
                notes.push(format!("Speech {what}: {e}"));
                None
            }
        }
    };
    Voices { reading: one(&cfg.tasks.voice_reading, "while reading"), cards: one(&cfg.tasks.voice_cards, "for cards") }
}

impl Services {
    /// Keep the model behind `voice` loaded while it speaks; a voice on another server needs nothing held.
    pub async fn voice_lease(&self, voice: &Voice) -> Result<Option<tokio::sync::OwnedRwLockReadGuard<()>>, String> {
        if voice.elsewhere {
            return Ok(None);
        }
        self.speech_lease().await
    }

    /// After cards are made: when the voice on this PC was loaded only for their sound, unload it. `true` when a
    /// loaded model left. Nothing happens while it is still speaking, or when reading uses the same voice.
    pub async fn release_card_voice(&self) -> bool {
        if !self.voices.local_voice_is_only_for_cards() {
            return false;
        }
        match &self.deferred_speech {
            Some(model) => matches!(model.release_idle().await, Ok(Some(true))),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maru_core::config::TtsConfig;

    /// A folder of its own for the audio cache, removed afterwards.
    struct Home(std::path::PathBuf);
    impl Home {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            Self(std::env::temp_dir().join(format!("maru-voices-test-{}-{nonce}", std::process::id())))
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn config(reading: VoiceRoute, cards: VoiceRoute) -> Config {
        let mut cfg = Config { tts: TtsConfig { enabled: true, ..Default::default() }, ..Default::default() };
        cfg.tasks.voice_reading = reading;
        cfg.tasks.voice_cards = cards;
        cfg
    }

    #[test]
    fn reading_can_go_to_another_server_while_cards_keep_the_voice_of_this_pc() {
        let home = Home::new();
        let paths = Paths::under(&home.0);
        let supertonic = VoiceRoute { endpoint: "http://127.0.0.1:18097".into(), voice: "F1".into(), model: "supertonic-3".into() };
        let cfg = config(supertonic, VoiceRoute::default());
        let base = Tts::new(&cfg.tts, &paths).unwrap();
        let mut notes = Vec::new();
        let voices = build(&cfg, &paths, Some(&base), &mut notes);

        let reading = voices.reading.as_ref().unwrap();
        assert!(reading.elsewhere && reading.tts.client().root().starts_with("http://127.0.0.1:18097"));
        let asked = reading.tts.client().request_body("안녕");
        assert_eq!((asked["voice"].as_str(), asked["model"].as_str()), (Some("F1"), Some("supertonic-3")));
        let cards = voices.cards.as_ref().unwrap();
        assert!(!cards.elsewhere && cards.tts.client().request_body("안녕")["voice"] == "sohee");
        assert!(voices.local_voice_is_only_for_cards(), "so the card voice can be unloaded after the cards are made");
        assert_eq!(notes, ["Speech while reading: from http://127.0.0.1:18097/v1"]);
        assert_ne!(reading.tts.client().cache_key("안녕"), cards.tts.client().cache_key("안녕"), "the two voices never share a cached clip");
    }

    #[test]
    fn nothing_chosen_is_one_voice_for_both_and_it_is_never_unloaded_behind_the_reader() {
        let home = Home::new();
        let paths = Paths::under(&home.0);
        let cfg = config(VoiceRoute::default(), VoiceRoute { voice: "vivian".into(), ..Default::default() });
        let base = Tts::new(&cfg.tts, &paths).unwrap();
        let voices = build(&cfg, &paths, Some(&base), &mut Vec::new());
        assert!(voices.reading.as_ref().is_some_and(|v| !v.elsewhere && v.tts.client().request_body("가")["voice"] == "sohee"));
        assert!(voices.cards.as_ref().is_some_and(|v| !v.elsewhere && v.tts.client().request_body("가")["voice"] == "vivian"), "another speaker of the same voice");
        assert!(!voices.local_voice_is_only_for_cards());

        // The voice of [tts] could not be set up: only a task with a server of its own can still speak.
        let cfg = config(VoiceRoute { endpoint: "http://nas:8080".into(), ..Default::default() }, VoiceRoute { voice: "vivian".into(), ..Default::default() });
        let voices = build(&cfg, &paths, None, &mut Vec::new());
        assert!(voices.reading.is_some() && voices.cards.is_none());

        let mut off = config(VoiceRoute { endpoint: "http://nas:8080".into(), ..Default::default() }, VoiceRoute::default());
        off.tts.enabled = false;
        let voices = build(&off, &paths, Some(&base), &mut Vec::new());
        assert!(voices.reading.is_none() && voices.cards.is_none(), "speech switched off is off for every task");
    }
}
