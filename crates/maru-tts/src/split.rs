//! Cutting text into sentences for speaking.
//!
//! A speech model reads one sentence well; a whole paragraph in one go drifts, takes long to start
//! and cannot be skipped. Scanned text arrives with hard line breaks in the middle of sentences, so
//! lines are joined unless they clearly end a sentence or the next one starts a new paragraph.

/// Sentences longer than this many characters are cut at a comma or space.
pub const MAX_CHARS: usize = 110;

const ENDERS: [char; 8] = ['.', '!', '?', '…', '。', '！', '？', '‥'];
const CLOSERS: [char; 12] = ['"', '\'', '”', '’', ')', ']', '」', '』', '）', '》', '〉', '>'];

/// The sentences of `text`, in order, trimmed and non-empty.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in paragraphs(text) {
        for s in split_paragraph(&paragraph) {
            out.extend(cap_length(s));
        }
    }
    out
}

/// Blank lines separate paragraphs; other line breaks are just where the line wrapped, unless the
/// line before ended a sentence (then the break also ends it, which suits dialogue and lists).
fn paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    for line in text.replace('\r', "").split('\n') {
        let line = line.trim();
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        }
        if !current.is_empty() {
            let ends = current.trim_end().trim_end_matches(CLOSERS).ends_with(ENDERS);
            if ends {
                paragraphs.push(std::mem::take(&mut current));
            } else {
                // Korean wraps between words (a space); a hyphen at a line end means a split word.
                current.push(' ');
            }
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs
}

fn split_paragraph(p: &str) -> Vec<String> {
    let chars: Vec<char> = p.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        if ENDERS.contains(&chars[i]) {
            // keep a run like "?!" or "..." together, then any closing quote or bracket
            let mut j = i + 1;
            while j < chars.len() && ENDERS.contains(&chars[j]) {
                j += 1;
            }
            while j < chars.len() && CLOSERS.contains(&chars[j]) {
                j += 1;
            }
            let at_end = j >= chars.len();
            let spaced = chars.get(j).is_some_and(|c| c.is_whitespace());
            // "3.5", "v1.2", "www.site.com": a dot between two non-space characters is not an end
            let inside_token = chars[i] == '.' && j == i + 1 && !at_end && !spaced;
            // CJK full stops need no space after them
            let cjk = matches!(chars[i], '。' | '！' | '？');
            // "1." or "a." heading an item is a list marker, not a sentence. Only digits and single
            // Latin letters count: "네." is a whole Korean sentence.
            let marker = chars[i] == '.' && j == i + 1 && spaced && is_list_marker(&chars[start..i]);
            if (at_end || spaced || cjk) && !inside_token && !marker {
                push(&mut out, &chars[start..j]);
                start = j;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    push(&mut out, &chars[start..]);
    out
}

/// The text before a dot is just a one or two digit number or a single Latin letter (`1`, `12`, `a`).
fn is_list_marker(before: &[char]) -> bool {
    let token: Vec<char> = before.iter().rev().take_while(|c| !c.is_whitespace()).copied().collect();
    let started_here = token.len() == before.len() || before.get(before.len() - token.len() - 1).is_some_and(|c| c.is_whitespace());
    started_here && ((1..=2).contains(&token.len()) && token.iter().all(char::is_ascii_digit) || token.len() == 1 && token[0].is_ascii_alphabetic())
}

fn push(out: &mut Vec<String>, chars: &[char]) {
    let s: String = chars.iter().collect::<String>().trim().to_string();
    // a piece with nothing to say aloud (only punctuation) belongs to the previous sentence
    if s.chars().any(char::is_alphanumeric) {
        out.push(s);
    } else if let Some(last) = out.last_mut() {
        last.push_str(&s);
    }
}

/// Cut an over-long sentence at its last comma (or space) before the limit.
fn cap_length(sentence: String) -> Vec<String> {
    if sentence.chars().count() <= MAX_CHARS {
        return vec![sentence];
    }
    let mut parts = Vec::new();
    let mut rest: Vec<char> = sentence.chars().collect();
    while rest.len() > MAX_CHARS {
        let window = &rest[..MAX_CHARS];
        let cut = window
            .iter()
            .rposition(|c| matches!(c, ',' | '，' | '、' | ';' | ':'))
            .filter(|&i| i > MAX_CHARS / 3)
            .or_else(|| window.iter().rposition(|c| c.is_whitespace()).filter(|&i| i > MAX_CHARS / 3))
            .map_or(MAX_CHARS, |i| i + 1);
        let head: String = rest[..cut].iter().collect();
        parts.push(head.trim().to_string());
        rest = rest[cut..].iter().copied().skip_while(|c| c.is_whitespace()).collect();
    }
    if !rest.is_empty() {
        parts.push(rest.iter().collect());
    }
    parts.retain(|p| !p.is_empty());
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_at_sentence_ends() {
        assert_eq!(sentences("안녕하세요. 저는 학생이에요! 어디 가요? 학교에 가요."), ["안녕하세요.", "저는 학생이에요!", "어디 가요?", "학교에 가요."]);
    }

    #[test]
    fn keeps_quotes_and_runs_of_punctuation_with_their_sentence() {
        assert_eq!(sentences("\"정말요?!\" 그가 물었다. 네…"), ["\"정말요?!\"", "그가 물었다.", "네…"]);
        assert_eq!(sentences("잠깐... 기다려요."), ["잠깐...", "기다려요."]);
    }

    #[test]
    fn decimals_versions_and_list_markers_are_not_sentence_ends() {
        assert_eq!(sentences("가격은 3.5달러예요. 버전 1.2.3 입니다."), ["가격은 3.5달러예요.", "버전 1.2.3 입니다."]);
        assert_eq!(sentences("1. 사과 2. 배"), ["1. 사과 2. 배"]);
        assert_eq!(sentences("네. 아니요."), ["네.", "아니요."], "one syllable can be a whole sentence");
    }

    #[test]
    fn wrapped_lines_are_joined_and_paragraphs_kept_apart() {
        let text = "저는 어제 친구와 함께\n영화를 봤어요. 재미있었어요.\n\n오늘은 비가 와요";
        assert_eq!(sentences(text), ["저는 어제 친구와 함께 영화를 봤어요.", "재미있었어요.", "오늘은 비가 와요"]);
    }

    #[test]
    fn a_line_that_ends_a_sentence_ends_it_for_dialogue_and_lists() {
        assert_eq!(sentences("네.\n아니요.\n모르겠어요"), ["네.", "아니요.", "모르겠어요"]);
    }

    #[test]
    fn long_sentences_are_cut_at_commas_then_spaces() {
        let long = format!("{}, {}", "가".repeat(60), "나".repeat(60));
        let parts = sentences(&long);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].ends_with(','), "{parts:?}");
        let no_commas = (0..60).map(|_| "단어").collect::<Vec<_>>().join(" ");
        let parts = sentences(&no_commas);
        assert!(parts.len() >= 2 && parts.iter().all(|p| p.chars().count() <= MAX_CHARS), "{parts:?}");
        assert_eq!(parts.join(" "), no_commas, "nothing is lost");
        let solid = "가".repeat(300);
        assert!(sentences(&solid).iter().all(|p| p.chars().count() <= MAX_CHARS));
    }

    #[test]
    fn nothing_in_nothing_out() {
        assert!(sentences("").is_empty());
        assert!(sentences("  \n\n  ").is_empty());
        assert_eq!(sentences("...").len(), 0, "punctuation alone is not speech");
        assert_eq!(sentences("네 ... 아니요"), ["네 ...", "아니요"], "a spoken pause is a boundary");
    }
}
