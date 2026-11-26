//! Word meanings for cards, in English and Spanish, from the learner's dictionary (`maru_dict::Nikl`, the
//! 한국어기초사전). A word with one sense takes both meanings from the dictionary and costs nothing; the words with
//! several senses, and those the dictionary lacks, share one model request per line, in which the model only names
//! a sense number (or, for a missing word, gives its own short meaning). Without the dictionary file nothing here
//! runs and cards are made as before.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use maru_core::{Eojeol, Pos};
use maru_dict::{Nikl, NiklStats};
use maru_llm::senses::{MAX_SENSES, MORE_SENSES, SenseAnswer, SenseOption, SenseRequest, SenseWord};
use serde::{Deserialize, Serialize};

use crate::engine::{Engine, EngineError, Inner, Result};

/// Where a card's meaning came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// A word of a line with what the dictionary says about it: what a model is asked to decide on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardWord {
    /// The word's index in the line.
    pub index: usize,
    /// As written in the line.
    pub surface: String,
    /// Dictionary form.
    pub lemma: String,
    pub pos: String,
    /// Every sense the dictionary has for it, in its order; empty when it lacks the word.
    pub senses: Vec<CardSense>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardSense {
    pub en: String,
    pub es: String,
    /// The dictionary's English definition, shown to the model beside the equivalent.
    pub definition: String,
}

/// The meaning decided for one word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decided {
    pub index: usize,
    pub en: String,
    pub es: String,
    pub source: MeaningSource,
}

/// The words a model has to be asked about: those with several senses (it names one) and those the dictionary
/// lacks (it gives its own short meaning). A word with one sense is never sent.
pub fn questions(words: &[CardWord]) -> Vec<SenseWord> {
    words
        .iter()
        .filter(|w| w.senses.len() != 1)
        .map(|w| SenseWord {
            index: w.index,
            surface: w.surface.clone(),
            lemma: w.lemma.clone(),
            senses: w.senses.iter().take(MAX_SENSES).map(|s| SenseOption { en: s.en.clone(), definition: s.definition.clone() }).collect(),
        })
        .collect()
}

/// Each word's meaning from the dictionary and the model's answers. A word with one sense takes it; an answer
/// naming a listed sense is taken; a missing or impossible answer leaves the first sense, to be checked; a word
/// the dictionary lacks and the model did not answer for is left out.
pub fn decide(words: &[CardWord], answers: &[SenseAnswer]) -> Vec<Decided> {
    resolve(words, answers, &[])
}

/// As [`decide`], with the answers of a second request (`later`) that showed the senses after the first
/// [`MAX_SENSES`] to the words for which the first answer was "none of these": a sense named there counts from
/// the ninth. A second "none of these" keeps the model's own words from the first answer.
pub fn resolve(words: &[CardWord], answers: &[SenseAnswer], later: &[SenseAnswer]) -> Vec<Decided> {
    words
        .iter()
        .filter_map(|w| {
            let of = |s: &CardSense, source| Decided { index: w.index, en: s.en.clone(), es: s.es.clone(), source };
            if let [only] = w.senses.as_slice() {
                return Some(of(only, MeaningSource::OneSense));
            }
            if let Some(SenseAnswer::Sense { sense, .. }) = later.iter().find(|a| a.index() == w.index)
                && let Some(found) = sense.checked_sub(1).and_then(|n| w.senses.get(MAX_SENSES + n)).filter(|_| *sense <= MORE_SENSES)
            {
                return Some(of(found, MeaningSource::Chosen));
            }
            match answers.iter().find(|a| a.index() == w.index) {
                Some(SenseAnswer::Sense { sense, .. }) if (1..=w.senses.len().min(MAX_SENSES)).contains(sense) => Some(of(&w.senses[sense - 1], MeaningSource::Chosen)),
                Some(SenseAnswer::Free { en, es, .. }) => Some(Decided { index: w.index, en: en.clone(), es: es.clone(), source: MeaningSource::Model }),
                _ => w.senses.first().map(|first| of(first, MeaningSource::FirstSense)),
            }
        })
        .collect()
}

