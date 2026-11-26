"""Tests for compare_ocr.py: which engines run, how they are named on disk and which tables are printed."""
import tempfile
import unittest
from pathlib import Path

import compare_ocr as co


class Candidates(unittest.TestCase):
    def test_todays_setup_is_first_and_names_make_distinct_file_names(self):
        self.assertTrue(co.CANDIDATES[0][0].startswith("today"))
        self.assertEqual(co.CANDIDATES[0][2], [], "today's setup needs nothing beyond what Maru installs")
        slugs = [co.slug(name) for name, _, _ in co.CANDIDATES]
        self.assertEqual(len(set(slugs)), len(slugs))
        self.assertEqual(co.slug("hybrid, reader GLM-OCR"), "hybrid-reader-glm-ocr")

    def test_a_candidate_whose_files_are_missing_is_skipped_and_named(self):
        with tempfile.TemporaryDirectory() as tmp:
            models = Path(tmp)
            (models / "ocr-candidates" / "pp-ocrv5-mobile-det").mkdir(parents=True)
            (models / "ocr-candidates" / "pp-ocrv5-mobile-det" / "det.onnx").write_bytes(b"x")
            usable, skipped = co.available(co.CANDIDATES, models)
            names = [n for n, _ in usable]
            self.assertIn("PP-OCR alone, v5 mobile detector", names)
            self.assertIn("hybrid, v5 mobile detector", names)
            self.assertNotIn("hybrid, reader PaddleOCR-VL-1.6", names)
            self.assertIn(("hybrid, reader PaddleOCR-VL-1.6", ["vlm/paddleocr-vl-1.6"]), skipped)
            det = dict(usable)["PP-OCR alone, v5 mobile detector"]
            self.assertEqual(det[det.index("--det") + 1], str(models) + "/ocr-candidates/pp-ocrv5-mobile-det/det.onnx", "the models folder is filled in")

    def test_only_narrows_the_engines_by_name(self):
        with tempfile.TemporaryDirectory() as tmp:
            usable, _ = co.available(co.CANDIDATES, Path(tmp), only=["today", "enlarged"])
            self.assertEqual([n for n, _ in usable], ["today: hybrid (PP-OCR + PaddleOCR-VL-1.5)", "hybrid, enlarged 2x", "PP-OCR alone, enlarged 2x"])

    def test_every_candidate_names_one_engine(self):
        for name, args, _ in co.CANDIDATES:
            self.assertEqual(args.count("--engine"), 1, name)
            self.assertNotIn("--name", args)


class Commands(unittest.TestCase):
    def test_a_run_is_named_scored_against_the_asked_labels_and_saved(self):
        cmd = co.eval_command("maru", "H", "set", "today", ["--engine", "hybrid"], "all", "out/today.json")
        self.assertEqual(cmd, ["maru", "--home", "H", "eval-ocr", "set", "--engine", "hybrid", "--labels", "all", "--name", "today", "--save-report", "out/today.json"])
        self.assertNotIn("--home", co.eval_command("maru", None, "set", "today", [], "person", "r.json"))

    def test_a_persons_labels_are_shown_whole_and_by_half_and_provisional_ones_always_apart_and_marked(self):
        person = co.table_views("person")
        self.assertEqual([v for _, v in person], [[], ["--split", "dev"], ["--split", "held-out"]])
        both = co.table_views("all")
        self.assertEqual(both[0][1], ["--tag", "gold"])
        self.assertTrue(all("PROVISIONAL" in heading for heading, view in both if "provisional" in view))
        self.assertEqual(len(both), 3)
        self.assertTrue(all("PROVISIONAL" in heading for heading, _ in co.table_views("provisional")))


if __name__ == "__main__":
    unittest.main()
