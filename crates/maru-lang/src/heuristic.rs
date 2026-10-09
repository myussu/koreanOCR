//! A small suffix-stripping analyser.
//!
//! It is *not* a replacement for Kiwi: it knows a few dozen particles and
//! endings and guesses the rest. It exists so that the pipeline degrades
//! gracefully (and tests stay deterministic) when Kiwi is not installed yet.

use crate::hangul;
use crate::morph::{MorphError, Morphology, RawToken};

/// Common particles, longest first so `에서는` wins over `는`.
const PARTICLES: &[(&str, &str)] = &[
    ("에서는", "JKB"), ("에게서", "JKB"), ("으로는", "JKB"), ("이라고", "JKQ"),
    ("에서도", "JKB"), ("에서", "JKB"), ("에게", "JKB"), ("한테", "JKB"), ("까지", "JX"),
    ("부터", "JX"), ("보다", "JKB"), ("처럼", "JKB"), ("으로", "JKB"), ("이나", "JX"),
    ("이랑", "JC"), ("라고", "JKQ"), ("에는", "JKB"), ("에도", "JKB"), ("께서", "JKS"),
    ("마다", "JX"), ("조차", "JX"), ("하고", "JC"),
    ("은", "JX"), ("는", "JX"), ("이", "JKS"), ("가", "JKS"), ("을", "JKO"), ("를", "JKO"),
    ("의", "JKG"), ("에", "JKB"), ("로", "JKB"), ("와", "JC"), ("과", "JC"), ("도", "JX"),
    ("만", "JX"), ("랑", "JC"),
];

/// Verb/adjective endings: (surface suffix, pieces). A piece is (form, tag).
/// Listed longest first. `stem_adds_ha` means the stem needs a 하 (합니다 → 하 + ㅂ니다).
type Piece = (&'static str, &'static str);
const ENDINGS: &[(&str, &[Piece])] = &[
    ("했습니다", &[("하", "XSV"), ("았", "EP"), ("습니다", "EF")]),
    ("했어요", &[("하", "XSV"), ("았", "EP"), ("어요", "EF")]),
    ("했다", &[("하", "XSV"), ("았", "EP"), ("다", "EF")]),
    ("했는데", &[("하", "XSV"), ("았", "EP"), ("는데", "EC")]),
    ("합니다", &[("하", "XSV"), ("ㅂ니다", "EF")]),
    ("합니까", &[("하", "XSV"), ("ㅂ니까", "EF")]),
    ("하세요", &[("하", "XSV"), ("세요", "EF")]),
    ("해요", &[("하", "XSV"), ("여요", "EF")]),
    ("해서", &[("하", "XSV"), ("여서", "EC")]),
    ("하고", &[("하", "XSV"), ("고", "EC")]),
    ("하는", &[("하", "XSV"), ("는", "ETM")]),
    ("하면", &[("하", "XSV"), ("면", "EC")]),
    ("하지만", &[("하", "XSV"), ("지만", "EC")]),
    ("하다", &[("하", "XSV"), ("다", "EF")]),
    ("입니다", &[("이", "VCP"), ("ㅂ니다", "EF")]),
    ("이에요", &[("이", "VCP"), ("에요", "EF")]),
    ("예요", &[("이", "VCP"), ("에요", "EF")]),
    ("이다", &[("이", "VCP"), ("다", "EF")]),
    ("았어요", &[("았", "EP"), ("어요", "EF")]),
    ("었어요", &[("었", "EP"), ("어요", "EF")]),
    ("습니다", &[("습니다", "EF")]),
    ("으세요", &[("으시", "EP"), ("어요", "EF")]),
    ("세요", &[("시", "EP"), ("어요", "EF")]),
    ("아요", &[("아요", "EF")]),
    ("어요", &[("어요", "EF")]),
    ("지만", &[("지만", "EC")]),
    ("는데", &[("는데", "EC")]),
    ("니까", &[("니까", "EC")]),
    ("으면", &[("으면", "EC")]),
    ("려고", &[("려고", "EC")]),
    ("아서", &[("아서", "EC")]),
    ("어서", &[("어서", "EC")]),
    ("고", &[("고", "EC")]),
    ("면", &[("면", "EC")]),
    ("다", &[("다", "EF")]),
];

#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicAnalyzer;

impl HeuristicAnalyzer {
    pub fn new() -> Self {
        Self
    }

    fn push(out: &mut Vec<RawToken>, form: &str, tag: &str, start: usize, end: usize) {
        out.push(RawToken::new(form, tag, start, end));
    }