/// The words to ask about again: those with more senses than the first request showed, for which the model said
/// none of the shown ones fits. Each comes with its next senses, numbered from 1.
pub fn second_questions(words: &[CardWord], answers: &[SenseAnswer]) -> Vec<SenseWord> {
    words
        .iter()
        .filter(|w| w.senses.len() > MAX_SENSES && matches!(answers.iter().find(|a| a.index() == w.index), Some(SenseAnswer::Free { .. })))
        .map(|w| SenseWord {
            index: w.index,
            surface: w.surface.clone(),
            lemma: w.lemma.clone(),
            senses: w.senses.iter().skip(MAX_SENSES).take(MORE_SENSES).map(|s| SenseOption { en: s.en.clone(), definition: s.definition.clone() }).collect(),
        })
        .collect()
}

/// What one model decided for the words of a line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Chosen {
    pub decided: Vec<Decided>,
    /// Words whose dictionary form the model says is another one: `(word index, form)`.
    pub corrected: Vec<(usize, String)>,
    /// Requests made: 0 when no word needed a model, 2 when a word with many senses was asked about again.
    pub requests: usize,
    /// Words that got a sense beyond the first eight in the second request.
    pub later: Vec<usize>,
}

/// Ask one model about a line's words as the cards sheet does: one request for the words with several senses or
/// none; then, for a word with more than eight senses for which it answered "none of these fits", a second
/// request with that word's next senses. `Ok(None)`: it answered nothing usable.
/// With `forms` the model may also correct a word's dictionary form (`Chosen::corrected`).
pub async fn choose(client: &maru_llm::LlmClient, sentence: &str, words: &[CardWord], forms: bool) -> maru_llm::Result<Option<Chosen>> {
    let asked = questions(words);
    if asked.is_empty() {
        return Ok(Some(Chosen { decided: decide(words, &[]), corrected: Vec::new(), requests: 0, later: Vec::new() }));
    }
    let (mut answers, mut corrected) = (Vec::new(), Vec::new());
    client.choose_senses_showing(&SenseRequest { sentence, words: &asked }, MAX_SENSES, forms, &mut |a| answers.push(a), &mut |index, form| corrected.push((index, form))).await?;
    if answers.is_empty() {
        return Ok(None);
    }
    let mut requests = 1;
    let mut later = Vec::new();
    // A word whose form the model corrected is another word: its remaining senses are not worth a second look.
    let again: Vec<SenseWord> = second_questions(words, &answers).into_iter().filter(|w| !corrected.iter().any(|(i, _)| *i == w.index)).collect();
    if !again.is_empty() {
        requests += 1;
        // The line is already answered: a failed second look only means the model's own words stay.
        if let Err(e) = client.choose_senses_showing(&SenseRequest { sentence, words: &again }, MORE_SENSES, false, &mut |a| later.push(a), &mut |_, _| {}).await {
            tracing::warn!("the second look at words with many senses failed: {e}");
            later.clear();
        }
    }
    let decided = resolve(words, &answers, &later);
    let took_later = later.iter().filter(|a| matches!(a, SenseAnswer::Sense { .. })).map(SenseAnswer::index).filter(|i| decided.iter().any(|d| d.index == *i && d.source == MeaningSource::Chosen)).collect();
    Ok(Some(Chosen { decided, corrected, requests, later: took_later }))
}

/// The senses a card can show for `key`: the dictionary's, with a bare romanization of the headword never taken
/// for a meaning (`romanized.rs`). A sense left without any English is dropped; a word left without senses is
/// then worded by a model, like a word the dictionary lacks.
pub(crate) fn card_senses(nikl: &Nikl, key: &str, pos: Pos) -> Vec<CardSense> {
    let senses = nikl.senses(key, pos).unwrap_or_else(|err| {
        tracing::warn!("learner's dictionary lookup of {key}: {err}");
        Vec::new()
    });
    senses
        .iter()
        .filter_map(|s| {
            let (en, _) = crate::romanized::usable_english(key, s);
            (!en.trim().is_empty()).then(|| CardSense { en, es: crate::romanized::usable_spanish(key, s), definition: s.en_definition.clone() })
        })
        .collect()
}

