//! Standard Korean pronunciation rules and Revised Romanization (RR).
//!
//! Everything here is deterministic and runs in microseconds, so learners see
//! how a word is *actually said* (합니다 → [함니다]) the instant they select it.
//!
//! The rules implemented, in the order they interact:
//!
//! 1. 연음 (liaison): a final consonant moves onto a following vowel (`먹어` → 머거),
//!    clusters split (`읽어` → 일거), final ㅎ is silent (`좋아` → 조아).
//! 2. 겹받침 simplification and 7-sound final neutralisation.
//! 3. ㅎ aspiration (`좋다` → 조타, `축하` → 추카, `입학` → 이팍).
//! 4. 경음화 (tensification) after ㄱ/ㄷ/ㅂ (`학교` → 학꾜).
//! 5. 비음화 (nasalisation): `합니다` → 함니다, `국물` → 궁물, `종로` → 종노.
//! 6. 유음화 (liquidisation): `신라` → 실라, `설날` → 설랄.
//! 7. 구개음화 (palatalisation): `같이` → 가치.
//!
//! Limits: rules that depend on morpheme boundaries or etymology (ㄴ-insertion
//! in compounds, 사이시옷, tensification after verb-stem ㄴ/ㅁ, the handful of
//! lexical exceptions like 의견란) are not guessed.

use crate::hangul::{self, FINALS, INITIALS, VOWELS};

#[derive(Debug, Clone)]
struct Syl {
    init: char,
    vowel: char,
    /// 0..=2 consonants (clusters decomposed).
    coda: Vec<char>,
}

#[derive(Debug, Clone)]
enum Unit {
    Syl(Syl),
    Other(char),
}

fn split_final(c: char) -> Vec<char> {
    match c {
        'ㄳ' => vec!['ㄱ', 'ㅅ'],
        'ㄵ' => vec!['ㄴ', 'ㅈ'],
        'ㄶ' => vec!['ㄴ', 'ㅎ'],
        'ㄺ' => vec!['ㄹ', 'ㄱ'],
        'ㄻ' => vec!['ㄹ', 'ㅁ'],
        'ㄼ' => vec!['ㄹ', 'ㅂ'],
        'ㄽ' => vec!['ㄹ', 'ㅅ'],
        'ㄾ' => vec!['ㄹ', 'ㅌ'],
        'ㄿ' => vec!['ㄹ', 'ㅍ'],
        'ㅀ' => vec!['ㄹ', 'ㅎ'],
        'ㅄ' => vec!['ㅂ', 'ㅅ'],
        other => vec![other],
    }
}

fn join_final(coda: &[char]) -> usize {
    match coda {
        [] => 0,
        [c] => hangul::final_index(*c).unwrap_or(0),
        [a, b] => {
            let cluster = match (a, b) {
                ('ㄱ', 'ㅅ') => 'ㄳ',
                ('ㄴ', 'ㅈ') => 'ㄵ',
                ('ㄴ', 'ㅎ') => 'ㄶ',
                ('ㄹ', 'ㄱ') => 'ㄺ',
                ('ㄹ', 'ㅁ') => 'ㄻ',
                ('ㄹ', 'ㅂ') => 'ㄼ',
                ('ㄹ', 'ㅅ') => 'ㄽ',
                ('ㄹ', 'ㅌ') => 'ㄾ',
                ('ㄹ', 'ㅍ') => 'ㄿ',
                ('ㄹ', 'ㅎ') => 'ㅀ',
                ('ㅂ', 'ㅅ') => 'ㅄ',
                _ => *a,
            };
            hangul::final_index(cluster).unwrap_or(0)
        }
        _ => 0,
    }
}

fn parse(word: &str) -> Vec<Unit> {
    word.chars()
        .map(|c| match hangul::decompose(c) {
            Some((l, v, t)) => Unit::Syl(Syl {
                init: INITIALS[l],
                vowel: VOWELS[v],
                coda: FINALS[t].map(split_final).unwrap_or_default(),
            }),
            None => Unit::Other(c),
        })
        .collect()
}

fn render(units: &[Unit]) -> String {
    units
        .iter()
        .map(|u| match u {
            Unit::Other(c) => *c,
            Unit::Syl(s) => hangul::compose(
                hangul::initial_index(s.init).unwrap_or(11),
                hangul::vowel_index(s.vowel).unwrap_or(0),
                join_final(&s.coda),
            ),
        })
        .collect()
}

