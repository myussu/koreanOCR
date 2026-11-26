"""Turn the 한국어기초사전 (Basic Korean Dictionary) into the file `maru dict nikl` imports.

Source: 국립국어원 «한국어기초사전», https://krdict.korean.go.kr, licence CC BY-SA 2.0 KR
(https://creativecommons.org/licenses/by-sa/2.0/kr/). Input: the LMF XML files from the site's
«사전 내려받기», or the 2019-08-02 backup at https://github.com/spellcheck-ko/korean-dict-nikl-krdict
(same format). The data is not part of this repository; the file made here carries the same licence.

It keeps what a word card needs: every word (단어) with its homonym number, part of speech and level,
and for each sense the dictionary's English and Spanish equivalents and definitions, as they come.

Output, JSON lines:
  {"meta": {"source": ..., "license": ..., "data": ..., "built": ...}}
  {"w": "배", "h": 1, "p": "명사", "l": "초급", "s": [{"en": "belly", "es": "barriga", "end": "...", "esd": "..."}]}

Usage: python tools/build_nikl.py <folder with the .xml files> <out.jsonl>
Then:  maru dict nikl <out.jsonl>

The XML reading follows tools/build_dictionary.py of the koreanDiary project (same author).
"""
import datetime
import io
import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

SOURCE = "국립국어원 한국어기초사전 (National Institute of Korean Language, Basic Korean Dictionary), https://krdict.korean.go.kr"
LICENSE = "CC BY-SA 2.0 KR, https://creativecommons.org/licenses/by-sa/2.0/kr/"
ATTR = re.compile(r'val="([^"]*)"')


def clean_xml(text):
    # Some values carry an unescaped "<" (a French translation).
    return ATTR.sub(lambda m: 'val="' + m.group(1).replace("<", "&lt;").replace(">", "&gt;") + '"', text)


def feats(el):
    out = {}
    for f in el.findall("feat"):
        out.setdefault(f.get("att"), []).append(f.get("val"))
    return out


def first(d, k):
    v = d.get(k)
    return v[0] if v else None


def tidy(s):
    return re.sub(r"\s+", " ", s or "").strip()


def plain_form(form):
    """The headword as it is written in a sentence: 공부-하다 → 공부하다."""
    return re.sub(r"[-^ ]", "", form or "")


def equivalent(sense_el, language):
    for eq in sense_el.findall("Equivalent"):
        e = feats(eq)
        if first(e, "language") == language:
            return tidy(first(e, "lemma")), tidy(first(e, "definition"))
    return "", ""


def entries(src):
    files = sorted(Path(src).glob("*.xml"), key=lambda p: int(re.sub(r"\D", "", p.stem) or 0))
    for path in files:
        data = io.BytesIO(clean_xml(path.read_text(encoding="utf-8")).encode("utf-8"))
        for _, el in ET.iterparse(data, events=("end",)):
            if el.tag == "LexicalEntry":
                yield el
                el.clear()


def records(src):
    """One record per word that has at least one sense with an English or Spanish equivalent."""
    for el in entries(src):
        top = feats(el)
        lemma = el.find("Lemma")
        form = plain_form(first(feats(lemma), "writtenForm")) if lemma is not None else ""
        if first(top, "lexicalUnit") != "단어" or not form:
            continue
        senses = []
        for s in el.findall("Sense"):
            en, en_def = equivalent(s, "영어")
            es, es_def = equivalent(s, "스페인어")
            if en or es:
                sense = {"en": en, "es": es, "end": en_def, "esd": es_def}
                senses.append({k: v for k, v in sense.items() if v})
        if not senses:
            continue
        level = first(top, "vocabularyLevel") or ""
        yield {
            "w": form,
            "h": int(first(top, "homonym_number") or 0),
            "p": first(top, "partOfSpeech") or "",
            "l": "" if level == "없음" else level,
            "s": senses,
        }


def main(src, out, today=None):
    meta = {
        "source": SOURCE,
        "license": LICENSE,
        "data": f"LMF XML in {Path(src).name}",
        "built": (today or datetime.date.today()).isoformat(),
        "changes": "words only; per sense the English and Spanish equivalents and definitions; headwords without hyphens",
    }
    words = senses = 0
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps({"meta": meta}, ensure_ascii=False) + "\n")
        for record in records(src):
            words += 1
            senses += len(record["s"])
            f.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n")
    print(f"{words} words, {senses} senses -> {out}")
    return words, senses


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
