# Live subtitle benchmark material

The local pack at `data/live-diagnostics/benchmark-v2.2/` prepares translation comparisons and selected OCR evidence.
It does not select an application model. Original videos and transcripts in `vidTestsKoreanSubs` remain untouched;
the pack copies small evidence images and transcript snapshots, with hashes, and points to the original videos.
Videos and generated packs stay under ignored local data paths and must not be committed.

## Prepared material and its limits

| Suite | Lines | Purpose |
| --- | ---: | --- |
| `corpora/legacy-70.jsonl` | 70 | Byte-identical earlier comparison corpus; regression material, not a holdout |
| `corpora/added-dialogue.jsonl` | 12 | Missing Elden Ring captions transcribed from exact seek screenshots |
| `corpora/restored-context.jsonl` | 30 | Opening four Elden Ring scenes in chronological order, including the 12 additions |
| `corpora/semantic-32.jsonl` | 32 | Authored checks for negation, numbers, roles, obligation, location and dialogue context |
| `corpora/new-game-validation.jsonl` | 6 | Two newly transcribed Sekiro scenes, reserved for validation |

There are **120 distinct cases**. The restored-context suite overlaps the other suites and must not add another
30 observations to a pooled quality score. The six Sekiro cases are a small validation sample; they cannot establish
performance across the game. The original 30 Elden Ring entries have reviewed onset/offset brackets and copied boundary
contact sheets. The other 12 Elden Ring captions and six Sekiro captions have evidence timestamps, not checked boundaries.

The agent reviewed subtitle crops for all original Elden Ring entries, the added captions, and candidate text changes
sampled at 2 Hz across the first 268 seconds. This restores observed missing context, but does **not** certify every frame
or full-video subtitle recall. Later Elden Ring scenes are still partial samples. Sekiro's blue uploader commentary was
excluded from game dialogue; only visually inspected white game captions were transcribed.

`annotations/cases.json` keeps source status, meaning aids, original annotations, localization, evidence and timing uncertainty
separate. English meaning aids are agent-created and require independent bilingual review; they are not certified gold.
The supplied official English localization can differ from the Korean meaning and is never substituted as the scoring reference.
`review/semantic-review.tsv` leaves reviewer and approval columns blank. `review/pending.jsonl` quarantines 73 rows from the older
unverified transcript drafts; these are not runnable comparison cases. Neither screenshot evidence nor a well-formed JSON file
proves a translation is correct.

`manifest.json` hashes source snapshots, copied evidence, every generated output and the two referenced videos. Video hashes
allow us to reject a different edit with the same filename. Existing output folders are never overwritten. The earlier
`benchmark-v2`/`benchmark-v2.1` folders are preparation snapshots; use `benchmark-v2.2`, which includes all four scenes
and the tags field required by the existing Rust comparison reader.

## Preparing another version

`tools/prepare_live_benchmark.py` uses the Python standard library and performs no downloads or inference. A curated additions
JSON supplies the usual `id`, `scene`, `source`, `reference`, `origin` fields, plus game, visual-review status, evidence path,
evidence seconds and video path. Optional `context_scenes` explicitly selects scenes for chronological reconstruction.
Evidence paths must stay inside the declared source directory. Only explicitly reviewed timing brackets are promoted.

The local inputs used for this preparation are kept in `data/live-diagnostics/benchmark-v2-preparation/`. To reproduce the pack
into a fresh folder:

```powershell
python tools/prepare_live_benchmark.py --source-root C:\Users\suiny\Documents\repositorios\vidTestsKoreanSubs --legacy data/live-diagnostics/elden-ring-comparison/corpus/dialogue-70.jsonl --additions data/live-diagnostics/benchmark-v2-preparation/curated-additions.json --timings data/live-diagnostics/benchmark-v2-preparation/reviewed-timings.json --out data/live-diagnostics/benchmark-next
python -m unittest discover -s tools -p test_prepare_live_benchmark.py
```

`tools/inspect_live_boundaries.py` proposes transitions using existing ffmpeg and Pillow installations. It creates before/after
contact sheets for visual review, not trusted timestamps. It produced a false transition for `er_sub_09`; manual review corrected
that entry. Do not automatically convert its proposals into reviewed timing. For the other entries the stored brackets span the
independently viewed absence/presence samples, usually 0.4 seconds. Preserve this uncertainty in latency calculations.

## Fair model comparisons

Use `bench-live-models --file PACK/corpora/restored-context.jsonl` for dialogue-context development; retain legacy-70 for comparison
with older results. Test native zero-context prompts first, then opt-in context extensions. Do not assume a model's existing CLI
profile supports another family's template. Record exact model hash, runtime version, quantization, prompt, decoding, context
length, output cap, CPU threads and offloaded layers. Unsupported native behavior must be stated before comparing results.

