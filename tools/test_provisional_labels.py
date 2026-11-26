"""Tests for provisional_labels.py: the agreement rule and what it leaves on disk."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import provisional_labels as pl

TOOL = Path(__file__).with_name("provisional_labels.py")


def run(*args):
    return subprocess.run([sys.executable, str(TOOL), *map(str, args)], capture_output=True, text=True, encoding="utf-8", check=True).stdout


class Rule(unittest.TestCase):
    def test_the_outside_reader_and_one_local_reader_agreeing_on_the_letters_is_a_label(self):
        self.assertEqual(agreed := pl.agreed("저게 뭐야?", ["저게뭐야", "저게 뮈야"]), "저게 뭐야?")
        self.assertEqual(pl.letters(agreed), "저게뭐야")

    def test_spaces_punctuation_and_line_breaks_do_not_count_but_a_letter_or_a_digit_does(self):
        self.assertEqual(pl.agreed("…고마워\n나를 데리고 와줘서", ["고마워 나를 데리고 와줘서"]), "…고마워 나를 데리고 와줘서")
        self.assertIsNone(pl.agreed("제47구가 봉쇄됐습니다", ["제41구가 봉쇄됐습니다"]))
        self.assertIsNone(pl.agreed("가지 않았어", ["가지 았어"]))

    def test_no_agreement_without_the_outside_reader_or_when_nothing_was_read(self):
        self.assertIsNone(pl.agreed("안녕", ["안넝", "안냥"]), "the local readers agreeing with each other is not enough")
        self.assertIsNone(pl.agreed("", [""]), "two readers seeing nothing is not a label")
        self.assertIsNone(pl.agreed("...", ["…"]))
        self.assertIsNone(pl.agreed(None, ["안녕"]))
        self.assertIsNone(pl.agreed("안녕", [None, None]))


class OnDisk(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "set"
        (self.root / "game").mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def picture(self, name, outside=None, hybrid=None, paddle=None, label=None):
        base = self.root / "game" / name
        base.with_suffix(".png").write_bytes(b"")
        for tag, text in (("outside", outside), ("hybrid", hybrid), ("paddle", paddle)):
            if text is not None:
                (self.root / "game" / f"{name}.{tag}.txt").write_text(text + "\n", encoding="utf-8")
        if label is not None:
            base.with_suffix(".txt").write_text(label, encoding="utf-8")
        return base

    def test_labels_are_written_only_where_the_readers_agree_and_never_over_a_persons(self):
        agree = self.picture("a00_s001_0", outside="잘 가.", hybrid="잘가", paddle="잘 가")
        differ = self.picture("a00_s002_0", outside="못 가", hybrid="옷 가", paddle="몬 가")
        person = self.picture("a00_s003_0", outside="이리 와", hybrid="이리 와", label="이리 와!")
        unread = self.picture("a00_s004_0", hybrid="아직", paddle="아직")
        out = run(self.root, "--outside", "outside", "--local", "hybrid", "paddle")
        self.assertEqual(agree.with_name("a00_s001_0.provisional.txt").read_text(encoding="utf-8"), "잘 가.\n")
        self.assertFalse(differ.with_name("a00_s002_0.provisional.txt").exists())
        self.assertFalse(person.with_name("a00_s003_0.provisional.txt").exists())
        self.assertFalse(unread.with_name("a00_s004_0.provisional.txt").exists())
        self.assertIn("1 provisional labels written; 1 pictures wait", out)
        self.assertIn("1 pictures have no reading from the outside reader yet", out)

    def test_a_label_the_drafts_no_longer_support_is_taken_away_again(self):
        p = self.picture("a00_s001_0", outside="잘 가", hybrid="잘 가")
        run(self.root, "--outside", "outside", "--local", "hybrid")
        (self.root / "game" / "a00_s001_0.hybrid.txt").write_text("갈 가\n", encoding="utf-8")
        run(self.root, "--outside", "outside", "--local", "hybrid")
        self.assertFalse(p.with_name("a00_s001_0.provisional.txt").exists())

    def test_pictures_without_korean_and_repeats_can_be_moved_out_of_the_set(self):
        self.picture("a00_s001_0", outside="잘 가", hybrid="잘 가")
        self.picture("a00_s002_0", outside="", hybrid="", paddle="OTK")
        self.picture("a00_s003_0", outside="잘 가.", hybrid="잘 가")  # the same subtitle again
        self.picture("a00_s004_0", outside="", hybrid="", paddle="뭐")  # one reader saw Korean: a person decides
        self.picture("a00_s000_0", outside="이리 와", hybrid="이리 와", label="이리 와!")
        self.picture("a00_s005_0", outside="이리 와", hybrid="이리 와")  # a subtitle a person already labelled
        aside = Path(self.tmp.name) / "aside"
        out = run(self.root, "--outside", "outside", "--local", "hybrid", "paddle", "--set-aside", aside)
        left = sorted(p.name for p in (self.root / "game").glob("*.png"))
        self.assertEqual(left, ["a00_s000_0.png", "a00_s001_0.png", "a00_s004_0.png"])
        self.assertTrue((aside / "no-korean" / "game" / "a00_s002_0.paddle.txt").exists(), "its drafts go with it")
        self.assertTrue((aside / "repeats" / "game" / "a00_s003_0.png").exists())
        self.assertTrue((aside / "repeats" / "game" / "a00_s005_0.png").exists())
        self.assertIn("1 with no Korean in any reading, 2 repeats", out)

    def test_the_check_says_how_often_the_rule_would_have_been_wrong_on_labelled_pictures(self):
        self.picture("a00_s001_0", outside="빛바랜 자여", hybrid="빛바랜 자여", label="빛바랜 자여")
        self.picture("a00_s002_0", outside="그 힘을 나에게", hybrid="그 힘을 나에게", label="그 힘을, 나에게…")
        self.picture("a00_s003_0", outside="왕의 자리애", hybrid="왕의 자리애", label="왕의 자리에")  # both wrong the same way
        self.picture("a00_s004_0", outside="다시 한번", hybrid="다시 한빈", label="다시 한번")  # no agreement
        out = run(self.root, "--outside", "outside", "--local", "hybrid", "--check")
        self.assertIn("4 pictures have a person's label; the rule would have given 3 of them a provisional label", out)
        self.assertIn("1 of those 3 provisional labels differ", out)
        self.assertIn("왕의 자리애", out)
        self.assertFalse(list(self.root.rglob("*.provisional.txt")), "a check writes nothing")


if __name__ == "__main__":
    unittest.main()