fn neutralize(c: char) -> char {
    match c {
        'ㄲ' | 'ㅋ' => 'ㄱ',
        'ㅅ' | 'ㅆ' | 'ㅈ' | 'ㅊ' | 'ㅌ' | 'ㅎ' => 'ㄷ',
        'ㅍ' => 'ㅂ',
        other => other,
    }
}

fn tense(c: char) -> char {
    match c {
        'ㄱ' => 'ㄲ',
        'ㄷ' => 'ㄸ',
        'ㅂ' => 'ㅃ',
        'ㅅ' => 'ㅆ',
        'ㅈ' => 'ㅉ',
        other => other,
    }
}

fn is_tensifiable(c: char) -> bool {
    matches!(c, 'ㄱ' | 'ㄷ' | 'ㅂ' | 'ㅅ' | 'ㅈ')
}

fn nasal_of(c: char) -> char {
    match c {
        'ㄱ' => 'ㅇ',
        'ㄷ' => 'ㄴ',
        'ㅂ' => 'ㅁ',
        other => other,
    }
}

/// Collapse a word-final or pre-consonant cluster to one sound.
fn simplify_cluster(coda: &[char], next_init: Option<char>) -> Option<char> {
    Some(match coda {
        [] => return None,
        [c] => *c,
        ['ㄱ', 'ㅅ'] => 'ㄱ',
        ['ㄴ', 'ㅈ'] | ['ㄴ', 'ㅎ'] => 'ㄴ',
        // 읽고 [일꼬] but 읽다 [익따]
        ['ㄹ', 'ㄱ'] => {
            if next_init == Some('ㄱ') {
                'ㄹ'
            } else {
                'ㄱ'
            }
        }
        ['ㄹ', 'ㅁ'] => 'ㅁ',
        ['ㄹ', 'ㅂ'] | ['ㄹ', 'ㅅ'] | ['ㄹ', 'ㅌ'] | ['ㄹ', 'ㅎ'] => 'ㄹ',
        ['ㄹ', 'ㅍ'] | ['ㅂ', 'ㅅ'] => 'ㅂ',
        [first, ..] => *first,
    })
}

fn aspirate_single(orig: char, next_vowel: char) -> char {
    match orig {
        'ㄱ' | 'ㅋ' | 'ㄲ' => 'ㅋ',
        'ㄷ' if next_vowel == 'ㅣ' => 'ㅊ', // 닫히다 → 다치다
        'ㅈ' => 'ㅊ',
        'ㄷ' | 'ㅅ' | 'ㅆ' | 'ㅌ' | 'ㅊ' => 'ㅌ',
        'ㅂ' | 'ㅍ' => 'ㅍ',
        other => other,
    }
}

