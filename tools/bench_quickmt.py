"""Model-only QuickMT CPU diagnostic. Requires approved local files and isolated Python dependencies."""
import argparse
import hashlib
import html
import json
import math
import platform
import time
from pathlib import Path


def load_cases(path):
    cases = [json.loads(line) for line in path.read_text(encoding="utf-8-sig").splitlines() if line.strip()]
    ids = set()
    for case in cases:
        if any(not case.get(key, "").strip() for key in ("id", "scene", "source", "reference", "origin")):
            raise ValueError("Corpus has an empty required field")
        if case["id"] in ids:
            raise ValueError("Corpus has duplicate IDs")
        ids.add(case["id"])
    if not cases:
        raise ValueError("Corpus is empty")
    return cases


def distribution(values):
    values = sorted(values)
    if not values:
        return None
    return dict(n=len(values), min_ms=values[0], p50_ms=values[math.ceil(len(values) * .5) - 1],
                p95_ms=values[math.ceil(len(values) * .95) - 1], max_ms=values[-1])


def measure(translator, source_tokenizer, target_tokenizer, source, beam):
    started_unix_ms = time.time_ns() // 1_000_000
    started = time.perf_counter()
    first = None
    pieces = []
    translation = ""
    tokens = []
    cap = min(len(source) * 2 + 40, 256)

    def receive(step):
        nonlocal first
        pieces.append(step.token)
        if first is None and target_tokenizer.decode(pieces).strip():
            first = (time.perf_counter() - started) * 1000
        return False

    try:
        # The publisher appends source EOS; bypass sentence splitting to retain subtitle fragments.
        source_tokens = source_tokenizer.encode(source, out_type=str) + ["</s>"]
        result = translator.translate_batch(
            [source_tokens], beam_size=beam, max_batch_size=1, max_input_length=0,
            max_decoding_length=cap, patience=1, length_penalty=1, coverage_penalty=0,
            repetition_penalty=1, disable_unk=True, replace_unknowns=False,
            return_end_token=True, callback=receive if beam == 1 else None,
        )[0]
        tokens = result.hypotheses[0]
        translation = target_tokenizer.decode(tokens).strip()
        error = None if translation else "Model returned no translation"
        if "</s>" not in tokens and len(tokens) >= cap:
            error = "QuickMT reached the live output cap; inspect the preserved output"
    except Exception as exc:
        error = f"{type(exc).__name__}: {exc}"
    return dict(started_unix_ms=started_unix_ms, first_text_ms=first,
                done_ms=(time.perf_counter() - started) * 1000, translation=translation,
                streamed_text=target_tokenizer.decode(pieces) if pieces else "",
                tokens=tokens, token_cap=cap, error=error)


def run(args):
    import ctranslate2
    import sentencepiece
    cases = load_cases(args.file)
    for name in ("model.bin", "config.json", "source_vocabulary.json", "target_vocabulary.json", "src.spm.model", "tgt.spm.model"):
        if not (args.model / name).is_file():
            raise ValueError(f"Missing local model file: {name}")
    args.out.mkdir(parents=True, exist_ok=False)
    source = sentencepiece.SentencePieceProcessor(model_file=str(args.model / "src.spm.model"))
    target = sentencepiece.SentencePieceProcessor(model_file=str(args.model / "tgt.spm.model"))
    start = time.perf_counter()
    translator = ctranslate2.Translator(str(args.model), device="cpu", compute_type=args.compute_type,
                                       inter_threads=1, intra_threads=args.threads)
    load_ms = (time.perf_counter() - start) * 1000
    warmup = measure(translator, source, target, "준비가 되면 시작하겠습니다.", args.beam_size)
    if warmup["error"]:
        raise ValueError(f"Warm-up failed: {warmup['error']}")
    records = []
    with (args.out / "records.jsonl").open("x", encoding="utf-8") as output:
        for repetition in range(1, args.runs + 1):
            for index, case in enumerate(cases):
                record = dict(pass_name="candidate", run=repetition, case=case, context=[],
                              profile="quickmt", beam_size=args.beam_size, compute_type=translator.compute_type,
                              measurement=measure(translator, source, target, case["source"], args.beam_size))
                records.append(record)
                output.write(json.dumps(record, ensure_ascii=False) + "\n")
                output.flush()
                if (index + 1) % 10 == 0:
                    print(f"QuickMT {args.compute_type} beam {args.beam_size}: run {repetition}/{args.runs}, {index + 1}/{len(cases)}", flush=True)
    good = [r["measurement"] for r in records if r["measurement"]["error"] is None]
    report = dict(status="completed" if len(good) == len(records) else "completed_with_errors",
                  limitation="Direct CPU calls, batch size 1, no HTTP, no OCR/overlay/game/LLM context or answer cache. Warm-up excluded. Beam 1 streams decoded text; beam search exposes only completed text. References are review aids, not certified gold. Latency includes tokenization and detokenization. CPU threads fixed; no game FPS measurement.",
                  model=str(args.model.resolve()), corpus_sha256=hashlib.sha256(args.file.read_bytes()).hexdigest(),
                  model_manifest=json.loads((args.model / "download.json").read_text(encoding="utf-8")) if (args.model / "download.json").exists() else None,
                  ctranslate2=ctranslate2.__version__, sentencepiece=sentencepiece.__version__, python=platform.python_version(),
                  processor=platform.processor(), device=translator.device, compute_type=translator.compute_type,
                  intra_threads=args.threads, inter_threads=1, beam_size=args.beam_size, batch_size=1, context_lines=0,
                  runs=args.runs, load_ms=load_ms, warmup=warmup, attempted=len(records), errors=len(records) - len(good),
                  first_text=distribution([m["first_text_ms"] for m in good if m["first_text_ms"] is not None]),
                  done=distribution([m["done_ms"] for m in good]), corpus=cases, game_state=args.game_state)
    (args.out / "report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")
    cell = lambda value: html.escape(value).replace("|", "&#124;").replace("\n", "<br>")
    review = ["# QuickMT CPU comparison", "", "References are review aids; all distinct outputs are retained.", "", "| ID / Korean | Reference | Outputs |", "|---|---|---|"]
    for case in cases:
        answers = dict.fromkeys(r["measurement"]["translation"] if r["measurement"]["error"] is None else "ERROR: " + r["measurement"]["error"] for r in records if r["case"]["id"] == case["id"])
        review.append(f"| {cell(case['id'])}: {cell(case['source'])} | {cell(case['reference'])} | {'<hr>'.join(cell(a) for a in answers)} |")
    (args.out / "review.md").write_text("\n".join(review) + "\n", encoding="utf-8")
    print(json.dumps({key: report[key] for key in ("attempted", "errors", "first_text", "done")}), flush=True)
    translator.unload_model()
    return int(report["errors"] > 0)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--file", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--beam-size", type=int, choices=(1, 2, 5), default=1)
    parser.add_argument("--compute-type", choices=("int8", "float32"), default="int8")
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--game-state", default="unspecified")
    args = parser.parse_args()
    if args.runs < 1 or args.threads < 1:
        parser.error("runs and threads must be positive")
    raise SystemExit(run(args))
