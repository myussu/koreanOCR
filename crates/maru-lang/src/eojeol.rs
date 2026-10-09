//! From a flat morpheme list to learner-facing 어절 (space-delimited words):
//! dictionary form, part of speech, and a "why is it built this way" formula.

use maru_core::event::ConjugationStep;
use maru_core::{Eojeol, Morpheme, Pos, Role, WordState};

use crate::morph::RawToken;
use crate::pronounce;
use crate::tags;

/// Character spans of whitespace-separated words.
fn spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut n = 0;
    for (i, c) in text.chars().enumerate() {
        n = i + 1;
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, n));
    }
    out
}

fn to_morpheme(t: &RawToken) -> Morpheme {
    Morpheme {
        form: t.form.clone(),
        tag: t.tag.clone(),
        role: tags::role_of(&t.tag),
        label: tags::describe(&t.form, &t.tag),
        start: t.start,
        end: t.end,
    }
}

/// How a morpheme is shown inside a formula: predicates in dictionary style.
pub fn display_form(m: &Morpheme) -> String {
    match tags::base_tag(&m.tag) {
        "VV" | "VA" | "VX" | "VCP" | "VCN" | "XSV" | "XSA" if !m.form.ends_with('다') => format!("{}다", m.form),
        _ => m.form.clone(),
    }
}

fn is_nominal(tag: &str) -> bool {
    matches!(tags::base_tag(tag), "NNG" | "NNP" | "NR" | "XR")
}

/// Dictionary form and part of speech of the main word.
fn derive_lemma(morphs: &[Morpheme]) -> (Option<String>, Pos) {
    // Prefer a real stem; auxiliary verbs (VX) only if nothing else is there.
    let first = morphs
        .iter()
        .position(|m| m.role == Role::Stem && tags::base_tag(&m.tag) != "VX")
        .or_else(|| morphs.iter().position(|m| m.role == Role::Stem));
    let Some(i) = first else { return (None, Pos::Other) };
    let m = &morphs[i];
    let tag = tags::base_tag(&m.tag);

    match tag {
        "VV" | "VX" => (Some(format!("{}다", m.form)), Pos::Verb),
        "VA" => (Some(format!("{}다", m.form)), Pos::Adjective),
        _ if is_nominal(tag) => {
            // Adjacent nominals inside one 어절 form a compound (한국 + 어 → 한국어).
            let mut end = i + 1;
            let mut word = m.form.clone();
            while end < morphs.len() && is_nominal(&morphs[end].tag) {
                word.push_str(&morphs[end].form);
                end += 1;
            }
            match morphs.get(end).map(|n| tags::base_tag(&n.tag)) {
                Some("XSV") => (Some(format!("{word}{}다", morphs[end].form)), Pos::Verb),
                Some("XSA") => (Some(format!("{word}{}다", morphs[end].form)), Pos::Adjective),
                _ if tag == "NR" => (Some(word), Pos::Numeral),
                _ => (Some(word), Pos::Noun),
            }
        }
        "NNB" => (Some(m.form.clone()), Pos::Noun),
        "NP" => (Some(m.form.clone()), Pos::Pronoun),
        "MAG" | "MAJ" => (Some(m.form.clone()), Pos::Adverb),
        "MM" => (Some(m.form.clone()), Pos::Determiner),
        "IC" => (Some(m.form.clone()), Pos::Interjection),
        _ => (Some(m.form.clone()), Pos::Other),
    }
}

fn meaningful(m: &Morpheme) -> bool {
    m.role != Role::Symbol
}

