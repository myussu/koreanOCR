import datetime
import json
import tempfile
import unittest
from pathlib import Path

from build_nikl import main, plain_form


def entry(form, homonym, pos, level, senses, unit="단어"):
    body = "".join(
        f'<Sense att="id" val="{n}"><feat att="definition" val="뜻" />'
        + "".join(
            f'<Equivalent><feat att="language" val="{lang}" /><feat att="lemma" val="{lemma}" /><feat att="definition" val="{definition}" /></Equivalent>'
            for lang, lemma, definition in sense
        )
        + "</Sense>"
        for n, sense in enumerate(senses, 1)
    )
    return (
        f'<LexicalEntry att="id" val="1"><feat att="homonym_number" val="{homonym}" /><feat att="lexicalUnit" val="{unit}" />'
        f'<feat att="partOfSpeech" val="{pos}" /><Lemma><feat att="writtenForm" val="{form}" /></Lemma>'
        f'<feat att="vocabularyLevel" val="{level}" />{body}</LexicalEntry>'
    )


class BuildNiklTests(unittest.TestCase):
    def build(self, *entries):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        src = Path(self.dir.name) / "krdict"
        src.mkdir()
        (src / "5000.xml").write_text("<LexicalResource><Lexicon>" + "".join(entries) + "</Lexicon></LexicalResource>", encoding="utf-8")
        out = Path(self.dir.name) / "nikl.jsonl"
        counts = main(src, out, today=datetime.date(2026, 11, 25))
        return counts, [json.loads(line) for line in out.read_text(encoding="utf-8").splitlines()]

    def test_a_word_keeps_its_english_and_spanish_per_sense_and_the_file_says_where_it_comes_from(self):
        belly = [("영어", "belly; stomach", "The part of the body."), ("스페인어", "barriga", "Parte del cuerpo."), ("일본어", "腹", "")]
        counts, lines = self.build(entry("배", 1, "명사", "초급", [belly, [("영어", "womb", "")]]))
        self.assertEqual(counts, (1, 2))
        self.assertIn("CC BY-SA 2.0 KR", lines[0]["meta"]["license"])
        self.assertIn("krdict.korean.go.kr", lines[0]["meta"]["source"])
        self.assertEqual(lines[0]["meta"]["built"], "2026-11-25")
        self.assertEqual(lines[1], {
            "w": "배", "h": 1, "p": "명사", "l": "초급",
            "s": [{"en": "belly; stomach", "es": "barriga", "end": "The part of the body.", "esd": "Parte del cuerpo."}, {"en": "womb"}],
        })

    def test_idioms_and_words_without_an_english_or_spanish_meaning_are_left_out(self):
        _, lines = self.build(
            entry("발이 넓다", 0, "", "없음", [[("영어", "have wide feet", "")]], unit="관용구"),
            entry("가", 0, "조사", "없음", [[("일본어", "が", "")]]),
            entry("공부-하다", 0, "동사", "없음", [[("스페인어", "estudiar", "")]]),
        )
        self.assertEqual([line["w"] for line in lines[1:]], ["공부하다"], "the headword is written as in a sentence")
        self.assertEqual(lines[1]["l"], "", "no level rather than the dictionary's word for none")

    def test_a_value_with_a_bare_angle_bracket_does_not_stop_the_reading(self):
        _, lines = self.build(entry("작다", 0, "형용사", "초급", [[("영어", "small", "a < b")]]))
        self.assertEqual(lines[1]["s"][0]["end"], "a < b")
        self.assertEqual(plain_form("눈^사람"), "눈사람")


if __name__ == "__main__":
    unittest.main()
