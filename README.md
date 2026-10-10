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

```powershell
cargo install --path crates/maru-cli       # builds it and puts `maru.exe` in %USERPROFILE%\.cargo\bin (already on your PATH)
maru config init      # writes the config file (with the model below); `maru config path` shows where it is
maru setup llm        # installs llama-server and downloads the model
maru setup kiwi       # accurate Korean grammar analysis; each candidate (Kiwi >= 0.23) is self-tested in a child process before it is trusted
maru setup ocr        # OCR models for images/video/games
maru setup ocr-vl     # optional, ~1.8 GB: a reading model that fixes most of PP-OCR's misread letters
maru dict download    # offline dictionary (Wiktionary)
maru doctor --deep    # checks everything, runs a real translation

cd app/src-tauri; cargo run --release      # the overlay (lives in the system tray)
```

(`cargo build --release -p maru-cli` also works, but it only creates `target\release\maru.exe`; run it as
`.\target\release\maru.exe setup llm` or add that folder to your PATH.)

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
| `Ctrl+Alt+C` | live mode for subtitles and dialogue boxes (translates when the text changes), on the last chosen area |
| `Ctrl+Alt+Shift+C` | choose a different live-mode area (and start it there) |
| `Ctrl+Alt+S` | label every Korean text on the screen (game menus, HUDs); press again to rescan |
| `Ctrl+Alt+L` | history, review and status |
| `Ctrl+Alt+Esc` | hide the panel and all screen labels |

Each action accepts several keys separated by `|` in `config.toml` (e.g. `translate = "Ctrl+Alt+T | F6"`), which helps in games that use Ctrl/Shift/Alt. A registered key is taken from every program while Maru runs, so pick ones you don't use (see `docs/config.example.toml`). `Status` shows which toggles are on, and the tray icon's tooltip lists them too.

**Speech.** `maru setup tts` installs a local Korean voice (Qwen3-TTS through qwentts.cpp's `tts-server`, about 2.5 GB; Windows build from this repository's `tts-server` workflow) and `maru tts-eval` writes a page that plays the same tricky sentences (sound changes, numbers, intonation, a paragraph) in each voice so you can judge by ear. To keep the GPU free for a game, point `[tts]` at a server on another PC or at any OpenAI-style speech API (`provider = "endpoint"`); both are in Settings → Speech, with a Test voice button. Long text is spoken sentence by sentence (the next one is made while one plays) and every sentence is cached. In the panel, 🔊 reads the text aloud with the spoken sentence and its words highlighted, with previous / pause / next / say-again / repeat-each-sentence controls; the deep dive has 🔊 buttons for a single word; `Ctrl+Alt+P` (`hotkeys.speak`) translates the selection and reads it, and stops when pressed again. The panel stays up while it reads and dismissing it stops the voice.

**Moving the panel.** Drag it by its top bar (the ⠿ grip) to put it anywhere: it pins itself and stays there while it translates the next thing, however much it grows. Unpin (📌) to let it go back next to the text again.

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
5. Live mode OCRs only after the picture changed *and then held still*, and ignores one-character OCR jitter.
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
