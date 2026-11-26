//! A subtitle read again, read only in part, or misread in a syllable or two while it stays on screen is the same
//! line: it is translated and listed once. OCR re-reads a line with different spacing or punctuation, catches only
//! one of its two rows, reads it while it fades in, and over a moving cutscene misreads a syllable on every read
//! (`운이`/`윤이`, `나쁘그든`/`나쁘고든`). Short lines must match exactly, and a difference that could be the meaning
//! keeps two lines: a negation in one reading and not the other (`간다` / `안 간다`), or a different number.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use maru_lang::hangul::is_syllable;

/// How long after its last reading a line is still taken to be on screen. Long lines stay up for several seconds
/// and are read again during that time.
const WINDOW: Duration = Duration::from_secs(8);
/// Shorter readings than this are never matched by containment (`네` is not part of `네가 왜 여기 있어?`).
const MIN_PART: usize = 4;
/// A line typing out letter by letter: a reading of its first syllables is followed by a longer one this soon.
const TYPING: Duration = Duration::from_secs(2);
/// The fewest syllables a typing line's first reading must have to be taken as its start (`그래` alone stays a reply).
const MIN_TYPED: usize = 3;
/// Lines with fewer syllables than this are only ever the same line when read exactly alike (`어? 왜?` / `어? 원?`).
const MIN_ALIKE: usize = 4;
/// How alike two readings must be (1 − edits / length) to be one line misread. Calibrated on a real cutscene:
/// 26 of 30 misread re-reads matched and no two different lines did.
const ALIKE: f32 = 0.8;
/// Syllables that can carry a negation or a prohibition (`안`, `못`, `않다`, `없다`, `말다`, `하지 마`).
const NEGATIONS: [char; 6] = ['안', '못', '않', '없', '말', '마'];

/// The letters a reading is compared on: spacing, quotes, periods, commas and dots vary between OCR reads;
/// `?` and `!` change the meaning and are kept.
pub(crate) fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '.' | ',' | '\'' | '"' | '“' | '”' | '‘' | '’' | '·' | '…' | '「' | '」' | '『' | '』' | '-' | '~' | '～' | '|'))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Seen {
    /// The same line again (`id`): answer with its translation, add nothing.
    Again { id: u64, translation: String },
    /// Part of a line already translated (`id`; one row of a two-row subtitle): answer with its translation, add nothing.
    Part { id: u64, translation: String },
    /// A fuller reading of a recent line: translate it, and let it replace that line in the history.
    Grew { replaces: u64 },
    New,
}

struct Line {
    id: u64,
    app: String,
    text: String,
    translation: String,
    /// Every reading answered by this line so far: the next misreading is compared with all of them.
    readings: Vec<String>,
    /// When the line was last read; it is taken off the screen a [`WINDOW`] later.
    last: Instant,
}

/// Could the difference between two readings be the meaning rather than OCR? A negation in one and not the other, or
/// two different numbers. (A digit read in place of a syllable, `1제` for `이제`, is not a different number.) The
/// screen watcher asks this too before it takes a changed reading for the line already shown.
pub(crate) fn meaning_differs(a: &str, b: &str) -> bool {
    let negations = |s: &str| NEGATIONS.map(|n| s.contains(n));
    let numbers = |s: &str| s.split(|c: char| !c.is_ascii_digit()).filter(|n| !n.is_empty()).map(str::to_owned).collect::<Vec<_>>();
    let (na, nb) = (numbers(a), numbers(b));
    negations(a) != negations(b) || (!na.is_empty() && !nb.is_empty() && na != nb)
}

/// The line typing out: the earlier reading is the start of this one, read moments ago, and long enough not to be a
/// reply of its own (`잠깐만` → `잠깐만 저기 누군가 우리를 보고 있어`).
fn typing(earlier: &str, now: &str, since: Duration) -> bool {
    since <= TYPING && earlier.chars().filter(|c| is_syllable(*c)).count() >= MIN_TYPED && now.len() > earlier.len() && now.starts_with(earlier)
}

