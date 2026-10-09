//! Human-readable rendering of the engine's event stream.

use std::io::Write;

use maru_core::event::{DeepSection, Side, Stage, Variant};
use maru_core::{Direction, Eojeol, Event, WordState};

pub struct Renderer<W: Write> {
    out: W,
    color: bool,
    /// Which streaming translation line is currently open (so deltas continue it).
    open: Option<Variant>,
    pub timings: Vec<(Stage, u32)>,
    pub errors: Vec<String>,
    /// Output that would interleave with a streaming line; printed at `Done`.
    pending: Vec<String>,
    section_start: bool,
}

fn label(v: Variant) -> &'static str {
    match v {
        Variant::Primary => "",
        Variant::Polite => "polite  (해요체) ",
        Variant::Formal => "formal  (합쇼체) ",
        Variant::Casual => "casual  (반말)   ",
    }
}

fn stage_name(s: Stage) -> &'static str {
    match s {
        Stage::Capture => "capture",
        Stage::Ocr => "ocr",
        Stage::Morphology => "analysis",
        Stage::Dictionary => "dictionary",
        Stage::Llm => "model",
        Stage::FirstToken => "first token",
        Stage::Total => "total",
    }
}

impl<W: Write> Renderer<W> {
    pub fn new(out: W, color: bool) -> Self {
        Self { out, color, open: None, timings: Vec::new(), errors: Vec::new(), pending: Vec::new(), section_start: false }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
    }

    fn close_line(&mut self) {
        if self.open.take().is_some() {
            let _ = writeln!(self.out);
        }
    }

    fn word_row(&self, e: &Eojeol) -> String {
        let mark = match e.state {
            WordState::Known => self.paint("2", "✓"),
            WordState::Learning => self.paint("33", "•"),
            WordState::New => " ".into(),
        };
        let mut row = format!(" {mark} {:<10} {}", e.surface, self.paint("2", &e.romanization));
        if e.pronunciation != e.surface.trim_matches(|c: char| !c.is_alphanumeric() && !maru_lang::hangul::is_hangul(c)) {
            row.push_str(&self.paint("2", &format!("  [{}]", e.pronunciation)));
        }
        if !e.dict_glosses.is_empty() {
            row.push_str(&format!("  {}", e.dict_glosses.join(" / ")));
        }
        if let Some(l) = &e.level {
            row.push_str(&self.paint("36", &format!("  ({l})")));
        }
        row
    }

    fn breakdown_text(&self, side: Side, eojeols: &[Eojeol], romanization: &str, pronunciation: &str) -> String {
        let title = match side {
            Side::Source => "breakdown",
            Side::Target => "the Korean, broken down",
        };
        let mut block = format!("\n{}\n", self.paint("1", title));
        for w in eojeols.iter().filter(|w| w.content) {
            block.push_str(&self.word_row(w));
            block.push('\n');
            if !w.formula.is_empty() {
                block.push_str(&format!("      {}\n", self.paint("2", &w.formula)));
            }
        }
        block.push_str(&format!("   {}  {}\n", self.paint("2", "romanized"), romanization));
        block.push_str(&format!("   {}  {}\n\n", self.paint("2", "sounds like"), pronunciation));
        block
    }

    fn flush_pending(&mut self) {
        for block in std::mem::take(&mut self.pending) {
            let _ = write!(self.out, "{block}");
        }
    }

