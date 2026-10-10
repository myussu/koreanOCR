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

> Status: the engine, command line and overlay are implemented and tested, but the Windows-specific parts and the
> Korean model quality could only be checked on a real machine. See **[What is verified](#what-is-verified)**.

## Quick start (Windows)

**You need** (once): Windows 10/11 (64-bit), [Rust](https://rustup.rs) with the *Visual Studio Build Tools* ("Desktop development with C++") it asks for, Git, and the WebView2 runtime (already part of Windows 11; the app installs it on Windows 10). About 12 GB of disk for the models. Anki is optional.

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
| `Ctrl+Alt+C` | live subtitles: the translation appears over the original line as soon as it is read, on the last chosen area |
| `Ctrl+Alt+Shift+C` | choose a different live-mode area (and start it there) |
| `Ctrl+Alt+S` | label every Korean text on the screen (game menus, HUDs); press again to rescan |
| `Ctrl+Alt+L` | history, review and status |
| `Ctrl+Alt+Esc` | hide the panel and all screen labels |

Each action accepts several keys separated by `|` in `config.toml` (e.g. `translate = "Ctrl+Alt+T | F6"`), which helps in games that use Ctrl/Shift/Alt. A registered key is taken from every program while Maru runs, so pick ones you don't use (see `docs/config.example.toml`). `Status` shows which toggles are on, and the tray icon's tooltip lists them too.

**Speech.** `maru setup tts` installs a local Korean voice (Qwen3-TTS through qwentts.cpp's `tts-server`, about 2.5 GB; Windows build from this repository's `tts-server` workflow) and `maru tts-eval` writes a page that plays the same tricky sentences (sound changes, numbers, intonation, a paragraph) in each voice so you can judge by ear. To keep the GPU free for a game, point `[tts]` at a server on another PC or at any OpenAI-style speech API (`provider = "endpoint"`); both are in Settings → Speech, with a Test voice button. Long text is spoken sentence by sentence (the next one is made while one plays) and every sentence is cached. In the panel, 🔊 reads the text aloud with the spoken sentence and its words highlighted, with previous / pause / next / say-again / repeat-each-sentence controls; the deep dive has 🔊 buttons for a single word; `Ctrl+Alt+P` (`hotkeys.speak`) translates the selection and reads it, and stops when pressed again. The panel stays up while it reads and dismissing it stops the voice.

**Moving the panel.** Drag it by its top bar (the ⠿ grip) to put it anywhere: it pins itself and stays there while it translates the next thing, however much it grows. Unpin (📌) to let it go back next to the text again.

**Live subtitles** draw the English in a dark band over the original line, stay while it is on screen and for a moment after (`continuous.linger_ms`), and every line goes into **History → Live subtitles** in order, with how long it took, so you can read a cutscene afterwards; click a line for its word-by-word breakdown. Each line is logged in `maru-app.log` (`live line: on screen → read … → first word … → done …`).

**Settings** (tray → Settings…, or the Settings tab) edits `config.toml` for you: shortcuts (press the keys to record them), the panel's look, learning, screen reading, live subtitles and the translation model. Your comments in the file are kept, invalid values are refused with a reason, and each setting says whether it applies at once (shortcuts, panel) or after a restart (everything the engine reads at start-up; the page offers a restart button).

## Game menus: screen labels

Press `Ctrl+Alt+S` and Maru reads the whole screen the pointer is on (borderless-windowed games; an exclusive-fullscreen game cannot be drawn over), finds every piece of Korean text, and puts a short English label beside each.

* **Fast the second time.** The dictionary's meaning appears at once; labels you have seen before come back instantly from a local memory (`labels.db`, kept per game); only genuinely new strings go to the model, in one batched request that streams in line by line. Words you already know are faded (or hidden, per `gloss_mode`).
* **Dense menus.** Each label tries positions around its text and avoids covering any other text or label; if there is no room it shrinks, and as a last resort becomes a dot that opens when you point at it.
* **Stays out of the game's way.** The label layer ignores the mouse everywhere except directly over a label. Click a label for its breakdown (a single word opens its deep dive); hover for a small **×** (or right-click) to delete one that is in the way; `Ctrl+Alt+Esc` clears everything.
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
5. Live subtitles read a line as soon as two reads in a row agree (or the picture holds still), without waiting for a fade to finish, read a moving scene without text slowly, and ignore one-character OCR jitter. Their translation is its own lane: translation only (no word breakdown), the earlier lines as chat turns, a short answer cap and low temperature. The normal translation is not touched.
6. Repeated lines come from a cache; words you know are not sent for glossing at all.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and the extension points.

## What is verified

Checked here, automatically (about 200 tests; `cargo test --workspace`, `node --test app/ui/test/core.test.mjs`):

* Korean pronunciation and romanization rules, 어절 grouping, dictionary import, FSRS-6 (replays the reference
  implementation's own test vector), vocabulary, Anki (against a fake AnkiConnect).
* The language-model client, streaming parsers and process supervision (against scripted servers), and the whole
  engine pipeline: event order, cancellation, cache, English→Korean follow-up analysis, degraded modes.
* The OCR pipeline against **real ONNX Runtime and real PP-OCR models** on rendered English text, including
  word-box coordinates; the real `maru` binary end to end; the panel rendered in real Chromium from event streams
  recorded from the real engine.
* The Windows-only code (UI Automation, clipboard, GDI capture, Windows OCR) **type-checks** against the real
  `windows` crate; the Tauri shell compiles against the real Tauri 2 API.

**Not verified; needs your machine** (each has a diagnostic: `maru doctor`, `maru eval-ocr`, `maru bench`):

* Korean **OCR accuracy** (PP-OCRv5 Korean, Windows OCR): no Korean model or font was available.
* Real **model quality and speed** (Gemma 4 12B on an RTX 5070 Ti): measure with `maru bench`.
* **Kiwi** live (downloading `libkiwi`, real tokenization): the adapter compiles and its mapping is tested.
* UI Automation selection across your apps, the Ctrl+C fallback, GDI capture, Windows OCR at run time.
* **Overlay behaviour on Windows**: that the panel never steals focus, is transparent, is excluded from capture.
* Download locations I could not reach from the build sandbox: the OCR model files on Hugging Face (several
  layouts are tried), the Wiktionary extract, llama.cpp release assets, the bundled ONNX Runtime (`ort` only
  publishes MSVC binaries). Every URL can be overridden on the command line.
* The CI workflow in `.github/workflows/` is the intended check for the Windows build.

Known limits: pronunciation rules that need morpheme knowledge (ㄴ-insertion in compounds, 사이시옷) are not
guessed; word meanings in context come from the model, so they are only as good as the model.

## Layout

```
crates/   maru-core  maru-lang  maru-dict  maru-srs  maru-llm  maru-ocr  maru-platform  maru-engine  maru-cli
app/      src-tauri (overlay shell)   ui (panel, picker, hub: plain ES modules, no build step)
docs/     ARCHITECTURE.md  config.example.toml
old/      the original Python prototype, kept for reference
```