fn apply_pair(a: &mut Syl, b: &mut Syl, tensify: bool) {
    if a.coda.is_empty() {
        return;
    }

    // ── vowel-initial next syllable: 연음 ──
    if b.init == 'ㅇ' {
        match a.coda.as_slice() {
            ['ㅇ'] => {}
            ['ㅎ'] => a.coda.clear(),
            [first, 'ㅎ'] => {
                b.init = *first;
                a.coda.clear();
            }
            [first, second] => {
                let (first, second) = (*first, *second);
                b.init = if tensify && second == 'ㅅ' && matches!(first, 'ㄱ' | 'ㅂ') {
                    'ㅆ' // 없어 [업써], 몫이 [목씨]
                } else {
                    second
                };
                a.coda = vec![first];
            }
            [single] => {
                let single = *single;
                b.init = if b.vowel == 'ㅣ' && single == 'ㄷ' {
                    'ㅈ' // 굳이 → 구지
                } else if b.vowel == 'ㅣ' && single == 'ㅌ' {
                    'ㅊ' // 같이 → 가치
                } else {
                    single
                };
                a.coda.clear();
            }
            _ => {}
        }
        return;
    }

    let orig = a.coda.clone();
    let next_init = b.init;

    // ── ㅎ-containing codas ──
    if orig == ['ㅎ'] {
        match next_init {
            'ㄱ' => { b.init = 'ㅋ'; a.coda.clear(); return; }
            'ㄷ' => { b.init = 'ㅌ'; a.coda.clear(); return; }
            'ㅈ' => { b.init = 'ㅊ'; a.coda.clear(); return; }
            'ㅅ' => { b.init = 'ㅆ'; a.coda.clear(); return; }
            'ㅎ' => { a.coda.clear(); return; }
            _ => a.coda = vec!['ㄷ'], // 놓는 → 논는 (nasalised below)
        }
    } else if orig.len() == 2 && orig[1] == 'ㅎ' {
        a.coda = vec![orig[0]]; // 않고 → 안코, 싫다 → 실타
        match next_init {
            'ㄱ' => b.init = 'ㅋ',
            'ㄷ' => b.init = 'ㅌ',
            'ㅈ' => b.init = 'ㅊ',
            'ㅅ' => b.init = 'ㅆ',
            _ => {}
        }
        if matches!(next_init, 'ㄱ' | 'ㄷ' | 'ㅈ' | 'ㅅ') {
            return;
        }
    } else if orig.len() == 2 && next_init == 'ㅎ' && matches!(orig[1], 'ㄱ' | 'ㅂ' | 'ㅈ') {
        // 읽히다 → 일키다, 밟히다 → 발피다, 앉히다 → 안치다
        b.init = aspirate_single(orig[1], b.vowel);
        a.coda = vec![orig[0]];
        return;
    } else if orig.len() == 1 && next_init == 'ㅎ' && orig[0] != 'ㅇ' && orig[0] != 'ㄴ' && orig[0] != 'ㄹ' && orig[0] != 'ㅁ' {
        // 축하 → 추카, 입학 → 이팍, 맞히다 → 마치다, 닫히다 → 다치다
        b.init = aspirate_single(orig[0], b.vowel);
        a.coda.clear();
        return;
    }

    // ── cluster simplification + neutralisation ──
    let cluster_tense = matches!(orig.as_slice(), ['ㄴ', 'ㅈ'] | ['ㄹ', 'ㅁ'] | ['ㄹ', 'ㅂ'] | ['ㄹ', 'ㅌ']);
    let log_gk = orig.as_slice() == ['ㄹ', 'ㄱ'] && next_init == 'ㄱ';
    let simple = simplify_cluster(&a.coda, Some(next_init)).map(neutralize);
    a.coda = simple.into_iter().collect();
    let Some(coda) = simple else { return };

    // ── 경음화 ──
    if tensify && is_tensifiable(b.init) && (matches!(coda, 'ㄱ' | 'ㄷ' | 'ㅂ') || cluster_tense || log_gk) {
        b.init = tense(b.init);
    }

    // ── 비음화 ──
    let mut coda = coda;
    if matches!(coda, 'ㄱ' | 'ㄷ' | 'ㅂ') && matches!(b.init, 'ㄴ' | 'ㅁ' | 'ㄹ') {
        coda = nasal_of(coda);
        if b.init == 'ㄹ' {
            b.init = 'ㄴ';
        }
    } else if matches!(coda, 'ㅁ' | 'ㅇ') && b.init == 'ㄹ' {
        b.init = 'ㄴ'; // 종로 → 종노, 심리 → 심니
    }

    // ── 유음화 ──
    if coda == 'ㄴ' && b.init == 'ㄹ' {
        coda = 'ㄹ'; // 신라 → 실라
    } else if coda == 'ㄹ' && b.init == 'ㄴ' {
        b.init = 'ㄹ'; // 설날 → 설랄
    }

    a.coda = vec![coda];
}

fn apply_rules(units: &mut [Unit], tensify: bool) {
    for i in 0..units.len().saturating_sub(1) {
        let (left, right) = units.split_at_mut(i + 1);
        if let (Unit::Syl(a), Unit::Syl(b)) = (&mut left[i], &mut right[0]) {
            apply_pair(a, b, tensify);
        }
    }
    // Word/segment-final codas: simplify + neutralise.
    for i in 0..units.len() {
        let last_in_run = !matches!(units.get(i + 1), Some(Unit::Syl(_)));
        if last_in_run && let Unit::Syl(s) = &mut units[i] {
            s.coda = simplify_cluster(&s.coda, None).map(neutralize).into_iter().collect();
        }
    }
}

/// How a single 어절 (no spaces) is actually pronounced, in Hangul.
pub fn pronounce_word(word: &str) -> String {
    let mut units = parse(word);
    apply_rules(&mut units, true);
    render(&units)
}

/// Pronunciation of free text: words are processed independently.
pub fn pronounce(text: &str) -> String {
    map_words(text, pronounce_word)
}