/// `했어요 = 하다 + 었 (past tense) + 어요 (polite ending)`
fn build_formula(morphs: &[Morpheme]) -> String {
    let parts: Vec<&Morpheme> = morphs.iter().filter(|m| meaningful(m)).collect();
    if parts.len() < 2 {
        return String::new();
    }
    parts
        .iter()
        .enumerate()
        .map(|(i, m)| if i == 0 { display_form(m) } else { format!("{} ({})", display_form(m), m.label) })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Steps for the deep-dive "how is this form built" view.
pub fn conjugation_steps(e: &Eojeol) -> Vec<ConjugationStep> {
    e.morphs
        .iter()
        .filter(|m| meaningful(m))
        .map(|m| ConjugationStep { form: display_form(m), label: m.label.clone() })
        .collect()
}

/// Strip trailing sentence punctuation before phonetic processing.
fn core_text(surface: &str) -> &str {
    surface.trim_matches(|c: char| !c.is_alphanumeric() && !crate::hangul::is_hangul(c))
}

pub fn group(text: &str, tokens: &[RawToken]) -> Vec<Eojeol> {
    let spans = spans(text);
    if spans.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();

    let mut buckets: Vec<Vec<Morpheme>> = vec![Vec::new(); spans.len()];
    for t in tokens {
        // The span that contains the token start; tolerate off-by-one analysers.
        let idx = spans
            .iter()
            .position(|&(s, e)| t.start >= s && t.start < e)
            .or_else(|| spans.iter().rposition(|&(s, _)| s <= t.start))
            .unwrap_or(0);
        buckets[idx].push(to_morpheme(t));
    }

    spans
        .into_iter()
        .zip(buckets)
        .enumerate()
        .map(|(index, ((start, end), mut morphs))| {
            morphs.sort_by_key(|m| (m.start, m.end));
            let surface: String = chars[start..end].iter().collect();
            let (lemma, pos) = derive_lemma(&morphs);
            let content = lemma.is_some()
                && !matches!(pos, Pos::Other if !morphs.iter().any(|m| m.role == Role::Stem));
            let pos = if morphs.is_empty() { Pos::Other } else { pos };
            let core = core_text(&surface);
            Eojeol {
                index,
                char_start: start,
                char_end: end,
                formula: build_formula(&morphs),
                romanization: pronounce::romanize_word(core),
                pronunciation: pronounce::pronounce_word(core),
                lemma,
                pos,
                content,
                morphs,
                surface,
                dict_glosses: Vec::new(),
                level: None,
                state: WordState::New,
                lookups: 0,
                rect: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokens as Kiwi reports them for "저는 한국어를 공부하고 있어요."
    fn sample() -> (&'static str, Vec<RawToken>) {
        (
            "저는 한국어를 공부하고 있어요.",
            vec![
                RawToken::new("저", "NP", 0, 1),
                RawToken::new("는", "JX", 1, 2),
                RawToken::new("한국어", "NNP", 3, 6),
                RawToken::new("를", "JKO", 6, 7),
                RawToken::new("공부", "NNG", 8, 10),
                RawToken::new("하", "XSV", 10, 11),
                RawToken::new("고", "EC", 11, 12),
                RawToken::new("있", "VX", 13, 14),
                RawToken::new("어요", "EF", 14, 16),
                RawToken::new(".", "SF", 16, 17),
            ],
        )
    }

    #[test]
    fn groups_into_five_words_with_correct_spans() {
        let (text, tokens) = sample();
        let e = group(text, &tokens);
        let surfaces: Vec<_> = e.iter().map(|x| x.surface.as_str()).collect();
        assert_eq!(surfaces, ["저는", "한국어를", "공부하고", "있어요."]);
        assert_eq!((e[1].char_start, e[1].char_end), (3, 7));
    }

    #[test]
    fn derives_lemmas_and_pos() {
        let (text, tokens) = sample();
        let e = group(text, &tokens);
        assert_eq!((e[0].lemma.as_deref(), e[0].pos), (Some("저"), Pos::Pronoun));
        assert_eq!((e[1].lemma.as_deref(), e[1].pos), (Some("한국어"), Pos::Noun));
        assert_eq!((e[2].lemma.as_deref(), e[2].pos), (Some("공부하다"), Pos::Verb));
        assert_eq!((e[3].lemma.as_deref(), e[3].pos), (Some("있다"), Pos::Verb));
    }

    #[test]
    fn formula_explains_the_construction() {
        let (text, tokens) = sample();
        let e = group(text, &tokens);
        assert_eq!(e[1].formula, "한국어 + 를 (object particle (marks what is acted on))");
        assert!(e[2].formula.starts_with("공부 + 하다 ('do' (turns a noun into a verb)) + 고 ("));
        assert!(e[3].formula.contains("어요 (polite ending"));
        let steps = conjugation_steps(&e[2]);
        assert_eq!(steps.iter().map(|s| s.form.as_str()).collect::<Vec<_>>(), ["공부", "하다", "고"]);
    }

    #[test]
    fn romanization_and_pronunciation_ignore_trailing_punctuation() {
        let (text, tokens) = sample();
        let e = group(text, &tokens);
        assert_eq!(e[3].romanization, "isseoyo");
        assert_eq!(e[3].pronunciation, "이써요");
        assert_eq!(e[1].romanization, "hangugeoreul");
    }

    #[test]
    fn contracted_morphemes_with_overlapping_spans_stay_in_one_word() {
        // 했어요: 하 + 었 both start at char 0 (the 했 syllable).
        let tokens = vec![
            RawToken::new("공부", "NNG", 0, 2),
            RawToken::new("하", "XSV", 2, 3),
            RawToken::new("었", "EP", 2, 3),
            RawToken::new("어요", "EF", 3, 5),
        ];
        let e = group("공부했어요", &tokens);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].morphs.len(), 4);
        assert_eq!(e[0].lemma.as_deref(), Some("공부하다"));
    }

    #[test]
    fn copula_keeps_the_noun_as_lemma() {
        let tokens = vec![
            RawToken::new("학생", "NNG", 0, 2),
            RawToken::new("이", "VCP", 2, 3),
            RawToken::new("ㅂ니다", "EF", 3, 5),
        ];
        let e = group("학생입니다", &tokens);
        assert_eq!((e[0].lemma.as_deref(), e[0].pos), (Some("학생"), Pos::Noun));
        assert!(e[0].formula.contains("이다"));
    }

    #[test]
    fn punctuation_only_and_foreign_words_are_not_content() {
        let tokens = vec![RawToken::new("...", "SE", 0, 3), RawToken::new("meeting", "SL", 4, 11)];
        let e = group("... meeting", &tokens);
        assert!(!e[0].content);
        assert!(!e[1].content);
    }

    #[test]
    fn compound_nouns_are_joined() {
        let tokens = vec![
            RawToken::new("한국", "NNP", 0, 2),
            RawToken::new("어", "NNG", 2, 3),
            RawToken::new("가", "JKS", 3, 4),
        ];
        let e = group("한국어가", &tokens);
        assert_eq!(e[0].lemma.as_deref(), Some("한국어"));
    }
}
