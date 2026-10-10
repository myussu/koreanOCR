//! Sejong/Kiwi tag semantics: role, part of speech and learner-friendly labels.

use maru_core::{Pos, Role};

/// Kiwi marks irregular/regular stems as `VV-I` / `VV-R`; we only need the base.
pub fn base_tag(tag: &str) -> &str {
    tag.split('-').next().unwrap_or(tag)
}

pub fn role_of(tag: &str) -> Role {
    match base_tag(tag) {
        "NNG" | "NNP" | "NNB" | "NR" | "NP" | "VV" | "VA" | "VX" | "MM" | "MAG" | "MAJ" | "IC" | "XR" => Role::Stem,
        "VCP" | "VCN" | "XPN" | "XSN" | "XSV" | "XSA" => Role::Affix,
        "JKS" | "JKC" | "JKG" | "JKO" | "JKB" | "JKV" | "JKQ" | "JX" | "JC" => Role::Particle,
        "EP" | "EF" | "EC" | "ETN" | "ETM" => Role::Ending,
        "SF" | "SP" | "SS" | "SSO" | "SSC" | "SE" | "SO" | "SW" => Role::Symbol,
        _ => Role::Other,
    }
}

pub fn pos_of(tag: &str) -> Pos {
    match base_tag(tag) {
        "NNG" | "NNP" | "NNB" | "XR" => Pos::Noun,
        "NP" => Pos::Pronoun,
        "NR" => Pos::Numeral,
        "VV" | "VX" | "XSV" => Pos::Verb,
        "VA" | "XSA" => Pos::Adjective,
        "MAG" | "MAJ" => Pos::Adverb,
        "MM" => Pos::Determiner,
        "IC" => Pos::Interjection,
        "JKS" | "JKC" | "JKG" | "JKO" | "JKB" | "JKV" | "JKQ" | "JX" | "JC" => Pos::Particle,
        "SF" | "SP" | "SS" | "SSO" | "SSC" | "SE" | "SO" | "SW" => Pos::Punctuation,
        "SL" | "SH" | "SN" => Pos::Foreign,
        _ => Pos::Other,
    }
}

/// Can a word (어절) end with a morpheme of this tag? Particles, endings, numbers, foreign
/// words, symbols and free-standing adverbs/determiners/interjections can; a noun, verb stem
/// or suffix cannot, which is what keeps compound nouns from being split.
pub fn can_end_word(tag: &str) -> bool {
    matches!(
        base_tag(tag),
        "JKS" | "JKC" | "JKG" | "JKO" | "JKB" | "JKV" | "JKQ" | "JX" | "JC"
            | "EP" | "EF" | "EC" | "ETN" | "ETM"
            | "SN" | "SL" | "SH" | "SF" | "SP" | "SS" | "SSO" | "SSC" | "SE" | "SO" | "SW"
            | "MAG" | "MAJ" | "MM" | "IC"
    )
}

