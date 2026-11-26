"""Two questions about the learner's dictionary (`nikl.db`), answered by counting. Nothing is written to it.

1. `romanized`: some English equivalents are only the headword in Latin letters (녀석 → "nyeoseok"), which is no
   meaning for a card. How many, in which parts of speech, and what is left to make a meaning from.
2. `senses`: the model that chooses a sense sees at most 8 of them (`MAX_SENSES` in maru-llm). How many words have
   more, and, with the marks of a card review, how its mistakes fall on those words.

  python tools/nikl_stats.py romanized nikl.db [--examples 30] [--json out.json]
  python tools/nikl_stats.py senses nikl.db [--json out.json]
         [--tasks tasks.jsonl [--key key.json --marks marks.json --candidate local] [--cards cards-a.jsonl …]]

The romanizer is a port of `crates/maru-lang/src/pronounce.rs`: `romanize(word)` gives what the app's
`romanize_word` gives, so the rule can be written again in Rust with what is already there.

The rule, per sense. The English equivalent is split at ";". A part is a romanization when, in lower case and with
everything but letters removed, it equals one of the headword's spellings in Latin letters (`spellings`). Then:
  * no part is one                      → translated
  * some parts are                      → mixed: the other parts are still a meaning
  * every part is, and
      the headword is a borrowed word whose source is that English word (`BORROWED`) → borrowed: a real word
      the definition does not start with a gloss and the part has a capital letter   → name (서울 → "Seoul")
      otherwise                                                                     → romanized: nothing usable
  * particles, endings and affixes (조사, 어미, 접사) are never translated by the dictionary, so their equivalent
    counts as romanized even where the dictionary's own spelling is irregular ("-ulji" for 을지).
Only an exact match with the headword's own spelling counts, so "camera" for 카메라 (kamera) is never taken for one.
"""
import argparse
import json
import re
import sqlite3
import sys
import unicodedata
from collections import Counter
from pathlib import Path

# ───────────────────────── the app's romanizer (pronounce.rs) ─────────────────────────

INITIALS = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ"
VOWELS = "ㅏㅐㅑㅒㅓㅔㅕㅖㅗㅘㅙㅚㅛㅜㅝㅞㅟㅠㅡㅢㅣ"
FINALS = ["", "ㄱ", "ㄲ", "ㄳ", "ㄴ", "ㄵ", "ㄶ", "ㄷ", "ㄹ", "ㄺ", "ㄻ", "ㄼ", "ㄽ", "ㄾ", "ㄿ", "ㅀ", "ㅁ", "ㅂ", "ㅄ", "ㅅ", "ㅆ", "ㅇ", "ㅈ", "ㅊ", "ㅋ", "ㅌ", "ㅍ", "ㅎ"]
CLUSTERS = {"ㄳ": "ㄱㅅ", "ㄵ": "ㄴㅈ", "ㄶ": "ㄴㅎ", "ㄺ": "ㄹㄱ", "ㄻ": "ㄹㅁ", "ㄼ": "ㄹㅂ", "ㄽ": "ㄹㅅ", "ㄾ": "ㄹㅌ", "ㄿ": "ㄹㅍ", "ㅀ": "ㄹㅎ", "ㅄ": "ㅂㅅ"}
NEUTRAL = {"ㄲ": "ㄱ", "ㅋ": "ㄱ", "ㅅ": "ㄷ", "ㅆ": "ㄷ", "ㅈ": "ㄷ", "ㅊ": "ㄷ", "ㅌ": "ㄷ", "ㅎ": "ㄷ", "ㅍ": "ㅂ"}
TENSE = {"ㄱ": "ㄲ", "ㄷ": "ㄸ", "ㅂ": "ㅃ", "ㅅ": "ㅆ", "ㅈ": "ㅉ"}
NASAL = {"ㄱ": "ㅇ", "ㄷ": "ㄴ", "ㅂ": "ㅁ"}
ASPIRATED = {"ㄱ": "ㅋ", "ㄷ": "ㅌ", "ㅈ": "ㅊ", "ㅅ": "ㅆ"}
RR_INITIAL = {"ㄱ": "g", "ㄲ": "kk", "ㄴ": "n", "ㄷ": "d", "ㄸ": "tt", "ㄹ": "r", "ㅁ": "m", "ㅂ": "b", "ㅃ": "pp", "ㅅ": "s", "ㅆ": "ss", "ㅇ": "", "ㅈ": "j", "ㅉ": "jj", "ㅊ": "ch", "ㅋ": "k", "ㅌ": "t", "ㅍ": "p", "ㅎ": "h"}
RR_VOWEL = dict(zip(VOWELS, "a ae ya yae eo e yeo ye o wa wae oe yo u wo we wi yu eu ui i".split()))
RR_FINAL = {"ㄱ": "k", "ㄴ": "n", "ㄷ": "t", "ㄹ": "l", "ㅁ": "m", "ㅂ": "p", "ㅇ": "ng"}
# A bare letter such as the ㅂ of ㅂ니다: as the app writes it, and as the dictionary does (a final).
RR_JAMO = {"ㅂ": "b", "ㄴ": "n", "ㄹ": "l", "ㅁ": "m", "ㅆ": "ss", "ㅇ": "ng"}
RR_JAMO_FINAL = {"ㅂ": "p", "ㄴ": "n", "ㄹ": "l", "ㅁ": "m", "ㅆ": "ss", "ㅇ": "ng"}