Reference history is an optimistic control with identical inputs. Rolling history uses each model's previous translations and
exposes accumulated errors. Both may use only earlier lines in the same scene; reset between scenes and runs. Never feed the
current reference, future dialogue or a plot summary containing future events. The optional name glossary contains names only;
compare glossary-assisted and unassisted runs separately, with the same glossary available to each model. These are proposed
experiment conditions, not newly implemented prompt profiles.

Choose prompts on development material, then run the six Sekiro cases without further tuning. Once their outputs influence a
prompt choice, treat them as regression cases and reserve another whole scene for validation. Keep each game's results separate;
authored semantic checks should not dominate a pooled game-dialogue score.

A transcription matching a screenshot does not certify its meaning. The current expanded round flags `er_sub_06`
(`…함께 자손된 용이여`) as semantically uncertain despite matching the preserved frame. Retain the frozen input for timing
and output comparison; do not use its agent-created meaning aid or the different English localization as a decisive quality
judgement. Likewise, `지수`/`구하다` can have mathematical or person/rescue readings without dialogue context. Inspect ambiguous
cases in context and prioritize clear polarity, actor, condition and unit errors when describing observed failures.

Review candidate identities blind when possible. Mark changed negation, actor, number, condition, obligation or instruction as
meaning errors; record invented details, omitted content and name inconsistency separately from acceptable wording or register.
Fragments should remain valid fragments: a model cannot be required to recover an object named only in the next subtitle.
`must_preserve` annotations help review but are not an automatic judge. Avoid calling a similarity metric an accuracy percentage.
Gemma 12B is a comparison reference, not the gold answer. No automatic quality scorer is installed by these tools.

## OCR, timing and game load

Measure clean Korean translation separately from OCR-to-translation. Run OCR against evidence with the real reader, save its raw
text and confidence, and report character errors and inserted commentary/UI independently. Do not invent OCR corruption and
describe it as an observed recognition error. The copied full frames permit inspecting the actual subtitle area; the recorded
crop in the replay manifest is for the Elden Ring video only.

`replay/selected-subtitles.json` is an annotation manifest, not an input directly accepted by the Rust replay test. That test
accepts a two-column timing file and uses stand-in OCR, so it measures watcher scheduling, not OCR recognition or translation.
Do not replay the partially annotated full clip and call extra real subtitles false positives or missing labels true negatives.
Selected presence checks are possible; full-scene precision/recall requires complete subtitle and no-subtitle annotation first.
No watcher code was changed in this preparation.

For a translation finishing at video time `t`, an offset bracket `[a,b]` means it is definitely before disappearance if `t < a`,
definitely late if `t > b`, and uncertain otherwise. Record capture, OCR, queue, first English text and completion separately;
translation-only latency is not end-to-end latency.

Run warm-up separately, then three paired sequential runs with a fresh baseline before and after the candidate. Report errors,
success-only p50/p95, output lengths and per-case meaning errors. Test with a repeatable cutscene actually playing, alongside
game-only control runs at the same resolution/settings. Record frame-time p95/p99, FPS and GPU memory/utilization; an idle open
game is a different condition. Small CPU models should be tested with a controlled thread budget as well as GPU offload.
Choose a model only after considering readable correct output before disappearance and game frame-time cost together.

## Additional native candidate runner