/// Short English description of a morpheme, refined by its surface form where
/// the tag alone is too coarse (은/는 vs 도 vs 만 are all `JX`).
pub fn describe(form: &str, tag: &str) -> String {
    let base = base_tag(tag);
    let specific = match base {
        "JKS" => match form {
            "께서" => Some("honorific subject particle"),
            _ => Some("subject particle (marks who/what does it)"),
        },
        "JKC" => Some("complement particle (marks what something becomes / is not)"),
        "JKG" => Some("possessive particle ('s / of)"),
        "JKO" => Some("object particle (marks what is acted on)"),
        "JKV" => Some("vocative particle (calling someone)"),
        "JKQ" => Some("quotation particle ('that…', reported speech)"),
        "JC" => match form {
            "와" | "과" | "하고" => Some("'and / with' particle"),
            "랑" | "이랑" => Some("'and / with' particle (casual)"),
            _ => Some("connecting particle ('and')"),
        },
        "JKB" => match form {
            "에" => Some("'at / to / in / on' (place, time, direction)"),
            "에서" => Some("'at / from' (where an action happens, or origin)"),
            "에게" | "한테" => Some("'to (a person)'"),
            "께" => Some("'to (a person)', honorific"),
            "에게서" | "한테서" => Some("'from (a person)'"),
            "로" | "으로" => Some("'by / to / as' (means, direction, role)"),
            "보다" => Some("'than' (comparison)"),
            "처럼" => Some("'like / as'"),
            "만큼" => Some("'as much as'"),
            "같이" | "와 같이" => Some("'like / together with'"),
            "부터" => Some("'from / starting at'"),
            "까지" => Some("'until / up to'"),
            "라고" | "이라고" | "고" => Some("quotation particle"),
            _ => Some("adverbial particle (adds place / time / manner)"),
        },
        "JX" => match form {
            "은" | "는" | "ㄴ" => Some("topic particle (marks what the sentence is about / contrast)"),
            "도" => Some("'also / even' particle"),
            "만" => Some("'only' particle"),
            "부터" => Some("'from / starting at' particle"),
            "까지" => Some("'until / even' particle"),
            "마다" => Some("'every / each' particle"),
            "조차" => Some("'even (the unexpected)' particle"),
            "밖에" => Some("'nothing but / only' (used with negatives)"),
            "이나" | "나" => Some("'or / as many as' particle"),
            "이라도" | "라도" => Some("'even if / at least' particle"),
            "이야" | "야" => Some("'as for / indeed' emphasis particle"),
            "요" => Some("polite-ending particle (요)"),
            "씩" => Some("'each / per' particle"),
            "들" => Some("plural marker"),
            _ => Some("auxiliary particle (adds nuance)"),
        },
        "EP" => match form {
            "았" | "었" | "였" | "ㅆ" => Some("past tense"),
            "았었" | "었었" => Some("past perfect ('used to')"),
            "겠" => Some("future / assumption / polite intent"),
            "시" | "으시" => Some("honorific (respect for the subject)"),
            _ => Some("pre-final ending (tense / honorific)"),
        },
        "EF" => match form {
            "어요" | "아요" | "에요" | "예요" | "여요" | "이에요" | "해요" => Some("polite ending (해요체)"),
            "ㅂ니다" | "습니다" | "ㅂ니까" | "습니까" => Some("formal ending (합쇼체)"),
            "세요" | "으세요" => Some("polite honorific ending / request"),
            "다" | "ㄴ다" | "는다" | "ㄴ다는" => Some("plain declarative ending (written / narrative)"),
            "어" | "아" | "야" | "여" => Some("casual ending (반말)"),
            "지" => Some("casual ending: 'right?' / softened statement"),
            "죠" | "지요" => Some("polite 'right?' / softened statement"),
            "네요" | "네" => Some("'oh, I see / how…' (new realisation)"),
            "군요" | "는군요" | "구나" => Some("exclamation of realisation"),
            "ㄹ까요" | "을까요" | "ㄹ까" | "을까" => Some("'shall we? / I wonder if…?'"),
            "거든요" | "거든" => Some("'you see… / because' (explanation)"),
            "는데요" | "은데요" | "ㄴ데요" | "는데" | "은데" | "ㄴ데" => Some("softened statement / background"),
            "니" | "냐" | "니까" => Some("question ending (casual)"),
            "자" | "ㅂ시다" | "읍시다" => Some("'let's…' (suggestion)"),
            "라" | "어라" | "아라" => Some("command ending"),
            "래요" | "대요" | "래" | "대" => Some("'I heard that…' (reported speech)"),
            "ㄹ게요" | "을게요" | "ㄹ게" | "을게" => Some("'I will… (for you)' promise"),
            "ㄹ래요" | "을래요" | "ㄹ래" | "을래" => Some("'I want to… / do you want to…?'"),
            _ => Some("sentence-final ending"),
        },
        "EC" => match form {
            "고" => Some("'and / and then' (connects actions)"),
            "어서" | "아서" | "여서" | "서" => Some("'because / so / and then' (cause or sequence)"),
            "지만" => Some("'but / although'"),
            "는데" | "은데" | "ㄴ데" => Some("'and / but' (background or contrast)"),
            "면" | "으면" => Some("'if / when'"),
            "니까" | "으니까" => Some("'because / since' (reason, esp. before commands)"),
            "려고" | "으려고" => Some("'in order to / intending to'"),
            "러" | "으러" => Some("'in order to (go/come to)'"),
            "게" => Some("'so that / -ly' (turns a verb into an adverb)"),
            "며" | "으며" => Some("'while / and' (simultaneous)"),
            "다가" => Some("'while doing… (then switch)'"),
            "도록" => Some("'so that / until'"),
            "거나" => Some("'or' (between verbs)"),
            "든지" | "더라도" => Some("'whether / even if'"),
            "어도" | "아도" | "여도" => Some("'even if / even though'"),
            "어야" | "아야" | "여야" => Some("'only if / must'"),
            "자마자" => Some("'as soon as'"),
            "지" => Some("negation / contrast connective"),
            _ => Some("connective ending (links clauses)"),
        },
        "ETM" => match form {
            "ㄴ" | "은" => Some("modifier ending: past / adjective ('who did / that is…')"),
            "는" => Some("modifier ending: present ('who does…')"),
            "ㄹ" | "을" => Some("modifier ending: future / not yet happened ('who will…')"),
            "던" => Some("modifier ending: remembered / unfinished past"),
            _ => Some("modifier ending (turns a verb into an adjective)"),
        },
        "ETN" => match form {
            "기" => Some("nominalizer (-ing / the act of)"),
            "ㅁ" | "음" => Some("nominalizer (turns a verb into a noun)"),
            _ => Some("nominalizer"),
        },
        "XSV" => match form {
            "하" => Some("'do' (turns a noun into a verb)"),
            "되" => Some("'become / be done' (passive)"),
            "시키" => Some("'make do' (causative)"),
            _ => Some("verb-forming suffix"),
        },
        "XSA" => match form {
            "하" => Some("'be -ful / be like' (turns a noun into an adjective)"),
            "스럽" | "롭" | "답" => Some("adjective-forming suffix ('-ful / -ish / worthy of')"),
            "되" => Some("adjective-forming suffix"),
            _ => Some("adjective-forming suffix"),
        },
        "XSN" => match form {
            "들" => Some("plural marker"),
            "님" => Some("honorific 'Mr/Ms/Sir'"),
            "씨" => Some("'Mr/Ms' (polite)"),
            "적" => Some("'-ic / -al' suffix"),
            "성" => Some("'-ness / -ity' suffix"),
            "화" => Some("'-ization' suffix"),
            "자" | "가" => Some("'person who…' suffix"),
            _ => Some("noun-forming suffix"),
        },
        "XPN" => Some("prefix"),
        "VCP" => Some("copula 이다 ('to be')"),
        "VCN" => Some("negative copula 아니다 ('to not be')"),
        "VX" => Some("auxiliary verb (adds meaning to the verb before it)"),
        _ => None,
    };
    if let Some(label) = specific {
        return label.to_string();
    }
    match base {
        "NNG" => "noun",
        "NNP" => "proper noun",
        "NNB" => "bound noun (needs a modifier)",
        "NR" => "numeral",
        "NP" => "pronoun",
        "VV" => "verb",
        "VA" => "adjective (descriptive verb)",
        "MM" => "determiner (modifies a noun)",
        "MAG" => "adverb",
        "MAJ" => "conjunctive adverb",
        "IC" => "interjection",
        "XR" => "root",
        "SF" => "sentence-ending mark",
        "SP" => "pause mark",
        "SS" | "SSO" | "SSC" => "quote / bracket",
        "SE" => "ellipsis",
        "SO" | "SW" => "symbol",
        "SN" => "number",
        "SL" => "foreign word",
        "SH" => "Hanja (Chinese character)",
        _ => "unknown",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn irregular_suffixes_are_stripped() {
        assert_eq!(base_tag("VV-I"), "VV");
        assert_eq!(base_tag("VA-R"), "VA");
        assert_eq!(base_tag("NNG"), "NNG");
    }

    #[test]
    fn roles_and_pos() {
        assert_eq!(role_of("NNG"), Role::Stem);
        assert_eq!(role_of("JKS"), Role::Particle);
        assert_eq!(role_of("EF"), Role::Ending);
        assert_eq!(role_of("XSV"), Role::Affix);
        assert_eq!(role_of("SF"), Role::Symbol);
        assert_eq!(role_of("SN"), Role::Other);
        assert_eq!(pos_of("VV-I"), Pos::Verb);
        assert_eq!(pos_of("MAG"), Pos::Adverb);
    }

    #[test]
    fn only_function_morphemes_and_free_words_can_end_a_word() {
        for t in ["JX", "JKB", "EC", "EF", "ETM", "SN", "SL", "MAG", "SF"] {
            assert!(can_end_word(t), "{t}");
        }
        for t in ["NNG", "NNP", "NNB", "VV", "VA", "XSN", "XPN", "XR", "NR"] {
            assert!(!can_end_word(t), "{t}");
        }
        assert!(can_end_word("EC-I"));
    }

    #[test]
    fn descriptions_depend_on_form() {
        assert!(describe("은", "JX").contains("topic"));
        assert!(describe("도", "JX").contains("also"));
        assert!(describe("었", "EP").contains("past"));
        assert!(describe("ㅂ니다", "EF").contains("formal"));
        assert!(describe("에서", "JKB").contains("from"));
        assert_eq!(describe("학교", "NNG"), "noun");
    }
}
