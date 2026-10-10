//! The four things we ask the model, as streaming operations.
//!
//! Each takes a `sink` that is called for every piece of output as it arrives,
//! so the overlay can paint the first words of a translation while the model
//! is still writing the rest. Cancelling = dropping the future.

use maru_core::event::{DeepSection, Variant};
use maru_core::{Gloss, Register};

use crate::Result;
use crate::client::{ChatOpts, LlmClient};
use crate::demux::{LabelDemux, LineSplitter};
use crate::prompts::{self, GlossWordPrompt, PromptContext};

/// One piece of structured output.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmChunk {
    /// Streaming text of a translation variant.
    Text { variant: Variant, delta: String },
    /// A variant is complete (cleaned text).
    TextDone { variant: Variant, text: String },
    Gloss(Gloss),
    Note(String),
    Deep { section: DeepSection, delta: String },
}

pub struct KoEnRequest<'a> {
    pub text: &'a str,
    pub ctx: &'a PromptContext,
}

/// One live subtitle line: translation only, with the lines before it as chat turns.
pub struct LiveRequest<'a> {
    pub text: &'a str,
    /// Hint and glossary (its `history` is ignored: `turns` carries the earlier lines).
    pub ctx: &'a PromptContext,
    /// Earlier `(source, translation)` pairs, oldest first.
    pub turns: &'a [(String, String)],
}

pub struct EnKoRequest<'a> {
    pub text: &'a str,
    pub register: Register,
    pub ctx: &'a PromptContext,
}

#[derive(Debug, Clone)]
pub struct GlossWord {
    pub index: usize,
    pub surface: String,
    pub parts: String,
    pub candidates: Vec<String>,
}

pub struct GlossRequest<'a> {
    pub sentence: &'a str,
    pub words: &'a [GlossWord],
    pub ctx: &'a PromptContext,
}

/// Short English labels for a batch of strings read off the screen.
pub struct LabelRequest<'a> {
    /// `(number, text)`; numbers come back on the answer lines.
    pub items: &'a [(usize, &'a str)],
    pub ctx: &'a PromptContext,
}

/// One answered line of a label batch.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelLine {
    pub index: usize,
    pub label: String,
}

pub struct DeepRequest<'a> {
    pub key: &'a str,
    pub surface: &'a str,
    pub sentence: &'a str,
    /// Plain-text dictionary summary to ground the answer ("noun: school").
    pub dictionary: &'a str,
    pub ctx: &'a PromptContext,
}

/// Strip wrapping quotes and whitespace the model sometimes adds.
pub fn clean_text(s: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix(['"', '“', '「'])
        .and_then(|x| x.strip_suffix(['"', '”', '」']))
        .unwrap_or(t);
    t.trim().to_string()
}

#[derive(Debug, PartialEq)]
pub enum GlossLine {
    Gloss(Gloss),
    Note(String),
}

/// Parse one `N|gloss|note` line. Tolerates `N. gloss`, `N: gloss`, bullets and bold.
pub fn parse_gloss_line(line: &str) -> Option<GlossLine> {
    let line = line.trim().trim_start_matches(['-', '*', '•', ' ']).trim_end_matches('*').trim();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix("NOTE").and_then(|r| r.strip_prefix(['|', ':'])) {
        let note = rest.trim();
        return (!note.is_empty()).then(|| GlossLine::Note(note.to_string()));
    }
    let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let index: usize = digits.parse().ok()?;
    let rest = line[digits.len()..].trim_start_matches(['|', '.', ':', ')', ' ']);
    let mut parts = rest.splitn(2, '|');
    let meaning = parts.next()?.trim().trim_matches('*').trim();
    if meaning.is_empty() {
        return None;
    }
    let note = parts.next().map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    Some(GlossLine::Gloss(Gloss { index, meaning: meaning.to_string(), note }))
}

