import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import build_kiwi_review as r
from build_kiwi_review import GROUPS, bare, build, main, rows, score, totals


def morph(form, tag, label=""):
    return {"form": form, "tag": tag, "role": "stem", "label": label, "start": 0, "end": 1}


def word(index, surface, lemma, pos, morphs, content=True):
    return {"index": index, "surface": surface, "lemma": lemma, "pos": pos, "content": content, "morphs": morphs, "dict_glosses": []}


# Three invented lines, one for each group.
TOKENS = [
    {"line_no": 1, "line": "밥을 먹었어, 정말", "eojeols": [
        word(0, "밥을", "밥", "noun", [morph("밥", "NNG", "noun"), morph("을", "JKO", "object particle")]),
        word(1, "먹었어,", "먹다", "verb", [morph("먹", "VV", "verb"), morph("었", "EP", "past tense"), morph("어", "EF"), morph(",", "SP")]),
        word(2, "정말", "정말", "adverb", [morph("정말", "MAG")]),
    ]},
    {"line_no": 2, "line": "밥 묵었나", "eojeols": [
        word(0, "밥", "밥", "noun", [morph("밥", "NNG")]),
        word(1, "묵었나", "묵다", "verb", [morph("묵", "VV"), morph("었", "EP"), morph("나", "EC")]),
    ]},
    {"line_no": 3, "line": "밥을먹었어 !", "eojeols": [
        word(0, "밥을먹었어", "밥", "noun", [morph("밥", "NNG"), morph("을", "JKO"), morph("먹", "VV"), morph("었", "EP"), morph("어", "EF")]),
        word(1, "!", None, "other", [morph("!", "SF")], content=False),
    ]},
]
GROUPING = {"made_by": "Sorted by a test.", "lines": {
    "1": {"group": "standard text", "why": "plain speech"},
    "2": {"group": "dialect", "why": "a regional verb and ending"},
    "3": {"group": "broken OCR", "why": "two words glued together"},
}}
KNOWN = [{"surface": "묵었나", "lemma": "묵다", "note": "reported wrong in run 1"}, {"surface": "밥", "lemma": "빵", "note": "reported wrong in run 1"}]


def data_of(page):
    return json.loads(page.split("const DATA = ", 1)[1].split(";\nconst TOTALS", 1)[0])


