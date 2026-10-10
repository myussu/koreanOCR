//! Matching lines from one OCR engine against a second reading of the same image.
//!
//! Used to combine a detector that knows *where* the lines are (PP-OCR) with a reader that
//! knows what they *say* (a vision model that reads the whole image but returns plain text).
//! Pure text work: no image, no model.

const INF: u32 = u32::MAX / 2;

// Edit costs, in half-units so that dropping a stray symbol can be cheaper than changing a letter.
// OCR noise is mostly stray punctuation (`'`, `|`, a lone `.`): it should be easy to ignore and must
// never be "corrected" into a letter that belongs to the next line.
const LETTER_EDIT: u32 = 2;
const SYMBOL_EDIT: u32 = 1;
const CLASS_CLASH: u32 = 4;

fn is_letter(c: char) -> bool {
    c.is_alphanumeric()
}

/// Cost of dropping `c` (from either side).
fn skip_cost(c: char) -> u32 {
    if is_letter(c) { LETTER_EDIT } else { SYMBOL_EDIT }
}

fn swap_cost(a: char, b: char) -> u32 {
    match (a == b, is_letter(a), is_letter(b)) {
        (true, ..) => 0,
        (false, true, true) | (false, false, false) => LETTER_EDIT,
        _ => CLASS_CLASH,
    }
}

