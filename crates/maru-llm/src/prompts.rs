//! Prompts.
//!
//! Design rules that keep latency low without hurting quality:
//! * the **system prompt of each task is a constant**, so llama-server's prefix
//!   cache re-uses it on every request (only the user turn changes);
//! * outputs are **line-oriented** (`N|gloss|note`, `POLITE: …`) rather than JSON:
//!   they stream token by token, cost fewer tokens, and a half-finished answer is
//!   still usable;
//! * the model is **told what to decide, not what we already know**: Kiwi and the
//!   dictionary supply structure and candidate meanings, the model only picks the
//!   meaning that fits the sentence.

use std::collections::BTreeMap;
use std::fmt::Write;

use maru_core::Register;

use crate::client::Message;

#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    /// Free-text hint about the source ("dialogue from a fantasy RPG").
    pub hint: Option<String>,
    /// Fixed translations for names and terms.
    pub glossary: BTreeMap<String, String>,
    /// Recent lines `(source, translation)`, oldest first.
    pub history: Vec<(String, String)>,
    /// The text came from OCR and may contain recognition mistakes.
    pub from_ocr: bool,
}

impl PromptContext {
    fn render_head(&self, out: &mut String) {
        if let Some(h) = &self.hint {
            let _ = writeln!(out, "Context: {h}");
        }
        if !self.glossary.is_empty() {
            out.push_str("Glossary (always use these translations):\n");
            for (k, v) in &self.glossary {
                let _ = writeln!(out, "- {k} = {v}");
            }
        }
    }

    fn render_ocr_note(&self, out: &mut String) {
        if self.from_ocr {
            out.push_str(
                "Note: the text was read from the screen by OCR and may contain small recognition errors; \
                 where the intended text is obvious, translate the intended text.\n",
            );
        }
    }

    fn render(&self, out: &mut String) {
        self.render_head(out);
        if !self.history.is_empty() {
            out.push_str("Previous lines (for reference only, do not translate them):\n");
            for (src, tr) in &self.history {
                let _ = writeln!(out, "- {src}  =>  {tr}");
            }
        }
        self.render_ocr_note(out);
    }
}

pub const KO_EN_SYSTEM: &str = "\
You translate Korean into natural, faithful English for a language learner who sees your translation next to the original.
Rules:
- Translate meaning and tone. Keep the politeness level audible in English where it is natural (please, sir, casual contractions).
- Fragments (menu items, buttons, titles, headlines) stay fragments.
- Names: use the glossary if given, otherwise standard romanization.
- Never add explanations, notes, quotes, labels or alternatives. Output only the English translation.";

/// Added to [`KO_EN_SYSTEM`] for live subtitles. Constant, so the server's prefix cache keeps it.
pub const LIVE_NOTE: &str = "\
These lines are subtitles shown one at a time, each continuing the lines before it. Keep names, pronouns and tone consistent with the earlier lines. \
Subtitles are spoken language: keep them short and natural, never explain.";