/// Parse one `N|label` line. Tolerates `N. label`, `N: label`, bullets, bold and quotes.
/// `?` (the model's "this is not text") is returned as is; the caller decides what to do with it.
pub fn parse_label_line(line: &str) -> Option<LabelLine> {
    let line = line.trim().trim_start_matches(['-', '*', '•', ' ']).trim();
    let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let index: usize = digits.parse().ok()?;
    let rest = line[digits.len()..].trim_start_matches(['|', '.', ':', ')', ' ']);
    let label = clean_text(rest.trim().trim_matches('*'));
    (!label.is_empty()).then_some(LabelLine { index, label })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnKoSection {
    Text(Variant),
    Note,
}

/// Incremental parser for the English → Korean answer. `feed` forwards new text to the
/// sink immediately; a variant is closed (`TextDone`) as soon as the next label starts.
struct EnKoStream {
    demux: LabelDemux<EnKoSection>,
    texts: Vec<(EnKoSection, String)>,
    last: Option<EnKoSection>,
}

impl EnKoStream {
    fn new() -> Self {
        Self {
            demux: LabelDemux::new(&[
                ("POLITE", EnKoSection::Text(Variant::Polite)),
                ("FORMAL", EnKoSection::Text(Variant::Formal)),
                ("CASUAL", EnKoSection::Text(Variant::Casual)),
                ("NOTE", EnKoSection::Note),
            ]),
            texts: Vec::new(),
            last: None,
        }
    }

    fn route(&mut self, parts: Vec<(EnKoSection, String)>, sink: &mut (dyn FnMut(LlmChunk) + Send)) {
        for (section, text) in parts {
            if self.last != Some(section) {
                if let Some(prev) = self.last {
                    finish_section(prev, &self.texts, sink);
                }
                self.last = Some(section);
            }
            match self.texts.iter_mut().find(|(s, _)| *s == section) {
                Some((_, t)) => t.push_str(&text),
                None => self.texts.push((section, text.clone())),
            }
            if let EnKoSection::Text(variant) = section {
                // The newline that ends a line is not part of the translation.
                let delta = text.trim_end_matches('\n');
                if !delta.is_empty() {
                    sink(LlmChunk::Text { variant, delta: delta.to_string() });
                }
            }
        }
    }

    fn feed(&mut self, delta: &str, sink: &mut (dyn FnMut(LlmChunk) + Send)) {
        let mut parts = Vec::new();
        self.demux.push(delta, &mut parts);
        self.route(parts, sink);
    }

    fn finish(&mut self, sink: &mut (dyn FnMut(LlmChunk) + Send)) {
        let mut parts = Vec::new();
        self.demux.finish(&mut parts);
        self.route(parts, sink);
        if let Some(prev) = self.last.take() {
            finish_section(prev, &self.texts, sink);
        }
    }
}

fn finish_section(section: EnKoSection, texts: &[(EnKoSection, String)], sink: &mut (dyn FnMut(LlmChunk) + Send)) {
    let Some((_, text)) = texts.iter().find(|(s, _)| *s == section) else { return };
    let text = clean_text(text);
    match section {
        EnKoSection::Text(variant) => sink(LlmChunk::TextDone { variant, text }),
        EnKoSection::Note => {
            if !text.is_empty() {
                sink(LlmChunk::Note(text));
            }
        }
    }
}

fn emit_gloss_line(line: &str, sink: &mut (dyn FnMut(LlmChunk) + Send)) {
    match parse_gloss_line(line) {
        Some(GlossLine::Gloss(g)) => sink(LlmChunk::Gloss(g)),
        Some(GlossLine::Note(n)) => sink(LlmChunk::Note(n)),
        None => {}
    }
}

impl LlmClient {
    /// Korean → English. Streams `Text` deltas, then one `TextDone`. Returns the cleaned translation.
    pub async fn translate_ko_en(&self, req: &KoEnRequest<'_>, sink: &mut (dyn FnMut(LlmChunk) + Send)) -> Result<String> {
        let messages = prompts::ko_en(req.text, req.ctx);
        let mut started = false;
        let full = self
            .stream_chat(&messages, &ChatOpts::default(), &mut |d| {
                // Drop leading whitespace so the panel does not start with a gap.
                let d = if started { d } else { d.trim_start() };
                if d.is_empty() {
                    return;
                }
                started = true;
                sink(LlmChunk::Text { variant: Variant::Primary, delta: d.to_string() });
            })
            .await?;
        let text = clean_text(&full);
        sink(LlmChunk::TextDone { variant: Variant::Primary, text: text.clone() });
        Ok(text)
    }

    /// Korean → English for a subtitle line: short answers, low temperature, no extras.
    pub async fn translate_live(&self, req: &LiveRequest<'_>, sink: &mut (dyn FnMut(LlmChunk) + Send)) -> Result<String> {
        let messages = prompts::ko_en_live(req.text, req.ctx, req.turns);
        // A subtitle is a sentence or two: cut off a model that starts to ramble.
        let max_tokens = (req.text.chars().count() as u32 * 2 + 40).min(256);
        let opts = ChatOpts { max_tokens: Some(max_tokens), temperature: Some(0.1), ..Default::default() };
        let mut started = false;
        let full = self
            .stream_chat(&messages, &opts, &mut |d| {
                let d = if started { d } else { d.trim_start() };
                if d.is_empty() {
                    return;
                }
                started = true;
                sink(LlmChunk::Text { variant: Variant::Primary, delta: d.to_string() });
            })
            .await?;
        let text = clean_text(&full);
        sink(LlmChunk::TextDone { variant: Variant::Primary, text: text.clone() });
        Ok(text)
    }

    /// Contextual word meanings, emitted one by one as soon as each line completes.
    pub async fn glosses(&self, req: &GlossRequest<'_>, sink: &mut (dyn FnMut(LlmChunk) + Send)) -> Result<()> {
        let words: Vec<GlossWordPrompt<'_>> = req
            .words
            .iter()
            .map(|w| GlossWordPrompt { index: w.index, surface: &w.surface, parts: &w.parts, candidates: &w.candidates })
            .collect();
        let messages = prompts::glosses(req.sentence, &words, req.ctx);
        let opts = ChatOpts { max_tokens: Some(64 + 40 * req.words.len() as u32), ..Default::default() };
        let mut lines = LineSplitter::default();
        self.stream_chat(&messages, &opts, &mut |d| lines.push(d, |l| emit_gloss_line(l, sink))).await?;
        lines.finish(|l| emit_gloss_line(l, sink));
        Ok(())
    }

    /// Short labels for many strings in one request, delivered line by line as the model writes them.
    pub async fn labels(&self, req: &LabelRequest<'_>, sink: &mut (dyn FnMut(LabelLine) + Send)) -> Result<()> {
        let messages = prompts::labels(req.items, req.ctx);
        let opts = ChatOpts { max_tokens: Some(32 + 14 * req.items.len() as u32), temperature: Some(0.1), ..Default::default() };
        let mut lines = LineSplitter::default();
        let mut emit = |l: &str| {
            if let Some(line) = parse_label_line(l) {
                sink(line);
            }
        };
        self.stream_chat(&messages, &opts, &mut |d| lines.push(d, &mut emit)).await?;
        lines.finish(&mut emit);
        Ok(())
    }

    /// English → Korean in three registers plus a nuance note.
    pub async fn translate_en_ko(&self, req: &EnKoRequest<'_>, sink: &mut (dyn FnMut(LlmChunk) + Send)) -> Result<()> {
        let messages = prompts::en_ko(req.text, req.register, req.ctx);
        let mut stream = EnKoStream::new();
        self.stream_chat(&messages, &ChatOpts::default(), &mut |d| stream.feed(d, sink)).await?;
        stream.finish(sink);
        Ok(())
    }

    /// Streaming explanation for the click-a-word panel.
    pub async fn deep_dive(&self, req: &DeepRequest<'_>, sink: &mut (dyn FnMut(LlmChunk) + Send)) -> Result<()> {
        let messages = prompts::deep(req.key, req.surface, req.sentence, req.dictionary, req.ctx);
        let opts = ChatOpts { max_tokens: Some(450), temperature: Some(0.4), ..Default::default() };
        let mut demux = LabelDemux::new(&[
            ("USAGE", DeepSection::Usage),
            ("EXAMPLES", DeepSection::Examples),
            ("NUANCE", DeepSection::Nuance),
        ]);
        self.stream_chat(&messages, &opts, &mut |d| {
            let mut parts = Vec::new();
            demux.push(d, &mut parts);
            for (section, delta) in parts {
                sink(LlmChunk::Deep { section, delta });
            }
        })
        .await?;
        let mut rest = Vec::new();
        demux.finish(&mut rest);
        for (section, delta) in rest {
            sink(LlmChunk::Deep { section, delta });
        }
        Ok(())
    }

    /// Fill the KV cache with each task's system prompt so the first real request is fast.
    pub async fn warm_up(&self) {
        for messages in prompts::warmup() {
            let opts = ChatOpts { max_tokens: Some(1), ..Default::default() };
            if let Err(e) = self.stream_chat(&messages, &opts, &mut |_| {}).await {
                tracing::debug!("warm-up request failed: {e}");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use maru_core::config::LlmConfig;

    use super::*;
    use crate::client::testing::*;

    fn client(url: &str) -> LlmClient {
        LlmClient::new(url, &LlmConfig { request_timeout_secs: 3, ..Default::default() })
    }

    /// Answer by inspecting the request, like a real model would.
    fn model(path: &str, body: &str) -> MockResponse {
        let _ = path;
        let answer: Vec<&str> = if body.contains("Translate into English") {
            vec!["\n", "I'm ", "studying ", "Korean."]
        } else if body.contains("Words:") {
            vec!["1|I|", "\n2|Korean|\n3|to study ", "+ progressive|-고 있다 = ", "ongoing action\nNOTE|Topic marker 는 sets the subject.", "\n"]
        } else if body.contains("Translate into Korean") {
            vec!["POL", "ITE: 감사", "합니다\nFORMAL: 감사합니", "다\nCASUAL: 고마워\nNOTE: Use CASUAL only with friends.\n"]
        } else {
            vec!["USAGE: Means to study.\nEXAM", "PLES:\n- 공부해요 — I study.\nNUANCE: Unlike 배우다, it is self-study.\n"]
        };
        MockResponse { status: 200, chunks: sse_answer(&answer), delay_ms: 1 }
    }

    async fn collect<F, Fut>(f: F) -> Vec<LlmChunk>
    where
        F: FnOnce(Vec<LlmChunk>) -> Fut,
        Fut: std::future::Future<Output = Vec<LlmChunk>>,
    {
        f(Vec::new()).await
    }

    #[test]
    fn parses_gloss_lines_in_several_shapes() {
        use GlossLine::Note;
        let g = |i, m: &str, n: Option<&str>| {
            GlossLine::Gloss(Gloss { index: i, meaning: m.into(), note: n.map(str::to_string) })
        };
        assert_eq!(parse_gloss_line("3|to study|-고 있다 = ongoing"), Some(g(3, "to study", Some("-고 있다 = ongoing"))));
        assert_eq!(parse_gloss_line("1|I|"), Some(g(1, "I", None)));
        assert_eq!(parse_gloss_line("- 2. Korean"), Some(g(2, "Korean", None)));
        assert_eq!(parse_gloss_line("**4|big**"), Some(g(4, "big", None)));
        assert_eq!(parse_gloss_line("NOTE|Hello there"), Some(Note("Hello there".into())));
        assert_eq!(parse_gloss_line("NOTE|"), None);
        assert_eq!(parse_gloss_line("Here are the glosses:"), None);
        assert_eq!(parse_gloss_line(""), None);
    }

    #[test]
    fn cleans_wrapping_quotes() {
        assert_eq!(clean_text("  \"Hello.\"\n"), "Hello.");
        assert_eq!(clean_text("“안녕하세요”"), "안녕하세요");
        assert_eq!(clean_text("He said \"hi\""), "He said \"hi\"");
    }

    #[test]
    fn label_lines_are_parsed_leniently() {
        let l = |s: &str| parse_label_line(s);
        assert_eq!(l("3|Inventory"), Some(LabelLine { index: 3, label: "Inventory".into() }));
        assert_eq!(l("  12. \"Save game\""), Some(LabelLine { index: 12, label: "Save game".into() }));
        assert_eq!(l("- **0:** OK"), Some(LabelLine { index: 0, label: "OK".into() }));
        assert_eq!(l("5|?"), Some(LabelLine { index: 5, label: "?".into() }));
        assert_eq!(l("Here are the labels:"), None);
        assert_eq!(l("4|"), None);
        assert_eq!(l(""), None);
    }

    #[tokio::test]
    async fn labels_stream_back_one_line_at_a_time_and_survive_chunking() {
        let mock = spawn_mock(|_, body| {
            assert!(body.contains("Strings:") && body.contains("0|확인"), "{body}");
            MockResponse { status: 200, chunks: sse_answer(&["0|O", "K\n1|Inven", "tory\n2|?", "\n"]), delay_ms: 1 }
        })
        .await;
        let items = [(0usize, "확인"), (1, "인벤토리"), (2, "ㅁㄴㅇ")];
        let mut got = Vec::new();
        client(&mock.url).labels(&LabelRequest { items: &items, ctx: &PromptContext::default() }, &mut |l| got.push(l)).await.unwrap();
        assert_eq!(got.iter().map(|l| (l.index, l.label.as_str())).collect::<Vec<_>>(), [(0, "OK"), (1, "Inventory"), (2, "?")]);
    }

    #[tokio::test]
    async fn ko_en_streams_and_finishes() {
        let server = spawn_mock(model).await;
        let c = client(&server.url);
        let ctx = PromptContext::default();
        let chunks = collect(|mut v| async move {
            let r = c.translate_ko_en(&KoEnRequest { text: "한국어를 공부해요", ctx: &ctx }, &mut |x| v.push(x)).await.unwrap();
            assert_eq!(r, "I'm studying Korean.");
            v
        })
        .await;
        assert_eq!(chunks.first(), Some(&LlmChunk::Text { variant: Variant::Primary, delta: "I'm ".into() }), "leading blank dropped");
        assert_eq!(chunks.last(), Some(&LlmChunk::TextDone { variant: Variant::Primary, text: "I'm studying Korean.".into() }));
        assert_eq!(chunks.len(), 4);
    }

    #[tokio::test]
    async fn glosses_arrive_line_by_line() {
        let server = spawn_mock(model).await;
        let c = client(&server.url);
        let ctx = PromptContext::default();
        let words = vec![GlossWord { index: 3, surface: "공부하고".into(), parts: "공부 + 하다 + 고".into(), candidates: vec![] }];
        let chunks = collect(|mut v| async move {
            c.glosses(&GlossRequest { sentence: "저는 한국어를 공부하고", words: &words, ctx: &ctx }, &mut |x| v.push(x)).await.unwrap();
            v
        })
        .await;
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0], LlmChunk::Gloss(Gloss { index: 1, meaning: "I".into(), note: None }));
        assert_eq!(
            chunks[2],
            LlmChunk::Gloss(Gloss { index: 3, meaning: "to study + progressive".into(), note: Some("-고 있다 = ongoing action".into()) })
        );
        assert_eq!(chunks[3], LlmChunk::Note("Topic marker 는 sets the subject.".into()));
    }

    #[tokio::test]
    async fn en_ko_streams_each_register_and_closes_them_in_order() {
        let server = spawn_mock(model).await;
        let c = client(&server.url);
        let ctx = PromptContext::default();
        let chunks = collect(|mut v| async move {
            c.translate_en_ko(&EnKoRequest { text: "Thank you", register: Register::Polite, ctx: &ctx }, &mut |x| v.push(x)).await.unwrap();
            v
        })
        .await;

        let done: Vec<_> = chunks.iter().filter_map(|c| match c {
            LlmChunk::TextDone { variant, text } => Some((*variant, text.as_str())),
            _ => None,
        }).collect();
        assert_eq!(done, [(Variant::Polite, "감사합니다"), (Variant::Formal, "감사합니다"), (Variant::Casual, "고마워")]);
        assert_eq!(chunks.last(), Some(&LlmChunk::Note("Use CASUAL only with friends.".into())));

        // streaming: the first Polite delta arrives before Formal has started
        let first_formal = chunks.iter().position(|c| matches!(c, LlmChunk::Text { variant: Variant::Formal, .. })).unwrap();
        let first_polite = chunks.iter().position(|c| matches!(c, LlmChunk::Text { variant: Variant::Polite, .. })).unwrap();
        assert!(first_polite < first_formal);
        let polite_text: String = chunks.iter().filter_map(|c| match c {
            LlmChunk::Text { variant: Variant::Polite, delta } => Some(delta.as_str()),
            _ => None,
        }).collect();
        assert_eq!(polite_text, "감사합니다");
    }

    #[tokio::test]
    async fn deep_dive_routes_sections() {
        let server = spawn_mock(model).await;
        let c = client(&server.url);
        let ctx = PromptContext::default();
        let chunks = collect(|mut v| async move {
            c.deep_dive(&DeepRequest { key: "공부하다", surface: "공부해요", sentence: "공부해요", dictionary: "verb: to study", ctx: &ctx }, &mut |x| v.push(x)).await.unwrap();
            v
        })
        .await;
        let text = |s: DeepSection| -> String {
            chunks.iter().filter_map(|c| match c {
                LlmChunk::Deep { section, delta } if *section == s => Some(delta.as_str()),
                _ => None,
            }).collect()
        };
        assert_eq!(text(DeepSection::Usage), "Means to study.\n");
        assert_eq!(text(DeepSection::Examples), "\n- 공부해요 — I study.\n");
        assert!(text(DeepSection::Nuance).starts_with("Unlike 배우다"));
    }

    #[tokio::test]
    async fn warm_up_sends_one_tiny_request_per_task() {
        let server = spawn_mock(model).await;
        client(&server.url).warm_up().await;
        let reqs = server.requests.lock().unwrap();
        assert_eq!(reqs.len(), 5);
        assert!(reqs.iter().all(|(_, b)| b.contains("\"max_tokens\":1")));
    }
}