/// For each line, the part of `reference` that is the same text read better, if any.
///
/// Lines are matched against `reference` by approximate substring search (edit distance), longest
/// line first, and each part of the reference can be used only once. The longest-first rule
/// matters: a short line like `승차권` is one letter away from `승차해` inside somebody else's
/// sentence, and must not steal it. Lines of one or two characters have to match exactly, and
/// longer ones may differ by a third of their length. A line with no acceptable match gets `None`
/// (the caller keeps its own text). Spacing in the returned text is the reference's.
pub fn better_readings(lines: &[&str], reference: &str) -> Vec<Option<String>> {
    let reference: Vec<char> = reference.chars().collect();
    // The non-space characters, each with its position in `reference`.
    let compact: Vec<(char, usize)> = reference.iter().copied().enumerate().filter(|(_, c)| !c.is_whitespace()).map(|(i, c)| (c, i)).collect();
    let patterns: Vec<Vec<char>> = lines.iter().map(|l| l.chars().filter(|c| !c.is_whitespace()).collect()).collect();

    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(patterns[i].len()), i));

    let mut claimed = vec![false; compact.len()];
    let mut out = vec![None; lines.len()];
    for i in order {
        let pattern = &patterns[i];
        if pattern.is_empty() {
            continue;
        }
        let allowed = if pattern.len() <= 2 { 0 } else { (pattern.len() / 3) as u32 * LETTER_EDIT };
        let Some((start, mut end, distance)) = best_window(pattern, &compact, &claimed) else { continue };
        if distance > allowed || end <= start {
            continue;
        }
        // The end of a sentence belongs to it even if the other engine did not see the full stop.
        // (Only punctuation that ends a word: the comma in `5,000` is part of the number.)
        while end < compact.len()
            && !claimed[end]
            && matches!(compact[end].0, '.' | ',' | '!' | '?' | ';' | ':' | '…' | '~' | '。')
            && compact.get(end + 1).is_none_or(|next| next.1 != compact[end].1 + 1)
        {
            end += 1;
        }
        claimed[start..end].iter_mut().for_each(|c| *c = true);
        let text: String = reference[compact[start].1..=compact[end - 1].1].iter().map(|&c| if c.is_whitespace() { ' ' } else { c }).collect();
        out[i] = Some(text.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    out
}

/// Best approximate occurrence of `pattern` in the unclaimed characters: `(start, end, distance)`
/// as a half-open range into `compact`.
fn best_window(pattern: &[char], compact: &[(char, usize)], claimed: &[bool]) -> Option<(usize, usize, u32)> {
    let (m, n) = (pattern.len(), compact.len());
    if n == 0 {
        return None;
    }
    let w = n + 1;
    // d[i][j]: cheapest way to turn the first i pattern characters into some stretch of the
    // reference that ends after its j-th character; st[i][j]: where that stretch starts.
    let mut d = vec![0u32; (m + 1) * w];
    let mut st: Vec<u32> = vec![0; (m + 1) * w];
    for (j, start) in st.iter_mut().take(n + 1).enumerate() {
        *start = j as u32;
    }
    for i in 1..=m {
        d[i * w] = d[(i - 1) * w] + skip_cost(pattern[i - 1]);
        for j in 1..=n {
            let up = (d[(i - 1) * w + j] + skip_cost(pattern[i - 1]), st[(i - 1) * w + j]);
            let mut best = up;
            if !claimed[j - 1] {
                let diag = (d[(i - 1) * w + j - 1] + swap_cost(pattern[i - 1], compact[j - 1].0), st[(i - 1) * w + j - 1]);
                let left = (d[i * w + j - 1] + skip_cost(compact[j - 1].0), st[i * w + j - 1]);
                best = diag;
                if up.0 < best.0 {
                    best = up;
                }
                if left.0 < best.0 {
                    best = left;
                }
            }
            d[i * w + j] = best.0.min(INF);
            st[i * w + j] = best.1;
        }
    }
    // Equal cost: the longer window (a substitution beats a deletion that stops one character short).
    let (end, distance) = (0..=n).map(|j| (j, d[m * w + j])).min_by_key(|&(j, dist)| (dist, std::cmp::Reverse(j)))?;
    Some((st[m * w + end] as usize, end, distance))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[&str], reference: &str) -> Vec<Option<&'static str>> {
        better_readings(lines, reference).into_iter().map(|o| o.map(|s| &*Box::leak(s.into_boxed_str()))).collect()
    }

    #[test]
    fn a_misread_line_takes_the_other_engines_text_and_spacing() {
        // PP-OCR's lines for the handwritten wall (b.png) against PaddleOCR-VL's reading.
        let lines = ["전 금도", "충분히 '잘하고 있어요", "오늘도 고생했어요"];
        let reference = "지금도\n충분히 잘하고 있어요.\n오늘도 고생했어요.";
        assert_eq!(run(&lines, reference), [Some("지금도"), Some("충분히 잘하고 있어요."), Some("오늘도 고생했어요.")]);
    }

    #[test]
    fn text_the_reader_skipped_keeps_the_detectors_line() {
        // c.png: the reader never produced 승차권 or the passport text.
        let lines = ["KORAIL", "승차권", "서울 → 부산", "* 출발 10분 천까지 승차해 주시기 바랍니다.", "PUBLIC OF KOREA"];
        let reference = "KORAIL 서울 → 부산 2024년 5월 18일 (토) 10:15 → 12:45 * 출발 10분 전까지 승차해 주시기 바랍니다.";
        let got = run(&lines, reference);
        assert_eq!(got[0], Some("KORAIL"));
        assert_eq!(got[1], None, "승차권 is one letter from 승차해, but that text belongs to the long line");
        assert_eq!(got[2], Some("서울 → 부산"));
        assert_eq!(got[3], Some("* 출발 10분 전까지 승차해 주시기 바랍니다."));
        assert_eq!(got[4], None);
    }

    #[test]
    fn reading_order_may_differ_between_the_engines() {
        // a.png: the detector lists the lantern's 호감집 second, the reader never saw it at all.
        let lines = ["좋은 사람들과", "호감집", "'좋은 시간을", "닭갈비 12,000"];
        let reference = "홍은 사람들과 좋은 시간을 닭갈비 12,000";
        assert_eq!(run(&lines, reference), [Some("홍은 사람들과"), None, Some("좋은 시간을"), Some("닭갈비 12,000")]);
        // the same lines in another order give the same answers
        let reversed: Vec<&str> = lines.iter().rev().copied().collect();
        let mut back = run(&reversed, reference);
        back.reverse();
        assert_eq!(back, run(&lines, reference));
    }

    #[test]
    fn short_lines_must_match_exactly_and_noise_matches_nothing() {
        assert_eq!(run(&["RG", "'"], "막걸리 5,000"), [None, None]);
        assert_eq!(run(&["5"], "막걸리 5,000"), [Some("5")]);
        assert_eq!(run(&["오늘"], "오는 하늘"), [None], "two characters: no tolerance");
    }

    #[test]
    fn each_part_of_the_reference_is_used_once() {
        // two identical detector lines, one reading: only one may claim it
        let got = run(&["고생했어요", "고생했어요"], "오늘도 고생했어요");
        assert_eq!(got.iter().filter(|g| g.is_some()).count(), 1);
    }

    #[test]
    fn empty_inputs_are_fine() {
        assert_eq!(run(&[], "abc"), Vec::<Option<&str>>::new());
        assert_eq!(run(&["abc"], ""), [None]);
        assert_eq!(run(&["", "  "], "abc"), [None, None]);
    }

    #[test]
    fn the_window_is_found_in_the_middle_of_a_longer_reading() {
        let got = run(&["서울 → 부선"], "KORAIL 승차권 서울 → 부산 2024년");
        assert_eq!(got, [Some("서울 → 부산")]);
    }
}
