import unittest

from build_card_review import assign, build, score
from rerank_senses import decide, pairs


def sense(en, es, definition=""):
    return {"en": en, "es": es, "definition": definition}


TASKS = [
    {"line": "배가 고파요", "words": [
        {"index": 0, "surface": "배가", "lemma": "배", "pos": "noun", "senses": [sense("belly", "barriga", "Part of the body."), sense("ship", "barco", "A vehicle on water."), sense("pear", "pera")]},
        {"index": 1, "surface": "고파요", "lemma": "고프다", "pos": "adjective", "senses": [sense("hungry", "hambriento")]},
    ]},
    {"line": "억수로 좋아", "words": [{"index": 0, "surface": "억수로", "lemma": "억수로", "pos": "adverb", "senses": []}]},
]


def record(candidate, line, words, error=None):
    return {"candidate": candidate, "line": line, "asked": True, "ms": 5, "error": error, "tokens_in": 0, "tokens_out": 0, "words": words}


RECORDS = [
    record("model one", 1, [{"index": 0, "en": "belly", "es": "barriga", "source": "chosen"}, {"index": 1, "en": "hungry", "es": "hambriento", "source": "one_sense"}]),
    record("model one", 2, [{"index": 0, "en": "extremely", "es": "muchísimo", "source": "model"}]),
    record("model two", 1, [{"index": 0, "en": "belly", "es": "barriga", "source": "first_sense"}, {"index": 1, "en": "hungry", "es": "hambriento", "source": "one_sense"}], error="HTTP 429"),
    record("model two", 2, []),
    record("dictionary", 1, [{"index": 0, "en": "ship", "es": "barco", "source": "first_sense"}, {"index": 1, "en": "hungry", "es": "hambriento", "source": "one_sense"}]),
    record("dictionary", 2, []),
]
CANDIDATES = ["model one", "model two", "dictionary"]


class CardReviewTests(unittest.TestCase):
    def test_every_line_has_its_own_order_of_letters_and_the_same_seed_gives_the_same_page(self):
        key = assign(CANDIDATES, 40, seed=7)
        self.assertTrue(all(sorted(line["letters"].values()) == sorted(CANDIDATES) for line in key))
        self.assertGreater(len({tuple(line["letters"].values()) for line in key}), 1, "the order changes between lines")
        self.assertEqual(key, assign(CANDIDATES, 40, seed=7))
        self.assertNotEqual(key, assign(CANDIDATES, 40, seed=8))

    def test_the_page_names_no_candidate_and_the_key_is_separate(self):
        page, key = build(TASKS, RECORDS, CANDIDATES, seed=3, translations={"배가 고파요": "I am hungry."})
        # The page's own wording may say "dictionary"; what it shows per line must not name who wrote it.
        shown = page.split("const DATA = ", 1)[1].split("const STORE", 1)[0]
        for name in CANDIDATES:
            self.assertFalse(name in shown, f"{name} is named in the page's data")
        self.assertEqual(key["candidates"], sorted(CANDIDATES))
        self.assertEqual([line["line"] for line in key["lines"]], [1, 2])
        self.assertIn("I am hungry.", page)
        self.assertIn("belly · barriga", page)
        self.assertIn("extremely · muchísimo", page)

    def test_a_failed_request_shows_no_answer_rather_than_the_fallback_and_one_sense_words_are_not_judged(self):
        page, key = build(TASKS, RECORDS, CANDIDATES, seed=3)
        data = page.split("const DATA = ", 1)[1].split(";\nconst STORE", 1)[0]
        import json
        lines = json.loads(data)
        first = lines[0]
        self.assertEqual([w["lemma"] for w in first["words"]], ["배"], "고프다 has one sense: nothing to judge")
        self.assertEqual(first["settled"], [{"lemma": "고프다", "meaning": "hungry · hambriento"}])
        letter_of = {name: letter for letter, name in key["lines"][0]["letters"].items()}
        cells = first["words"][0]["cells"]
        self.assertEqual(cells[letter_of["model one"]], "belly · barriga")
        self.assertIsNone(cells[letter_of["model two"]], "its request failed: the first sense was not its choice")
        self.assertEqual(cells[letter_of["dictionary"]], "ship · barco")
        second = lines[1]["words"][0]["cells"]
        letter_of = {name: letter for letter, name in key["lines"][1]["letters"].items()}
        self.assertIsNone(second[letter_of["dictionary"]], "not in the dictionary: no meaning without a model")

    def test_marks_are_counted_for_the_candidate_behind_each_lines_letter(self):
        _, key = build(TASKS, RECORDS, CANDIDATES, seed=3)
        letter = lambda line, name: next(l for l, n in key["lines"][line - 1]["letters"].items() if n == name)
        marks = {f"1|0|{letter(1, 'model one')}": "right", f"1|0|{letter(1, 'dictionary')}": "wrong", f"2|0|{letter(2, 'model one')}": "right"}
        self.assertEqual(score(marks, key), {"dictionary": {"right": 0, "wrong": 1}, "model one": {"right": 2, "wrong": 0}, "model two": {"right": 0, "wrong": 0}})


class RerankTests(unittest.TestCase):
    def test_every_sense_is_scored_against_the_line_and_the_best_is_taken(self):
        seen = []

        def scorer(batch):
            seen.append(batch)
            return [0.1, 0.9, 0.3]

        words = decide(TASKS[0], scorer)
        self.assertEqual(words, [{"index": 0, "en": "ship", "es": "barco", "source": "reranked"}, {"index": 1, "en": "hungry", "es": "hambriento", "source": "one_sense"}])
        self.assertEqual(len(seen), 1, "a word with one sense is not scored")
        self.assertEqual(seen[0][0], ("배가 고파요\n배가 (배)", "belly: Part of the body."))
        self.assertEqual(pairs("배가 고파요", TASKS[0]["words"][0])[2][1], "pear", "a sense without a definition is its equivalent alone")

    def test_a_word_the_dictionary_lacks_gets_no_meaning(self):
        self.assertEqual(decide(TASKS[1], lambda batch: self.fail("nothing to score")), [])


if __name__ == "__main__":
    unittest.main()