fn card_word(nikl: &Nikl, e: &Eojeol) -> CardWord {
    CardWord { index: e.index, surface: e.surface.clone(), lemma: e.vocab_key().to_string(), pos: e.pos.label().to_string(), senses: card_senses(nikl, e.vocab_key(), e.pos) }
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
    /// The dictionary form the model says the word really has in this line, when it is not the analyser's.
    pub corrected: Option<String>,
}

pub(crate) struct LineSenses {
    pub words: Vec<LineSense>,
    /// What to tell the person: a meaning that needs checking, a connection that did not answer.
    pub note: Option<String>,
    /// The connection that chose the senses; `None` when no model was needed, or none answered.
    pub answered: Option<crate::routes::Answered>,
    /// The connections that were asked before it and did not answer.
    pub attempts: Vec<crate::routes::Attempt>,
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

    /// The Spanish meaning of a word added from the panel. One dictionary sense: that one. Several senses and a
    /// sentence it was seen in: the card-meanings connection (then this PC's model) names the sense that fits, as
    /// on the cards sheet. `None` when the dictionary lacks the word or nobody could choose: the field stays empty
    /// rather than guessing.
    pub(crate) async fn sense_es_in(&self, key: &str, surface: &str, pos_label: &str, sentence: Option<&str>) -> Option<String> {
        let senses = card_senses(self.inner.nikl.get()?, key, pos_of(pos_label));
        let es_of = |s: &CardSense| Some(s.es.clone()).filter(|es| !es.trim().is_empty());
        if let [only] = senses.as_slice() {
            return es_of(only);
        }
        let sentence = sentence.map(str::trim).filter(|s| !s.is_empty())?;
        let routes = self.inner.routes();
        if senses.is_empty() || routes.card_meanings.is_empty() {
            return None;
        }
        let word = SenseWord {
            index: 0,
            surface: if surface.trim().is_empty() { key.to_string() } else { surface.to_string() },
            lemma: key.to_string(),
            senses: senses.iter().take(MAX_SENSES).map(|s| SenseOption { en: s.en.clone(), definition: s.definition.clone() }).collect(),
        };
        let words = [word];
        let request = SenseRequest { sentence, words: &words };
        let request = &request;
        let outcome = crate::routes::ask_in_order(&self.inner, routes.card_meanings.clone(), |client| async move {
            let mut chosen = None;
            client.choose_senses(request, &mut |a| if let SenseAnswer::Sense { sense, .. } = a { chosen = Some(sense) }).await?;
            Ok(chosen)
        })
        .await;
        outcome.value.and_then(|n| senses.get(n.checked_sub(1)?)).and_then(es_of)
    }