    pub fn event(&mut self, e: &Event) {
        match e {
            Event::Started { direction, capture, cached, .. } => {
                let arrow = match direction {
                    Direction::KoToEn => "한국어 → English",
                    Direction::EnToKo => "English → 한국어",
                };
                let src = format!("{:?}", capture.source).to_lowercase();
                let _ = writeln!(self.out, "{} {}{}", self.paint("1", arrow), self.paint("2", &format!("via {src}")), if *cached { " (cached)" } else { "" });
            }
            Event::Breakdown { side, eojeols, romanization, pronunciation, .. } => {
                let block = self.breakdown_text(*side, eojeols, romanization, pronunciation);
                match side {
                    // The source breakdown comes first, before anything streams.
                    Side::Source => {
                        self.close_line();
                        let _ = write!(self.out, "{block}");
                    }
                    // The breakdown of our own Korean output arrives mid-stream: hold it back.
                    Side::Target => self.pending.push(block),
                }
            }
            Event::Translation { variant, delta, .. } => {
                if self.open != Some(*variant) {
                    self.close_line();
                    let _ = write!(self.out, "{}", self.paint("1", label(*variant)));
                    self.open = Some(*variant);
                }
                let _ = write!(self.out, "{delta}");
                let _ = self.out.flush();
            }
            Event::TranslationDone { .. } => self.close_line(),
            Event::Gloss { gloss, .. } => {
                let note = gloss.note.as_deref().map(|n| format!("  — {n}")).unwrap_or_default();
                let line = format!("  #{} {}{}\n", gloss.index, self.paint("32", &gloss.meaning), self.paint("2", &note));
                self.pending.push(line);
            }
            Event::Note { text, .. } => {
                let line = format!("  {} {text}\n", self.paint("33", "note:"));
                self.pending.push(line);
            }
            Event::DeepEntries { entries, conjugation, .. } => {
                self.close_line();
                if !conjugation.is_empty() {
                    let steps = conjugation.iter().map(|c| format!("{} ({})", c.form, c.label)).collect::<Vec<_>>().join("\n    + ");
                    let _ = writeln!(self.out, "{}\n    {steps}", self.paint("1", "built from"));
                }
                for en in entries {
                    let hanja = en.hanja.as_deref().map(|h| format!(" 〔{h}〕")).unwrap_or_default();
                    let _ = writeln!(self.out, "{} {}{}", self.paint("1", &en.lemma), self.paint("2", &en.pos), hanja);
                    for (i, s) in en.senses.iter().take(4).enumerate() {
                        let _ = writeln!(self.out, "  {}. {}", i + 1, s.gloss);
                    }
                }
            }
            Event::DeepDelta { section, delta, .. } => {
                let key = match section {
                    DeepSection::Usage => Variant::Polite,
                    DeepSection::Examples => Variant::Formal,
                    DeepSection::Nuance => Variant::Casual,
                };
                if self.open != Some(key) {
                    self.close_line();
                    let name = match section {
                        DeepSection::Usage => "usage",
                        DeepSection::Examples => "examples",
                        DeepSection::Nuance => "nuance",
                    };
                    let _ = writeln!(self.out, "{}", self.paint("1", name));
                    self.open = Some(key);
                    self.section_start = true;
                }
                let delta = if self.section_start { delta.trim_start() } else { delta.as_str() };
                if !delta.is_empty() {
                    self.section_start = false;
                    let _ = write!(self.out, "{delta}");
                }
            }
            Event::WordState { status, .. } => {
                let _ = writeln!(self.out, "{} {:?}, looked up {}×{}", self.paint("2", &status.key), status.state, status.lookups, if status.saved { ", queued for review" } else { "" });
            }
            Event::Timing { stage, ms, .. } => self.timings.push((*stage, *ms)),
            Event::Error { message, .. } => {
                self.close_line();
                self.errors.push(message.clone());
                let _ = writeln!(self.out, "{} {message}", self.paint("31", "error:"));
            }
            Event::Done { .. } => {
                self.close_line();
                self.flush_pending();
                if !self.timings.is_empty() {
                    let line = self.timings.iter().map(|(s, ms)| format!("{} {ms} ms", stage_name(*s))).collect::<Vec<_>>().join(" · ");
                    let _ = writeln!(self.out, "{}", self.paint("2", &line));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use maru_core::event::Variant;
    use maru_core::{Capture, Event, Gloss};

    use super::*;

    fn render(events: &[Event]) -> String {
        let mut buf = Vec::new();
        let mut r = Renderer::new(&mut buf, false);
        for e in events {
            r.event(e);
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn streaming_text_continues_one_line_and_glosses_follow() {
        let id = 1;
        let out = render(&[
            Event::Started { id, direction: Direction::KoToEn, capture: Capture::typed("x"), cached: false },
            Event::Translation { id, variant: Variant::Primary, delta: "I studied ".into() },
            Event::Translation { id, variant: Variant::Primary, delta: "at school.".into() },
            Event::TranslationDone { id, variant: Variant::Primary, text: "I studied at school.".into() },
            Event::Gloss { id, side: Side::Source, gloss: Gloss { index: 2, meaning: "to study".into(), note: Some("past".into()) } },
            Event::Timing { id, stage: Stage::FirstToken, ms: 180 },
            Event::Timing { id, stage: Stage::Total, ms: 640 },
            Event::Done { id },
        ]);
        assert!(out.contains("한국어 → English via typed"));
        assert!(out.contains("I studied at school.\n"), "{out}");
        assert!(out.contains("#2 to study  — past"));
        assert!(out.trim_end().ends_with("first token 180 ms · total 640 ms"));
    }

    #[test]
    fn meanings_are_held_back_so_the_streaming_line_stays_contiguous() {
        let id = 1;
        let out = render(&[
            Event::Translation { id, variant: Variant::Primary, delta: "I studied ".into() },
            Event::Gloss { id, side: Side::Source, gloss: Gloss { index: 0, meaning: "I".into(), note: None } },
            Event::Translation { id, variant: Variant::Primary, delta: "at school.".into() },
            Event::Note { id, text: "topic marker".into() },
            Event::TranslationDone { id, variant: Variant::Primary, text: "I studied at school.".into() },
            Event::Done { id },
        ]);
        let first_line = out.lines().next().unwrap();
        assert_eq!(first_line, "I studied at school.");
        assert!(out.find("#0 I").unwrap() > out.find("at school.").unwrap());
        assert!(out.contains("note: topic marker"));
    }

    #[test]
    fn deep_dive_sections_have_no_blank_line_after_their_heading() {
        let id = 1;
        let out = render(&[
            Event::DeepDelta { id, section: DeepSection::Examples, delta: "\n- 공부해요 — I study.\n".into() },
            Event::Done { id },
        ]);
        assert!(out.starts_with("examples\n- 공부해요"), "{out:?}");
    }

    #[test]
    fn errors_are_collected_and_shown() {
        let mut buf = Vec::new();
        let mut r = Renderer::new(&mut buf, false);
        r.event(&Event::Error { id: 1, stage: Stage::Llm, message: "boom".into() });
        assert_eq!(r.errors, ["boom"]);
        assert!(String::from_utf8(buf).unwrap().contains("error: boom"));
    }

    #[test]
    fn colour_is_optional() {
        let mut plain = Vec::new();
        let mut coloured = Vec::new();
        for (buf, color) in [(&mut plain, false), (&mut coloured, true)] {
            let mut r = Renderer::new(buf, color);
            r.event(&Event::Note { id: 1, text: "hi".into() });
            r.event(&Event::Done { id: 1 }); // held-back output is printed at Done
        }
        assert!(!String::from_utf8(plain).unwrap().contains('\x1b'));
        assert!(String::from_utf8(coloured).unwrap().contains('\x1b'));
    }
}
