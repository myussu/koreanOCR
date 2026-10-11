# Working in this repository (for agents and people)

Maru is a local Korean reading assistant: a Rust engine with thin front ends (a Tauri overlay and a CLI). **Read
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first** — especially §2 (layering rules), §4 (the feature map: every feature, its files,
its tests) and §6 (the contracts). This file is the short version of *how to change things without breaking other work*.

## Before you start
1. `git pull --rebase origin main`; read the feature-map entry for the area you will touch (or the closest one).
2. Prefer **new files and modules** for new work. A new feature should mostly be code that did not exist before, plus small, additive
   registrations in the shared files below.
3. If the task would change a contract (§6 of the architecture doc) in a way that is not purely additive, stop and ask the user.

## Areas and who owns what
Work inside one area at a time. Touching another area's files is fine for a small, necessary change — say so in the commit message — but
never refactor or restyle them.

| Area | Owns (files) |
|---|---|
| Translate · hover · deep dive | `maru-engine/src/{pipeline,capture,annotate,cache,memory}.rs`, `maru-llm/src/{prompts,tasks,demux}.rs`, `app/ui/{panel,core}.js` |
| Learning · Anki | `maru-srs/`, `maru-engine/src/anki_flow.rs`, `app/ui/anki.js`, the `anki_*` commands |
| Screen scan · manual join | `maru-engine/src/{scan,labels,joins}.rs`, `app/ui/{scan,scan-core,layout}.js`, `scan.css/html`, `scan_*` commands |
| Live subtitles | `maru-engine/src/{continuous,change}.rs`, `maru-llm` `translate_live`, `app/ui/{live,live-core}.js`, `flow::start_continuous`, `logic::band_*` |
| Speech | `maru-tts/`, `maru-cli/src/tts.rs`, `app/ui/reader.js`, `tts_*` commands, `.github/workflows/tts-server.yml` |
| Settings | `maru-core/src/settings.rs`, `app/src-tauri/src/settings.rs`, `app/ui/{settings,settings-core}.js` |
| Setup wizard · installers | `maru-engine/src/{hardware,plan}.rs`, `maru-cli/src/{wizard,setup}.rs` |
| OCR tiers | `maru-ocr/`, `maru-engine/src/{hybrid_ocr,vlm_ocr,vision,respace}.rs` |
| Language data | `maru-lang/`, `maru-dict/` |
| Windows integration | `maru-platform/` (everything Windows-specific stays here, behind traits) |

## Shared files: edit minimally, additively
These are touched by almost every feature, so they are where parallel work collides. Add; do not reorder, rename or reformat.
Put new logic in your own module and only *register* it here.

`maru-core/src/event.rs` (new variants at the end of their group) · `maru-core/src/config.rs` (new field with `#[serde(default)]`) ·
`maru-engine/src/engine.rs` (new `Engine` method / `Inner` field) · `maru-engine/src/bootstrap.rs` · `app/src-tauri/src/main.rs`
(handler list, tray menu) · `commands.rs` · `flow.rs` · `state.rs` · `windows.rs` · `capabilities/default.json` ·
`docs/config.example.toml` · `app/ui/settings-core.js` · `app/ui/hub.js` · `.github/workflows/ci.yml` · `README.md` · `docs/*`.

## Contracts you must not break
* **Events:** add variants, never repurpose or reorder; consumers tolerate unknown ones. Order guarantees are in architecture §6.1.
* **Config:** old `config.toml` files must keep parsing (new fields have defaults); never rename or repurpose a key.
* **Databases:** additive only (`CREATE TABLE IF NOT EXISTS`, new tables/columns); never change what an existing column means.
* **Commands and window labels:** a page and the shell are built together, but keep names stable; JS passes camelCase, Rust is snake_case.
* **`bin/`** holds llama.cpp's loose files only; other tools get their own subfolder (their DLLs would replace llama.cpp's).
* **Layering:** the engine never touches UI; front ends hold no business logic; Windows code only behind `maru-platform` traits.

## Definition of done
Run all of these; CI runs the same.
```
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo clippy --manifest-path app/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path app/src-tauri/Cargo.toml
node --test app/ui/test/*.test.mjs
```
* New logic comes with tests, written as sentences (`a_join_on_a_scan_that_is_no_longer_current_is_refused…`). Pure logic gets unit
  tests; engine behaviour goes in `crates/maru-engine/tests/pipeline.rs` against the scripted model server and fake OCR/screen;
  UI decisions go in a pure `*-core.js` with a `node --test` file.
* **A new `*.test.mjs` file must be added to the list in `.github/workflows/ci.yml`** (it does not use a glob).
* Changes to the live-subtitle watcher (`continuous.rs`, `change.rs`) are also replayed against a recorded clip with
  `crates/maru-engine/tests/replay.rs` (see its header; needs `ffmpeg` and a clip you were given — never commit a clip).
* Layout and interaction changes in `app/ui` are checked in real Chromium through the page's browser demo (`?demo=…`, mocked commands);
  look at a screenshot, do not assume.
* After pushing, check the CI run on the commit; a red push is yours to fix.
* If the disk fills, delete `target/` and `app/src-tauri/target/` (always safe, regenerable).

## Documentation is part of the change
* **README.md** — what a user can do and how to install/use it.
* **docs/ARCHITECTURE.md** — add or update your feature-map entry (§4), the contract tables (§6) and the persistence table (§6.4).
* **docs/MANUAL_TESTS.md** — add an `M-n` entry for anything that needs a real PC, and say honestly why it is unverified.
* **docs/config.example.toml** — every new setting, documented.
* **Say what is verified and what is not.** "Tested against a fake server" is not "works with the real thing". Never put a result in the
  results log (`MANUAL_TESTS.md`) that a person did not actually observe.

## Style
* Match the surrounding code: comments explain *why* and are no denser than their neighbours; names are plain; no clever abstractions
  for one use. No `unwrap()`/`expect()` on paths a user can reach; failures become `Error` events or `Result`s with a plain-language message.
* Anything the user must do differently (a new step, a changed key, a new prerequisite) goes in the README in the same commit.
* Commits: an imperative subject, a body that says *why*. Do not open a pull request unless asked. Never rewrite history on a shared branch.

## Ask the user first
New downloads or services, new dependencies with native code, changing a default that changes behaviour for existing users, anything that
sends data off the machine, deleting user data, or anything the architecture doc calls a contract.

## Known pitfalls
* Windows-only code only **type-checks** on Linux CI; treat it as unverified until a person has run it (and say so).
* The app crate (`app/src-tauri`) has its own lockfile and `target/`; dependency changes in `maru-engine` can touch its `Cargo.lock`.
* A scan's "program" is captured when the scan starts (`Inner::scan_scope`); clicking the overlay can change the foreground window.
* The scan and live windows are click-through; the shell enables clicks only where the page reports interactive rectangles — a new clickable
  element there must be added to what `scan.js::report()` sends.
* ONNX Runtime links only with a recent Visual Studio C++ toolset (`unresolved external symbol __std_…` means update it).
