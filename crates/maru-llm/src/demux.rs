//! Incremental parsing of model output.
//!
//! The model writes labelled lines (`POLITE: …`, `USAGE: …`). We want each
//! label's text to reach the UI *while it is still being generated*, not after
//! the whole answer is in, so parsing has to work on arbitrary chunk boundaries
//! (a label can be split across two network chunks).

/// Collects text deltas and yields complete lines.
#[derive(Debug, Default)]
pub struct LineSplitter {
    buf: String,
}

impl LineSplitter {
    pub fn push(&mut self, delta: &str, mut on_line: impl FnMut(&str)) {
        self.buf.push_str(delta);
        while let Some(i) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=i).collect();
            on_line(line.trim_end_matches(['\n', '\r']));
        }
    }

    /// Flush a trailing line that had no newline.
    pub fn finish(&mut self, mut on_line: impl FnMut(&str)) {
        if !self.buf.trim().is_empty() {
            on_line(self.buf.trim_end_matches(['\n', '\r']));
        }
        self.buf.clear();
    }
}

/// Routes text to the section named by the most recent `LABEL:` at a line start.
///
/// Text before the first label is dropped. Markdown decoration around labels
/// (`**POLITE:** …`) is tolerated.
#[derive(Debug)]
pub struct LabelDemux<T: Copy + PartialEq> {
    labels: Vec<(&'static str, T)>,
    current: Option<T>,
    at_line_start: bool,
    /// Characters of the line start that might still turn out to be a label.
    pending: String,
    /// Markdown emphasis seen before a possible label (`**POLITE:**`).
    lead: String,
    /// Swallow `*` and spaces right after a label.
    after_label: bool,
}

impl<T: Copy + PartialEq> LabelDemux<T> {
    pub fn new(labels: &[(&'static str, T)]) -> Self {
        Self {
            labels: labels.to_vec(),
            current: None,
            at_line_start: true,
            pending: String::new(),
            lead: String::new(),
            after_label: false,
        }
    }

    /// The section currently receiving text, if a label has been seen.
    pub fn current(&self) -> Option<T> {
        self.current
    }

    fn could_become_label(&self, pending: &str) -> bool {
        self.labels.iter().any(|(l, _)| l.starts_with(pending) || format!("{l}:").starts_with(pending))
    }

    fn emit(&self, out: &mut Vec<(T, String)>, text: &str) {
        if text.is_empty() {
            return;
        }
        let Some(section) = self.current else { return };
        match out.last_mut() {
            Some((s, buf)) if *s == section => buf.push_str(text),
            _ => out.push((section, text.to_string())),
        }
    }

    /// Feed a chunk; new section text is appended to `out` as `(section, text)`.
    pub fn push(&mut self, delta: &str, out: &mut Vec<(T, String)>) {
        for c in delta.chars() {
            if self.at_line_start {
                if c == '*' && self.pending.is_empty() {
                    self.lead.push(c);
                    continue;
                }
                if c == '\n' {
                    let flushed = format!("{}{}\n", self.lead, self.pending);
                    self.lead.clear();
                    self.pending.clear();
                    self.emit(out, &flushed);
                    continue;
                }
                self.pending.push(c);
                if let Some(&(_, section)) = self.labels.iter().find(|(l, _)| self.pending == format!("{l}:")) {
                    self.current = Some(section);
                    self.pending.clear();
                    self.lead.clear();
                    self.at_line_start = false;
                    self.after_label = true;
                } else if !self.could_become_label(&self.pending) {
                    let flushed = format!("{}{}", self.lead, self.pending);
                    self.lead.clear();
                    self.pending.clear();
                    self.at_line_start = false;
                    self.emit(out, &flushed);
                }
            } else if c == '\n' {
                self.after_label = false;
                self.at_line_start = true;
                self.emit(out, "\n");
            } else if self.after_label && (c == ' ' || c == '*') {
                // decoration between the label and its text
            } else {
                self.after_label = false;
                let mut buf = [0u8; 4];
                self.emit(out, c.encode_utf8(&mut buf));
            }
        }
    }

    /// Release anything held back while waiting to see if it was a label.
    pub fn finish(&mut self, out: &mut Vec<(T, String)>) {
        let flushed = format!("{}{}", self.lead, self.pending);
        self.lead.clear();
        self.pending.clear();
        self.emit(out, &flushed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum S {
        A,
        B,
    }

    fn run(chunks: &[&str]) -> Vec<(S, String)> {
        let mut d = LabelDemux::new(&[("POLITE", S::A), ("NOTE", S::B)]);
        let mut all: Vec<(S, String)> = Vec::new();
        for c in chunks {
            let mut out = Vec::new();
            d.push(c, &mut out);
            all.extend(out);
        }
        let mut out = Vec::new();
        d.finish(&mut out);
        all.extend(out);
        // merge for easy comparison
        let mut merged: Vec<(S, String)> = Vec::new();
        for (s, t) in all {
            match merged.last_mut() {
                Some((ls, lt)) if *ls == s => lt.push_str(&t),
                _ => merged.push((s, t)),
            }
        }
        merged
    }

    #[test]
    fn splits_sections_in_one_chunk() {
        let r = run(&["POLITE: 안녕하세요\nNOTE: greeting\n"]);
        assert_eq!(r, vec![(S::A, "안녕하세요\n".into()), (S::B, "greeting\n".into())]);
    }

    #[test]
    fn same_result_for_every_chunking() {
        let text = "POLITE: 감사합니다\nNOTE: formal thanks\n";
        let whole = run(&[text]);
        for size in 1..=7 {
            let chunks: Vec<String> = text.chars().collect::<Vec<_>>().chunks(size).map(|c| c.iter().collect()).collect();
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            assert_eq!(run(&refs), whole, "chunk size {size}");
        }
    }

    #[test]
    fn label_split_across_chunks_is_recognised() {
        let r = run(&["POL", "ITE", ": 네", "\nNO", "TE: ok"]);
        assert_eq!(r, vec![(S::A, "네\n".into()), (S::B, "ok".into())]);
    }

    #[test]
    fn markdown_bold_labels_are_tolerated() {
        let r = run(&["**POLITE:** 좋아요\n"]);
        assert_eq!(r, vec![(S::A, "좋아요\n".into())]);
    }

    #[test]
    fn continuation_lines_stay_in_the_current_section() {
        let r = run(&["NOTE: first\n- second line\n- third\n"]);
        assert_eq!(r, vec![(S::B, "first\n- second line\n- third\n".into())]);
    }

    #[test]
    fn text_before_any_label_is_dropped_and_lookalikes_are_text() {
        let r = run(&["Sure! Here you go:\nPOLITE: 네\nPOLISHED words\n"]);
        assert_eq!(r, vec![(S::A, "네\nPOLISHED words\n".into())]);
    }

    #[test]
    fn line_splitter_handles_partial_lines() {
        let mut s = LineSplitter::default();
        let mut lines = Vec::new();
        s.push("1|to st", |l| lines.push(l.to_string()));
        s.push("udy|\n2|big|x\r\n3|la", |l| lines.push(l.to_string()));
        s.finish(|l| lines.push(l.to_string()));
        assert_eq!(lines, ["1|to study|", "2|big|x", "3|la"]);
    }
}