pub const GLOSS_SYSTEM: &str = "\
You help a learner read Korean. You are given a sentence and a numbered list of its words, each with its morpheme breakdown and optional dictionary candidates.
For every listed word give its meaning IN THIS SENTENCE:
- 1 to 4 English words, dictionary style, lowercase, no trailing punctuation.
- Verbs and adjectives: the base meaning without tense or politeness (\"to study\", \"big\"). If an ending or auxiliary adds meaning that matters (-고 싶다 = want to, -아/어 보다 = try), include it in the gloss.
- Prefer one of the dictionary candidates when it fits; choose another meaning when the sentence demands it (idioms, names).
Output exactly one line per word, in order, in this format and nothing else:
N|gloss|note
where N is the word's number and note is optional (at most 12 words: an idiom, nuance, or a grammar point worth remembering; leave empty if nothing is worth saying).
After the words, add one final line `NOTE|...` with a single short remark on the sentence's grammar or nuance only if it genuinely helps a learner; otherwise omit it.";

pub const EN_KO_SYSTEM: &str = "\
You translate English into natural, idiomatic Korean for a language learner. Write what a native speaker would actually say, not a word-for-word rendering.
Output exactly these lines and nothing else (Korean in Hangul only, no romanization, no quotes):
POLITE: <해요체, everyday polite>
FORMAL: <합쇼체, formal/deferential (-ㅂ니다/-습니다)>
CASUAL: <반말, casual speech between close friends>
NOTE: <one short English sentence: when to use which, or any nuance the learner should know>
Use the glossary if given. Keep proper nouns as given or in standard Korean transliteration.";

pub const DEEP_SYSTEM: &str = "\
You are a concise Korean tutor explaining one word to an English-speaking learner who just met it in a real sentence.
Output exactly these sections, each starting on its own line with the label shown:
USAGE: <2-3 sentences: what the word means in this sentence, how it is typically used, register (polite/casual/written), common collocations>
EXAMPLES:
- <Korean sentence> — <English> (three short examples at beginner level using this exact dictionary form or a common conjugation)
NUANCE: <one or two sentences: how it differs from a similar word learners confuse it with; omit the line if there is nothing useful to say>
No other text.";

pub const LABEL_SYSTEM: &str = "\
You label Korean text seen on a screen (video game menus, buttons, tooltips, HUD, signs) with a SHORT English meaning, for a learner who keeps the original text visible.
You receive numbered strings. Each comes from a different place on the screen, so they are usually unrelated to each other.
For each string:
- Give the wording an English version of that game or app would use: 확인 -> OK, 저장하기 -> Save, 인벤토리 -> Inventory, 체력 -> HP.
- Labels and buttons: 1 to 4 words. A full sentence: a faithful translation of at most 12 words.
- People and places: standard romanization. Keep digits, units and symbols as they are.
- If a string is not meaningful Korean (screen-reading noise), write ?
Output exactly one line per string, in order, and nothing else:
N|label";

fn register_name(r: Register) -> &'static str {
    match r {
        Register::Polite => "POLITE",
        Register::Formal => "FORMAL",
        Register::Casual => "CASUAL",
    }
}

pub fn ko_en(text: &str, ctx: &PromptContext) -> Vec<Message> {
    let mut user = String::new();
    ctx.render(&mut user);
    let _ = write!(user, "Translate into English:\n{text}");
    vec![Message::system(KO_EN_SYSTEM), Message::user(user)]
}

/// Live subtitles: the earlier lines are real chat turns (same format as the question), so the model
/// continues the pattern, and the part of the prompt before the newest line changes as little as
/// possible from one request to the next.
pub fn ko_en_live(text: &str, ctx: &PromptContext, turns: &[(String, String)]) -> Vec<Message> {
    let mut system = format!("{KO_EN_SYSTEM}\n{LIVE_NOTE}\n");
    ctx.render_head(&mut system);
    ctx.render_ocr_note(&mut system);
    let mut messages = vec![Message::system(system.trim_end())];
    for (source, translation) in turns {
        messages.push(Message::user(format!("Translate into English:\n{source}")));
        messages.push(Message::assistant(translation.as_str()));
    }
    messages.push(Message::user(format!("Translate into English:\n{text}")));
    messages
}

pub struct GlossWordPrompt<'a> {
    pub index: usize,
    pub surface: &'a str,
    /// "공부 + 하다 + 고"
    pub parts: &'a str,
    /// Dictionary candidates, best first.
    pub candidates: &'a [String],
}

