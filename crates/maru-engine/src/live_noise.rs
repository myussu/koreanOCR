//! OCR noise that should not reach the translator: a UI bar read as `ㅣ`, a dash read as `ㅡ`, a stray syllable,
//! and stray marks read beside a subtitle (`OTK`, `SX`, `11.`, `[0`, `000`).

use maru_lang::hangul::is_syllable;

/// A word at the edge of a Korean line that is OCR noise: short, no Hangul, and Latin letters, a list number (`11.`),
/// a bracket with digits (`[0`), a number with leading zeros (`000`) or only marks (`.·`, `~~`). Ordinary numbers stay.
fn stray(word: &str) -> bool {
    if word.chars().count() > 5 || word.chars().any(is_syllable) {
        return false;
    }
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    word.chars().any(|c| c.is_ascii_alphabetic())
        || word.strip_suffix('.').is_some_and(digits)
        || word.strip_prefix(['[', '(']).is_some_and(|rest| rest.is_empty() || digits(rest))
        || (word.len() > 1 && word.starts_with('0') && digits(word))
        || !word.chars().any(char::is_alphanumeric)
}

/// The line without stray marks at its start or end, as long as some Hangul remains (`네. AUTO LOG` is `네.`).
pub(crate) fn trim_stray_marks(text: &str) -> String {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    let syllables = |words: &[&str]| words.iter().flat_map(|w| w.chars()).filter(|c| is_syllable(*c)).count();
    while words.len() > 1 && stray(words[0]) && syllables(&words[1..]) >= 1 {
        words.remove(0);
    }
    while words.len() > 1 && stray(words[words.len() - 1]) && syllables(&words[..words.len() - 1]) >= 1 {
        words.pop();
    }
    if words.len() == text.split_whitespace().count() { text.to_string() } else { words.join(" ") }
}

/// A live line is worth a model request when it has at least one complete Hangul syllable; a line of a single
/// syllable (`네`, `응`) only when the recogniser was sure of it.
pub(crate) fn worth_translating(text: &str, confidence: Option<f32>, min_confidence: f32) -> bool {
    match text.chars().filter(|c| is_syllable(*c)).count() {
        0 => false,
        1 => confidence.is_none_or(|c| c >= min_confidence),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{trim_stray_marks, worth_translating};

    #[test]
    fn stray_marks_read_beside_a_subtitle_are_dropped_from_its_edges() {
        for (read, line) in [
            ("OTK AANAA 아~ 이제 알 것 같아", "아~ 이제 알 것 같아"),
            ("내일 다시 오면 알려 줄게 SX", "내일 다시 오면 알려 줄게"),
            ("11. 그래서 어떻게 할 건데?", "그래서 어떻게 할 건데?"),
            ("[0 성하를 뵙는 건 내일이야", "성하를 뵙는 건 내일이야"),
            ("어떻게 된 거야 000", "어떻게 된 거야"),
            ("여기에 올 줄 어떻게 .·", "여기에 올 줄 어떻게"),
            ("네. ▶ AUTO LOG", "네."),
            ("왜? AUTO LOG", "왜?"),
        ] {
            assert_eq!(trim_stray_marks(read), line, "{read:?}");
        }
    }

    #[test]
    fn numbers_words_in_the_line_and_short_lines_are_kept() {
        for line in ["100 골드만 더 줘", "HP가 부족해", "이 정도면 SP 회복은 충분해", "OK", "어떻게   된 거야"] {
            assert_eq!(trim_stray_marks(line), line, "{line:?}");
        }
    }

    #[test]
    fn lone_jamo_and_symbols_read_from_ui_lines_are_noise() {
        for text in ["ㅣ", "ㅡ", "ㅇㅇ", "|", "—", "1", "ㅡ ㅡ"] {
            assert!(!worth_translating(text, Some(0.99), 0.8), "{text:?}");
        }
    }

    #[test]
    fn a_one_syllable_answer_is_kept_when_the_reader_is_sure_of_it() {
        assert!(worth_translating("네.", Some(0.95), 0.8));
        assert!(worth_translating("응?", None, 0.8), "no confidence reported: keep it");
        assert!(!worth_translating("이", Some(0.41), 0.8), "an unsure single syllable is usually an icon or a texture");
    }

    #[test]
    fn ordinary_lines_are_always_translated() {
        assert!(worth_translating("왜 아직도 여기 있는 거야?", Some(0.3), 0.8));
        assert!(worth_translating("HP 회복", Some(0.9), 0.8));
    }
}
