# Maru

A local, always-on reading assistant for learning Korean. Select text **anywhere on your screen**, press a
hotkey, and a small panel appears next to it with:

* each Korean word with **romanization**, **how it is actually pronounced** (`학교에서` → `[학꾜에서]`),
  its **meaning in this sentence**, and its **TOPIK level**;
* the **natural translation below**, streamed as it is written (English → Korean gives you polite, formal and
  casual versions side by side);
* a click on any word for a **deep dive**: how the form is built (`공부 + 하다 + 았 + 어요`), dictionary entries,
  Hanja, example sentences, and what it is easily confused with.

Everything runs on your machine: the language model (llama.cpp), OCR (PP-OCR / Windows OCR), the Korean analyser
(Kiwi) and the dictionary. Nothing is sent anywhere.

It also learns with you: every word you look up is tracked, queued for **spaced repetition (FSRS-6)** after the
second lookup, can be sent to **Anki**, and words you know **fade** from the panel so it gets lighter as you improve.

**Anki** is the main review. In a deep dive, *Add to Anki* lets you pick a deck (or type a new one; the program you are
playing suggests one), tags are added automatically, and each note gets a Korean voice (the TTS you set up) plus a
reverse card (English → Korean) in a `::Reverse` subdeck. Maru reads your intervals back, so words Anki holds for long
enough fade from the panel. Needs the AnkiConnect add-on (`2055492159`); log in to AnkiWeb inside Anki and Settings →
Anki *Sync* triggers it. See `docs/config.example.toml`.