pub fn glosses(sentence: &str, words: &[GlossWordPrompt<'_>], ctx: &PromptContext) -> Vec<Message> {
    let mut user = String::new();
    ctx.render(&mut user);
    let _ = writeln!(user, "Sentence: {sentence}\nWords:");
    for w in words {
        let _ = write!(user, "{}. {}", w.index, w.surface);
        if !w.parts.is_empty() {
            let _ = write!(user, " ({})", w.parts);
        }
        if !w.candidates.is_empty() {
            let _ = write!(user, " candidates: {}", w.candidates.join("; "));
        }
        user.push('\n');
    }
    vec![Message::system(GLOSS_SYSTEM), Message::user(user)]
}

/// A batch of screen strings to label. `items` are `(number, text)`.
pub fn labels(items: &[(usize, &str)], ctx: &PromptContext) -> Vec<Message> {
    let mut user = String::new();
    ctx.render(&mut user);
    user.push_str("Strings:\n");
    for (n, text) in items {
        let _ = writeln!(user, "{n}|{text}");
    }
    vec![Message::system(LABEL_SYSTEM), Message::user(user)]
}

pub fn en_ko(text: &str, register: Register, ctx: &PromptContext) -> Vec<Message> {
    let mut user = String::new();
    ctx.render(&mut user);
    let _ = write!(
        user,
        "Translate into Korean. Write the {} line first.\n{text}",
        register_name(register)
    );
    vec![Message::system(EN_KO_SYSTEM), Message::user(user)]
}

pub fn deep(key: &str, surface: &str, sentence: &str, dictionary: &str, ctx: &PromptContext) -> Vec<Message> {
    let mut user = String::new();
    ctx.render(&mut user);
    let _ = writeln!(user, "Word: {key}  (appears as: {surface})\nSentence: {sentence}");
    if !dictionary.is_empty() {
        let _ = writeln!(user, "Dictionary: {dictionary}");
    }
    vec![Message::system(DEEP_SYSTEM), Message::user(user)]
}

/// Messages that only exist to fill the KV cache with each task's system prompt.
pub fn warmup() -> Vec<Vec<Message>> {
    [KO_EN_SYSTEM, GLOSS_SYSTEM, EN_KO_SYSTEM, DEEP_SYSTEM, LABEL_SYSTEM]
        .into_iter()
        .map(|s| vec![Message::system(s), Message::user("ok")])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_prompt_keeps_the_start_constant_and_adds_earlier_lines_as_turns() {
        let mut ctx = PromptContext { hint: Some("fantasy RPG".into()), from_ocr: true, history: vec![("무시".into(), "ignored".into())], ..Default::default() };
        ctx.glossary.insert("이도현".into(), "Lee Do-hyun".into());
        let turns = vec![("안녕".to_string(), "Hi".to_string()), ("누구세요".to_string(), "Who are you?".to_string())];
        let a = ko_en_live("저는 이도현입니다", &ctx, &turns);
        let b = ko_en_live("다음 줄", &ctx, &turns);

        assert_eq!(a.len(), 6);
        assert_eq!(a.iter().map(|m| m.role).collect::<Vec<_>>(), ["system", "user", "assistant", "user", "assistant", "user"]);
        assert_eq!(a[0].content, b[0].content, "the system part does not depend on the line");
        for needle in [KO_EN_SYSTEM, LIVE_NOTE, "fantasy RPG", "이도현 = Lee Do-hyun", "OCR"] {
            assert!(a[0].content.contains(needle), "missing {needle}");
        }
        assert!(!a.iter().any(|m| m.content.contains("ignored")), "the single-message history is not used");
        assert_eq!(a[1].content, "Translate into English:\n안녕");
        assert_eq!(a[2].content, "Hi");
        assert_eq!(a[5].content, "Translate into English:\n저는 이도현입니다");
        assert_eq!(ko_en_live("x", &PromptContext::default(), &[]).len(), 2);
    }

    #[test]
    fn context_is_rendered_into_the_user_turn_only() {
        let mut ctx = PromptContext { hint: Some("fantasy RPG".into()), from_ocr: true, ..Default::default() };
        ctx.glossary.insert("이도현".into(), "Lee Do-hyun".into());
        ctx.history.push(("안녕".into(), "Hi".into()));
        let m = ko_en("저는 이도현입니다", &ctx);
        assert_eq!(m[0].content, KO_EN_SYSTEM, "system prompt must stay constant for prefix caching");
        let user = &m[1].content;
        for needle in ["fantasy RPG", "이도현 = Lee Do-hyun", "안녕  =>  Hi", "OCR", "저는 이도현입니다"] {
            assert!(user.contains(needle), "missing {needle}: {user}");
        }
    }

    #[test]
    fn gloss_prompt_lists_numbered_words_with_candidates() {
        let cands = vec!["to study".to_string(), "to learn".to_string()];
        let words = [GlossWordPrompt { index: 2, surface: "공부하고", parts: "공부 + 하다 + 고", candidates: &cands }];
        let m = glosses("공부하고 있어요", &words, &PromptContext::default());
        assert!(m[1].content.contains("2. 공부하고 (공부 + 하다 + 고) candidates: to study; to learn"));
    }

    #[test]
    fn requested_register_goes_first() {
        let m = en_ko("Thank you", Register::Formal, &PromptContext::default());
        assert!(m[1].content.contains("Write the FORMAL line first"));
    }

    #[test]
    fn every_task_has_a_warmup_prompt() {
        assert_eq!(warmup().len(), 5);
    }

    #[test]
    fn label_prompt_numbers_the_strings_and_keeps_the_system_prompt_constant() {
        let ctx = PromptContext { hint: Some("farming sim menus".into()), ..Default::default() };
        let m = labels(&[(0, "확인"), (7, "인벤토리")], &ctx);
        assert_eq!(m[0].content, LABEL_SYSTEM);
        assert!(m[1].content.contains("farming sim menus") && m[1].content.contains("0|확인\n7|인벤토리\n"), "{}", m[1].content);
    }
}