    /// The words to learn in a line with their dictionary senses, as the cards sheet finds them (no model).
    pub fn card_words(&self, line: &str) -> Result<Vec<CardWord>> {
        let nikl = self.inner.nikl.get().ok_or_else(|| EngineError::Other("the learner's dictionary is not installed (maru dict nikl <file>)".into()))?;
        let mut seen = HashSet::new();
        Ok(self
            .inner
            .analyzer
            .analyze(line.trim())
            .eojeols
            .iter()
            .filter(|e| e.content && seen.insert(e.vocab_key().to_string()))
            .map(|e| card_word(nikl, e))
            .collect())
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
                let senses = card_senses(nikl, e.vocab_key(), e.pos).len();
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

/// The model the cards sheet may ask, or why there is none. Only for an engine built without task routes
/// (`routes.rs`): the capture model, as before `[tasks]`.
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

/// Each word's meaning in two languages. Words with several senses, and words the dictionary lacks, are put to
/// the card-meanings connection, then to this PC's model (`routes.rs`); `ask` narrows that to one connection.
/// Words the dictionary lacks and no model answered for are left out: the caller falls back to what it had before.
pub(crate) async fn line_senses(inner: &Inner, nikl: &Nikl, sentence: &str, eojeols: &[Eojeol], ask: &crate::routes::CardAsk) -> LineSenses {
    let cards: Vec<CardWord> = eojeols.iter().map(|e| card_word(nikl, e)).collect();
    let mut chosen: Option<Chosen> = None;
    let forms = inner.cfg.learning.anki.correct_forms;
    let mut failure = None;
    let (mut answered, mut attempts) = (None, Vec::new());
    if !questions(&cards).is_empty() {
        let routes = inner.routes();
        let cards = &cards;
        if routes.card_meanings.is_empty() && ask.connection.is_none() {
            match sheet_model(inner).await {
                Err(why) => failure = Some(why),
                Ok(llm) => match choose(&llm, sentence, cards, forms).await {
                    Ok(found) => chosen = found,
                    Err(e) => failure = Some(e.to_string()),
                },
            }
        } else {
            let order = crate::routes::candidates(&routes, &routes.card_meanings, ask);
            let outcome = crate::routes::ask_in_order(inner, order, |client| async move { choose(&client, sentence, cards, forms).await }).await;
            failure = outcome.note("The dictionary was used.");
            chosen = outcome.value;
            answered = outcome.answered;
            attempts = outcome.attempts;
        }
    }
    let corrected = chosen.as_ref().map(|c| c.corrected.clone()).unwrap_or_default();
    let decided = chosen.map(|c| c.decided).unwrap_or_else(|| decide(&cards, &[]));
    // A first sense nobody chose, or a word left without any meaning, has to be looked at.
    let unchecked = decided.iter().any(|d| d.source == MeaningSource::FirstSense) || decided.len() < cards.len();
    let words = decided
        .into_iter()
        .map(|d| {
            let options = cards.iter().find(|c| c.index == d.index).map(|c| c.senses.iter().map(|s| SenseChoice { en: s.en.clone(), es: s.es.clone() }).collect()).unwrap_or_default();
            let corrected = corrected.iter().find(|(i, _)| *i == d.index).map(|(_, form)| form.clone());
            LineSense { index: d.index, en: d.en, es: d.es, source: d.source, options, corrected }
        })
        .collect();
    let routed = answered.is_some() || !attempts.is_empty();
    let note = match (unchecked, routed) {
        // A connection failed and another answered everything: say so, nothing needs checking.
        (false, true) => failure,
        (false, false) => None,
        (true, true) => {
            let base = "The dictionary's first sense is shown where no model chose one: check that each fits.";
            Some(failure.map_or(base.to_string(), |why| format!("{why} {base}")))
        }
        (true, false) => {
            let base = "Some meanings could not be chosen for this line: the dictionary's first sense is shown for them; check that each fits.";
            Some(failure.map_or(base.to_string(), |why| format!("{base} ({why})")))
        }
    };
    LineSenses { words, note, answered, attempts }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(index: usize, lemma: &str, senses: &[(&str, &str)]) -> CardWord {
        CardWord { index, surface: lemma.into(), lemma: lemma.into(), pos: "noun".into(), senses: senses.iter().map(|(en, es)| CardSense { en: (*en).into(), es: (*es).into(), definition: String::new() }).collect() }
    }

    fn line() -> Vec<CardWord> {
        vec![word(0, "배", &[("belly", "barriga"), ("ship", "barco"), ("pear", "pera")]), word(1, "학교", &[("school", "escuela")]), word(2, "억수로", &[])]
    }

    #[test]
    fn only_words_with_several_senses_or_none_are_put_to_a_model() {
        let asked = questions(&line());
        assert_eq!(asked.iter().map(|w| (w.index, w.senses.len())).collect::<Vec<_>>(), [(0, 3), (2, 0)]);
        let many: Vec<(String, String)> = (0..20).map(|n| (format!("sense {n}"), String::new())).collect();
        let many: Vec<(&str, &str)> = many.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        assert_eq!(questions(&[word(0, "하다", &many)])[0].senses.len(), MAX_SENSES, "a long list is cut for the request");
    }

    #[test]
    fn each_word_takes_its_only_sense_the_chosen_one_the_models_words_or_the_first_sense() {
        let answers = [SenseAnswer::Sense { index: 0, sense: 2 }, SenseAnswer::Free { index: 2, en: "extremely".into(), es: "muchísimo".into() }];
        let got = decide(&line(), &answers);
        let of = |i: usize| got.iter().find(|d| d.index == i).map(|d| (d.en.as_str(), d.es.as_str(), d.source));
        assert_eq!(of(0), Some(("ship", "barco", MeaningSource::Chosen)));
        assert_eq!(of(1), Some(("school", "escuela", MeaningSource::OneSense)));
        assert_eq!(of(2), Some(("extremely", "muchísimo", MeaningSource::Model)));

        // No answers, an impossible sense number, and a sense beyond what was shown to the model.
        for answers in [vec![], vec![SenseAnswer::Sense { index: 0, sense: 4 }], vec![SenseAnswer::Sense { index: 0, sense: 0 }]] {
            let got = decide(&line(), &answers);
            assert_eq!(got.iter().map(|d| (d.index, d.source)).collect::<Vec<_>>(), [(0, MeaningSource::FirstSense), (1, MeaningSource::OneSense)], "{answers:?}: the missing word is left out");
            assert_eq!(got[0].en, "belly");
        }
    }

    fn long_word(index: usize, lemma: &str, n: usize) -> CardWord {
        CardWord { index, surface: lemma.into(), lemma: lemma.into(), pos: "verb".into(), senses: (1..=n).map(|k| CardSense { en: format!("sense {k}"), es: format!("acepción {k}"), definition: String::new() }).collect() }
    }

    #[test]
    fn a_word_with_more_senses_than_were_shown_is_asked_about_again_only_when_none_of_them_fits() {
        let words = [long_word(0, "하다", 20), word(1, "배", &[("belly", "barriga"), ("ship", "barco")]), long_word(2, "가다", 30)];
        let none_fits = [SenseAnswer::Free { index: 0, en: "to do".into(), es: "hacer".into() }, SenseAnswer::Sense { index: 1, sense: 2 }, SenseAnswer::Sense { index: 2, sense: 4 }];
        let again = second_questions(&words, &none_fits);
        assert_eq!(again.iter().map(|w| w.index).collect::<Vec<_>>(), [0], "a word that got one of the shown senses is not asked about again");
        assert_eq!((again[0].senses.len(), again[0].senses[0].en.as_str()), (12, "sense 9"), "the next senses, from the ninth");
        assert_eq!(second_questions(&[long_word(0, "가다", 30)], &[SenseAnswer::Free { index: 0, en: "x".into(), es: String::new() }])[0].senses.len(), MORE_SENSES, "a very long list is cut again");
        // A word the dictionary lacks answered in its own words is not a long word.
        assert!(second_questions(&[word(0, "억수로", &[])], &[SenseAnswer::Free { index: 0, en: "very".into(), es: "muy".into() }]).is_empty());
    }

    #[test]
    fn the_second_answer_counts_from_the_ninth_sense_and_a_second_none_keeps_the_models_own_words() {
        let words = [long_word(0, "하다", 20)];
        let first = [SenseAnswer::Free { index: 0, en: "to do".into(), es: "hacer".into() }];
        let got = resolve(&words, &first, &[SenseAnswer::Sense { index: 0, sense: 3 }]);
        assert_eq!((got[0].en.as_str(), got[0].es.as_str(), got[0].source), ("sense 11", "acepción 11", MeaningSource::Chosen));
        for later in [vec![], vec![SenseAnswer::Free { index: 0, en: "to make".into(), es: "hacer".into() }], vec![SenseAnswer::Sense { index: 0, sense: 13 }], vec![SenseAnswer::Sense { index: 0, sense: 0 }]] {
            let got = resolve(&words, &first, &later);
            assert_eq!((got[0].en.as_str(), got[0].source), ("to do", MeaningSource::Model), "{later:?}");
        }
        assert_eq!(resolve(&words, &first, &[]), decide(&words, &first), "without a second request nothing changes");
    }
}
