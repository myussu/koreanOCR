//! Which dictionary sense a word has in its sentence.
//!
//! The meanings on a word card come from the learner's dictionary, not from the model: the model is given the
//! sentence and each word's numbered senses and only answers with a number. A word the dictionary does not have
//! gets a short English and Spanish meaning from the model instead, in the same request. As everywhere, the system
//! prompt is constant and the answer is one line per word.

use std::fmt::Write as _;

use crate::client::{ChatOpts, LlmClient, Message};
use crate::demux::LineSplitter;
use crate::Result;

pub const SENSE_SYSTEM: &str = "\
You help a learner of Korean make flashcards. You are given a Korean sentence and a numbered list of words from it. Most words come with the numbered senses a dictionary lists for them.
For every listed word answer with one line:
- A word with senses: `N|k`, where N is the word's number and k is the number of the sense the word has IN THIS SENTENCE. Choose by the sentence, not by which sense is listed first. Only if no listed sense fits at all, answer `N|0|english|spanish` instead.
- A word without senses: `N|0|english|spanish`, its meaning in this sentence in English and in Spanish: 1 to 4 words each, dictionary style, lowercase, verbs in the base form (\"to study\" and \"estudiar\").
Output exactly one line per word, in the order given, and nothing else.";

/// The instruction added when the model may also correct a word's dictionary form (`learning.anki.correct_forms`).
/// It is a separate prompt because it changes which senses a small model chooses: on the 50 review lines of run 2
/// the local model corrected no form with it and gave another meaning for words it had right without it.
const FORMS_RULE: &str = "The dictionary form in parentheses was found by a program. Only when it is clearly not the word used in this sentence (dialect, or letters misread on screen), answer `N|0|english|spanish|=form`, where form is the right dictionary form.";

/// [`SENSE_SYSTEM`] with [`FORMS_RULE`] before its last line.
pub fn sense_system_with_forms() -> String {
    let (rules, last) = SENSE_SYSTEM.rsplit_once('\n').unwrap_or((SENSE_SYSTEM, ""));
    format!("{rules}\n{FORMS_RULE}\n{last}")
}

/// How many senses of one word are shown to the model, and how much of each definition.
pub const MAX_SENSES: usize = 8;
/// How many more are shown in a second request, when the model said none of the first ones fits.
pub const MORE_SENSES: usize = 16;
const MAX_DEFINITION: usize = 110;

#[derive(Debug, Clone, PartialEq)]
pub struct SenseOption {
    pub en: String,
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SenseWord {
    /// The number the answer line carries.
    pub index: usize,
    /// As written in the sentence (학교에서).
    pub surface: String,
    /// Dictionary form (학교).
    pub lemma: String,
    /// Empty when the dictionary does not have the word.
    pub senses: Vec<SenseOption>,
}

pub struct SenseRequest<'a> {
    pub sentence: &'a str,
    pub words: &'a [SenseWord],
}

#[derive(Debug, Clone, PartialEq)]
pub enum SenseAnswer {
    /// The `sense`-th listed sense (from 1).
    Sense { index: usize, sense: usize },
    /// A meaning of the model's own, for a word without (fitting) senses.
    Free { index: usize, en: String, es: String },
}

impl SenseAnswer {
    pub fn index(&self) -> usize {
        match self {
            SenseAnswer::Sense { index, .. } | SenseAnswer::Free { index, .. } => *index,
        }
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

pub fn messages(req: &SenseRequest<'_>) -> Vec<Message> {
    messages_showing(req, MAX_SENSES, false)
}

/// As [`messages`], showing up to `max` senses of each word; `forms` also lets the model correct a dictionary form.
pub fn messages_showing(req: &SenseRequest<'_>, max: usize, forms: bool) -> Vec<Message> {
    let mut user = String::new();
    let _ = writeln!(user, "Sentence: {}\nWords:", req.sentence);
    for w in req.words {
        if w.surface == w.lemma || w.lemma.is_empty() {
            let _ = writeln!(user, "{}. {}", w.index, w.surface);
        } else {
            let _ = writeln!(user, "{}. {} ({})", w.index, w.surface, w.lemma);
        }
        for (n, s) in w.senses.iter().take(max).enumerate() {
            let definition = clip(s.definition.trim(), MAX_DEFINITION);
            if definition.is_empty() {
                let _ = writeln!(user, "   {}) {}", n + 1, s.en);
            } else {
                let _ = writeln!(user, "   {}) {} — {}", n + 1, s.en, definition);
            }
        }
    }
    vec![if forms { Message::system(sense_system_with_forms()) } else { Message::system(SENSE_SYSTEM) }, Message::user(user)]
}

/// Parse one answer line: `3|2`, or `3|0|to study|estudiar`. Tolerates bullets, bold, `3. 2` and `3: 2`.
pub fn parse_line(line: &str) -> Option<SenseAnswer> {
    let line = line.trim().trim_start_matches(['-', '*', '•', '`', ' ']).trim_end_matches(['*', '`']).trim();
    let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
    let index: usize = digits.parse().ok()?;
    let rest = line[digits.len()..].trim_start_matches(['|', '.', ':', ')', ' ']);
    let mut parts = rest.split('|').map(|p| p.trim().trim_matches(['*', '`', '"']).trim());
    let choice = parts.next()?;
    let number: String = choice.chars().take_while(char::is_ascii_digit).collect();
    let sense: usize = number.parse().ok()?;
    if sense > 0 {
        return Some(SenseAnswer::Sense { index, sense });
    }
    let mut meanings = parts.filter(|p| !p.starts_with('='));
    let en = meanings.next().unwrap_or_default().to_string();
    let es = meanings.next().unwrap_or_default().to_string();
    (!en.is_empty() || !es.is_empty()).then_some(SenseAnswer::Free { index, en, es })
}

/// The dictionary form a model says a word really has in the sentence: the `=form` at the end of `3|0|to give|dar|=주다`.
pub fn parse_correction(line: &str) -> Option<(usize, String)> {
    let line = line.trim().trim_start_matches(['-', '*', '•', '`', ' ']);
    let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
    let index: usize = digits.parse().ok()?;
    let form = line.rsplit('|').next()?.trim().strip_prefix('=')?.trim().trim_matches(['*', '`', '"', '.', ' ']);
    // A dictionary form is Korean: anything else after `=` is the model explaining itself.
    let korean = !form.is_empty() && form.chars().count() <= 12 && form.chars().all(|c| ('가'..='힣').contains(&c));
    korean.then(|| (index, form.to_string()))
}

impl LlmClient {
    /// One request for every word of the line; answers arrive line by line. Lines that name no listed word are dropped.
    pub async fn choose_senses(&self, req: &SenseRequest<'_>, sink: &mut (dyn FnMut(SenseAnswer) + Send)) -> Result<()> {
        self.choose_senses_showing(req, MAX_SENSES, false, sink, &mut |_, _| {}).await
    }

    /// As [`LlmClient::choose_senses`], showing up to `max` senses of each word. With `forms` the model may also say
    /// that a word's dictionary form is wrong and give the right one (`corrected(word number, form)`).
    pub async fn choose_senses_showing(&self, req: &SenseRequest<'_>, max: usize, forms: bool, sink: &mut (dyn FnMut(SenseAnswer) + Send), corrected: &mut (dyn FnMut(usize, String) + Send)) -> Result<()> {
        let messages = messages_showing(req, max, forms);
        let opts = ChatOpts { max_tokens: Some(32 + 32 * req.words.len() as u32), temperature: Some(0.0), ..Default::default() };
        let mut lines = LineSplitter::default();
        let mut emit = |l: &str| {
            if let Some(answer) = parse_line(l)
                && req.words.iter().any(|w| w.index == answer.index())
            {
                if forms
                    && let Some((index, form)) = parse_correction(l)
                    && req.words.iter().any(|w| w.index == index && w.lemma != form)
                {
                    corrected(index, form);
                }
                sink(answer);
            }
        };
        self.stream_chat(&messages, &opts, &mut |d| lines.push(d, &mut emit)).await?;
        lines.finish(&mut emit);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::testing::{MockResponse, sse_answer, spawn_mock};
    use maru_core::config::LlmConfig;

    fn words() -> Vec<SenseWord> {
        let sense = |en: &str, definition: &str| SenseOption { en: en.into(), definition: definition.into() };
        vec![
            SenseWord { index: 1, surface: "배가".into(), lemma: "배".into(), senses: vec![sense("belly; stomach", "The part of the body."), sense("ship; boat", "A vehicle on water."), sense("pear", "")] },
            SenseWord { index: 3, surface: "억수로".into(), lemma: "억수로".into(), senses: vec![] },
        ]
    }

    #[test]
    fn the_request_lists_each_word_with_its_numbered_senses_after_a_constant_system_prompt() {
        let m = messages(&SenseRequest { sentence: "배가 억수로 고파요", words: &words() });
        assert_eq!(m[0].content, SENSE_SYSTEM);
        assert_eq!(
            m[1].content,
            "Sentence: 배가 억수로 고파요\nWords:\n1. 배가 (배)\n   1) belly; stomach — The part of the body.\n   2) ship; boat — A vehicle on water.\n   3) pear\n3. 억수로\n"
        );
    }

    #[test]
    fn long_definitions_and_long_sense_lists_are_cut_so_one_word_cannot_fill_the_request() {
        let long = "x".repeat(400);
        let many: Vec<SenseOption> = (0..20).map(|n| SenseOption { en: format!("sense {n}"), definition: long.clone() }).collect();
        let m = messages(&SenseRequest { sentence: "하다", words: &[SenseWord { index: 0, surface: "하다".into(), lemma: "하다".into(), senses: many }] });
        assert_eq!(m[1].content.matches(") sense ").count(), MAX_SENSES);
        assert!(m[1].content.lines().all(|l| l.chars().count() < 160), "{}", m[1].content);
    }

    #[test]
    fn answer_lines_are_a_sense_number_or_a_meaning_in_two_languages() {
        assert_eq!(parse_line("1|2"), Some(SenseAnswer::Sense { index: 1, sense: 2 }));
        assert_eq!(parse_line(" - **3. 1**"), Some(SenseAnswer::Sense { index: 3, sense: 1 }));
        assert_eq!(parse_line("3|2) ship; boat"), Some(SenseAnswer::Sense { index: 3, sense: 2 }), "a model that repeats the sense");
        assert_eq!(parse_line("3|0|extremely|muchísimo"), Some(SenseAnswer::Free { index: 3, en: "extremely".into(), es: "muchísimo".into() }));
        assert_eq!(parse_line("3|0|extremely"), Some(SenseAnswer::Free { index: 3, en: "extremely".into(), es: String::new() }));
        for junk in ["", "Here are the answers:", "3|0", "3|", "three|1", "3|x"] {
            assert_eq!(parse_line(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn a_line_may_end_with_the_dictionary_form_the_model_says_the_word_really_has() {
        let line = "3|0|to give|dar|=주다";
        assert_eq!(parse_line(line), Some(SenseAnswer::Free { index: 3, en: "to give".into(), es: "dar".into() }), "the meanings are read as before");
        assert_eq!(parse_correction(line), Some((3, "주다".to_string())));
        assert_eq!(parse_correction(" - 3|0|to give|dar| = 주다. "), Some((3, "주다".to_string())));
        assert_eq!(parse_line("3|0|to give|=주다"), Some(SenseAnswer::Free { index: 3, en: "to give".into(), es: String::new() }), "the form is never taken for the Spanish");
        for none in ["3|2", "3|0|to give|dar", "3|0|to give|dar|=give", "3|0|a|b|=", "x|0|a|b|=주다", "3|0|a|b|=주다 (dialect of 주다)"] {
            assert_eq!(parse_correction(none), None, "{none:?}");
        }
        assert!(!SENSE_SYSTEM.contains("|=form"), "the prompt measured in run 1 is unchanged");
        let with_forms = sense_system_with_forms();
        assert!(with_forms.contains("|=form") && with_forms.starts_with(&SENSE_SYSTEM[..200]) && with_forms.ends_with("and nothing else."), "{with_forms}");
    }

    #[tokio::test]
    async fn a_corrected_form_is_passed_on_only_for_a_listed_word_and_only_when_it_differs() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["1|0|belly|barriga|=배\n3|0|extremely|muchísimo|=억수\n9|0|x|y|=가다\n"]), delay_ms: 0 }).await;
        let client = LlmClient::new(&server.url, &LlmConfig::default());
        let (mut got, mut forms) = (Vec::new(), Vec::new());
        client.choose_senses_showing(&SenseRequest { sentence: "배가 억수로 고파요", words: &words() }, MAX_SENSES, true, &mut |a| got.push(a), &mut |i, f| forms.push((i, f))).await.unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(forms, [(3, "억수".to_string())], "배 → 배 is no correction, and word 9 was not asked about");
        assert!(server.requests.lock().unwrap()[0].1.contains("=form"), "the request asked for forms");
        // Not asked for: the same answer's form is ignored and the prompt is run 1's.
        let (mut got, mut forms) = (Vec::new(), Vec::new());
        client.choose_senses_showing(&SenseRequest { sentence: "배가 억수로 고파요", words: &words() }, MAX_SENSES, false, &mut |a| got.push(a), &mut |i, f: String| forms.push((i, f))).await.unwrap();
        assert!(got.len() == 2 && forms.is_empty());
        assert!(!server.requests.lock().unwrap()[1].1.contains("=form"));
    }

    #[test]
    fn a_second_request_can_show_more_senses_than_the_first() {
        let many: Vec<SenseOption> = (0..30).map(|n| SenseOption { en: format!("sense {n}"), definition: String::new() }).collect();
        let req = SenseRequest { sentence: "하다", words: &[SenseWord { index: 0, surface: "하다".into(), lemma: "하다".into(), senses: many }] };
        assert_eq!(messages_showing(&req, MORE_SENSES, false)[1].content.matches(") sense ").count(), MORE_SENSES);
        assert_eq!(messages(&req)[1].content.matches(") sense ").count(), MAX_SENSES);
    }

    #[tokio::test]
    async fn one_request_answers_every_word_and_lines_for_other_numbers_are_dropped() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["1|2\n7|1\n", "3|0|extremely|muchísimo"]), delay_ms: 0 }).await;
        let client = LlmClient::new(&server.url, &LlmConfig::default());
        let mut got = Vec::new();
        client.choose_senses(&SenseRequest { sentence: "배가 억수로 고파요", words: &words() }, &mut |a| got.push(a)).await.unwrap();
        assert_eq!(got, [SenseAnswer::Sense { index: 1, sense: 2 }, SenseAnswer::Free { index: 3, en: "extremely".into(), es: "muchísimo".into() }]);
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].1.contains("1) belly; stomach"), "{}", requests[0].1);
    }
}
