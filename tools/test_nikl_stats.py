import contextlib
import io
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

from nikl_stats import groups, judge, main, read, romanize, romanized_stats, sense_stats, spanish_word, spellings, task_stats

# A dictionary of a dozen words, invented for these tests: (headword, homonym, part of speech, level, senses),
# a sense being (English, Spanish, English definition).
WORDS = [
    ("녀석", 0, "의존 명사", "중급", [("nyeoseok", "", "(impolite form) A bound noun used to refer to a man."), ("nyeoseok", "", "(endearing) A bound noun used to refer to a boy.")]),
    ("수", 1, "명사", "", [("means; way", "medio, modo", "A method of doing something.")]),
    ("수", 2, "의존 명사", "", [("su", "", "A bound noun meaning the ability to do something.")]),
    ("수", 3, "접사", "", [("-su", "", "A suffix used to mean a player.")]),
    ("먹", 0, "명사", "", [("meok", "tinta china", "ink stick: A black stick ground with water to write with a brush.")]),
    ("항아리", 0, "명사", "초급", [("hang-ari", "hang-ari", "A jar with a narrow top and bottom.")]),
    ("서울", 0, "명사", "초급", [("Seoul", "Seúl", "The capital of Korea.")]),
    ("라디오", 0, "명사", "초급", [("radio", "radio", "A machine that receives broadcasts.")]),
    ("카메라", 0, "명사", "초급", [("camera", "cámara", "A device used to take photographs.")]),
    ("김치", 0, "명사", "초급", [("gimchi; kimchi", "gimchi", "A staple of the Korean diet.")]),
    ("을지", 0, "어미", "", [("-ulji", "", "A connective ending used when having a vague doubt.")]),
    ("않다", 1, "동사", "", [("do not", "no hacer", "Not to do an action.")]),
    ("않다", 2, "보조 동사", "", [("anta", "", "An auxiliary verb used to negate the preceding act.")]),
    ("하다", 1, "동사", "초급", [(f"do {n}", f"hacer {n}", f"To do the thing numbered {n}.") for n in range(1, 10)]),
    ("하다", 2, "보조 동사", "", [("hada", "", "An auxiliary verb used to make the preceding act happen.")]),
    ("곱다", 0, "형용사", "", [("pretty", "bonito", "Looking beautiful."), ("", "suave", "Being soft to the touch.")]),
]


def dictionary(folder):
    """The fixture as a file with the app's tables (`Nikl::with` in maru-dict)."""
    path = Path(folder) / "nikl.db"
    db = sqlite3.connect(path)
    db.executescript(
        "CREATE TABLE nikl_entries (id INTEGER PRIMARY KEY, lemma TEXT NOT NULL, homonym INTEGER NOT NULL DEFAULT 0, pos TEXT NOT NULL DEFAULT '', level TEXT NOT NULL DEFAULT '');"
        "CREATE TABLE nikl_senses (entry INTEGER NOT NULL, n INTEGER NOT NULL, en TEXT NOT NULL DEFAULT '', es TEXT NOT NULL DEFAULT '', en_def TEXT NOT NULL DEFAULT '', es_def TEXT NOT NULL DEFAULT '', PRIMARY KEY (entry, n));"
    )
    for lemma, homonym, pos, level, senses in WORDS:
        entry = db.execute("INSERT INTO nikl_entries (lemma, homonym, pos, level) VALUES (?, ?, ?, ?)", (lemma, homonym, pos, level)).lastrowid
        for n, (en, es, definition) in enumerate(senses):
            db.execute("INSERT INTO nikl_senses (entry, n, en, es, en_def) VALUES (?, ?, ?, ?, ?)", (entry, n, en, es, definition))
    db.commit()
    db.close()
    return path