`tools/bench_live_candidates.py` uses Python 3.11+ standard-library modules and the existing local llama-server; it installs nothing.
Supply existing approved weights, a corpus, server and a fresh output path as shown in the README. It performs sequential
Gemma/candidate/Gemma passes, three repetitions by default, with separate excluded warm-ups. Each report records exact
HTTP requests, source/context, raw and final answers, partial errors, model/tool/corpus/server hashes and effective arguments.
`nvidia-smi` telemetry measures total GPU pressure, not game FPS. The HTTP transport differs from the Rust diagnostic;
compare a candidate to its own paired Gemma controls rather than combining timings from different runners without checking drift.
The runner gives its owned server a private CUDA cache under the new output folder, or an explicit shared test cache via
`--cuda-cache-path`. This permits caching with restricted filesystem access and keeps generated driver files in test storage.
The report records CUDA cache/JIT environment variables; no parent environment or app configuration is edited. Treat startup,
excluded warm-up and first-use outliers separately from warmed latency. CUDA caching may affect initialization; it does not
establish a translation quality improvement. [NVIDIA environment-variable reference](https://docs.nvidia.com/cuda/cuda-programming-guide/05-appendices/environment-variables.html).

The profiles preserve Tower's language-labelled user prompt, Seed's plain prompt/target tag, and the embedded TranslateGemma
template's structured content. llama.cpp receives TranslateGemma language codes in template kwargs; `/apply-template` must
confirm Korean/English and the source before inference. TranslateGemma uses 2048 tokens per slot and zero dialogue context.
EXAONE and Konan retain their published system instructions; Kanana uses its embedded chat template. These general models
receive an explicit English-only subtitle instruction, not a claimed publisher translation template. Greedy candidates retain
the live output cap. HY-MT2 uses the Tencent core sampling settings. Seed's recommended four-beam decoding is unverified in
this runtime; community GGUF results do not establish equivalence to the publisher's original weights.

For supported profiles, `--context-lines 4 --context-mode fixed` supplies earlier references, while `rolling` supplies only
earlier successful model answers. Scenes/repetitions reset context. MiLMMT completed examples and Tower/general chat history
are experiments, not native publisher context formats. Seed and TranslateGemma reject history for this native comparison.
`translategemma-context` is a separate experimental profile: earlier user turns keep their structured Korean/English language
codes, assistant turns hold completed translations, and the current untranslated source remains last. The embedded template
permits alternating roles, but that does not establish that dialogue context improves this model's translation quality.
The `sub` profile retains the published aligned previous-source/translation blocks, allowing the same Sub prompt on CPU.
For a candidate CPU run use `--gpu-layers 0 --threads 4`; device, KV and operator offload are disabled while paired Gemma
controls stay fully offloaded. This is still a model-only test with no demonstrated gameplay benefit.

`python -m unittest discover -s tools -p test_bench_live_candidates.py` checks language formatting, context isolation,
failed-history handling, Unicode, raw labels and rejection of capped/incomplete/empty streams. These are scripted checks,
not proof that an actual converted model loads correctly. Use its saved server log and rendered prompt to verify that separately.

## Verification status, 2026-10-12

The material was visually inspected by the agent and pack integrity is covered by Python tests. Independent bilingual review,
real OCR on the new frames, full-video annotation and repeatable game FPS trials remain unverified.
See M-16 in `MANUAL_TESTS.md` and the researched queue in `LIVE_MODEL_CANDIDATES.md`.

The subsequently approved expanded native comparison loaded fifteen candidate weight variants plus the Gemma control.
All 16,200 measured requests completed without execution errors: 120 distinct texts, three repetitions and three passes per
candidate. This is translation-only evidence; meaning changes remain visible in the saved outputs. Dialogue and CPU
follow-ups are separate conditions. The source pack and no-context results are not certified accuracy benchmarks.
Raw reports and the per-case review are in `data/live-diagnostics/benchmark-expanded-2026-10-12`.

The completed round also includes ten dialogue/CPU follow-ups: **27,000 measured requests** across 25 conditions,
all without execution errors, plus 75 excluded warm-ups. `RESULTS.md` contains all model timings, memory measurements,
meaning observations and exact check-log links; `review-by-case.md` compares distinct answers. MT2 7B Q4 and MiLMMT 4B Q6
are provisional real-game candidates, not quality-equivalent defaults. Context introduced meaning errors as well as fixing
some scene readings. CPU rolling medians were 932 ms completion for MiLMMT Q6 and 854 ms for Kanana, with completion p95
of 1,749/2,880 ms respectively. Maru's existing live prompt did not remove Kanana's important scene errors.

Current round verification: all 26 Python tests, 21 app tests, app Clippy and 81 UI tests with no isolation passed.
Dynamic-ONNX workspace tests passed (424 passed, two ignored). Standard workspace Clippy still fails on three existing
warnings in unchanged Windows capture/UIA files; default workspace tests still fail to link `DMLCreateDevice1`.
Default Node isolation still encounters sandbox `spawn EPERM`; fallbacks and their commands are separately recorded.
Final hashes matched for the installed configuration, original baseline/server/CLI, all fifteen candidate weights,
frozen evidence outputs and the two source videos; all benchmark/app model ports were closed. This round changed
benchmark tooling/documentation/CI, with no production Rust/UI or installed model-selection change. Main was not rebased
over the existing dirty diagnostics branch. No new dependency or NAS upload was needed for these comparisons.

Verification performed for this preparation: nine new Python integrity tests passed, as did four existing QuickMT logic tests;
all generated corpus fields/tags and manifest output hashes were checked. All 81 UI tests passed with Node's
`--test-isolation=none` because this sandbox refused child-process spawning in the default test runner. App Clippy passed and
21 app tests passed using the existing cached ONNX library. Workspace tests passed with `--features maru-ocr/ort-dynamic`
(424 passed, two ignored); the default offline static-link run remained blocked by `DMLCreateDevice1` linking.
Strict workspace Clippy failed on three existing warnings in unchanged Windows capture/UIA code. No dependency was downloaded
and no production Rust, UI or installed configuration file was edited for this data preparation.
