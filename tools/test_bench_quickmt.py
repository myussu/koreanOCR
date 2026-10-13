import json
import unittest
from types import SimpleNamespace
from unittest.mock import Mock

from bench_quickmt import distribution, load_cases, measure


class Tokenizer:
    def encode(self, text, out_type):
        return [text]

    def decode(self, pieces):
        return "".join(p for p in pieces if p != "</s>")


class QuickMtTests(unittest.TestCase):
    def test_source_eos_and_full_input_and_streamed_first_text_are_preserved(self):
        class Translator:
            def translate_batch(self, tokens, **kwargs):
                self.tokens, self.kwargs = tokens, kwargs
                kwargs["callback"](SimpleNamespace(token="Don't "))
                kwargs["callback"](SimpleNamespace(token="go."))
                return [SimpleNamespace(hypotheses=[["Don't ", "go.", "</s>"]])]
        translator = Translator()
        result = measure(translator, Tokenizer(), Tokenizer(), "가지 마세요.", 1)
        self.assertEqual(translator.tokens, [["가지 마세요.", "</s>"]])
        self.assertEqual(translator.kwargs["max_input_length"], 0)
        self.assertEqual(result["translation"], "Don't go.")
        self.assertIsNone(result["error"])
        self.assertLessEqual(result["first_text_ms"], result["done_ms"])

    def test_beam_search_has_no_invented_first_text_and_capped_output_is_an_error(self):
        class Translator:
            def translate_batch(self, tokens, **kwargs):
                assert kwargs["callback"] is None
                return [SimpleNamespace(hypotheses=[["x"] * kwargs["max_decoding_length"]])]
        result = measure(Translator(), Tokenizer(), Tokenizer(), "네", 5)
        self.assertIsNone(result["first_text_ms"])
        self.assertIn("output cap", result["error"])
        self.assertTrue(result["translation"])

    def test_empty_answers_and_exceptions_are_preserved_as_errors(self):
        class Translator:
            def translate_batch(self, *args, **kwargs):
                return [SimpleNamespace(hypotheses=[["</s>"]])]
        self.assertIsNotNone(measure(Translator(), Tokenizer(), Tokenizer(), "네", 2)["error"])
        class Broken:
            def translate_batch(self, *args, **kwargs):
                raise RuntimeError("model failed")
        self.assertIn("model failed", measure(Broken(), Tokenizer(), Tokenizer(), "네", 2)["error"])

    def test_duplicate_corpus_ids_are_refused_and_percentiles_use_nearest_rank(self):
        row = dict(id="a", scene="scene", source="네", reference="Yes", origin="test")
        path = Mock()
        path.read_text.return_value = json.dumps(row) + "\n" + json.dumps(row)
        with self.assertRaises(ValueError):
            load_cases(path)
        self.assertEqual(distribution([30, 10, 20])["p50_ms"], 20)
        self.assertIsNone(distribution([]))


if __name__ == "__main__":
    unittest.main()