class RomanizerTests(unittest.TestCase):
    def test_it_writes_what_the_apps_romanizer_writes(self):
        # The expectations of the tests in crates/maru-lang/src/pronounce.rs.
        same = {
            "합니다": "hamnida", "국물": "gungmul", "종로": "jongno", "신라": "silla", "설날": "seollal", "한라산": "hallasan",
            "한국어": "hangugeo", "먹었어요": "meogeosseoyo", "강아지": "gangaji", "읽고": "ilgo", "없어요": "eopseoyo", "좋다": "jota",
            "괜찮아요": "gwaenchanayo", "축하": "chuka", "같이": "gachi", "해돋이": "haedoji", "학교": "hakgyo", "있다": "itda",
            "학생": "haksaeng", "밥": "bap", "한국": "hanguk", "감사합니다": "gamsahamnida", "안녕하세요!": "annyeonghaseyo!", "ABC123": "ABC123", "ㅂ니다": "bnida",
        }
        for word, written in same.items():
            self.assertEqual(romanize(word), written, word)

    def test_a_headword_has_the_few_spellings_the_dictionary_uses_beside_the_apps(self):
        self.assertIn("anta", spellings("않다"), "by the pronunciation, as the app writes it")
        self.assertEqual(romanize("옷깃"), "otgit")
        self.assertIn("otkkit", spellings("옷깃"), "with the tense sound written")
        self.assertEqual(romanize("식혜"), "sikye")
        self.assertIn("sikhye", spellings("식혜"), "each syllable on its own keeps the h")
        self.assertIn("seupnida", spellings("습니다"), "an ending spelled without its sound change")
        self.assertIn("pnida", spellings("ㅂ니다"), "a bare letter as a final")
        self.assertIn("llae", spellings("ㄹ래"))
        self.assertIn("bojim", spellings("봇짐"), "without the linking ㅅ")
        self.assertIn("botjim", spellings("봇짐"))


class RomanizedEquivalentTests(unittest.TestCase):
    def test_an_equivalent_that_is_only_the_headword_in_latin_letters_is_romanized_whatever_its_hyphens_and_spaces(self):
        self.assertEqual(judge("녀석", "의존 명사", "nyeoseok")[0], "romanized")
        self.assertEqual(judge("항아리", "명사", "hang-ari")[0], "romanized")
        self.assertEqual(judge("않다", "보조 동사", "anta")[0], "romanized")
        self.assertEqual(judge("먹", "명사", "Meok", "ink stick: A black stick.")[0], "romanized", "a capital does not make a name of a word the definition has to gloss")
        self.assertEqual(judge("제삿밥", "명사", "jesabap; jesatbap")[0], "romanized", "every part is a spelling of the headword")

    def test_a_translation_is_never_flagged_even_when_it_looks_like_the_headword(self):
        self.assertEqual(judge("카메라", "명사", "camera")[0], "translated", "kamera is close, not equal")
        self.assertEqual(judge("수", "명사", "means; way")[0], "translated")
        self.assertEqual(judge("곱다", "형용사", "")[0], "empty")

    def test_a_romanization_beside_a_real_word_leaves_the_real_word(self):
        verdict, parts = judge("김치", "명사", "gimchi; kimchi")
        self.assertEqual((verdict, parts), ("mixed", [("gimchi", True), ("kimchi", False)]))

    def test_names_and_borrowed_words_are_real_words_and_kept_apart(self):
        self.assertEqual(judge("서울", "명사", "Seoul", "The capital of Korea.")[0], "name")
        self.assertEqual(judge("홍길동", "명사", "Hong Gildong", "The hero of an old novel.")[0], "name")
        self.assertEqual(judge("라디오", "명사", "radio")[0], "borrowed", "the English word the headword was borrowed from")
        self.assertEqual(judge("게네", "대명사", "gene")[0], "romanized", "an English word by accident, not by origin")

    def test_particles_endings_and_affixes_are_romanized_even_in_the_dictionarys_own_spelling(self):
        verdict, parts = judge("을지", "어미", "-ulji")
        self.assertEqual((verdict, parts), ("romanized", [("-ulji", False)]), "eulji by the rule; the part of speech decides")
        self.assertEqual(judge("수", "접사", "-su")[0], "romanized")

    def test_a_spanish_equivalent_counts_only_when_it_is_a_word(self):
        self.assertTrue(spanish_word("먹", "tinta china"))
        self.assertTrue(spanish_word("고추장", "gochujang, pasta de chile coreana"))
        self.assertFalse(spanish_word("항아리", "hang-ari"))
        self.assertFalse(spanish_word("녀석", ""))


class DictionaryTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.path = dictionary(self.dir.name)
        db = sqlite3.connect(self.path)
        self.addCleanup(db.close)
        self.senses = read(db)

    def test_the_counts_say_how_many_are_romanized_and_what_is_left_to_make_a_meaning_from(self):
        stats = romanized_stats(self.senses, examples=3)
        self.assertEqual(stats["verdicts"], {"romanized": 9, "translated": 13, "name": 1, "borrowed": 1, "mixed": 1, "empty": 1})
        self.assertEqual(stats["romanized_senses_by_pos"], {"의존 명사": 3, "접사": 1, "명사": 2, "어미": 1, "보조 동사": 2})
        self.assertEqual(stats["romanized_senses_on_cards"], 7, "affixes and endings are not what a card shows")
        self.assertEqual(stats["romanized_senses_by_part_of_speech_alone"], [{"headword": "을지", "pos": "어미", "en": "-ulji"}])
        self.assertEqual((stats["affected_entries"], stats["entries_without_usable_english"]), (8, 8))
        self.assertEqual((stats["with_english_definition"], stats["with_gloss_in_definition"], stats["with_spanish_word"]), (9, 1, 1), "먹 has both a gloss and a Spanish word")
        self.assertEqual(stats["with_neither_gloss_nor_spanish_word"], 8)
        # 수 as a noun takes the noun and the bound noun: one of its two senses is romanized, and not the first.
        self.assertEqual(stats["card_groups_with_a_romanized_sense"], 6)
        self.assertEqual(stats["card_groups_with_only_romanized_senses"], 3, "녀석, 먹 and 항아리")
        self.assertEqual(stats["card_groups_with_one_sense_and_it_is_romanized"], 2)
        self.assertEqual([row["headword"] for row in stats["examples"]], ["항아리", "녀석", "않다"], "one of each part of speech in turn, the commonest words first")

    def test_senses_are_grouped_as_the_app_groups_them(self):
        grouped = groups(self.senses)
        self.assertEqual([s["en"] for s in grouped[("수", "noun")]], ["means; way", "su"], "the noun and the bound noun, not the affix")
        self.assertEqual(len(grouped[("하다", "verb")]), 10, "the verb and the auxiliary verb")
        self.assertEqual(len(grouped[("하다", "adjective")]), 9, "an adjective takes the verb but not the auxiliary verb")
        self.assertNotIn(("않다", "noun"), grouped)
        self.assertNotIn(("을지", "noun"), grouped, "an ending is no part of speech the analyser asks for")

    def test_words_with_more_senses_than_the_model_is_shown_are_counted(self):
        stats = sense_stats(self.senses, top=2)
        self.assertEqual((stats["over_eight"], stats["largest"], stats["over_eight_by_pos"]), (2, 10, {"verb": 1, "adjective": 1}))
        self.assertEqual(stats["top"][0], {"headword": "하다", "pos": "verb", "senses": 10, "homonyms": 2, "largest_homonym": 9})
        self.assertEqual(stats["over_eight_where_homonym_then_sense_fits"], 0, "one homonym alone has nine senses")
        self.assertEqual(stats["distribution"]["1"], stats["one_sense"])

    def test_the_command_prints_the_counts_and_leaves_the_dictionary_as_it_was(self):
        before = self.path.read_bytes()
        out = Path(self.dir.name) / "stats.json"
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            main(["romanized", str(self.path), "--examples", "2", "--json", str(out)])
            main(["senses", str(self.path)])
        self.assertIn("romanized senses: 9", printed.getvalue())
        self.assertIn("over eight: 2", printed.getvalue())
        self.assertEqual(json.loads(out.read_text(encoding="utf-8"))["romanized_senses"], 9)
        self.assertEqual(self.path.read_bytes(), before)


def sense(en, es=""):
    return {"en": en, "es": es, "definition": f"What {en} means."}


