//! Word meanings for cards, in English and Spanish, from the learner's dictionary (`maru_dict::Nikl`, the
//! 한국어기초사전). A word with one sense takes both meanings from the dictionary and costs nothing; the words with
//! several senses, and those the dictionary lacks, share one model request per line, in which the model only names
//! a sense number (or, for a missing word, gives its own short meaning). Without the dictionary file nothing here
//! runs and cards are made as before.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use maru_core::{Eojeol, Pos};
use maru_dict::{Nikl, NiklSense, NiklStats};
use maru_llm::senses::{MAX_SENSES, SenseAnswer, SenseOption, SenseRequest, SenseWord};
use serde::Serialize;

use crate::engine::{Engine, EngineError, Inner, Result};

/// Where a card's meaning came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeaningSource {
    /// The dictionary's only sense: no model was asked.
    OneSense,
    /// A model chose among the dictionary's senses.
    Chosen,
    /// Several senses and no usable answer: the dictionary's first one, to be checked.
    FirstSense,
    /// Not in the dictionary: the model's own words.
    Model,
}

/// A dictionary sense the sheet can offer instead of the one shown.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SenseChoice {
    pub en: String,
    pub es: String,
}

/// The meaning of one word of the line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LineSense {
    /// The word's index in the line.
    pub index: usize,
    pub en: String,
    pub es: String,
    pub source: MeaningSource,
    /// Every sense the dictionary has for it, in its order.
    pub options: Vec<SenseChoice>,
}

pub(crate) struct LineSenses {
    pub words: Vec<LineSense>,
    /// What to tell the person when a meaning needs checking.
    pub note: Option<String>,
}

/// How much of some lines' vocabulary the dictionary covers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Coverage {
    pub lines: usize,
    /// Words to learn, counted once per line.
    pub words: usize,
    /// … that the dictionary has.
    pub found: usize,
    /// … with exactly one sense: both meanings without a model.
    pub one_sense: usize,
    /// Different words over all lines, and the same two counts for them.
    pub distinct: usize,
    pub distinct_found: usize,
    pub distinct_one_sense: usize,
    /// Lines whose every word has one sense: a sheet that asks no model at all.
    pub lines_without_model: usize,
}

const CREDIT: &str = "Meanings: 한국어기초사전 (국립국어원, krdict.korean.go.kr), CC BY-SA 2.0 KR";

fn choice(sense: &NiklSense) -> SenseChoice {
    SenseChoice { en: sense.en.clone(), es: sense.es.clone() }
}

fn pos_of(label: &str) -> Pos {
    [Pos::Noun, Pos::Pronoun, Pos::Numeral, Pos::Verb, Pos::Adjective, Pos::Adverb, Pos::Determiner, Pos::Interjection]
        .into_iter()
        .find(|p| p.label() == label)
        .unwrap_or(Pos::Other)
}

impl Engine {
    /// Use the learner's dictionary at `path` for card meanings; `None` when there is no such file or it is empty.
    pub fn open_learner_dictionary(&self, path: &Path) -> Result<Option<NiklStats>> {
        if !path.exists() {
            return Ok(None);
        }
        let nikl = Nikl::open(path).map_err(|e| EngineError::Other(format!("the learner's dictionary could not be opened: {e}")))?;
        let stats = nikl.stats().map_err(|e| EngineError::Other(format!("the learner's dictionary could not be read: {e}")))?;
        if stats.words == 0 {
            return Ok(None);
        }
        self.set_learner_dictionary(Arc::new(nikl));
        Ok(Some(stats))
    }

    /// As [`Engine::open_learner_dictionary`], for a dictionary already open. The first one set stays.
    pub fn set_learner_dictionary(&self, nikl: Arc<Nikl>) {
        let _ = self.inner.nikl.set(nikl);
    }

    /// The attribution to show where its meanings are shown; `None` without the dictionary.
    pub fn learner_dictionary_credit(&self) -> Option<&'static str> {
        self.inner.nikl.get().map(|_| CREDIT)
    }

    /// The Spanish meaning of a word that has exactly one dictionary sense (so no model is needed to pick it).
    pub(crate) fn only_sense_es(&self, key: &str, pos_label: &str) -> Option<String> {
        let senses = self.inner.nikl.get()?.senses(key, pos_of(pos_label)).ok()?;
        match senses.as_slice() {
            [only] if !only.es.trim().is_empty() => Some(only.es.clone()),
            _ => None,
        }
    }

    /// How much of these lines the learner's dictionary covers, with the same analysis the cards sheet uses.
    pub fn sense_coverage(&self, lines: &[String]) -> Result<Coverage> {
        let nikl = self.inner.nikl.get().ok_or_else(|| EngineError::Other("the learner's dictionary is not installed (maru dict nikl <file>)".into()))?;
        let mut out = Coverage::default();
        let mut distinct: HashSet<String> = HashSet::new();
        for line in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
            out.lines += 1;
            let mut seen = HashSet::new();
            let mut every_word_has_one = true;
            for e in self.inner.analyzer.analyze(line).eojeols.iter().filter(|e| e.content) {
                if !seen.insert(e.vocab_key().to_string()) {
                    continue;
                }
                let senses = nikl.senses(e.vocab_key(), e.pos).map(|s| s.len()).unwrap_or(0);
                let new = distinct.insert(e.vocab_key().to_string());
                out.words += 1;
                out.distinct += usize::from(new);
                if senses >= 1 {
                    out.found += 1;
                    out.distinct_found += usize::from(new);
                }
                if senses == 1 {
                    out.one_sense += 1;
                    out.distinct_one_sense += usize::from(new);
                } else {
                    every_word_has_one = false;
                }
            }
            if every_word_has_one {
                out.lines_without_model += 1;
            }
        }
        Ok(out)
    }
}