To test with your AnkiWeb account, open Anki desktop with AnkiConnect, sign in and complete your normal sync there.
In Maru Settings → Anki, enable **Review in Anki**, save and restart, then **Check Anki**. Maru uses its own `Maru Korean`
note type; legacy `Basic` settings already map to that type without modifying Anki's built-in Basic model. Add a word
from a deep dive to a test deck, then use **Sync now** and inspect it on AnkiWeb. **Refresh progress** reads your current
Anki intervals immediately. Suspended or deleted cards no longer retain old Anki-based fading; local saves and manual
known marks stay intact. The Review count is tracked Anki membership, separate from Maru's local review queue.
See [Anki's syncing instructions](https://docs.ankiweb.net/manual/syncing) and [M-24](docs/MANUAL_TESTS.md#m-24-anki-add-recovery-and-learning-progress).

Adding validates the word, note type and existing note before requesting audio. Repeated submissions are refused while
an add is running. If a note was created but setting up its cards failed, the result says so and retains a local recovery
record. Retry **Add to Anki** to complete that note's card setup without creating another note or requesting audio again.
Spoken audio can still load the configured voice for a genuinely new word; uncheck it for the initial game-memory test.

> Status: the engine, command line and overlay are implemented and tested, but the Windows-specific parts and the
> Korean model quality could only be checked on a real machine. See **[What is verified](#what-is-verified)**.

**Documentation:** [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (how it is built, the feature map, the contracts between parts) ·
[docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md) (what only a real PC can verify, and what to bring when something fails) ·
[docs/config.example.toml](docs/config.example.toml) (every setting) · [AGENTS.md](AGENTS.md) (how to change this repository without
breaking other work).

## Quick start (Windows)

**You need** (once): Windows 10/11 (64-bit), [Rust](https://rustup.rs) with the *Visual Studio Build Tools* ("Desktop development with C++") it asks for, **kept up to date** (see below), Git, and the WebView2 runtime (already part of Windows 11; the app installs it on Windows 10). About 12 GB of disk for the models. Anki is optional.

```powershell
git clone https://github.com/myussu/koreanOCR.git
cd koreanOCR
cargo install --path crates/maru-cli       # builds it and puts `maru.exe` in %USERPROFILE%\.cargo\bin (already on your PATH)
maru setup                                 # the setup wizard, see below
maru doctor --deep                         # (the wizard ends with this) checks everything, runs a real translation

cd app\src-tauri; cargo run --release      # the overlay (lives in the system tray)
```

(`cargo build --release -p maru-cli` also works, but it only creates `target\release\maru.exe`; run it as
`.\target\release\maru.exe setup` or add that folder to your PATH.) The first `cargo` build takes several minutes. To update later: `git pull`, run the `cargo install` line again, and `cargo run --release` in `app\src-tauri`.

**If the build ends with `LNK2001: unresolved external symbol __std_…` (for example `__std_max_8u`) and `LNK1120`**, the Visual C++ tools on that PC are too old: the prebuilt ONNX Runtime that Maru links was compiled with a newer Microsoft C++ standard library than the one installed (a toolset such as `14.34`, from Visual Studio 2022 17.4, is too old). Open the **Visual Studio Installer**, press **Update** on your Visual Studio / Build Tools 2022 (or install the latest *Build Tools for Visual Studio 2022*) with **"MSVC v143 - VS 2022 C++ x64/x86 build tools (Latest)"** ticked, then build again with `cargo install --path crates/maru-cli`. The wizard and every other command are unaffected: this only concerns building.

### The setup wizard

`maru setup` looks at the PC (graphics card and its driver, memory, free disk), recommends what to install and why, asks the few things only you can answer, and then installs it. It chooses between three ways of running the model, and between the full 12B model and the small one:

| It finds | It recommends |
|---|---|
| An NVIDIA card with 10 GB or more | CUDA, the full model, and the OCR reading model |
| A smaller NVIDIA card | CUDA with the small model (`lite = true`) |
| An NVIDIA card whose driver is too old for CUDA | Vulkan, and tells you to update the driver |
| An AMD or Intel graphics card | Vulkan with the small model (Windows does not report their memory; turn `lite` off in Settings if the card has 12 GB or more) |
| Only integrated graphics, or no card | CPU with the small model; it warns if memory is low |

It asks: whether to use that recommendation (if not, which build and which model), whether to add the OCR reading model (about 1.8 GB, only offered with the full model) and the Korean voice (about 2.5 GB, off unless you say yes), then shows what will be downloaded, how much, and whether the disk has room. Nothing is installed before you confirm, and ending the input (Ctrl+D) cancels. It writes your choices into `config.toml` (an existing file keeps its comments and other settings), installs, and finishes with the full check. A step that fails (usually the network) does not stop the others; run `maru setup` again and it skips what is already installed.

`maru setup wizard --dry-run` shows what it found and what it would do, without changing anything. `--yes` takes every recommendation without asking, `--backend cuda|vulkan|cpu` replaces the recommended llama.cpp build, and `--force` downloads llama.cpp again even if a `llama-server` already works (needed to switch a PC from one build to another; quit Maru first). Each step is also its own command (`maru setup llm`, `kiwi`, `ocr`, `ocr-vl`, `tts`, and `maru dict download`) if you prefer to run them yourself.

The model on another PC is not part of the wizard; set it up afterwards in **Settings → Translation model** (below).

Optional: `maru setup tts` (a local Korean voice, see **Speech** below) and the AnkiConnect add-on if you use Anki.

### Choosing the build yourself, and PCs without an NVIDIA GPU

The wizard does this for you. By hand: only one thing is tied to NVIDIA, because `maru setup llm` downloads llama.cpp's **CUDA** build by default (the newest one your NVIDIA driver can run). Tell it what the machine has:

| This PC has | Run | What to expect |
|---|---|---|
| NVIDIA GPU | `maru setup llm` (CUDA; `--cuda 12.4` pins a build; RTX 50-series needs CUDA 12.8+) | the fast path |
| AMD or Intel GPU, or an NVIDIA one without the CUDA driver | `maru setup llm --backend vulkan --force` | works with nothing extra installed; usually a bit slower than CUDA |
| No usable GPU | `maru setup llm --backend cpu --force` | works, but the 12B model is slow: expect several seconds per line, too slow for live subtitles |

`--force` matters when something is already installed: without it `maru setup llm` keeps any `llama-server` that starts, and with it the old llama.cpp files are removed first so a Vulkan or CPU build never loads the CUDA libraries left beside it (quit Maru first). `maru setup llm` ends by starting the real server once and loading the model, so a build that cannot use this PC's hardware fails there, not later.

Everything else already works without CUDA: OCR runs on the CPU by default (`[ocr] device = "cpu"`, or `directml:0` on any DirectX 12 GPU in a build with the `directml` feature), the dictionary, Kiwi and the learning features are CPU-only, and `backend = "auto"` can use the Windows OCR that ships with Windows. On a weaker PC:

* set `lite = true` (a 4B model, no OCR reading model, about 2.5 GB less memory), and skip `maru setup ocr-vl`;
* or keep the model on the strong PC: start `llama-server` there (or LM Studio, with the server reachable on your network) and set `[llm] endpoint = "http://<that-pc>:8080/v1"` on this one. Maru then uses it instead of starting its own, so on this PC skip `maru setup llm` and only the OCR and the overlay run locally;
* point `[tts]` at that PC too (`provider = "endpoint"`), as in **Speech** below, or leave speech off.

### A model on another PC (optional)

The translation model can run on a second PC on your network, so the PC you play on keeps its GPU for the game. It is a setting, not part of
the wizard: on the PC that will serve the model, install llama.cpp (the same release `maru setup llm` downloads, or `maru setup llm` itself) and start
the server the way Maru starts its own, but listening on the network:

```powershell
llama-server.exe -hf unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL --no-mmproj --host 0.0.0.0 --port 8080 -c 8192 -np 2 -ngl all -fa on --cache-reuse 256 --reasoning off
```
Let it through the firewall on private networks (`New-NetFirewallRule -DisplayName llama-server -Direction Inbound -Protocol TCP -LocalPort 8080 -Action Allow -Profile Private`)
and check it from your PC with `curl http://<that-pc>:8080/health`. The server has no login: keep it on your home network and never forward the port.
Then on your PC, either set it for good in **Settings → Translation model** (or `[llm] endpoint = "http://<that-pc>:8080"`), or try it for one command only:
`maru bench --live --endpoint http://<that-pc>:8080` times the live-subtitle lane against it (`maru bench --live` alone times the local model; quit the
overlay first, it holds the local model's port). The reading model and the speech server still start on your PC unless you turn them off
(`[ocr] vision_reader = false`, `[tts] enabled = false` or `provider = "endpoint"`). To compare local and remote on a real cutscene, follow M-11 in
[docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md).

**Ollama** (for example on a TrueNAS SCALE box) works as that server too, with a few differences from llama-server: it chooses the model by name, and a "thinking" model must be told not to think (Maru sends `reasoning_effort = "none"` to such servers; `[llm] reasoning_effort` changes or empties it), so set
**Settings → Translation model → Model name that server expects** (`[llm] model_name`) to exactly what `ollama list` shows, and give `maru bench` the same with
`--model`; and it unloads an idle model after five minutes by default, so set `OLLAMA_KEEP_ALIVE` (for example `-1`) on the server, or the first subtitle after a
pause waits for the model to load. It is not yet verified by a person; M-11 covers it.

Config file (`maru config init` writes it; [docs/config.example.toml](docs/config.example.toml)
shows every option):

```toml
[llm]
hf_repo = "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL"
```

| Hotkey | Does |
|---|---|
| `Ctrl+Alt+T` | translate the selection (falls back to the text under the cursor) |
| `Ctrl+Alt+R` | draw a rectangle anywhere and translate what is inside |
| `Ctrl+Alt+H` | hover mode: rest the pointer on a Korean word for its meaning |
| `Ctrl+Alt+C` | live subtitles: the translation appears just above or below the original line (whichever side has room) as soon as it is read, on the last chosen area |
| `Ctrl+Alt+Shift+C` | choose a different live-mode area (and start it there) |
| `Ctrl+Alt+S` | label every Korean text on the screen (game menus, HUDs); press again to rescan |
| `Ctrl+Alt+L` | history, review and status |
| `Ctrl+Alt+Esc` | hide the panel and all screen labels |

Each action accepts several keys separated by `|` in `config.toml` (e.g. `translate = "Ctrl+Alt+T | F6"`), which helps in games that use Ctrl/Shift/Alt. A registered key is taken from every program while Maru runs, so pick ones you don't use (see `docs/config.example.toml`). `Status` shows which toggles are on, and the tray icon's tooltip lists them too.

**Speech.** `maru setup tts` installs a local Korean voice (Qwen3-TTS through qwentts.cpp's `tts-server`, about 2.5 GB; Windows build from this repository's `tts-server` workflow) and `maru tts-eval` writes a page that plays the same tricky sentences (sound changes, numbers, intonation, a paragraph) in each voice so you can judge by ear. To keep the GPU free for a game, point `[tts]` at a server on another PC or at any OpenAI-style speech API (`provider = "endpoint"`); both are in Settings → Speech, with a Test voice button. Long text is spoken sentence by sentence (the next one is made while one plays) and every sentence is cached. In the panel, 🔊 reads the text aloud with the spoken sentence and its words highlighted, with previous / pause / next / say-again / repeat-each-sentence controls; the deep dive has 🔊 buttons for a single word; `Ctrl+Alt+P` (`hotkeys.speak`) translates the selection and reads it, and stops when pressed again. The panel stays up while it reads and dismissing it stops the voice.

**Moving the panel.** Drag it by its top bar (the ⠿ grip) to put it anywhere: it pins itself and stays there while it translates the next thing, however much it grows. Unpin (📌) to let it go back next to the text again.

**Live subtitles** draw the English in a dark band next to the original line — above it or below it, on the side with room, never over it — stay while it is on screen and for a moment after (`continuous.linger_ms`), and every line goes into **History → Live subtitles** in order, with how long it took, so you can read a cutscene afterwards; click a line for its word-by-word breakdown. Each line is logged in `maru-app.log` (`live line: on screen → read … → first word … → done …`).

For live speed comparisons, **Settings → Live subtitles** has experimental options (save and restart Maru):

* **Use CPU PP-OCR for live subtitles** (`continuous.cpu_ocr = true`) skips PaddleOCR-VL requests for the live area. Manual translation,
  hover and scans keep their configured OCR. The reading model stays loaded for those features; this option does not free its VRAM.
  PP-OCR must already be installed. Recognition can differ, so compare the captured Korean as well as the English on your own game.
* **Active live translations** (`continuous.max_in_flight`): `0` keeps the existing detached requests, `1` queues lines in capture order
  so each can see the previous completed answer, and `2` permits two overlapping translations. Waiting lines are retained for history,
  rather than discarded. With `2`, a request can start before the preceding answer is available as context.
* **Check subtitle lettering for OCR duplicates** (`continuous.filter_ocr_jitter = true`) checks changed OCR readings against the
  lettering at the subtitle's position. It can ignore an added `SP` or a changed OCR character when the lettering stayed the same,
  and an empty OCR reading cannot clear a subtitle whose lettering is still there. Changed Korean or numbers require identical
  lettering to count as duplicates. A real gap still allows the same dialogue again. Fonts without clear contrast can produce more
  repeated readings: the filter keeps uncertain changes rather than discarding them on text similarity alone.

These options default to the existing behavior (`false`, `0`, `false`). They keep the translation model, prompt and context settings.
For the duplicate experiment, keep CPU PP-OCR off and active translations at `1`. A recording comparison found fewer noisy readings
with a tighter subtitle area, but no consistent speed or duplicate reduction from the visual filter after warming the model.
Start with the filter off and try a tighter box with `Ctrl+Alt+Shift+C`: include all dialogue rows and a little margin, excluding
interface buttons. Then compare the filter off/on with that area fixed. Compare missed and misread dialogue as well as latency;
these recording results do not establish performance with a game running. See [M-12](docs/MANUAL_TESTS.md#m-12-live-ocr-duplicates-and-subtitle-area).
The log adds request IDs, waiting/active counts, time before the model call, time to response headers/first text, and llama.cpp's
prompt/decode timings when the server supplies them. **History → Live subtitles** uses capture order and the estimated appearance time;
other history views keep completion order. `maru bench --live` tests the model alone, so use the same cutscene for a full OCR-to-English
comparison; see M-11 in [manual tests](docs/MANUAL_TESTS.md).

To compare a lighter model **without replacing any app model**, build the diagnostic CLI with
`cargo build --release --locked -p maru-cli`, quit Maru from the tray, and run:

```powershell
.\target\release\maru.exe bench-live-models --baseline "C:\models\gemma.gguf" --candidate "C:\models\HY-MT1.5-1.8B-Q8_0.gguf" --out "data\live-diagnostics\comparison-1" --game-state open -n 3
```

Supply existing local files; the command never downloads them. It uses the installed llama-server (or `--server PATH`),
loads Gemma / candidate / Gemma one at a time, excludes a separate warm-up, and sends the same bundled 40 Korean lines
(30 normalized lines from the supplied cutscene transcript and 10 authored meaning checks).
The candidate defaults to the user-only translation prompts in [Tencent's HY-MT model card](https://huggingface.co/tencent/HY-MT1.5-1.8B); `--candidate-prompt maru` tests the existing live prompt
instead. By default both use temperature `0.1` and the live output cap; other sampling parameters are llama-server defaults, so this is not
Tencent's recommended sampling preset. Context resets between scenes and runs. By default preceding reference translations are
identical for both models; `--rolling-context` instead uses each model's preceding answers. `--file PATH` accepts a JSONL corpus
with the fields shown in `crates/maru-cli/testdata/live-dialogue.jsonl`. References are review aids, not certified answers.
The new output folder contains every input, exact context, translation, first-text/total latency and error in `records.jsonl`,
success-only percentiles in `report.json`, and distinct answers side by side in `review.md`. Errors produce a nonzero exit status.
No installed settings or model choice change; screen capture, OCR, vision reader, speech and overlay are absent from this test.
For GPU telemetry and full server logs, use `tools/compare_live_models.ps1` with `-Baseline`, `-Candidate`, `-Out`, `-Runs 3`
and `-GameState open`; it calls the release CLI and records `nvidia-smi` alongside the report when available.
If PowerShell blocks scripts, invoke it with a process-only bypass:
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\compare_live_models.ps1 ...` (replace `...` with those arguments).
Use a new report path each time. Compare latency **and** translation meaning; then follow
[M-15](docs/MANUAL_TESTS.md#m-15-live-translation-model-comparison) before judging another game's subtitle quality.

For versioned dialogue/evidence packs, use the [live benchmark material guide](docs/LIVE_BENCHMARK.md).
`tools/prepare_live_benchmark.py` freezes existing local inputs, preserves source hashes and separates reviewed dialogue,
authored meaning checks and unverified transcript drafts. It never downloads or changes app settings. The local v2.2 pack
adds missing Elden Ring context and a small Sekiro validation sample; its English references still need bilingual review.
The [candidate research queue](docs/LIVE_MODEL_CANDIDATES.md) records model sizes, evidence and native-prompt requirements.

`tools/bench_live_candidates.py` adds isolated comparisons for Tower+, official TranslateGemma, Seed-X-PPO, EXAONE,
Konan and Kanana using the existing llama-server and Python 3.11+ standard library. Supply approved local GGUFs; it never
downloads. For example: `python tools/bench_live_candidates.py --baseline BASELINE.gguf --candidate CANDIDATE.gguf
--server PATH/llama-server.exe --file CORPUS.jsonl --out NEW_FOLDER --profile tower -n 3` (one command line).
It saves exact requests, raw outputs, errors, model/tool hashes, paired Gemma timings and GPU telemetry.
`--context-lines 4 --context-mode rolling` tests experimental earlier dialogue; Seed and TranslateGemma accept zero
context only. `--gpu-layers 0 --threads 4` tests the candidate on CPU with device/KV/operator offload disabled;
the Gemma controls remain on GPU. No installed model is selected by this tool. See the benchmark guide for limits.
The `sub` profile uses the published Sub-model subtitle blocks. `translategemma-context` separately experiments with
completed structured translation turns; it does not change the native zero-context TranslateGemma comparison.
The runner puts CUDA's generated cache files inside its new report folder. Use `--cuda-cache-path TEST_CACHE_FOLDER`
to share a test cache across comparisons; the process environment is recorded and the parent environment is unchanged.
Run its dependency-free checks with `python -m unittest discover -s tools -p test_bench_live_candidates.py`.

For an independently approved local MiLMMT v1.0 GGUF, use `--candidate-prompt milmmt --candidate-sampling greedy --context-lines 0`.
This diagnostic uses Xiaomi's plain Korean-to-English completion prompt (no chat template), temperature 0 and top-k 1.
It requires zero context; context experiments are not part of this published prompt. The live output cap is retained and capped
or incomplete streams are recorded as errors. This does not select MiLMMT for the app.

For an experimental dialogue-context comparison, use `--candidate-prompt milmmt-context --candidate-sampling greedy`
with `--context-lines 1` or `4`; add `--rolling-context` to use each model's own prior translations. The PowerShell wrapper
accepts `-CandidatePrompt milmmt-context -CandidateSampling greedy -ContextLines 4 -RollingContext`. Earlier Korean/English
pairs are completed plain translation examples, followed by the current untranslated line. This extension is not Xiaomi's
published context template. Fixed references are an optimistic control; rolling answers expose error propagation.

QuickMT has a separate CPU-only diagnostic because its CTranslate2 model is not a GGUF. After approval for the model and native
test dependencies, create an isolated Python environment with `python -m venv data/live-diagnostics/quickmt-venv`, then install
`ctranslate2==4.8.2 sentencepiece==0.2.1` using that environment's `Scripts/python.exe -m pip install` command. Supply a local
QuickMT model folder (weights, config, vocabulary files and source/target SentencePiece tokenizers); the benchmark never downloads.
Run `tools/compare_quickmt_cpu.ps1 -Python ENV/Scripts/python.exe -Model MODEL_FOLDER -File CORPUS.jsonl -Out NEW_FOLDER`
with `-BeamSize 1`, `2`, or `5`, `-ComputeType int8` or `float32`, and `-Threads 4`. It records sequential batch-size-one CPU
calls, warm-up separately, translations, timing, errors and GPU telemetry when available. Beam 1 records first decoded text;
beam search records completion only. There is no chat context, sentence splitting, answer cache, HTTP service, OCR or overlay.
For its dependency-free logic checks, run `python -m unittest discover -s tools -p test_bench_quickmt.py`.
`tools/download_translation_tests.py --out NEW_MODEL_FOLDER` is a separate, opt-in downloader for these two candidates;
use it only after approving the downloads. It pins repository revisions and verifies published hashes for large model files.

For an existing local [HY-MT2 GGUF](https://huggingface.co/tencent/Hy-MT2-1.8B-GGUF), add `--candidate-prompt hy-mt2`
to use its published English translation/context templates. Sampling remains matched by default. Add
`--candidate-sampling tencent` to use temperature `0.7`, top-p `0.6`, top-k `20`, and repetition penalty `1.05` for the
HY-MT candidate only; both Gemma passes retain their existing live settings. Other sampler parameters remain llama-server
defaults, and the subtitle output cap is retained instead of the model card's 4,096-token allowance. Each pass records its
sampling, temperature and server arguments. The wrapper accepts `-CandidatePrompt hy-mt2 -CandidateSampling tencent`.
Run matched and Tencent settings into separate new folders to distinguish a model improvement from a sampling effect.
The Tencent option also supports the MT1.5 prompt for a fair follow-up; it refuses the Maru candidate prompt.

Other existing local candidates have separate diagnostic profiles: `--candidate-prompt lfm --candidate-sampling lfm`
uses [LFM2 KoEn's](https://huggingface.co/gyung/lfm2-1.2b-koen-mt-v8-rl-10k-merged-GGUF) English translation system instruction
and temperature `0.3`; `--candidate-prompt index --candidate-sampling greedy` uses
[Index Translate's](https://huggingface.co/IndexTeam/Index-Translate-2B) Chinese Korean-to-English instruction and temperature `0`,
with thinking disabled by the local client. These options do not download or select models for the app. First run with
`--context-lines 0` to check the published single-source prompts, then with four prior lines. LFM and Index use preceding chat
pairs in that second run, an experimental context format rather than a published context template. All candidates retain the
live output cap; unspecified samplers stay at server defaults. The wrapper also accepts `-ContextLines 0` (default `4`).
Use a reviewed custom `--file` corpus with exact visible Korean and a separate faithful meaning reference; English game
localization can differ from the Korean. Record video/dataset provenance and report each game's results separately.

For local Translate Gemma 4 Sub E4B/E2B GGUFs, use `--candidate-prompt translate-gemma-sub --candidate-sampling greedy`.
The diagnostic uses the publisher's subtitle system instruction, neutral style and separate `[PREVIOUS_SOURCE]`,
`[PREVIOUS_TRANSLATION]` and `[CURRENT_SOURCE]` blocks. It adds no speaker identity or glossary. Start with zero history,
then four reference lines and `--rolling-context` to check each model's own earlier answers.
For a local LMT-60 GGUF, use `--candidate-prompt lmt --candidate-sampling greedy`; its published user-only single-source
instruction is preserved, while preceding chat pairs are experimental. Greedy streaming does not reproduce the publisher's
five-beam example. Both profiles use temperature 0, top-k 1 and the live output cap, preserve unwanted labels in review,
and reject incomplete, malformed or token-capped streams. The wrapper accepts `-CandidatePrompt translate-gemma-sub` or
`lmt` with `-CandidateSampling greedy`. Downloads still require approval; these diagnostics do not select an app model.

**Settings** (tray → Settings…, or the Settings tab) edits `config.toml` for you: shortcuts (press the keys to record them), the panel's look, learning, screen reading, live subtitles and the translation model. Your comments in the file are kept, invalid values are refused with a reason, and each setting says whether it applies at once (shortcuts, panel) or after a restart (everything the engine reads at start-up; the page offers a restart button).

## Game menus: screen labels

Press `Ctrl+Alt+S` and Maru reads the whole screen the pointer is on (borderless-windowed games; an exclusive-fullscreen game cannot be drawn over), finds every piece of Korean text, and puts a short English label beside each.

* **Fast the second time.** The dictionary's meaning appears at once; labels you have seen before come back instantly from a local memory (`labels.db`, kept per game); only genuinely new strings go to the model, in one batched request that streams in line by line. Words you already know are faded (or hidden, per `gloss_mode`).
* **Dense menus.** Each label tries positions around its text and avoids covering any other text or label; if there is no room it shrinks, and as a last resort becomes a dot that opens when you point at it.
* **Loading and failures are distinct.** Pulsing dots mean a label is waiting for translation, including a slow NAS response. An amber **!** means the request failed or timed out; hover for its reason. A static **?** means the request finished without a translation. **Refresh labels** tries again. Existing translations stay visible if a model refinement fails; reduced-motion preferences disable the pulsing.
* **Stays out of the game's way.** The label layer ignores the mouse everywhere except directly over a label. Click a label for its breakdown (a single word opens its deep dive); right-click for **Correct Korean OCR…** or **Hide this label**; `Ctrl+Alt+Esc` clears everything.
* **Join what the detector cut in two.** Use floating **Join words** (or Shift+click) to pick pieces, then press **Join**. Words retain spaces even when their boxes touch: `카메라` + `설정` becomes `카메라 설정`. Use the chip's **One word** button only for fragments such as `게` + `임` → `게임`. Joins and their spacing choice are remembered for that program when pieces sit next to each other.
* **Separate a joined label.** Right-click → **Separate joined label** restores the original boxes and forgets the join, including saved joins from earlier sessions and joins you corrected. Selecting a joined label also offers **Separate** on the chip. Existing collapsed joins are rebuilt with word boundaries on rescan; they do not need to be recreated before separating. Sentence joins use the full snapshot analysis, with individually clickable words and deep dives.
* **Fixed translations.** Put game terms in an app profile's `glossary` and they are used verbatim and never sent to the model.
* **Try it on a screenshot first:** `maru scan menu.png --boxes found.png` lists what is found and labelled, and outlines it, without any overlay.
* **Lighter beside a game.** `lite = true` in `config.toml` (or `MARU_LITE=1` for one run) turns the OCR reading model off and swaps in a smaller translation model; see `[lite_profile]`.

## The command line

`maru translate "저는 학교에서 공부했어요"` · `maru explain 공부했어요 --in "…"` · `maru doctor` ·
`maru bench` (latency percentiles) · `maru eval-ocr <folder>` (score OCR engines on **your own** screenshots:
`name.png` + `name.txt` pairs) · `maru vocab …` · `maru dict …`. The CLI uses the same engine as the overlay.

`eval-ocr` can also pit vision-language models against PP-OCR (each runs in its own `llama-server`, one at a time;
the first run downloads the model): `maru eval-ocr <folder> --engine paddle paddle-vl hybrid gemma windows`.
On a first set of Korean screenshots PaddleOCR-VL misread far less than PP-OCR (3.5% vs 15% CER), but skipped some
blocks of text; a general chat model (Gemma 4) "corrected" clean text into different sentences and is not usable as OCR.
`hybrid` runs PP-OCR (lines and positions) and PaddleOCR-VL (whole-image reading) together and matches them by text.
(Feeding the vision model one crop per line was tried and was much worse: crops carry pieces of neighbouring lines.)
If a model will not start, run it yourself and use `--engine endpoint --vlm-endpoint http://127.0.0.1:8080`
(`--vlm-args "…"` passes extra flags such as `--jinja --chat-template-file …`). Vision models return text only, no word
positions, so they are a candidate for a "double-check" mode rather than a replacement for PP-OCR's geometry.

## How it stays fast

1. The offline breakdown (analysis, pronunciation, dictionary, your vocabulary state) is shown **before** any model
   call. With the built-in rule-based analyser the whole offline step measures about 1 ms; Kiwi's own cost
   is not measured here (run `maru bench` to see the "breakdown" row on your machine).
2. Translation and per-word meanings are requested **at the same time** on separate model slots and stream
   token by token.
3. The model server is started in the background at launch, kept warm, and its prompt prefix is cached
   (constant system prompts, `--cache-reuse`); thinking is switched off.
4. Selecting text is read **exactly** through UI Automation (no OCR error, no OCR time); OCR is the fallback.
5. Live subtitles read a line as soon as two reads in a row agree (or the picture holds still), without waiting for a fade to finish, read a moving scene without text slowly, and ignore one-character OCR jitter. They also notice lettering the instant it appears (a cheap check for bright-next-to-dark contrast, which a moving 3D backdrop does not produce), so even a one-second subtitle over a busy cutscene is read straight away and taken down as soon as it goes. Their translation is its own lane: translation only (no word breakdown), the earlier lines as chat turns, a short answer cap and low temperature. The normal translation is not touched.
6. Repeated lines reuse completed translations; missing word meanings are retried without retranslating. Words you know
   are not sent for glossing at all.

Model requests time out while waiting for response headers or stalled output (`llm.request_timeout_secs`, 60 seconds
by default). Incomplete, malformed or token-capped answers show an error and are excluded from cache and history.
English-to-Korean requests also finish with an error if the requested register is missing. Late events cannot replace
a newer panel answer; overlapping live translations retain dialogue context in capture order.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and the extension points.

## Correcting snapshot OCR

In a region snapshot's translation panel or on a whole-screen scan label, **right-click → Correct Korean OCR…**.
The editor shows what OCR originally read: replace `ㅅ컵` with `스킵`, for example, and press
**Apply and retranslate**. It reuses the captured text and area; it does not capture a new screen or change the live watcher.
Stale editors are refused after a newer snapshot/scan replaces their source. Old word boxes are discarded for corrected text.

Left-click opens the label's explanation; correction and **Hide this label** live in the right-click menu. The floating
menu's **Label screen** button toggles labels on/off (green means on). While labels are on, **Refresh labels** reads the
screen again, and **Join words** enables selection: click each word, then press the **Join** chip to form one translated
sentence. Joining returns to ordinary left-click behavior; press Join words again or cancel to exit selection without
joining. Shift+click joining and the tray/`Ctrl+Alt+S` rescan action remain available.

**Remember this exact correction for this game / app** is optional and unchecked for new rules. Remembered fixes apply
before the translation/cache lookup, to the exact detected text in the captured executable only. They are not substring
replacements or fuzzy spelling rules. Games running through the same executable, such as `eden.exe`, share that scope.
**Forget saved rule** removes the remembered mapping; unchecking Remember merely applies the current edit once.
See [M-25](docs/MANUAL_TESTS.md#m-25-snapshot-ocr-corrections).

The different kinds of remembered data have different lifetimes:

* **Translation cache:** at most 256 results in RAM, keyed by normalized source text, direction, register and profile
  name/glossary/hint. Live translation has separate entries, including the chosen live model. Surrounding dialogue is
  not part of the key. Special live-model/quality-server snapshots currently bypass the ordinary translation cache.
* **Recent history:** bounded in-memory entries (300 by default), cleared on exit. A new uncached translation adds a
  corrected entry without erasing the earlier one; ordinary cache hits replay in the panel without adding another
  history entry. Context is also temporary, kept per program for up to 15 minutes.
* **Screen labels:** short translations persist in `labels.db`, with a normalized text key and program scope. A matching
  label from another program can be a fallback for common UI text. These are not durable full-sentence history.
* **OCR rules:** exact, program-scoped detected-text → corrected-text mappings persist separately in `labels.db`.
  They never use cross-program fallback. Vocabulary, learning contexts and Anki recovery records persist in `vocab.db`;
  actual Anki notes and reviews live in your Anki collection.

## What is verified

**Checked automatically** (about 465 tests: `cargo test --workspace`, `cargo test --manifest-path app/src-tauri/Cargo.toml`,
`node --test app/ui/test/*.test.mjs`; CI runs them on Windows and Linux, plus clippy with warnings denied):

* Korean pronunciation and romanization rules, 어절 grouping, dictionary import, FSRS-6 (replays the reference
  implementation's own test vector), vocabulary, Anki (against a **fake** AnkiConnect).
* The language-model client, streaming parsers and process supervision (against scripted servers), and the whole
  engine pipeline: event order, cancellation, cache, English→Korean follow-up analysis, degraded modes.
* The screen scan end to end through the real engine with a fake screen, OCR and model: dictionary-first labels, remembered
  labels, glossary, known-word fading, and manual joins (made, remembered per program, forgotten, refused when stale).
* Live subtitles: the watcher's change logic, the translation-only lane (context turns, caching, history with timings) and the band's
  placement and fade logic.
* The setup wizard's decisions: parsers fed with real `nvidia-smi` and PowerShell output, the hardware → install recommendation,
  the questions (scripted answers, cancelling on end of input) and in-place edits of `config.toml`.
* The OCR pipeline against **real ONNX Runtime and real PP-OCR models** on rendered English text, including word-box
  coordinates; the real `maru` binary end to end; pages rendered in real Chromium (panel, scan labels, the Join chip, the live band,
  History) from recorded or mocked event streams.
* The Windows-only code (UI Automation, clipboard, GDI capture, Windows OCR) **type-checks** against the real `windows` crate, and the
  Tauri shell builds against the real Tauri 2 API on the Windows CI job.

**Not verified; needs a real PC.** Each item has a step-by-step test and a list of what to bring in
**[docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md)**; results are recorded there, and an empty results log means *not yet checked*:

* Setup wizard hardware detection on Windows, and the full install path (M-1, M-2).
* Live subtitles: real OCR on a moving picture, band placement over real games, and the latency numbers (M-3).
* Screen scan and manual join in a real game, including click-through behaviour (M-4).
* UI Automation selection across programs, the Ctrl+C fallback, GDI capture, Windows OCR (M-5) and that the overlay never steals
  focus, is transparent and is excluded from capture (M-6).
* Korean **OCR accuracy**, real **model quality and speed**, **Kiwi** live (M-7).
* **Speech**: the `tts-server` Windows build and the Qwen3-TTS models (M-8).
* **Anki** against the real AnkiConnect (M-9).
* The settings page with a real config, and lite mode beside a game (M-10).
* Download locations I could not reach from the build sandbox: the OCR model files on Hugging Face (several layouts are tried), the
  Wiktionary extract, llama.cpp release assets. Every URL can be overridden on the command line.

Known limits: pronunciation rules that need morpheme knowledge (ㄴ-insertion in compounds, 사이시옷) are not
guessed; word meanings in context come from the model, so they are only as good as the model; the scan never merges OCR boxes by itself
(you join them by hand), because tight menus and inventory grids would be glued together wrongly.

## Layout

```
crates/   maru-core  maru-lang  maru-dict  maru-srs  maru-llm  maru-ocr  maru-platform  maru-tts  maru-engine  maru-cli
app/      src-tauri (overlay shell: windows, hotkeys, tray, commands)   ui (panel, picker, hub, scan overlay, live band: plain ES modules, no build step)
docs/     ARCHITECTURE.md  MANUAL_TESTS.md  config.example.toml
AGENTS.md CLAUDE.md   rules for anyone (person or agent) changing the code; CLAUDE.md just imports AGENTS.md
.github/  workflows: ci.yml (build + tests), tts-server.yml (builds the speech server)
old/      the original Python prototype, kept for reference
```


### Live model trials and floating controls

Settings now has categories and search, with staged Save/Discard edits. In **Live subtitles**, use
**Test connection** beside the snapshot server fields to check the entered address and model before saving.
The same button appears in **Translation model**. It sends a fixed Korean greeting with a translation instruction,
shows the server's reply and elapsed time, and reports connection/model errors. It may load the server model;
it does not save settings or load a local Maru model. Allow up to 60 seconds for a cold server, then retry if needed.
You can enter just an IP or hostname and press **Detect server**: Maru checks ports **11434, 1234 and 8080**
and offers the server's models in a dropdown. **Test connection** also detects a bare address first; choose a model
if its name differs from the current field. Explicit ports and custom URL prefixes are preserved. Selecting a server
or model stages ordinary edits; **Save** and the usual restart still apply them. Manual model names remain available.
**Find LAN servers** first lists your local Windows IPv4 networks. Choose one and press **Search this network**
to search those three ports with progress and **Cancel search**. Searches are limited to private networks of 256
addresses or fewer; a larger network shows the specific /24 portion that will be searched. Cancelled LAN searches
keep already found servers. Discovery sends metadata requests only, without inference or model downloads, and does
not run automatically when Settings opens. Authentication, custom ports and isolated networks may need manual setup.
For a real-game trial, choose **HY-MT2 7B Q4_K_M**, **MiLMMT 4B Q6_K**, **HY-MT2 1.8B Q4_K_M** or
**LFM2 KoEn 1.2B Q5_K_M**. The cards discover the already downloaded
benchmark files in a development checkout, or files in Maru's `data/models/live/` folder; you can paste an
explicit GGUF path instead. No model is downloaded by this page. Save and restart Maru to apply model changes.
GPU is fastest; the CPU option uses four threads by default. Dialogue context is experimental and off by default
because earlier model answers sometimes introduced meaning errors. Keep the same clip and subtitle area for comparisons.

For a tight GPU budget, try HY-MT2 1.8B first. Its preset uses one slot, a 1,024-token context and smaller batch buffers;
LFM2 uses one slot and a 2,048-token context. In a model-only test on the RTX 5070 Ti, dedicated VRAM peaked at
1.44 GB and 1.13 GB respectively (decimal GB). Both made meaning errors; LFM2 changed more names, roles and instructions.
These are optional choices, not changes to your saved model. Keep experimental dialogue context off for the first trial.
The hybrid OCR vision reader is an additional cost: 2.36 GB for the measured subtitle crop, bringing the two owned-server
totals to approximately 3.80 GB / 3.48 GB before the game, desktop and Maru itself. **Use CPU PP-OCR for live subtitles**
bypasses that reader for live mode, with a separate recognition-quality tradeoff. Other crop sizes and GPUs can differ.
See [M-22](docs/MANUAL_TESTS.md#m-22-small-live-models-and-remote-deep-dive-recovery) for the real-game check.

With a configured external main model, a missing or failed local live translator no longer disables normal remote
screen labels or deep dives after a restart. Remote request failures keep the offline breakdown and can be retried;
they do not ask you to install a local model just because the optional live model is unloaded.

The floating **마** button opens the tray actions above a borderless game: live toggle, area selection, snapshots,
hover, screen labels, speech, history/review/status/settings and Quit. Drag the button to move it; its position is
remembered. **Ctrl+Alt+M** or the tray's **Show / hide floating controls** brings it back after Hide. You can change
that shortcut or turn startup visibility off in Settings. Transparent space passes clicks through, and the controls
are excluded from captures. Exclusive fullscreen and the new layout still need a real-PC check (M-17).

The menu also shows the selected **live translator and device**, **live OCR mode/readiness**, and how many lines are
translating or waiting, including pending lines after live is turned off. CPU PP-OCR shows when an unused vision reader
is still resident from another feature. The VRAM bar continues to include all apps, including the game.

Before a cutscene, leave live off and let pending requests finish, then click **Prepare live session**. It loads and
warms the selected translator and OCR with a fixed Korean greeting and a bundled synthetic subtitle image. Progress
shows translator/OCR preparation and any failure. It does not capture your screen, enable live, add history/context,
load speech or use the snapshot quality server. If your main translator is external and selected for live, that server
receives the fixed greeting. Preparation reduces first-use work but does not guarantee every image shape or later
subtitle will be fast. Turn live on after it reports Prepared. Preparing is optional; ordinary on-demand loading remains.

**Release idle models** reclaims memory from Maru's owned local translator, OCR reader and voice. Active requests and
loads keep their leases; the translator and hybrid reader needed by live capture remain resident while live is on.
With CPU PP-OCR live selected, an unused reader from a snapshot can be released. The menu says what was released or kept;
external servers, model files, history and settings are retained. Stop live and let pending work finish to release all
idle servers. The next request reloads a released model, so it can take longer. See [M-23](docs/MANUAL_TESTS.md#m-23-explicit-model-preparation-release-and-live-status).

While live subtitles are on, Korean snapshots reuse the selected live translator and show the offline breakdown
and dictionary meanings. They do not load local Gemma for richer analysis. An optional **Snapshot quality server**
address and model name sends snapshot/deep-dive text and context to that explicitly configured server; leave it blank
to stay local. If it is unavailable, Korean snapshots fall back to local translation plus offline meanings. Turning
live off restores snapshots to the usual `[llm]` Gemma setup, loaded on the next request. Switching waits for active
model streams and can take time to load the weights. Pending subtitles remain in history; after live is turned off,
lines not yet started use the restored main model. English-to-Korean and model explanations need the quality server
or live mode off. Main-model live mode keeps its existing prompts and capabilities. OCR and speech can still have
separate model processes. Current trial instructions and verification limits are in [M-17](docs/MANUAL_TESTS.md#m-17-live-model-settings-snapshot-routing-and-floating-controls).

### Resources and local debug recording

Opening Maru, Settings, or Resources does not load the local translation, OCR vision, or speech models.
Live subtitles load the selected translator when switched on; other translation requests load Gemma when needed.
OCR waits for its configured vision reader on first use, and speech loads its voice on first use. Enabling
**Use CPU PP-OCR for live subtitles** bypasses that vision reader for live mode; other features keep their OCR setup.
The first request can therefore take longer. Once loaded, models remain available for later requests; translation
model switching still unloads the previous owned translator. The CLI keeps its existing startup behavior.

Open **Resources** in the hub or floating menu to see CPU, process RAM, GPU activity, dedicated VRAM and shared GPU
memory for Maru and its owned translation, OCR-reader and speech servers. The main and selected live model have separate
rows; unloaded or external models have no local measurements. PP-OCR and Windows OCR run inside Maru, so their resource
cost is included in the Maru row. GPU activity is the busiest engine, not a percentage that can be added across rows.
Memory counters are estimates and shared allocations can overlap; missing readings say **Unavailable**.

The floating menu's VRAM bar includes **all applications**, including the game. With several GPUs it shows the greatest
valid VRAM occupancy and names that adapter. **High** starts at 80%, **critical** at 95%; these are visual thresholds,
not a diagnosis of a bottleneck. Unsupported and stale readings never appear as a green zero.

Counters update about once per second while Resources or the expanded menu is visible. **Record resource session**
keeps sampling after the view closes and writes timestamps, process/model identities, queue depth and live/panel request
timings to `data/resource-logs/resources-*.jsonl`; the page shows the exact path. It does not copy subtitle or error text.
Stop recording before sending the file with `maru-app.log`. Logs are limited to 16 MiB per session and 64 MiB total;
old logs are retained, so move them yourself if the folder reaches its limit. **Pause monitoring** stops recording and
counter collection; Resume starts monitoring on demand again. Pause/recording controls reset when Maru restarts and need
no config edit or model download. Compare the same scene with monitoring paused, visible, and recording to measure its
actual cost while playing. See [M-18](docs/MANUAL_TESTS.md#m-18-resource-monitor-and-vram-menu-bar).