def is_syllable(c):
    return "가" <= c <= "힣"


def is_jamo(c):
    return "ㄱ" <= c <= "ㅣ"


def parse(word):
    """Syllables as `[initial, vowel, [final letters]]`; anything else stays a character."""
    units = []
    for c in word:
        if is_syllable(c):
            n = ord(c) - 0xAC00
            final = FINALS[n % 28]
            units.append([INITIALS[n // 588], VOWELS[(n % 588) // 28], list(CLUSTERS.get(final, final))])
        else:
            units.append(c)
    return units


def simplify(coda, next_initial):
    """A final cluster as the one sound it keeps before a consonant or at the end."""
    if not coda:
        return None
    pair = "".join(coda)
    if len(pair) == 1:
        return pair
    if pair == "ㄹㄱ":
        return "ㄹ" if next_initial == "ㄱ" else "ㄱ"  # 읽고 [일꼬] but 읽다 [익따]
    if pair in ("ㄹㅁ",):
        return "ㅁ"
    if pair in ("ㄹㅍ", "ㅂㅅ"):
        return "ㅂ"
    return pair[0]


def aspirate(final, next_vowel):
    if final in "ㄱㅋㄲ":
        return "ㅋ"
    if final == "ㄷ" and next_vowel == "ㅣ":
        return "ㅊ"  # 닫히다 → 다치다
    if final == "ㅈ":
        return "ㅊ"
    if final in "ㄷㅅㅆㅌㅊ":
        return "ㅌ"
    if final in "ㅂㅍ":
        return "ㅍ"
    return final


def apply_pair(a, b, tensify):
    """The sound changes between a syllable and the next, in the order pronounce.rs applies them."""
    coda = a[2]
    if not coda:
        return
    if b[0] == "ㅇ":  # liaison
        if coda == ["ㅇ"]:
            return
        if coda == ["ㅎ"]:
            a[2] = []
        elif len(coda) == 2 and coda[1] == "ㅎ":
            b[0], a[2] = coda[0], []
        elif len(coda) == 2:
            b[0] = "ㅆ" if tensify and coda[1] == "ㅅ" and coda[0] in "ㄱㅂ" else coda[1]
            a[2] = [coda[0]]
        else:
            b[0] = "ㅈ" if b[1] == "ㅣ" and coda[0] == "ㄷ" else "ㅊ" if b[1] == "ㅣ" and coda[0] == "ㅌ" else coda[0]
            a[2] = []
        return
    was, nxt = list(coda), b[0]
    if was == ["ㅎ"]:
        if nxt in ASPIRATED:
            b[0], a[2] = ASPIRATED[nxt], []
            return
        if nxt == "ㅎ":
            a[2] = []
            return
        a[2] = ["ㄷ"]  # 놓는 → 논는, by the nasalisation below
    elif len(was) == 2 and was[1] == "ㅎ":
        a[2] = [was[0]]  # 않고 → 안코
        if nxt in ASPIRATED:
            b[0] = ASPIRATED[nxt]
            return
    elif len(was) == 2 and nxt == "ㅎ" and was[1] in "ㄱㅂㅈ":
        b[0], a[2] = aspirate(was[1], b[1]), [was[0]]  # 읽히다 → 일키다
        return
    elif len(was) == 1 and nxt == "ㅎ" and was[0] not in "ㅇㄴㄹㅁ":
        b[0], a[2] = aspirate(was[0], b[1]), []  # 축하 → 추카
        return
    tense_after_cluster = "".join(was) in ("ㄴㅈ", "ㄹㅁ", "ㄹㅂ", "ㄹㅌ") or (was == ["ㄹ", "ㄱ"] and nxt == "ㄱ")
    sound = simplify(a[2], nxt)
    sound = NEUTRAL.get(sound, sound)
    if tensify and b[0] in TENSE and (sound in "ㄱㄷㅂ" or tense_after_cluster):
        b[0] = TENSE[b[0]]
    if sound in "ㄱㄷㅂ" and b[0] in "ㄴㅁㄹ":
        sound = NASAL[sound]
        if b[0] == "ㄹ":
            b[0] = "ㄴ"
    elif sound in "ㅁㅇ" and b[0] == "ㄹ":
        b[0] = "ㄴ"  # 종로 → 종노
    if sound == "ㄴ" and b[0] == "ㄹ":
        sound = "ㄹ"  # 신라 → 실라
    elif sound == "ㄹ" and b[0] == "ㄴ":
        b[0] = "ㄹ"  # 설날 → 설랄
    a[2] = [sound]


def sounds(word, tensify):
    units = parse(word)
    for a, b in zip(units, units[1:]):
        if isinstance(a, list) and isinstance(b, list):
            apply_pair(a, b, tensify)
    for n, unit in enumerate(units):
        ends_a_run = n + 1 == len(units) or not isinstance(units[n + 1], list)
        if ends_a_run and isinstance(unit, list):
            sound = simplify(unit[2], None)
            unit[2] = [NEUTRAL.get(sound, sound)] if sound else []
    return units


def romanize(word, tensify=False):
    """Revised Romanization of one word as the app writes it (`romanize_word`): 학교 → hakgyo, 좋다 → jota.
    With `tensify`, the tense sounds of the pronunciation are written too (옷깃 → otkkit)."""
    out, after_l = [], False
    for unit in sounds(word, tensify):
        if isinstance(unit, list):
            out.append("l" if unit[0] == "ㄹ" and after_l else RR_INITIAL[unit[0]])
            out.append(RR_VOWEL[unit[1]])
            after_l = bool(unit[2]) and unit[2][0] == "ㄹ"
            if unit[2]:
                out.append(RR_FINAL.get(unit[2][0], ""))
        else:
            after_l = False
            out.append(RR_JAMO.get(unit) or RR_INITIAL.get(unit, "") if is_jamo(unit) else unit)
    return "".join(out)


# ───────────────────────── romanized equivalents ─────────────────────────

# The dictionary never translates these; it writes them in Latin letters.
BOUND = ("조사", "어미", "접사")
# What the analyser's part of speech takes from the dictionary (`pos_matches` in maru-dict/src/nikl.rs).
ANALYSER = {
    "noun": ("명사", "의존 명사"),
    "pronoun": ("대명사",),
    "numeral": ("수사",),
    "verb": ("동사", "보조 동사"),
    "adjective": ("형용사", "보조 형용사", "동사"),
    "adverb": ("부사",),
    "determiner": ("관형사",),
    "interjection": ("감탄사",),
}
ON_CARDS = tuple(dict.fromkeys(pos for group in ANALYSER.values() for pos in group))
# Borrowed words that, written by the romanizer, happen to be their English source: the equivalent is a real word.
# Picked out of the dictionary of 2026-11-25 by reading once every lower-case match that has no gloss; a word that
# is missing here is counted as romanized, so the list errs towards flagging. The durable guard would be the
# dictionary's own «원어» (origin) field, which `tools/build_nikl.py` does not keep.
BORROWED = set(
    "이데아 톱 라디오 호텔 팁 펜 모델 링 오페라 레 미 솔 라 메달 미네랄 피아노 바나나 이온 파라솔 헬멧 소나타 템포 핀 모뎀 "
    "사우나 아로마 안단테 알토 오보에 토마토 페달 고릴라 네온 데모 로봇 메모 모텔 발레리나 비키니 사무라이 사탄 살사 세레나데 "
    "아리아 아멘 아하 안단티노 암모니아 에나멜 오랑우탄 요가 요요 이구아나 칩 키위 토템 팀파니 파노라마 판다 펜팔 딩동 딩동딩동 "
    "하하 헤헤 톤 원 메가 기가".split()
)
GLOSS = re.compile(r"^([^:.]{1,60}): (?=\S)")


def letters(text):
    """Lower case, accents dropped, letters only: "Hang-ari" and "hangari" are the same spelling."""
    plain = unicodedata.normalize("NFD", text.lower())
    return re.sub(r"[^a-z]", "", plain)


def spellings(headword):
    """The ways the dictionary writes a headword in Latin letters, most of them the first."""
    per_syllable = "".join(romanize(c) if is_syllable(c) else RR_JAMO_FINAL.get(c, c) for c in headword).replace("lr", "ll")
    # A linking ㅅ (사이시옷) is often left out: 봇짐 → "bojim" beside "botjim".
    unlinked = "".join(chr(ord(c) - 19) if is_syllable(c) and (ord(c) - 0xAC00) % 28 == 19 and n + 1 < len(headword) else c for n, c in enumerate(headword))
    return {
        letters(romanize(headword)),  # as the app writes it, by the pronunciation: 않다 → anta, 국물 → gungmul
        letters(romanize(headword, tensify=True)),  # with tense sounds: 옷깃 → otkkit
        letters(per_syllable),  # each syllable on its own: 습니다 → seupnida, 식혜 → sikhye, ㅂ니다 → pnida
        letters(romanize(unlinked)),
    } - {""}


def gloss(definition):
    """The short meaning some definitions start with ("ink stick: A black stick…"), or an empty string."""
    found = GLOSS.match(definition or "")
    return found.group(1).strip() if found else ""


def judge(headword, pos, en, definition=""):
    """`(verdict, parts)` for one sense; `parts` is each part of the equivalent with whether it is a romanization."""
    ways = spellings(headword)
    parts = [(part.strip(), letters(part) in ways) for part in en.split(";") if part.strip()]
    if not parts:
        return "empty", parts
    matched = [text for text, is_one in parts if is_one]
    if pos in BOUND:
        return "romanized", parts
    if not matched:
        return "translated", parts
    if len(matched) < len(parts):
        return "mixed", parts
    if headword in BORROWED:
        return "borrowed", parts
    if not gloss(definition) and all(text.lstrip("-")[:1].isupper() for text in matched):
        return "name", parts
    return "romanized", parts


def spanish_word(headword, es):
    """Whether the Spanish equivalent has a part that is a word and not the headword in Latin letters again."""
    ways = spellings(headword)
    return any(letters(part) and letters(part) not in ways for part in re.split(r"[,;/]", es))


def open_readonly(path):
    return sqlite3.connect(Path(path).resolve().as_uri() + "?mode=ro", uri=True)


def read(db):
    """Every sense with its entry, in the order the app reads them."""
    rows = db.execute(
        "SELECT e.id, e.lemma, e.homonym, e.pos, e.level, s.n, s.en, s.es, s.en_def FROM nikl_entries e JOIN nikl_senses s ON s.entry = e.id ORDER BY e.homonym, e.id, s.n"
    )
    names = ("entry", "lemma", "homonym", "pos", "level", "n", "en", "es", "definition")
    return [dict(zip(names, row)) for row in rows]


def groups(senses):
    """`{(lemma, analyser's part of speech): [senses]}` as `Nikl::senses` returns them, for every part of speech
    that matches an entry of the word. An adjective also takes the verbs, so a word that is only a verb would be
    counted twice; that second, equal group is left out."""
    by_lemma = {}
    for sense in senses:
        by_lemma.setdefault(sense["lemma"], []).append(sense)
    out = {}
    for lemma, all_senses in by_lemma.items():
        for pos, takes in ANALYSER.items():
            found = [s for s in all_senses if s["pos"] in takes]
            if found and not (pos == "adjective" and found == out.get((lemma, "verb"))):
                out[(lemma, pos)] = found
    return out


def romanized_stats(senses, examples=30):
    judged = [judge(s["lemma"], s["pos"], s["en"], s["definition"]) for s in senses]
    verdicts = [verdict for verdict, _ in judged]
    flagged = [s for s, v in zip(senses, verdicts) if v == "romanized"]
    # Particles, endings and affixes are flagged for what they are; these are the ones the romanizer did not confirm.
    unconfirmed = [s for s, (v, parts) in zip(senses, judged) if v == "romanized" and not all(is_one for _, is_one in parts)]
    entries, by_entry = {}, {}
    for s, v in zip(senses, verdicts):
        entries[s["entry"]] = s
        by_entry.setdefault(s["entry"], []).append(v)
    affected = [entries[e] for e, vs in by_entry.items() if "romanized" in vs]
    nothing = [entries[e] for e, vs in by_entry.items() if "romanized" in vs and all(v in ("romanized", "empty") for v in vs)]
    verdict_of = {(s["entry"], s["n"]): v for s, v in zip(senses, verdicts)}
    grouped = groups(senses)
    with_one = {k: g for k, g in grouped.items() if any(verdict_of[(s["entry"], s["n"])] == "romanized" for s in g)}
    # Examples: a few of every part of speech in turn, the commonest words first (the dictionary's own levels),
    # one row per entry, the parts of speech a card can show before the others.
    level = {"초급": 0, "중급": 1, "고급": 2}
    queues, seen = {}, set()
    for s in sorted(flagged, key=lambda s: (level.get(s["level"], 3), s["entry"], s["n"])):
        if s["entry"] not in seen:
            seen.add(s["entry"])
            queues.setdefault(s["pos"], []).append(s)
    order = sorted(queues, key=lambda pos: (pos not in ON_CARDS, -len(queues[pos])))
    shown = []
    while len(shown) < examples and any(queues.values()):
        for pos in order:
            if queues[pos] and len(shown) < examples:
                s = queues[pos].pop(0)
                shown.append({"headword": s["lemma"], "pos": s["pos"], "level": s["level"], "en": s["en"], "es": s["es"], "definition": s["definition"]})
    return {
        "senses": len(senses),
        "entries": len(entries),
        "verdicts": dict(Counter(verdicts)),
        "verdicts_by_pos": {v: dict(Counter(s["pos"] for s, w in zip(senses, verdicts) if w == v)) for v in sorted(set(verdicts))},
        "romanized_senses": len(flagged),
        "romanized_senses_on_cards": sum(1 for s in flagged if s["pos"] in ON_CARDS),
        "romanized_senses_matched_by_the_romanizer": len(flagged) - len(unconfirmed),
        "romanized_senses_by_part_of_speech_alone": [{"headword": s["lemma"], "pos": s["pos"], "en": s["en"]} for s in unconfirmed],
        "romanized_senses_by_pos": dict(Counter(s["pos"] for s in flagged)),
        "affected_entries": len(affected),
        "affected_entries_by_pos": dict(Counter(s["pos"] for s in affected)),
        "with_english_definition": sum(1 for s in flagged if s["definition"].strip()),
        "with_gloss_in_definition": sum(1 for s in flagged if gloss(s["definition"])),
        "with_gloss_in_definition_by_pos": dict(Counter(s["pos"] for s in flagged if gloss(s["definition"]))),
        "with_spanish_word": sum(1 for s in flagged if spanish_word(s["lemma"], s["es"])),
        "with_spanish_word_by_pos": dict(Counter(s["pos"] for s in flagged if spanish_word(s["lemma"], s["es"]))),
        "with_neither_gloss_nor_spanish_word": sum(1 for s in flagged if not gloss(s["definition"]) and not spanish_word(s["lemma"], s["es"])),
        "entries_without_usable_english": len(nothing),
        "entries_without_usable_english_by_pos": dict(Counter(s["pos"] for s in nothing)),
        "entries_without_usable_english_on_cards": sum(1 for s in nothing if s["pos"] in ON_CARDS),
        # What a card meets: the senses of one word under the analyser's part of speech.
        "card_groups": len(grouped),
        "card_groups_with_a_romanized_sense": len(with_one),
        "card_groups_with_only_romanized_senses": sum(1 for g in with_one.values() if all(verdict_of[(s["entry"], s["n"])] in ("romanized", "empty") for s in g)),
        "card_groups_whose_first_sense_is_romanized": sum(1 for g in with_one.values() if verdict_of[(g[0]["entry"], g[0]["n"])] == "romanized"),
        "card_groups_with_one_sense_and_it_is_romanized": sum(1 for g in with_one.values() if len(g) == 1),
        "definitions_with_html_entities": sum(1 for s in senses if re.search(r"&#?\w+;", s["definition"])),
        "examples": shown,
    }


# ───────────────────────── words with many senses ─────────────────────────

MAX_SENSES = 8


def sense_stats(senses, top=30):
    grouped = groups(senses)
    sizes = Counter(len(g) for g in grouped.values())
    over = {k: g for k, g in grouped.items() if len(g) > MAX_SENSES}
    # Whether choosing the homonym first and then its sense would fit both steps in the limit.
    by_homonym = {k: Counter(s["entry"] for s in g) for k, g in over.items()}
    fits = sum(1 for c in by_homonym.values() if len(c) <= MAX_SENSES and max(c.values()) <= MAX_SENSES)
    largest = sorted(grouped.items(), key=lambda kv: (-len(kv[1]), kv[0]))[:top]
    return {
        "groups": len(grouped),
        "distribution": {str(n): sizes[n] for n in sorted(sizes)},
        "one_sense": sizes[1],
        "two_to_eight": sum(c for n, c in sizes.items() if 2 <= n <= MAX_SENSES),
        "over_eight": len(over),
        "over_sixteen": sum(1 for g in over.values() if len(g) > 2 * MAX_SENSES),
        "over_eight_by_pos": dict(Counter(pos for _, pos in over)),
        "largest": len(largest[0][1]) if largest else 0,
        "over_eight_where_homonym_then_sense_fits": fits,
        "over_eight_with_one_homonym": sum(1 for c in by_homonym.values() if len(c) == 1),
        "top": [{"headword": lemma, "pos": pos, "senses": len(g), "homonyms": len({s["entry"] for s in g}), "largest_homonym": max(Counter(s["entry"] for s in g).values())} for (lemma, pos), g in largest],
    }


def load(path):
    with open(path, encoding="utf-8") as lines:
        return [json.loads(line) for line in lines if line.strip()]


def load_json(path):
    with open(path, encoding="utf-8") as whole:
        return json.load(whole)


def position(word, answer):
    """Where a candidate's meaning sits among the word's senses, from 0; None when it is not one of them."""
    return next((n for n, s in enumerate(word["senses"]) if (s["en"], s["es"]) == (answer["en"], answer["es"])), None)


def task_stats(tasks, key=None, marks=None, candidate=None, cards=None):
    """How the long words of a set of lines went in a marked review. Lines are counted, never quoted."""
    words = [(n, w) for n, task in enumerate(tasks, 1) for w in task["words"]]
    long_words = {(n, w["index"]): w for n, w in words if len(w["senses"]) > MAX_SENSES}
    out = {
        "lines": len(tasks),
        "words": len(words),
        "senses_per_word": {str(k): v for k, v in sorted(Counter(len(w["senses"]) for _, w in words).items())},
        "words_over_eight": len(long_words),
        "words_over_sixteen": sum(1 for w in long_words.values() if len(w["senses"]) > 2 * MAX_SENSES),
        "different_words_over_eight": len({(w["lemma"], w["pos"]) for w in long_words.values()}),
        "lines_with_a_word_over_eight": len({n for n, _ in long_words}),
        # What showing every sense would add to the requests, in characters of "equivalent — definition" lines.
        "characters_of_senses_shown_now": sum(len(s["en"]) + min(len(s["definition"]), 110) for _, w in words for s in w["senses"][:MAX_SENSES]),
        "characters_of_senses_beyond_eight": sum(len(s["en"]) + min(len(s["definition"]), 110) for w in long_words.values() for s in w["senses"][MAX_SENSES:]),
    }
    if not (key and marks):
        return out
    letters_of = {line["line"]: line["letters"] for line in key["lines"]}
    named = [name for name in key["candidates"] if (candidate or "").lower() in name.lower()]
    if candidate and len(named) != 1:
        sys.exit(f"--candidate {candidate!r} names {len(named)} candidates: {key['candidates']}")
    answers = {(r["candidate"], r["line"], w["index"]): w for r in cards or [] for w in r["words"]}
    by_word = {}
    for where, mark in marks.items():
        line, index, letter = where.split("|")
        by_word.setdefault((int(line), int(index)), {})[letters_of[int(line)][letter]] = mark
    sense_count = {(n, w["index"]): len(w["senses"]) for n, w in words}
    bucket = lambda count: "not in the dictionary" if count == 0 else "over eight" if count > MAX_SENSES else "two to eight"
    if candidate:
        tally = {}
        for where, given in by_word.items():
            if named[0] in given:
                tally.setdefault(bucket(sense_count[where]), Counter())[given[named[0]]] += 1
        out["candidate"] = named[0]
        out["candidate_marks"] = {b: dict(c) for b, c in tally.items()}
    marked_long = {where: given for where, given in by_word.items() if where in long_words}
    reach = Counter()
    for where, given in marked_long.items():
        right = [name for name, mark in given.items() if mark == "right"]
        if not right:
            reach["no candidate was marked right"] += 1
        elif not cards:
            reach["some candidate was marked right"] += 1
        else:
            # A right answer among the first eight means the model was shown it; the reranker saw every sense
            # and a model may write a meaning of its own, so "right" alone does not show that.
            spots = [position(long_words[where], answers[(name, where[0], where[1])]) for name in right if (name, where[0], where[1]) in answers]
            if any(n is not None and n < MAX_SENSES for n in spots):
                reach["a right answer is one of the first eight senses"] += 1
            elif any(n is not None for n in spots):
                reach["the right answers are senses after the eighth"] += 1
            else:
                reach["the right answers are meanings a model wrote"] += 1
    out["marked_words_over_eight"] = len(marked_long)
    out["marked_words_over_eight_reach"] = dict(reach)
    if candidate and cards:
        # The candidate's own marks on the long words, by whether a sense marked right was among the eight it saw:
        # mistakes where none was are the cut's, the others are its own.
        shown_right, lines_to_ask_again = {}, set()
        for where, given in marked_long.items():
            spots = [position(long_words[where], answers[(name, where[0], where[1])]) for name, mark in given.items() if mark == "right" and (name, where[0], where[1]) in answers]
            was_shown = any(n is not None and n < MAX_SENSES for n in spots)
            if not was_shown:
                lines_to_ask_again.add(where[0])
            if named[0] in given:
                shown_right.setdefault("a right sense was among the eight it was shown" if was_shown else "no right sense among the eight it was shown", Counter())[given[named[0]]] += 1
        out["candidate_on_words_over_eight"] = {k: dict(c) for k, c in shown_right.items()}
        out["lines_with_a_long_word_whose_right_sense_was_not_shown"] = len(lines_to_ask_again)
    return out


def show(stats, indent=""):
    for name, value in stats.items():
        if isinstance(value, dict):
            print(f"{indent}{name.replace('_', ' ')}:")
            show(value, indent + "  ")
        elif isinstance(value, list):
            print(f"{indent}{name.replace('_', ' ')}:")
            for row in value:
                print(f"{indent}  " + " | ".join(str(v) for v in row.values()))
        else:
            print(f"{indent}{name.replace('_', ' ')}: {value}")


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    r = sub.add_parser("romanized")
    r.add_argument("db")
    r.add_argument("--examples", type=int, default=30)
    r.add_argument("--json")
    s = sub.add_parser("senses")
    s.add_argument("db")
    s.add_argument("--tasks")
    s.add_argument("--key")
    s.add_argument("--marks")
    s.add_argument("--candidate")
    s.add_argument("--cards", nargs="*")
    s.add_argument("--json")
    args = parser.parse_args(argv)
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")  # a Windows console would refuse the Korean otherwise
    db = open_readonly(args.db)
    senses = read(db)
    db.close()
    if args.command == "romanized":
        stats = romanized_stats(senses, args.examples)
    else:
        stats = {"dictionary": sense_stats(senses)}
        if args.tasks:
            key = load_json(args.key) if args.key else None
            marks = load_json(args.marks)["marks"] if args.marks else None
            cards = [record for path in args.cards or [] for record in load(path)]
            stats["lines"] = task_stats(load(args.tasks), key, marks, args.candidate, cards)
    show(stats)
    if args.json:
        with open(args.json, "w", encoding="utf-8", newline="\n") as out:
            json.dump(stats, out, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main(sys.argv[1:])