fn map_words(text: &str, f: impl Fn(&str) -> String) -> String {
    text.split_whitespace().map(f).collect::<Vec<_>>().join(" ")
}

// ───────────────────────────── Revised Romanization ─────────────────────────────

fn rr_initial(c: char, after_l: bool) -> &'static str {
    match c {
        'ㄱ' => "g",
        'ㄲ' => "kk",
        'ㄴ' => "n",
        'ㄷ' => "d",
        'ㄸ' => "tt",
        'ㄹ' => {
            if after_l {
                "l"
            } else {
                "r"
            }
        }
        'ㅁ' => "m",
        'ㅂ' => "b",
        'ㅃ' => "pp",
        'ㅅ' => "s",
        'ㅆ' => "ss",
        'ㅇ' => "",
        'ㅈ' => "j",
        'ㅉ' => "jj",
        'ㅊ' => "ch",
        'ㅋ' => "k",
        'ㅌ' => "t",
        'ㅍ' => "p",
        'ㅎ' => "h",
        _ => "",
    }
}

fn rr_vowel(c: char) -> &'static str {
    match c {
        'ㅏ' => "a",
        'ㅐ' => "ae",
        'ㅑ' => "ya",
        'ㅒ' => "yae",
        'ㅓ' => "eo",
        'ㅔ' => "e",
        'ㅕ' => "yeo",
        'ㅖ' => "ye",
        'ㅗ' => "o",
        'ㅘ' => "wa",
        'ㅙ' => "wae",
        'ㅚ' => "oe",
        'ㅛ' => "yo",
        'ㅜ' => "u",
        'ㅝ' => "wo",
        'ㅞ' => "we",
        'ㅟ' => "wi",
        'ㅠ' => "yu",
        'ㅡ' => "eu",
        'ㅢ' => "ui",
        'ㅣ' => "i",
        _ => "",
    }
}

fn rr_final(c: char) -> &'static str {
    match c {
        'ㄱ' => "k",
        'ㄴ' => "n",
        'ㄷ' => "t",
        'ㄹ' => "l",
        'ㅁ' => "m",
        'ㅂ' => "p",
        'ㅇ' => "ng",
        _ => "",
    }
}

fn rr_compat_jamo(c: char) -> &'static str {
    // Bare jamo such as the ㅂ of ㅂ니다 or the ㄴ of ㄴ/은.
    match c {
        'ㅂ' => "b",
        'ㄴ' => "n",
        'ㄹ' => "l",
        'ㅁ' => "m",
        'ㅆ' => "ss",
        'ㅇ' => "ng",
        other => rr_initial(other, false),
    }
}

/// Revised Romanization of one 어절. Tensification is deliberately *not*
/// reflected, as the official rules prescribe (학교 → hakgyo).
pub fn romanize_word(word: &str) -> String {
    let mut units = parse(word);
    apply_rules(&mut units, false);
    let mut out = String::new();
    let mut prev_coda_l = false;
    for u in &units {
        match u {
            Unit::Syl(s) => {
                out.push_str(rr_initial(s.init, prev_coda_l));
                out.push_str(rr_vowel(s.vowel));
                prev_coda_l = false;
                if let Some(&c) = s.coda.first() {
                    out.push_str(rr_final(c));
                    prev_coda_l = c == 'ㄹ';
                }
            }
            Unit::Other(c) => {
                prev_coda_l = false;
                if hangul::is_compat_jamo(*c) {
                    out.push_str(rr_compat_jamo(*c));
                } else {
                    out.push(*c);
                }
            }
        }
    }
    out
}

