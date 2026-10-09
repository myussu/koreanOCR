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

> Status: the engine, command line and overlay are implemented and tested, but the Windows-specific parts and the
> Korean model quality could only be checked on a real machine. See **[What is verified](#what-is-verified)**.

## Quick start (Windows)

```powershell
cargo install --path crates/maru-cli       # builds it and puts `maru.exe` in %USERPROFILE%\.cargo\bin (already on your PATH)
maru config init      # writes the config file (with the model below); `maru config path` shows where it is
maru setup llm        # installs llama-server and downloads the model
maru setup kiwi       # accurate Korean grammar analysis (tries older Kiwi releases if the newest has no Windows build)
maru setup ocr        # OCR models for images/video/games
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
| `Ctrl+Alt+C` | live mode for subtitles and dialogue boxes (translates when the text changes) |
| `Ctrl+Alt+L` | history, review and status |

## The command line

`maru translate "저는 학교에서 공부했어요"` · `maru explain 공부했어요 --in "…"` · `maru doctor` ·
`maru bench` (latency percentiles) · `maru eval-ocr <folder>` (score OCR engines on **your own** screenshots:
`name.png` + `name.txt` pairs) · `maru vocab …` · `maru dict …`. The CLI uses the same engine as the overlay.

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
  implementation's own test vector), vocabulary, Anki export (against a fake AnkiConnect).
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
