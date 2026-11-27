import io
import json
import tempfile
import unittest
from pathlib import Path

import scan_lines as s


class ScanLinesTest(unittest.TestCase):
    def test_a_line_is_agreed_when_a_local_reader_read_its_letters_anywhere_on_the_screen(self):
        outside = "정령 칼날\n새로운 능력을 배웠습니다!\n누님…\nA"
        scan = "유다희 게임월드\n정령칼날 새로운 능력을 배웠습니다"
        self.assertEqual(s.agreed_lines(outside, [scan, None]), ["정령 칼날", "새로운 능력을 배웠습니다!"])

    def test_short_lines_and_lines_nobody_else_read_are_left_for_a_person(self):
        self.assertEqual(s.agreed_lines("네\n대체 어디 계십니까…", ["네 대체 어디 계십니가"]), [])

    def test_labelling_writes_only_the_agreed_lines_and_leaves_person_labels_alone(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d) / "game"
            root.mkdir()
            for name in ("a", "b", "c"):
                (root / f"{name}.png").write_bytes(b"x")
            (root / "a.outside.txt").write_text("정령 칼날\n조용한 톱니\n", encoding="utf-8")
            (root / "a.scan.txt").write_text("정령 칼날\n", encoding="utf-8")
            (root / "b.outside.txt").write_text("누님…\n", encoding="utf-8")
            (root / "b.hybrid.txt").write_text("누남\n", encoding="utf-8")
            (root / "c.txt").write_text("사람이 쓴 것\n", encoding="utf-8")
            self.assertEqual(s.label(d), (1, 1, 1))
            self.assertEqual((root / "a.provisional.txt").read_text(encoding="utf-8"), "정령 칼날\n")
            self.assertFalse((root / "b.provisional.txt").exists())
            self.assertFalse((root / "c.provisional.txt").exists(), "a person's label wins")

    def test_a_line_is_scored_against_the_closest_stretch_of_what_was_read(self):
        self.assertEqual(s.line_error("정령 칼날", "체력 100 정령칼날 새로운"), 0.0)
        self.assertAlmostEqual(s.line_error("대체 어디 계십니까", "누님 대체 어디 계십니가"), 1 / 8)
        self.assertEqual(s.line_error("정령 칼날", ""), 1.0)

    def test_scoring_compares_runs_line_by_line(self):
        def run(name, got):
            pictures = [{"rel": "g/a.png", "truth": "정령 칼날\n조용한 톱니", "got": got, "ms": 100, "split": "dev", "tags": ["provisional"]}]
            return {"engines": [{"engine": name, "pictures": pictures}], "labels": "all"}

        with tempfile.TemporaryDirectory() as d:
            a, b = Path(d) / "a.json", Path(d) / "b.json"
            a.write_text(json.dumps(run("today", "정령 칼날 조용한 톱미")), encoding="utf-8")
            b.write_text(json.dumps(run("new", "정령 칼날 조용한 톱니")), encoding="utf-8")
            out = io.StringIO()
            s.score([a, b], out=out)
            text = out.getvalue()
        self.assertIn("PROVISIONAL", text)
        self.assertIn("| today | 2 | 100.0% | 50.0% |", text)
        self.assertIn("| new | 1 | 0 | 1 | 1.000 |", text)

    def test_the_sign_test_is_two_sided_and_exact(self):
        self.assertAlmostEqual(s.sign_test(5, 0), 0.0625)
        self.assertEqual(s.sign_test(0, 0), 1.0)
        self.assertAlmostEqual(s.sign_test(16, 4), 0.01181793212890625)


if __name__ == "__main__":
    unittest.main()