    fn analyze_word(word: &[char], offset: usize, out: &mut Vec<RawToken>) {
        // Trailing punctuation first.
        let mut end = word.len();
        while end > 0 && !word[end - 1].is_alphanumeric() && !hangul::is_hangul(word[end - 1]) {
            end -= 1;
        }
        let (body, trailing) = word.split_at(end);
        if !body.is_empty() {
            Self::analyze_body(body, offset, out);
        }
        for (i, c) in trailing.iter().enumerate() {
            let tag = match c {
                '.' | '?' | '!' | '。' => "SF",
                ',' | ':' | ';' | '、' => "SP",
                '"' | '\'' | '(' | ')' | '[' | ']' | '“' | '”' | '‘' | '’' | '「' | '」' => "SS",
                '…' => "SE",
                _ => "SW",
            };
            let pos = offset + end + i;
            Self::push(out, &c.to_string(), tag, pos, pos + 1);
        }
    }

    fn analyze_body(body: &[char], offset: usize, out: &mut Vec<RawToken>) {
        let text: String = body.iter().collect();
        if body.iter().all(|c| c.is_ascii_digit()) {
            return Self::push(out, &text, "SN", offset, offset + body.len());
        }
        if !body.iter().any(|&c| hangul::is_hangul(c)) {
            let tag = if body.iter().any(|c| c.is_alphabetic()) { "SL" } else { "SW" };
            return Self::push(out, &text, tag, offset, offset + body.len());
        }

        // Verbal endings first (they are the more specific match).
        for (suffix, pieces) in ENDINGS {
            let sfx: Vec<char> = suffix.chars().collect();
            if body.len() > sfx.len() && body.ends_with(&sfx) {
                let stem_len = body.len() - sfx.len();
                let stem: String = body[..stem_len].iter().collect();
                let is_noun_plus_suffix = pieces[0].1 == "XSV" || pieces[0].1 == "VCP";
                let stem_tag = if is_noun_plus_suffix { "NNG" } else { "VV" };
                Self::push(out, &stem, stem_tag, offset, offset + stem_len);
                let mut pos = offset + stem_len;
                let step = sfx.len().div_ceil(pieces.len());
                for (i, (form, tag)) in pieces.iter().enumerate() {
                    let end = if i + 1 == pieces.len() { offset + body.len() } else { (pos + step).min(offset + body.len()) };
                    Self::push(out, form, tag, pos, end);
                    pos = end;
                }
                return;
            }
        }
        for (suffix, tag) in PARTICLES {
            let sfx: Vec<char> = suffix.chars().collect();
            if body.len() > sfx.len() && body.ends_with(&sfx) {
                let stem_len = body.len() - sfx.len();
                let stem: String = body[..stem_len].iter().collect();
                Self::push(out, &stem, "NNG", offset, offset + stem_len);
                Self::push(out, suffix, tag, offset + stem_len, offset + body.len());
                return;
            }
        }
        Self::push(out, &text, "NNG", offset, offset + body.len());
    }
}

impl Morphology for HeuristicAnalyzer {
    fn name(&self) -> &'static str {
        "heuristic"
    }

    fn tokenize(&self, text: &str) -> Result<Vec<RawToken>, MorphError> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            Self::analyze_word(&chars[start..i], start, &mut out);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms(text: &str) -> Vec<(String, String)> {
        HeuristicAnalyzer.tokenize(text).unwrap().into_iter().map(|t| (t.form, t.tag)).collect()
    }

    #[test]
    fn splits_noun_and_particle() {
        assert_eq!(
            forms("학교에서"),
            vec![("학교".into(), "NNG".into()), ("에서".into(), "JKB".into())]
        );
    }

    #[test]
    fn splits_polite_verb() {
        let f = forms("공부했어요.");
        let forms: Vec<_> = f.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(forms, ["공부", "하", "았", "어요", "."]);
    }

    #[test]
    fn offsets_are_character_based_and_cover_the_word() {
        let t = HeuristicAnalyzer.tokenize("저는 학생입니다").unwrap();
        assert_eq!(t.first().unwrap().start, 0);
        assert_eq!(t.last().unwrap().end, 8);
        assert!(t.iter().all(|x| x.start < x.end));
    }

    #[test]
    fn latin_and_numbers_are_not_korean() {
        let f = forms("2024 meeting");
        assert_eq!(f, vec![("2024".into(), "SN".into()), ("meeting".into(), "SL".into())]);
    }
}