/// The model the cards sheet may ask, or why there is none.
async fn sheet_model(inner: &Inner) -> std::result::Result<maru_llm::LlmClient, String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let em = crate::pipeline::Emitter::new(tx, 0);
    let live = inner.live_active.load(std::sync::atomic::Ordering::Acquire);
    match crate::pipeline::snapshot_model(inner, &em, live).await {
        Some((llm, false)) => Ok(llm),
        Some((_, true)) => Err("the live translation model gives no word meanings; turn live subtitles off for it".into()),
        None => Err(std::iter::from_fn(|| rx.try_recv().ok())
            .find_map(|e| match e {
                maru_core::Event::Error { message, .. } => Some(message),
                _ => None,
            })
            .unwrap_or_else(|| "no model is running".into())),
    }
}

/// Each word's meaning in two languages. Words the dictionary lacks and no model answered for are left out: the
/// caller falls back to what it had before.
pub(crate) async fn line_senses(inner: &Inner, nikl: &Nikl, sentence: &str, eojeols: &[Eojeol]) -> LineSenses {
    let mut words = Vec::new();
    // (word, its dictionary senses) still to be decided.
    let mut open: Vec<(SenseWord, Vec<NiklSense>)> = Vec::new();
    for e in eojeols {
        let senses = nikl.senses(e.vocab_key(), e.pos).unwrap_or_else(|err| {
            tracing::warn!("learner's dictionary lookup of {}: {err}", e.vocab_key());
            Vec::new()
        });
        if let [only] = senses.as_slice() {
            words.push(LineSense { index: e.index, en: only.en.clone(), es: only.es.clone(), source: MeaningSource::OneSense, options: vec![choice(only)] });
            continue;
        }
        let asked = SenseWord {
            index: e.index,
            surface: e.surface.clone(),
            lemma: e.vocab_key().to_string(),
            senses: senses.iter().take(MAX_SENSES).map(|s| SenseOption { en: s.en.clone(), definition: s.en_definition.clone() }).collect(),
        };
        open.push((asked, senses));
    }
    if open.is_empty() {
        return LineSenses { words, note: None };
    }

    let mut answers: Vec<SenseAnswer> = Vec::new();
    let failure = match sheet_model(inner).await {
        Err(why) => Some(why),
        Ok(llm) => {
            let asked: Vec<SenseWord> = open.iter().map(|(w, _)| w.clone()).collect();
            let result = llm.choose_senses(&SenseRequest { sentence, words: &asked }, &mut |a| answers.push(a)).await;
            result.err().map(|e| e.to_string())
        }
    };

    let mut unchecked = false;
    for (asked, senses) in &open {
        let answer = answers.iter().find(|a| a.index() == asked.index);
        let options: Vec<SenseChoice> = senses.iter().map(choice).collect();
        let picked = match answer {
            Some(SenseAnswer::Sense { sense, .. }) => senses.get(sense.wrapping_sub(1)).filter(|_| *sense <= asked.senses.len()).map(|s| (s.en.clone(), s.es.clone(), MeaningSource::Chosen)),
            Some(SenseAnswer::Free { en, es, .. }) => Some((en.clone(), es.clone(), MeaningSource::Model)),
            None => None,
        };
        match (picked, senses.first()) {
            (Some((en, es, source)), _) => words.push(LineSense { index: asked.index, en, es, source, options }),
            (None, Some(first)) => {
                unchecked = true;
                words.push(LineSense { index: asked.index, en: first.en.clone(), es: first.es.clone(), source: MeaningSource::FirstSense, options });
            }
            // Not in the dictionary and no answer: the caller's own fallback.
            (None, None) => unchecked = true,
        }
    }
    let note = unchecked.then(|| {
        let base = "Some meanings could not be chosen for this line: the dictionary's first sense is shown for them; check that each fits.";
        failure.map_or(base.to_string(), |why| format!("{base} ({why})"))
    });
    LineSenses { words, note }
}