/// Two readings of one subtitle with a syllable or two misread: long enough, alike enough, and no difference that could
/// be the meaning. Latin letters and `?`/`!` are left out of the comparison (OCR adds and drops them on its own).
fn misread(a: &str, b: &str) -> bool {
    let loose = |s: &str| s.chars().filter(|c| !c.is_ascii_alphabetic() && !matches!(c, '?' | '!')).collect::<String>();
    let (a, b) = (loose(a), loose(b));
    let syllables = |s: &str| s.chars().filter(|c| is_syllable(*c)).count();
    if syllables(&a).min(syllables(&b)) < MIN_ALIKE || meaning_differs(&a, &b) {
        return false;
    }
    let longest = a.chars().count().max(b.chars().count());
    longest > 0 && 1.0 - crate::change::edit_distance(&a, &b) as f32 / longest as f32 >= ALIKE
}

/// The live lines of the last few seconds, newest last.
#[derive(Default)]
pub(crate) struct Recent {
    lines: VecDeque<Line>,
    /// Live ids answered as a repeat → the line they repeat, so a star given while a repeat is shown finds the line.
    aliases: VecDeque<(u64, u64)>,
}

impl Recent {
    pub(crate) fn seen(&self, app: &str, text: &str, now: Instant) -> Seen {
        let text = normalize(text);
        if text.is_empty() {
            return Seen::New;
        }
        let count = |s: &str| s.chars().count();
        let part_of = |short: &str, long: &str| count(short) >= MIN_PART && count(short) * 4 >= count(long) && long.contains(short);
        for line in self.lines.iter().rev().filter(|l| l.app == app && now.duration_since(l.last) <= WINDOW) {
            if line.readings.contains(&text) {
                return Seen::Again { id: line.id, translation: line.translation.clone() };
            }
            if part_of(&text, &line.text) {
                return Seen::Part { id: line.id, translation: line.translation.clone() };
            }
            if part_of(&line.text, &text) || typing(&line.text, &text, now.duration_since(line.last)) {
                return Seen::Grew { replaces: line.id };
            }
            if line.readings.iter().any(|r| misread(r, &text)) {
                return Seen::Again { id: line.id, translation: line.translation.clone() };
            }
        }
        Seen::New
    }

    pub(crate) fn remember(&mut self, id: u64, app: &str, text: &str, translation: &str, now: Instant) {
        self.lines.retain(|l| now.duration_since(l.last) <= WINDOW);
        let text = normalize(text);
        self.lines.push_back(Line { id, app: app.into(), readings: vec![text.clone()], text, translation: translation.into(), last: now });
    }

    /// Another reading (`repeat`) of the line `original`, answered with its translation: the line is still on screen,
    /// and the next reading is also compared with this one.
    pub(crate) fn read_again(&mut self, repeat: u64, original: u64, text: &str, now: Instant) {
        self.alias(repeat, original);
        if let Some(line) = self.lines.iter_mut().find(|l| l.id == original) {
            line.last = now;
            let text = normalize(text);
            if !line.readings.contains(&text) && line.readings.len() < 16 {
                line.readings.push(text);
            }
        }
    }

    /// A line that a fuller reading replaced no longer answers for itself; stars given to it follow the new line.
    pub(crate) fn forget(&mut self, id: u64, replaced_by: u64) {
        self.lines.retain(|l| l.id != id);
        self.alias(id, replaced_by);
    }

    pub(crate) fn alias(&mut self, repeat: u64, original: u64) {
        for alias in self.aliases.iter_mut().filter(|(_, to)| *to == repeat) {
            alias.1 = original;
        }
        self.aliases.push_back((repeat, original));
        while self.aliases.len() > 256 {
            self.aliases.pop_front();
        }
    }