pub fn romanize(text: &str) -> String {
    map_words(text, romanize_word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> String {
        pronounce_word(s)
    }
    fn r(s: &str) -> String {
        romanize_word(s)
    }

    #[test]
    fn nasalisation() {
        assert_eq!(p("합니다"), "함니다");
        assert_eq!(p("국물"), "궁물");
        assert_eq!(p("먹는"), "멍는");
        assert_eq!(p("종로"), "종노");
        assert_eq!(p("독립"), "동닙");
        assert_eq!(p("협력"), "혐녁");
        // ㄴ-insertion (십육 → 심뉵) needs morpheme knowledge and is not guessed.
        assert_eq!(p("십육"), "시뷱");
        assert_eq!(r("합니다"), "hamnida");
        assert_eq!(r("국물"), "gungmul");
        assert_eq!(r("종로"), "jongno");
    }

    #[test]
    fn liquidisation() {
        assert_eq!(p("신라"), "실라");
        assert_eq!(p("설날"), "설랄");
        assert_eq!(r("신라"), "silla");
        assert_eq!(r("설날"), "seollal");
        assert_eq!(r("한라산"), "hallasan");
    }

    #[test]
    fn liaison() {
        assert_eq!(p("한국어"), "한구거");
        assert_eq!(p("먹어요"), "머거요");
        assert_eq!(p("먹었어요"), "머거써요");
        assert_eq!(p("옷이"), "오시");
        assert_eq!(p("꽃이"), "꼬치");
        assert_eq!(p("부엌에"), "부어케");
        assert_eq!(p("밖에"), "바께");
        assert_eq!(p("십일"), "시빌");
        assert_eq!(r("한국어"), "hangugeo");
        assert_eq!(r("먹었어요"), "meogeosseoyo");
        // ㅇ final does not link
        assert_eq!(p("강아지"), "강아지");
        assert_eq!(r("강아지"), "gangaji");
    }

    #[test]
    fn clusters() {
        assert_eq!(p("읽어"), "일거");
        assert_eq!(p("앉아"), "안자");
        assert_eq!(p("닭이"), "달기");
        assert_eq!(p("값이"), "갑씨");
        assert_eq!(p("없어요"), "업써요");
        assert_eq!(p("읽고"), "일꼬");
        assert_eq!(p("읽다"), "익따");
        assert_eq!(p("닭"), "닥");
        assert_eq!(p("값"), "갑");
        assert_eq!(p("여덟"), "여덜");
        assert_eq!(p("삶"), "삼");
        assert_eq!(p("넓다"), "널따");
        assert_eq!(r("읽고"), "ilgo");
        assert_eq!(r("없어요"), "eopseoyo");
    }

    #[test]
    fn hieut_rules() {
        assert_eq!(p("좋아요"), "조아요");
        assert_eq!(p("많이"), "마니");
        assert_eq!(p("괜찮아요"), "괜차나요");
        assert_eq!(p("좋다"), "조타");
        assert_eq!(p("좋고"), "조코");
        assert_eq!(p("축하"), "추카");
        assert_eq!(p("입학"), "이팍");
        assert_eq!(p("않고"), "안코");
        assert_eq!(p("싫다"), "실타");
        assert_eq!(p("놓는"), "논는");
        assert_eq!(p("앉히다"), "안치다");
        assert_eq!(p("읽히다"), "일키다");
        assert_eq!(r("좋다"), "jota");
        assert_eq!(r("괜찮아요"), "gwaenchanayo");
        assert_eq!(r("축하"), "chuka");
    }

    #[test]
    fn palatalisation() {
        assert_eq!(p("같이"), "가치");
        assert_eq!(p("굳이"), "구지");
        assert_eq!(p("닫히다"), "다치다");
        assert_eq!(p("해돋이"), "해도지");
        assert_eq!(r("같이"), "gachi");
        assert_eq!(r("해돋이"), "haedoji");
    }

    #[test]
    fn tensification_shown_in_pronunciation_but_not_romanization() {
        assert_eq!(p("학교"), "학꾜");
        assert_eq!(p("있다"), "읻따");
        assert_eq!(p("학생"), "학쌩");
        assert_eq!(r("학교"), "hakgyo");
        assert_eq!(r("있다"), "itda");
        assert_eq!(r("학생"), "haksaeng");
    }

    #[test]
    fn final_neutralisation_at_word_end() {
        assert_eq!(p("낮"), "낟");
        assert_eq!(p("꽃"), "꼳");
        assert_eq!(p("부엌"), "부억");
        assert_eq!(p("앞"), "압");
        assert_eq!(r("밥"), "bap");
        assert_eq!(r("한국"), "hanguk");
    }

    #[test]
    fn sentences_and_non_hangul_pass_through() {
        assert_eq!(pronounce("저는 학생입니다."), "저는 학쌩임니다.");
        assert_eq!(romanize("감사합니다"), "gamsahamnida");
        assert_eq!(romanize("안녕하세요!"), "annyeonghaseyo!");
        assert_eq!(p("ABC123"), "ABC123");
        assert_eq!(r("ㅂ니다"), "bnida");
    }
}
