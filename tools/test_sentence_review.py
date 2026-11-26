import contextlib
import io
import json
import os
import tempfile
import unittest

from build_sentence_review import answers, assign, build, main, read_json, score, write

# Invented lines, as every fixture in the repository is.
LINES = ["오늘은 비가 와서 집에 있었어요", "그 책은 벌써 다 읽었어", "내일 같이 점심 먹을래요?"]


def record(line, translation, error=None, run=1, source=None):
    return {"connection": "a server", "model": "a model", "pass": run, "line": line, "source": source or LINES[line - 1],
            "first_ms": 40, "total_ms": 90, "translation": translation, "capped": False, "error": error}


TRANSLATIONS = {
    "model one": ["I stayed home today because it rained.", "I already finished that book.", "Shall we have lunch together tomorrow?"],
    "model two": ["It rained today, so I was at home.", "I already finished that book.", None],
    "model three": ["Today the rain came and I was a house.", "That book is already read all.", None],
}
CANDIDATES = list(TRANSLATIONS)


def shown_data(page):
    return json.loads(page.split("const DATA = ", 1)[1].split(";\nconst STORE", 1)[0])


def letter_of(key, line, name):
    return next(letter for letter, names in key["lines"][line - 1]["letters"].items() if name in names)


class SentenceReviewTests(unittest.TestCase):
    def test_every_line_has_its_own_order_of_letters_and_the_same_seed_gives_the_same_page(self):
        different = {name: [f"{name} on line {n}" for n in range(40)] for name in CANDIDATES}
        key, _ = assign(different, seed=7)
        self.assertTrue(all(sorted(sum(line["letters"].values(), [])) == sorted(CANDIDATES) for line in key))
        self.assertTrue(all(sorted(line["letters"]) == ["A", "B", "C"] for line in key))
        self.assertGreater(len({tuple(map(tuple, line["letters"].values())) for line in key}), 1, "the order changes between lines")
        self.assertEqual(key, assign(different, seed=7)[0])
        self.assertNotEqual(key, assign(different, seed=8)[0])
        self.assertEqual(build(LINES, TRANSLATIONS, seed=7), build(LINES, TRANSLATIONS, seed=7))

    def test_the_page_names_no_candidate_and_the_key_is_separate(self):
        page, key = build(LINES, TRANSLATIONS, seed=3)
        shown = page.split("const DATA = ", 1)[1].split("const STORE", 1)[0]
        for name in CANDIDATES:
            self.assertFalse(name in shown, f"{name} is named in the page's data")
        self.assertEqual(key["candidates"], sorted(CANDIDATES))
        self.assertEqual([line["line"] for line in key["lines"]], [1, 2, 3])
        self.assertEqual([line["text"] for line in shown_data(page)], LINES)
        self.assertIn("It rained today, so I was at home.", page)

    def test_two_candidates_with_the_same_translation_share_one_letter_and_the_key_names_both(self):
        page, key = build(LINES, TRANSLATIONS, seed=3)
        second = shown_data(page)[1]
        self.assertEqual(sorted(c["text"] for c in second["choices"]), ["I already finished that book.", "That book is already read all."])
        shared = letter_of(key, 2, "model one")
        self.assertEqual(sorted(key["lines"][1]["letters"][shared]), ["model one", "model two"])
        self.assertNotEqual(letter_of(key, 2, "model three"), shared)
        self.assertIsNone(key["lines"][1]["no_answer"])

    def test_a_line_a_candidate_failed_on_is_shown_as_no_answer_and_counted_apart_not_as_wrong(self):
        page, key = build(LINES, TRANSLATIONS, seed=3)
        third = shown_data(page)[2]
        none = key["lines"][2]["no_answer"]
        self.assertEqual(sorted(key["lines"][2]["letters"][none]), ["model three", "model two"])
        self.assertEqual({c["letter"]: c["text"] for c in third["choices"]}, {none: None, letter_of(key, 3, "model one"): "Shall we have lunch together tomorrow?"})
        self.assertIn("(no answer)", page)
        # Even a mark that somehow sits on the "(no answer)" letter does not turn it into a wrong translation.
        tally = score({"stamp": key["stamp"], "marks": {f"3|{none}": "wrong"}}, key)
        self.assertEqual(tally["model two"], {"right": 0, "wrong": 0, "not_marked": 2, "no_answer": 1})

    def test_marks_are_counted_for_every_candidate_behind_each_lines_letter(self):
        _, key = build(LINES, TRANSLATIONS, seed=3)
        marks = {
            f"1|{letter_of(key, 1, 'model one')}": "right",
            f"1|{letter_of(key, 1, 'model three')}": "wrong",
            f"2|{letter_of(key, 2, 'model two')}": "right",
            f"3|{letter_of(key, 3, 'model one')}": "right",
        }
        self.assertEqual(score({"stamp": key["stamp"], "marks": marks}, key), {
            "model one": {"right": 3, "wrong": 0, "not_marked": 0, "no_answer": 0},
            "model three": {"right": 0, "wrong": 1, "not_marked": 1, "no_answer": 1},
            "model two": {"right": 1, "wrong": 0, "not_marked": 1, "no_answer": 1},
        })

    def test_marks_saved_from_another_page_are_refused(self):
        _, key = build(LINES, TRANSLATIONS, seed=3)
        _, other = build(LINES, {**TRANSLATIONS, "model one": ["Another answer.", "Another.", "Another one."]}, seed=3)
        self.assertNotEqual(key["stamp"], other["stamp"], "the stamp follows what the page shows, not only the seed")
        with self.assertRaises(ValueError):
            score({"stamp": other["stamp"], "marks": {}}, key)

    def test_a_failed_request_and_an_empty_reply_are_both_no_answer(self):
        records = [record(1, "  I stayed home\ntoday.  "), record(2, "", error="HTTP 500"), record(3, "   ")]
        self.assertEqual(answers(records, LINES), ["I stayed home today.", None, None])
        self.assertEqual(answers([record(1, "Only the first line came back.")], LINES), ["Only the first line came back.", None, None], "a line with no record has no answer")

    def test_only_the_first_pass_over_the_lines_is_shown(self):
        records = [record(1, "First pass."), record(1, "Second pass.", run=2)]
        self.assertEqual(answers(records, LINES)[0], "First pass.")

    def test_results_made_from_other_lines_are_refused(self):
        with self.assertRaises(ValueError):
            answers([record(1, "Hello.", source="안녕하세요")], LINES)

    def test_the_commands_write_the_page_and_the_key_and_print_each_candidates_marks(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = lambda name: os.path.join(tmp, name)
            write(path("lines.txt"), "\n".join(LINES) + "\n\n")
            os.mkdir(path("one"))
            for name, folder in (("model one", path("one")), ("model two", tmp)):
                rows = [record(n, t or "", error=None if t else "HTTP 429") for n, t in enumerate(TRANSLATIONS[name], 1)]
                write(os.path.join(folder, "live-bench.jsonl"), "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                # A candidate is given as its bench folder or as the records file itself.
                main(["build", path("lines.txt"), "--candidate", f"model one={path('one')}", f"model two={path('live-bench.jsonl')}", "--out", path("review.html"), "--key", path("key.json"), "--seed", "3"])
            self.assertIn("model two: 2 of 3 lines answered", printed.getvalue())
            key = read_json(path("key.json"))
            with open(path("review.html"), encoding="utf-8") as f:
                page = f.read()
            self.assertEqual((page, key), build(LINES, {name: TRANSLATIONS[name] for name in ("model one", "model two")}, seed=3))
            write(path("marks.json"), json.dumps({"stamp": key["stamp"], "marks": {f"1|{letter_of(key, 1, 'model two')}": "wrong"}}))
            printed = io.StringIO()
            with contextlib.redirect_stdout(printed):
                main(["score", path("marks.json"), path("key.json")])
            self.assertEqual(printed.getvalue().splitlines(), ["model one: 0 right, 0 wrong, 3 not marked, 0 no answer", "model two: 0 right, 1 wrong, 1 not marked, 1 no answer"])
            with self.assertRaises(SystemExit):
                main(["build", path("lines.txt"), "--candidate", f"model one={path('missing')}", "--out", path("x.html"), "--key", path("x.json")])


if __name__ == "__main__":
    unittest.main()
