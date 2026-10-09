//! Hangul syllable arithmetic and script/direction detection.

use maru_core::{Direction, Lang};

pub const SYLLABLE_BASE: u32 = 0xAC00;
pub const SYLLABLE_END: u32 = 0xD7A3;

/// Initial consonants in Unicode order (index 11 = silent ㅇ).
pub const INITIALS: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
/// Vowels in Unicode order.
pub const VOWELS: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ', 'ㅟ', 'ㅠ', 'ㅡ',
    'ㅢ', 'ㅣ',
];
/// Final consonants in Unicode order (index 0 = none), clusters as single compat jamo.
pub const FINALS: [Option<char>; 28] = [
    None,
    Some('ㄱ'),
    Some('ㄲ'),
    Some('ㄳ'),
    Some('ㄴ'),
    Some('ㄵ'),
    Some('ㄶ'),
    Some('ㄷ'),
    Some('ㄹ'),
    Some('ㄺ'),
    Some('ㄻ'),
    Some('ㄼ'),
    Some('ㄽ'),
    Some('ㄾ'),
    Some('ㄿ'),
    Some('ㅀ'),
    Some('ㅁ'),
    Some('ㅂ'),
    Some('ㅄ'),
    Some('ㅅ'),
    Some('ㅆ'),
    Some('ㅇ'),
    Some('ㅈ'),
    Some('ㅊ'),
    Some('ㅋ'),
    Some('ㅌ'),
    Some('ㅍ'),
    Some('ㅎ'),
];

pub fn is_syllable(c: char) -> bool {
    (SYLLABLE_BASE..=SYLLABLE_END).contains(&(c as u32))
}

/// Compatibility jamo (ㄱ…ㅣ) as used by Kiwi for bare endings like `ㅂ니다`.
pub fn is_compat_jamo(c: char) -> bool {
    ('\u{3131}'..='\u{3163}').contains(&c)
}

pub fn is_hangul(c: char) -> bool {
    is_syllable(c) || is_compat_jamo(c) || ('\u{1100}'..='\u{11FF}').contains(&c)
}

/// (initial, vowel, final) indices of a syllable.
pub fn decompose(c: char) -> Option<(usize, usize, usize)> {
    if !is_syllable(c) {
        return None;
    }
    let s = c as u32 - SYLLABLE_BASE;
    Some(((s / 588) as usize, ((s % 588) / 28) as usize, (s % 28) as usize))
}

pub fn compose(initial: usize, vowel: usize, fin: usize) -> char {
    char::from_u32(SYLLABLE_BASE + (initial as u32 * 21 + vowel as u32) * 28 + fin as u32).unwrap_or('?')
}

pub fn initial_index(c: char) -> Option<usize> {
    INITIALS.iter().position(|&x| x == c)
}
pub fn final_index(c: char) -> Option<usize> {
    FINALS.iter().position(|&x| x == Some(c))
}
pub fn vowel_index(c: char) -> Option<usize> {
    VOWELS.iter().position(|&x| x == c)
}

/// Share of letters (Hangul + Latin) that are Hangul.
fn script_counts(text: &str) -> (usize, usize) {
    let mut hangul = 0;
    let mut latin = 0;
    for c in text.chars() {
        if is_hangul(c) {
            hangul += 1;
        } else if c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c) {
            latin += 1;
        }
    }
    (hangul, latin)
}

/// Dominant language of a selection. Korean wins if it makes up at least a
/// third of the letters, so "오늘 meeting 있어요" stays Korean.
pub fn detect_lang(text: &str) -> Lang {
    let (hangul, latin) = script_counts(text);
    match (hangul, latin) {
        (0, 0) => Lang::Other,
        (h, l) if h * 2 >= l => Lang::Ko,
        _ => Lang::En,
    }
}

/// Translation direction for a selection; `None` if there are no letters at all.
pub fn detect_direction(text: &str) -> Option<Direction> {
    match detect_lang(text) {
        Lang::Ko => Some(Direction::KoToEn),
        Lang::En => Some(Direction::EnToKo),
        Lang::Other => None,
    }
}

pub fn contains_hangul(text: &str) -> bool {
    text.chars().any(is_hangul)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompose_and_compose_roundtrip_every_syllable() {
        for cp in SYLLABLE_BASE..=SYLLABLE_END {
            let c = char::from_u32(cp).unwrap();
            let (l, v, t) = decompose(c).unwrap();
            assert_eq!(compose(l, v, t), c);
        }
    }

    #[test]
    fn known_decompositions() {
        let (l, v, t) = decompose('한').unwrap();
        assert_eq!((INITIALS[l], VOWELS[v], FINALS[t]), ('ㅎ', 'ㅏ', Some('ㄴ')));
        let (l, v, t) = decompose('없').unwrap();
        assert_eq!((INITIALS[l], VOWELS[v], FINALS[t]), ('ㅇ', 'ㅓ', Some('ㅄ')));
    }

    #[test]
    fn direction_detection() {
        assert_eq!(detect_direction("저는 한국어를 공부해요"), Some(Direction::KoToEn));
        assert_eq!(detect_direction("Where is the nearest station?"), Some(Direction::EnToKo));
        assert_eq!(detect_direction("오늘 meeting 있어요"), Some(Direction::KoToEn));
        assert_eq!(detect_direction("Let's try 한국어 today, okay everyone"), Some(Direction::EnToKo));
        assert_eq!(detect_direction("12345 !!!"), None);
    }
}