    /// The line a live id stands for, when that id was a repeat or a replaced partial reading.
    pub(crate) fn original(&self, id: u64) -> Option<u64> {
        self.aliases.iter().rev().find(|(from, _)| *from == id).map(|(_, to)| *to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recent(lines: &[(u64, &str)], now: Instant) -> Recent {
        let mut r = Recent::default();
        for (id, text) in lines {
            r.remember(*id, "game.exe", text, &format!("translation {id}"), now);
        }
        r
    }

    #[test]
    fn a_line_read_again_with_other_spacing_or_punctuation_is_the_same_line() {
        let now = Instant::now();
        let r = recent(&[(1, "왜 아직도 여기 있는 거야?")], now);
        for again in ["왜 아직도 여기있는 거야?", "\"왜 아직도 여기 있는 거야?\"", "왜  아직도 여기 있는 거야 ?"] {
            assert_eq!(r.seen("game.exe", again, now), Seen::Again { id: 1, translation: "translation 1".into() }, "{again}");
        }
    }

    #[test]
    fn a_question_and_an_exclamation_are_different_lines() {
        let now = Instant::now();
        let r = recent(&[(1, "가자?")], now);
        assert_eq!(r.seen("game.exe", "가자!", now), Seen::New);
    }

    #[test]
    fn one_row_of_a_two_row_subtitle_is_part_of_the_line_and_the_full_line_replaces_its_first_reading() {
        let now = Instant::now();
        let r = recent(&[(1, "이걸 타면 위로 갈 수 있어요. 빨리 가자!")], now);
        assert_eq!(r.seen("game.exe", "빨리 가자!", now), Seen::Part { id: 1, translation: "translation 1".into() });
        let r = recent(&[(2, "이걸 타면 위로")], now);
        assert_eq!(r.seen("game.exe", "이걸 타면 위로 갈 수 있어요.", now), Seen::Grew { replaces: 2 });
    }

    #[test]
    fn a_negation_or_a_number_that_changes_the_meaning_keeps_two_lines() {
        let now = Instant::now();
        let r = recent(&[(1, "나는 학교에 간다"), (2, "3시에 성문 앞에서 기다릴게")], now);
        assert_eq!(r.seen("game.exe", "나는 학교에 안 간다", now), Seen::New);
        assert_eq!(r.seen("game.exe", "5시에 성문 앞에서 기다릴게", now), Seen::New);
        // A prohibition typed onto the end is the line growing: the full line is translated and replaces the first reading.
        let r = recent(&[(1, "그냥 집에 가지")], now);
        assert_eq!(r.seen("game.exe", "그냥 집에 가지 마", now), Seen::Grew { replaces: 1 });
        let r = recent(&[(1, "오늘은 성에 간다고 했어")], now);
        assert_eq!(r.seen("game.exe", "오늘은 성에 못 간다고 했어", now), Seen::New, "a negation inside the line");
    }

    #[test]
    fn a_line_misread_in_a_syllable_or_two_while_it_stays_on_screen_is_the_same_line() {
        let now = Instant::now();
        let mut r = recent(&[(1, "오늘따라 우리 왕자님 윤이 억수로 나쁘그든")], now);
        let again = Seen::Again { id: 1, translation: "translation 1".into() };
        for (n, read) in ["오늘따라 우리 왕자님 운이 억수로 나쁘그듣", "오늘따라 우리 왕자님 운이 억수로 나쁘고든", "오늘따라 우리 장자 운이 억수로 나쁘고든"].iter().enumerate() {
            assert_eq!(r.seen("game.exe", read, now), again, "{read}");
            r.read_again(10 + n as u64, 1, read, now);
        }
        assert_eq!(r.original(12), Some(1));
        // A stuttered reading, and a digit read for a syllable, are the same line too.
        let r = recent(&[(1, "저기, 구하러 가지 않아도 되나요?"), (2, "이제 됐나요? 지크님")], now);
        assert_eq!(r.seen("game.exe", "저기,구하러 가지 않 않아도 도 되나요?", now), Seen::Again { id: 1, translation: "translation 1".into() });
        assert_eq!(r.seen("game.exe", "1제 됐나요? 지크 님", now), Seen::Again { id: 2, translation: "translation 2".into() });
    }

    #[test]
    fn a_line_typing_out_replaces_its_first_syllables_but_a_short_reply_and_an_old_start_stay() {
        let now = Instant::now();
        let r = recent(&[(1, "잠깐만,")], now);
        assert_eq!(r.seen("game.exe", "잠깐만, 저기 누군가 우리를 보고 있어.", now + Duration::from_millis(800)), Seen::Grew { replaces: 1 });
        assert_eq!(r.seen("game.exe", "잠깐만, 저기 누군가 우리를 보고 있어.", now + TYPING + Duration::from_secs(1)), Seen::New, "a new line that starts the same, later");
        let r = recent(&[(1, "그래")], now);
        assert_eq!(r.seen("game.exe", "그래서 어떻게 할 건데?", now + Duration::from_millis(500)), Seen::New, "a two-syllable reply");
    }

    #[test]
    fn a_line_stays_on_screen_while_it_keeps_being_read() {
        let then = Instant::now();
        let mut r = recent(&[(1, "기세 하나는 좋네 그래야 하늘의 성배지")], then);
        let later = then + WINDOW - Duration::from_secs(1);
        r.read_again(2, 1, "기세 하나는 종네 그래야 하늘의 성배지", later);
        let much_later = later + WINDOW - Duration::from_secs(1);
        assert!(matches!(r.seen("game.exe", "기세 하나는 총네 그래야 하늘의 성배지", much_later), Seen::Again { id: 1, .. }), "read again since");
        assert_eq!(r.seen("game.exe", "기세 하나는 총네 그래야 하늘의 성배지", much_later + WINDOW), Seen::New, "gone a window after its last reading");
    }

    #[test]
    fn short_answers_are_never_swallowed_by_a_longer_line() {
        let now = Instant::now();
        let r = recent(&[(1, "네가 왜 여기 있어?")], now);
        assert_eq!(r.seen("game.exe", "네", now), Seen::New);
        assert_eq!(r.seen("game.exe", "왜", now), Seen::New);
    }

    #[test]
    fn readings_from_another_game_or_after_the_window_are_new_lines() {
        let then = Instant::now();
        let r = recent(&[(1, "그 사람은 이미 떠났어.")], then);
        assert_eq!(r.seen("other.exe", "그 사람은 이미 떠났어.", then), Seen::New);
        assert_eq!(r.seen("game.exe", "그 사람은 이미 떠났어.", then + WINDOW + Duration::from_millis(1)), Seen::New);
    }

    /// Calibration on a real session: `MARU_HISTORY_DB=<history.db> cargo test -p maru-engine --lib -- --ignored
    /// --nocapture replays_a_saved_session` prints which saved live lines the rules would have merged. The lines are
    /// the player's game text, so they stay in their file and out of this repository.
    #[test]
    #[ignore]
    fn replays_a_saved_session() {
        let Some(path) = std::env::var_os("MARU_HISTORY_DB") else { return };
        let db = rusqlite::Connection::open(path).unwrap();
        let mut stmt = db.prepare("SELECT id, COALESCE(appeared_at, at), app, source FROM live_lines ORDER BY id").unwrap();
        let rows: Vec<(i64, i64, String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default(), r.get(3)?))).unwrap().map(Result::unwrap).collect();
        let (start, t0) = (Instant::now(), rows.first().map_or(0, |r| r.1));
        let mut r = Recent::default();
        let mut merged = 0;
        for (id, at, app, source) in &rows {
            let now = start + Duration::from_secs((at - t0).max(0) as u64);
            let text = crate::live_noise::trim_stray_marks(source);
            match r.seen(app, &text, now) {
                Seen::Again { id: original, .. } | Seen::Part { id: original, .. } => {
                    merged += 1;
                    println!("{id:>4} → {original:<4} {source}");
                    r.read_again(*id as u64, original, &text, now);
                }
                Seen::Grew { replaces } => {
                    println!("{id:>4} ⇒ {replaces:<4} {source} (grew)");
                    r.forget(replaces, *id as u64);
                    r.remember(*id as u64, app, &text, "", now);
                }
                Seen::New => r.remember(*id as u64, app, &text, "", now),
            }
        }
        println!("{merged} of {} saved lines would have been merged", rows.len());
    }
}
