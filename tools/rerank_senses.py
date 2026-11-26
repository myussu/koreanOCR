"""Choose each word's dictionary sense with a small multilingual reranker on the CPU, no language model.

A candidate for the card-meaning comparison (`maru bench --cards`): for every word with several senses in the
learner's dictionary, the reranker scores each sense against the line and the best one is taken. Words with one
sense take it; words the dictionary lacks get nothing (a reranker cannot write a meaning).

Input: the tasks file made by `maru dict senses`. Output: one record per line in the format `maru bench --cards`
writes, so `tools/build_card_review.py` can put it beside the models.

Usage: python tools/rerank_senses.py tasks.jsonl cards-reranker.jsonl [--model Alibaba-NLP/gte-multilingual-reranker-base]
Needs `torch` (CPU) and `sentence-transformers`; the model (about 1.2 GB) is downloaded on first use.
"""
import argparse
import json
import time

DEFAULT_MODEL = "Alibaba-NLP/gte-multilingual-reranker-base"


def query(line, word):
    """What each sense is scored against: the line, and which of its words is meant."""
    return f"{line}\n{word['surface']} ({word['lemma']})"


def passage(sense):
    return f"{sense['en']}: {sense['definition']}" if sense.get("definition") else sense["en"]


def pairs(line, word):
    return [(query(line, word), passage(s)) for s in word["senses"]]


def decide(task, score):
    """`score(pairs) -> [float]`. The record's words: one sense as it is, several by the best score, none left out."""
    words = []
    for w in task["words"]:
        senses = w["senses"]
        if len(senses) == 1:
            words.append({"index": w["index"], "en": senses[0]["en"], "es": senses[0]["es"], "source": "one_sense"})
        elif len(senses) > 1:
            scores = list(score(pairs(task["line"], w)))
            best = max(range(len(senses)), key=lambda n: scores[n])
            words.append({"index": w["index"], "en": senses[best]["en"], "es": senses[best]["es"], "source": "reranked"})
    return words


def main():
    args = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    args.add_argument("tasks")
    args.add_argument("out")
    args.add_argument("--model", default=DEFAULT_MODEL)
    args = args.parse_args()

    from sentence_transformers import CrossEncoder  # imported here: the tests need no model

    started = time.perf_counter()
    # No `trust_remote_code`: only models that load with the library's own code are used.
    model = CrossEncoder(args.model, device="cpu")
    load_s = time.perf_counter() - started
    tasks = [json.loads(line) for line in open(args.tasks, encoding="utf-8") if line.strip()]
    label = f"dictionary + reranker · {args.model}"
    times = []
    with open(args.out, "w", encoding="utf-8", newline="\n") as out:
        for n, task in enumerate(tasks, 1):
            asked = any(len(w["senses"]) > 1 for w in task["words"])
            t = time.perf_counter()
            words = decide(task, lambda p: model.predict(p))
            ms = round((time.perf_counter() - t) * 1000)
            if asked:
                times.append(ms)
            record = {"candidate": label, "line": n, "asked": asked, "ms": ms if asked else 0, "error": None, "tokens_in": 0, "tokens_out": 0, "words": words}
            out.write(json.dumps(record, ensure_ascii=False) + "\n")
    times.sort()
    middle = times[len(times) // 2] if times else 0
    slow = times[min(len(times) - 1, int(len(times) * 0.95))] if times else 0
    print(f"{label}: {len(tasks)} lines, {len(times)} scored; model loaded in {load_s:.1f} s; per line p50 {middle} ms, p95 {slow} ms -> {args.out}")


if __name__ == "__main__":
    main()