LONG = [sense(f"do {n}", f"hacer {n}") for n in range(1, 11)]
TASKS = [
    {"line": "밥을 하다", "words": [
        {"index": 0, "surface": "밥을", "lemma": "밥", "pos": "noun", "senses": [sense("rice", "arroz"), sense("meal", "comida")]},
        {"index": 1, "surface": "하다", "lemma": "하다", "pos": "verb", "senses": LONG},
    ]},
    {"line": "일을 하다", "words": [
        {"index": 0, "surface": "일을", "lemma": "일", "pos": "noun", "senses": [sense("work", "trabajo")]},
        {"index": 1, "surface": "하다", "lemma": "하다", "pos": "verb", "senses": LONG},
    ]},
]
KEY = {"candidates": ["cloud", "local · small", "reranker"], "lines": [
    {"line": 1, "letters": {"A": "local · small", "B": "cloud", "C": "reranker"}},
    {"line": 2, "letters": {"A": "cloud", "B": "reranker", "C": "local · small"}},
]}


def card(candidate, line, *words):
    return {"candidate": candidate, "line": line, "words": [{"index": index, "en": en, "es": es, "source": "chosen"} for index, en, es in words]}


CARDS = [
    card("local · small", 1, (0, "rice", "arroz"), (1, "do 2", "hacer 2")),
    card("cloud", 1, (0, "rice", "arroz"), (1, "do 3", "hacer 3")),
    card("reranker", 1, (0, "meal", "comida"), (1, "do 10", "hacer 10")),
    card("local · small", 2, (1, "do 1", "hacer 1")),
    card("cloud", 2, (1, "to cook", "cocinar")),
    card("reranker", 2, (1, "do 9", "hacer 9")),
]
# Line 1: the local model chose a wrong sense of 하다 although the right one (the third) was shown to it.
# Line 2: nothing among the first eight was right; only a meaning the cloud model wrote itself was.
MARKS = {"1|0|A": "right", "1|0|B": "right", "1|0|C": "wrong", "1|1|A": "wrong", "1|1|B": "right", "1|1|C": "wrong", "2|1|A": "right", "2|1|B": "wrong", "2|1|C": "wrong"}


class LineTests(unittest.TestCase):
    def test_long_words_are_counted_in_the_lines_without_quoting_them(self):
        stats = task_stats(TASKS)
        self.assertEqual((stats["words"], stats["words_over_eight"], stats["different_words_over_eight"], stats["lines_with_a_word_over_eight"]), (4, 2, 1, 2))
        self.assertEqual(stats["senses_per_word"], {"1": 1, "2": 1, "10": 2})
        self.assertNotIn("밥을 하다", json.dumps(stats, ensure_ascii=False))

    def test_a_candidates_marks_are_split_by_how_many_senses_the_word_has(self):
        stats = task_stats(TASKS, KEY, MARKS, "local", CARDS)
        self.assertEqual(stats["candidate"], "local · small")
        self.assertEqual(stats["candidate_marks"], {"two to eight": {"right": 1}, "over eight": {"wrong": 2}})

    def test_a_right_mark_shows_the_sense_was_reachable_only_when_it_is_one_of_the_first_eight(self):
        stats = task_stats(TASKS, KEY, MARKS, "local", CARDS)
        self.assertEqual(stats["marked_words_over_eight"], 2)
        self.assertEqual(stats["marked_words_over_eight_reach"], {"a right answer is one of the first eight senses": 1, "the right answers are meanings a model wrote": 1})
        self.assertEqual(stats["candidate_on_words_over_eight"], {"a right sense was among the eight it was shown": {"wrong": 1}, "no right sense among the eight it was shown": {"wrong": 1}})
        self.assertEqual(stats["lines_with_a_long_word_whose_right_sense_was_not_shown"], 1)

    def test_a_sense_after_the_eighth_that_only_the_reranker_could_choose_is_told_apart(self):
        marks = {"1|1|A": "wrong", "1|1|B": "wrong", "1|1|C": "right"}
        stats = task_stats(TASKS, KEY, marks, "local", CARDS)
        self.assertEqual(stats["marked_words_over_eight_reach"], {"the right answers are senses after the eighth": 1})
        without_cards = task_stats(TASKS, KEY, marks)
        self.assertEqual(without_cards["marked_words_over_eight_reach"], {"some candidate was marked right": 1}, "without the answers a right mark is all that is known")

    def test_a_name_that_fits_no_candidate_or_several_stops_the_count(self):
        with self.assertRaises(SystemExit):
            task_stats(TASKS, KEY, MARKS, "gemini", CARDS)


if __name__ == "__main__":
    unittest.main()