class KiwiReviewTests(unittest.TestCase):
    def test_every_word_is_a_row_under_its_lines_group_with_its_lemma_and_morphemes(self):
        data = rows(TOKENS, GROUPING)
        self.assertEqual([g["name"] for g in data], list(GROUPS), "the three groups, always in this order")
        self.assertEqual([[line["line"] for line in g["lines"]] for g in data], [[1], [2], [3]])
        first = data[0]["lines"][0]
        self.assertEqual((first["text"], first["why"]), ("밥을 먹었어, 정말", "plain speech"), "the whole line is shown above its rows")
        self.assertEqual([(r["surface"], r["lemma"], r["pos"]) for r in first["rows"]], [("밥을", "밥", "noun"), ("먹었어,", "먹다", "verb"), ("정말", "정말", "adverb")])
        self.assertEqual(first["rows"][1]["morphs"][:2], [{"form": "먹", "tag": "VV", "label": "verb"}, {"form": "었", "tag": "EP", "label": "past tense"}])
        symbol = data[2]["lines"][0]["rows"][1]
        self.assertEqual((symbol["lemma"], symbol["content"]), ("", False), "a word without a card still has a row")
        self.assertEqual(totals(data), {"group": {"standard text": 3, "dialect": 2, "broken OCR": 2}, "pos": {"noun": 3, "verb": 2, "adverb": 1, "other": 1}})

    def test_a_line_without_a_group_or_with_a_group_that_does_not_exist_stops_the_build(self):
        with self.assertRaises(SystemExit) as stopped:
            rows(TOKENS, {"lines": {"1": {"group": "standard text"}, "2": {"group": "slang"}}})
        self.assertIn("line 2, 3", str(stopped.exception))

    def test_a_known_failure_is_flagged_only_where_the_word_and_the_lemma_are_both_the_same(self):
        data = rows(TOKENS, GROUPING, KNOWN)
        flagged = [(line["line"], r["surface"], r["known"]) for g in data for line in g["lines"] for r in line["rows"] if r["known"]]
        self.assertEqual(flagged, [(2, "묵었나", "reported wrong in run 1")], "밥 with the lemma 밥 is not the failure that was reported")
        self.assertEqual(bare("먹었어,"), "먹었어", "the punctuation around a word is not part of it")
        self.assertIn("reported wrong in run 1", build(TOKENS, GROUPING, KNOWN))

    def test_the_page_says_the_grouping_was_automatic_and_where_to_correct_it(self):
        page = build(TOKENS, GROUPING, groups_file="groups.json")
        self.assertIn("The grouping was made automatically.", page)
        self.assertIn("Sorted by a test.", page)
        self.assertIn("<code>groups.json</code>", page)
        self.assertEqual(len(data_of(page)), 3)
        self.assertIn("Save marks", page)
        self.assertIn("rows marked", page, "a count of what has been marked")

    def test_moving_a_line_to_another_group_keeps_the_marks_made_on_it(self):
        stamp = lambda page: page.split("const STAMP = ", 1)[1].split(";", 1)[0]
        moved = {"lines": dict(GROUPING["lines"], **{"2": {"group": "broken OCR"}})}
        self.assertEqual(stamp(build(TOKENS, GROUPING)), stamp(build(TOKENS, moved)), "marks are stored by line and word, under the same name")
        other = [dict(TOKENS[0], line="빵을 먹었어")] + TOKENS[1:]
        self.assertNotEqual(stamp(build(TOKENS, GROUPING)), stamp(build(other, GROUPING)), "other lines are another review")

    def test_marks_are_counted_per_group_and_per_part_of_speech_with_what_was_left_unmarked(self):
        saved = {"totals": {"group": {"standard text": 3, "dialect": 2, "broken OCR": 2}, "pos": {"noun": 3, "verb": 2, "adverb": 1, "other": 1}}, "marks": {
            "1|0": {"mark": "right", "correct": "", "group": "standard text", "pos": "noun"},
            "1|1": {"mark": "right", "correct": "", "group": "standard text", "pos": "verb"},
            "2|1": {"mark": "wrong", "correct": "먹다", "group": "dialect", "pos": "verb"},
            "3|0": {"mark": "wrong", "correct": "", "group": "broken OCR", "pos": "noun"},
        }}
        result = score(saved)
        self.assertEqual(result["group"], {"standard text": {"right": 2, "wrong": 0, "not marked": 1}, "dialect": {"right": 0, "wrong": 1, "not marked": 1}, "broken OCR": {"right": 0, "wrong": 1, "not marked": 1}})
        self.assertEqual(result["pos"]["noun"], {"right": 1, "wrong": 1, "not marked": 1})
        self.assertEqual(result["pos"]["verb"], {"right": 1, "wrong": 1, "not marked": 0})
        self.assertEqual(result["corrections"], 1, "one wrong row says what the lemma should have been")

    def test_the_commands_write_the_page_and_print_the_score(self):
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            (folder / "tokens.jsonl").write_text("\n".join(json.dumps(t, ensure_ascii=False) for t in TOKENS) + "\n", encoding="utf-8")
            (folder / "groups.json").write_text(json.dumps(GROUPING, ensure_ascii=False), encoding="utf-8")
            (folder / "known.json").write_text(json.dumps(KNOWN, ensure_ascii=False), encoding="utf-8")
            (folder / "marks.json").write_text(json.dumps({"totals": {"group": {"dialect": 2}, "pos": {"verb": 2}}, "marks": {"2|1": {"mark": "wrong", "correct": "먹다", "group": "dialect", "pos": "verb"}}}, ensure_ascii=False), encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                main(["build", str(folder / "tokens.jsonl"), "--groups", str(folder / "groups.json"), "--known", str(folder / "known.json"), "--out", str(folder / "index.html")])
                main(["score", str(folder / "marks.json")])
            page = (folder / "index.html").read_text(encoding="utf-8")
        self.assertTrue(page.startswith("<!doctype html>"))
        self.assertEqual(data_of(page)[1]["lines"][0]["rows"][1]["known"], "reported wrong in run 1")
        self.assertIn("rows: standard text 3, dialect 2, broken OCR 2; flagged as known failures: 1", printed.getvalue())
        self.assertIn("  dialect: 0 right, 1 wrong, 1 not marked", printed.getvalue())
        self.assertIn("  verb: 0 right, 1 wrong, 1 not marked", printed.getvalue())


class Changes(unittest.TestCase):
    def test_only_words_whose_dictionary_form_changed_are_listed_by_group_with_known_failures_noted(self):
        line = lambda no, text, words: {"line_no": no, "line": text, "eojeols": [{"index": i, "surface": s, "lemma": l} for i, (s, l) in enumerate(words)]}
        base = [line(1, "주꾸마, 고마워", [("주꾸마,", "꾸다"), ("고마워", "고맙다")]), line(2, "학교에 가", [("학교에", "학교"), ("가", "가다")])]
        other = [line(1, "주꾸마, 고마워", [("주꾸마,", "주다"), ("고마워", "고맙다")]), line(2, "학교에 가", [("학교에", "학교"), ("가", "가다")])]
        groups = {"lines": {"1": {"group": "dialect"}, "2": {"group": "standard text"}}}
        rows = r.changed_rows(base, other, groups, [{"surface": "주꾸마", "lemma": "꾸다", "note": "reported wrong in run 1"}])
        self.assertEqual(rows, [("dialect", 1, "주꾸마, 고마워", "주꾸마,", "꾸다", "주다", "reported wrong in run 1")])
        page = r.changes_page(base, [("Gyeongsang", other), ("Typo correction", "Kiwi crashed")], groups)
        self.assertIn("주다", page)
        self.assertIn("Not measured: Kiwi crashed", page)
        self.assertNotIn("학교에</td>", page, "unchanged words are not listed")


if __name__ == "__main__":
    unittest.main()
