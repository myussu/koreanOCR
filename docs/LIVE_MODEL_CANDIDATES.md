# Additional live translation candidates

## Tight GPU budget follow-up (2026-10-12)

For a strict 1.5 GB translator budget, the measured first choice is **HY-MT2 1.8B Q4_K_M** with a 1,024-token
context, one slot and 128-token batch/microbatch buffers. Dedicated VRAM peaked at 1.444 GB; its existing two-slot
configuration used 1.960 GB. The 2,048-token compact variant still exceeded 1.5 GB (1.512 GB).
**LFM2 KoEn v8 Q5_K_M** with 2,048 tokens and the same slot/batch limits used 1.126 GB. Q4 used 1.012 GB;
Q8 exceeded the limit at 1.526 GB. MiLMMT 1B Q8 used 1.340 GB but remains excluded from Settings because of its
earlier meaning failures. The current MiLMMT 4B Q6 baseline used 4.047 GB.

Each successful condition ran 120 Korean sources three times with native formats, no dialogue history, no game
and no Maru/OCR/speech process. HY's selected preset completed at p50/p95 55/80 ms; LFM Q5 at 37/53 ms.
Those are model-only timings, with warm-up excluded; first-use graph/shader outliers still reached seconds.
Actual per-process Windows counters were sampled once per second, rather than inferring VRAM from file size or
subtracting total device occupancy. Persistent shared allocations around 84 MiB do not by themselves establish a leak.
The HY 1.13 GB and LFM Q5 0.84 GB file sizes are smaller than their full runtime footprints.
[Official HY files/templates](https://huggingface.co/tencent/Hy-MT2-1.8B-GGUF),
[LFM conversion, prompt and quantization results](https://huggingface.co/gyung/lfm2-1.2b-koen-mt-v8-rl-10k-merged-GGUF).

The publisher's LFM chrF++ comparison is an internal quantization comparison, not evidence of parity with HY or
game dialogue quality. Inspection of the first repetition of all 120 outputs found errors in both small models.
HY preserved more of the checked roles, negation, conditions and units, but mishandled several fantasy terms.
LFM changed a dismissed consul into a replaced speaker, a sister into a father, and a reprieve into "less than a week".
Use HY as the initial low-memory trial and LFM as a larger quality sacrifice when GPU space is the priority.
These observations are not a certified bilingual score or a percentage of accurate translations.

Tencent's official 1.25-bit (0.462 GB) and 2-bit (0.601 GB) STQ files were downloaded and hash-verified, but both
failed during loading in the installed llama.cpp build 11540. Neither generated a translation. They are not available
as app choices: a smaller file is useful only when the actual runtime supports it. At research time the CPU STQ
kernel PR was open and the CUDA PR was closed without merging; this does not establish that either would work here.
[STQ 1.25-bit card](https://huggingface.co/tencent/Hy-MT2-1.8B-1.25Bit-GGUF),
[STQ 2-bit card](https://huggingface.co/tencent/Hy-MT2-1.8B-2Bit-GGUF),
[CPU work](https://github.com/ggml-org/llama.cpp/pull/22836),
[CUDA work](https://github.com/ggml-org/llama.cpp/pull/23332).

The installed hybrid OCR reader separately used 2.359 GB for the supplied 1,280×180 subtitle crop. Combined
owned-server memory was 3.801 GB with HY and 3.480 GB with LFM, excluding Maru, desktop and game. This crop
test establishes a measured cost, not the reader's maximum. CPU PP-OCR bypasses the reader for live mode only.
Reports, native requests, telemetry and downloaded-file receipts are under the ignored local folder
`data/live-diagnostics/low-vram-2026-10-12/`; the diagnostic source is `tools/bench_low_vram.py`.

## Earlier candidate research

Research checked 2026-10-12. This records the research queue; the approved local comparison is described below.
Neither published scores nor these local tests establish a quality-equivalent application default.
Sizes below are published GGUF **file sizes in decimal GB**, not total VRAM: runtime buffers and context add memory.
Community conversions require load/template checks before their results represent the original model.

| Candidate | Decision | Useful starting weights | Why |
| --- | --- | --- | --- |
| Tower+ 2B | High priority: local CPU and GPU | Q8_0 2.78 GB; Q5_K_M 1.92 GB | Translation training includes Korean; small enough to investigate reduced game interference |
| Seed-X-PPO 7B | Add to NAS quality comparisons | Community Q4_K_M 4.56 GB | Translation specialist supporting Korean; larger than the small local candidates |
| EXAONE 3.5 2.4B Instruct | Exploratory local/CPU | Q8_0 2.84 GB; Q5_K_M 1.91 GB | Bilingual Korean/English general model; subtitle translation needs measurement |
| Konan-LLM-OND 4B | Exploratory, after specialists | Q8_0 4.34 GB; Q5_K_M 2.93 GB | Korean vocabulary adaptation; general benchmarks do not establish translation fidelity |
| Kanana 2 3B Instruct | Exploratory compatibility check, then local | Q8_0 3.73 GB | Compact Korean instruction model; published chat results are not translation benchmarks |
| Kanana 2 1.3B Instruct | Defer faithful-model ranking | One modified Q4_K_M conversion is 0.854 GB | Its converter removes original attention features; equivalence is unverified |

Tower+ uses Gemma 2's chat format with a user translation request and greedy decoding. Use the publisher's source/target-language
labels; start with the single-source prompt before experimenting with subtitle context.
[Publisher card](https://huggingface.co/Unbabel/Tower-Plus-2B), [GGUF sizes](https://huggingface.co/DZgas/Tower-Plus-2B-GGUF/tree/main).

Seed-X-PPO requires **plain completion**, no conversation template, and an English target tag at the end. The recommended decoder
uses beam width four; greedy is also documented. Compare greedy latency and a separate native beam-search quality run if the
runtime supports it. The publisher advises against unofficial quants, so a community GGUF result must be identified as such;
official AWQ/GPTQ alternatives involve another runtime and require a separate approved environment.
[Publisher instructions](https://huggingface.co/ByteDance-Seed/Seed-X-PPO-7B),
[community file sizes](https://huggingface.co/mradermacher/Seed-X-PPO-7B-GGUF).

The MiLMMT report's shared FLORES+ Korean-to-English XCOMET comparison lists Tower+ 2B at 91.86, Tower+ 9B at 94.64,
Seed-X-PPO 7B at 93.84 and MiLMMT 4B at 94.19. These are publisher-reported automatic metric scores, not percentages of correct
game subtitles, quantized-model results or evidence of speed on our hardware. They support testing Tower and Seed, not choosing
them without our own dialogue review. [Report, Appendix D](https://arxiv.org/html/2608.10812v1#A4.SS2).

Konan derives from Qwen3-4B-Base and expands Korean vocabulary; its card claims lower Korean token counts. That may reduce input
work but does not prove faster first text or better Korean-to-English translation. Inspect the converted tokenizer and template;
do not request chain-of-thought for a live subtitle. [Publisher card](https://huggingface.co/konantech/Konan-LLM-OND),
[GGUF sizes](https://huggingface.co/mykor/Konan-LLM-OND-gguf).

Kanana's **KoMT-Bench is a chat benchmark**, not a machine translation benchmark. Its 1.3B design has hybrid sliding-window/full
attention and per-layer positional handling. The inspected converter explicitly substitutes Qwen3 architecture and removes
SWA/YaRN. Treat it as a modified derivative until validated against the official runtime. The 3B SLM is separate from the much
larger Kanana 30B-A3B family; active-parameter count does not make the latter a 3B-weight download.
[Official SLM card](https://huggingface.co/kakaocorp/kanana-2-3b-instruct),
[modified 1.3B conversion](https://huggingface.co/ssolunar/Kanana-2-1.3b-instruct-Q4_K_M-GGUF),
[3B GGUF sizes](https://huggingface.co/mradermacher/kanana-2-3b-instruct-GGUF).

The inspected official **3B** configuration uses Qwen3 with 32 full-attention layers and sliding-window attention disabled.
Do not transfer the modified 1.3B converter's attention warning to this tested 3B model. Numerical equivalence between the
original weights and the community Q8 conversion is still unverified.
[Official 3B configuration](https://huggingface.co/kakaocorp/kanana-2-3b-instruct/blob/main/config.json).

EXAONE's smallest 3.5 model is the relevant starting point. Its Korean and English instruction capability is useful evidence of
coverage, but its general evaluations do not replace translation tests. Retain the EXAONE template and check short English-only
output. [Official card](https://huggingface.co/LGAI-EXAONE/EXAONE-3.5-2.4B-Instruct),
[GGUF sizes/template](https://huggingface.co/bartowski/EXAONE-3.5-2.4B-Instruct-GGUF).

The initial research order proposed: existing MiLMMT 4B control; MiLMMT 1B and Tower+ 2B on CPU/GPU; MiLMMT 4B Q6/Q5 and official
TranslateGemma 4B; HY-MT2 7B and Seed-X-PPO on the NAS; then EXAONE, Konan and Kanana 3B exploratory runs. This order favors
translation evidence and potential lower game cost. The subsequently confirmed retest ran all selected variants locally;
no corpus was uploaded to the NAS. It does not establish that any candidate is faster while playing.
The official TranslateGemma model must be distinguished from the already tested community Translate Gemma Sub E4B/E2B.

The research/data preparation itself performed no new-model inference. The subsequently approved 2026-10-12 comparison
downloaded the ten missing weight files and ran all fifteen confirmed candidate variants against Gemma, using native
formats and the expanded 120-text pack. Each native condition completed three repetitions and paired controls without
execution errors; this does not certify translation fidelity or original/conversion equivalence. The additional families use
the isolated standard-library runner, not new application defaults. Raw evidence and derived tables are under
`data/live-diagnostics/benchmark-expanded-2026-10-12`; see `LIVE_BENCHMARK.md` for the method and remaining manual checks.
That folder's `RESULTS.md` includes all fifteen native variants and ten follow-ups (27,000 measured requests), with
per-game timings, raw-answer review and separately labelled CPU/context results. MT2 7B Q4 and MiLMMT 4B Q6 are
provisional candidates for a matched playing-cutscene trial; no tested configuration established quality-equivalent
replacement status. The weights, frozen evidence and installed configuration passed the final integrity audit.
