# What only a real PC can verify

Everything in `cargo test` and `node --test` runs without Windows, a GPU, a game or Anki. The items below cannot, so they are
checked by hand. Each has an id (`M-n`) that [ARCHITECTURE.md](ARCHITECTURE.md) and the README point to.

**How to use this file**
1. Do the setup, then the tests you care about. Tests are independent unless noted.
2. For anything that fails, bring the **exact text** of the message, or a screenshot, plus the items under *Bring*.
3. Record the outcome in the [results log](#results-log) at the bottom, so the next person (or agent) knows what is still unproven.
   Nothing here counts as verified until it is in that log.

**Where the files are:** `maru config path` prints the config file. Maru's data folder (it holds `labels.db`, `vocab.db`,
`maru-app.log`) is printed by `maru doctor` as `data:`.

## Setup, on each PC

```powershell
git pull
cargo install --path crates/maru-cli      # the `maru` command
cd app\src-tauri; cargo run --release     # the overlay (quit the running one first)
```
If the build ends with `unresolved external symbol __std_…`, update Visual Studio's C++ build tools (see the README).

---

## M-1 Setup wizard: hardware detection and recommendation
**Needs:** any Windows PC. Best on each different PC you own (NVIDIA, AMD/Intel, laptop with integrated graphics).
**Steps:** `maru setup wizard --dry-run` (changes nothing).
**Expect:** your graphics card(s) with the right name and (NVIDIA) memory, the NVIDIA driver and its CUDA version, memory and free disk
sensible, a recommendation that matches the hardware (CUDA + full model for a 10 GB+ NVIDIA card, Vulkan for AMD/Intel, CPU for
integrated or none), no card listed that is not there.
**Bring:** the whole output; the real specs of the PC if anything looks wrong (card, VRAM, RAM, drive).
**Why unverified:** the PowerShell/CIM probe and Windows' adapter names were only tested against sample text.

## M-2 Install and build
**Steps:** run the setup above; then `maru setup` (answer the questions) or `maru setup wizard --yes`; then `maru doctor --deep`.
Also check: running `maru setup` a second time skips what is installed; ending the input (Ctrl+D) at the last question installs nothing;
`maru setup wizard --backend vulkan --force` replaces a CUDA build (quit Maru first).
**Expect:** every step finishes or fails with a clear message and a way to re-run; `doctor --deep` translates a sentence.
**Bring:** the output of any failing step; `maru doctor` output.
**Why unverified:** downloads (llama.cpp, Kiwi, OCR models, dictionary) could not be reached from the build sandbox; the Windows link
step depends on the C++ toolset.

## M-3 Live subtitles
**Needs:** a game or video with Korean subtitles, borderless/windowed (an exclusive-fullscreen game cannot be drawn over).
**Steps:** `Ctrl+Alt+Shift+C`, drag a tight box around the subtitle strip; play a cutscene; then open History (`Ctrl+Alt+L`) → **Live subtitles**.
Press `Ctrl+Alt+C` to stop and start. Try a subtitle that fades, one that appears over a moving scene, and one that is two lines.
**Expect:** the English appears just above or below the original (the side with more room) soon after it appears and never covers it; it stays while the original is on screen
and about `linger_ms` after; every line is in History → Live subtitles, in order, with timings; the band never gets read back as input.
**Bring:** the "N s to the first word · N s in all" numbers for 5–10 lines; any `live line:` lines from `maru-app.log`; which side the band sits on and whether it ever touches the original, whether it
sits in the right place, wraps badly, lingers too long or too briefly; the game and its resolution.
Optional: a 15–30 s clip (under ~50 MB) pushed to a `live-clip` branch under `clips/`, or 4–5 screenshots of one subtitle line (appearing, mid-fade, gone).
**Replay tool:** a recorded clip can be replayed through the real watcher without the game: `MARU_CLIP=clip.mp4 MARU_CLIP_TRUTH=truth.txt
cargo test -p maru-engine --release --test replay -- --ignored --nocapture` (needs `ffmpeg`; `truth.txt` has one `onset off` pair, in seconds,
per subtitle; see the file header). It checks the watcher's timing, not recognition quality or the model's speed.
**Why unverified:** needs real OCR, a real model and a real screen. Latency numbers can only come from your hardware.

## M-11 Live subtitles: the model on this PC against the model on another PC
**Why:** the model, the reading model, the speech server and the game all want the same GPU; a few lines took 1–1.8 s to the first word when most
took 0.4–0.5 s. This test says how much of that is the GPU being shared, and what a second PC buys.
**Needs:** a second PC on the same network with a GPU, serving the same model (README → "A model on another PC"), and the recorded cutscene (M-3).
**Part 1: the model alone, no screen, no OCR (about a minute each).** Quit the Maru overlay first (it holds the local model's port and GPU memory).
| Run | Setup | Command |
|---|---|---|
| 1 | game closed, local model | `maru bench --live -n 3` |
| 2 | game running (a scene on screen), local model | `maru bench --live -n 3` |
| 3 | game running, model on the other PC | `maru bench --live --endpoint http://<that-pc>:8080 -n 3` |
Run 1 against 2 is what sharing the GPU with the game costs; 2 against 3 is what the second PC buys. Run 3 also with the game closed if you can.
**With Ollama on the other PC** (for example TrueNAS SCALE) instead of llama-server: pull the same model (`ollama pull hf.co/unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL`, if
that Ollama is new enough for it), check `ollama list` for the name and `ollama ps` for `100% GPU` while it is loaded, set `OLLAMA_KEEP_ALIVE=-1` (and
`OLLAMA_NUM_PARALLEL=2`) on the server, and use `maru bench --live --endpoint http://<that-pc>:<port> --model "<name from ollama list>" -n 3`. For the cutscene runs set
the same endpoint and `[llm] model_name` (Settings → Translation model). Maru asks such servers not to think (`[llm] reasoning_effort = "none"`; a person testing live subtitles reported that without it a thinking model used the whole
token limit on reasoning and returned nothing: 4.95 s and no translation, against 1.15 s and a correct one with thinking off). If you still see reasoning text, or the first-word
times are far higher than llama-server's, say so, with the model name.
**Part 2: the whole thing on the cutscene (as in M-3).** Run it twice and keep every other setting equal:
* **A, everything local** as it is now (model, reading model and speech server on your GPU).
* **C, model on the other PC** with `[ocr] vision_reader = false` and `[tts] enabled = false` (so nothing else of ours is on the GPU; the reading model is a separate model
  that every live read uses once it has loaded).
Optionally **B**: local model with the same two switched off, which separates "our other servers" from "the model itself".
In another terminal during each cutscene run: `nvidia-smi --query-gpu=timestamp,memory.used,memory.total,utilization.gpu --format=csv -l 1 > gpu-A.csv` (stop with Ctrl+C afterwards).
**Expect:** local model idle ≈ the first-word times of the typical lines; with the game running slower and more variable; the second PC's numbers close to idle plus a
few milliseconds of network, as long as its GPU is no slower.
**Bring, for each run/condition:** the `maru bench` output; for Part 2 the History → Live subtitles text and the `live line:` log lines, and the `gpu-*.csv`; the GPU of each PC.
**Why unverified:** needs your two PCs; the benchmark and the external-endpoint path are only tested against fake servers.

## M-4 Screen scan and manual join
**Steps:** `Ctrl+Alt+S` on a game menu, an inventory grid and a HUD. Click a label (its breakdown opens), hover for the ×, right-click to remove one.
Then find a phrase that got two labels: **Shift+click** both, press **Join**; scan again (`Ctrl+Alt+S`) and check it comes back joined without
you; right-click the joined label and choose **Separate joined label**, or select it and press **Separate**.
**Expect:** labels do not cover text or each other; dictionary meanings appear first and model labels fill in; a second scan of the same
screen is instant; the joined label is read in order with sensible spacing, translated as a whole, remembered for that game only, and Separate
undoes and forgets it; the overlay never blocks the game except on a label or the Join chip.
**Bring:** screenshots of anything wrong (before joining for join problems); the game's `.exe` name; which join came back and which did not.
**Why unverified:** needs a real game, Korean OCR and the real window system (click-through hit-testing).

## M-5 Selection and capture across programs
**Steps:** select Korean text in a browser, a PDF reader, a chat app and a game; press `Ctrl+Alt+T`; also drag a region with `Ctrl+Alt+R`
over text that cannot be selected.
**Expect:** exact text when the program supports UI Automation; the Ctrl+C fallback otherwise (your clipboard is restored); OCR for regions.
**Bring:** which programs worked and which did not; whether the text came out exact (selection/copy) or looked misread (OCR); screenshots of misreads.
**Why unverified:** UI Automation, the clipboard route, GDI capture and Windows OCR only type-check on Linux.

## M-6 Overlay behaviour on Windows
**Steps:** use the panel with a game in front: does it steal focus? Is its background transparent? Drag it by the ⠿ grip, pin it. Is the
panel/labels/band absent from a screenshot you take (they are excluded from capture)? Does closing the hub keep Maru in the tray?
**Expect:** the game keeps focus and keyboard input; nothing opaque around the panel; screenshots show no overlay.
**Bring:** what differed, the Windows version, and whether the game is windowed, borderless or exclusive fullscreen.

## M-7 Korean OCR accuracy, model quality and speed, Kiwi
**Steps:** `maru doctor --deep`; `maru bench`; `maru eval-ocr <folder>` on a few screenshots (`name.png` + `name.txt` with the true text);
translate a few sentences you know well.
**Expect:** Kiwi loads (accurate breakdowns); OCR character error rate you can live with (the vision reader, `maru setup ocr-vl`, helps);
translations you judge good; latency in `maru bench` that feels fast.
**Bring:** the `eval-ocr` table, the `bench` percentiles, and translations you think are wrong with the context.

## M-8 Speech
**Needs:** `maru setup tts` done (or a speech server on another PC and `[tts] provider = "endpoint"`).
**Steps:** Settings → Speech → **Test voice**; in the panel press 🔊 on a paragraph (sentence highlight, previous/pause/next/repeat),
🔊 on a single word in a deep dive, and `Ctrl+Alt+P` (translate and read; press again to stop). Try a long paragraph.
**Expect:** natural Korean, no gap between sentences, correct highlighting, the voice stops when the panel is dismissed, a second play of
the same sentence is instant (cache). In lite mode the local voice stays off unless `run_in_lite = true`.
**Bring:** a note of how it sounds on tricky cases (`maru tts-eval` writes a page that plays the same sentences in each voice); GPU/VRAM
use while a game runs; any error text.
**Why unverified:** the `tts-server` Windows build and the Qwen3-TTS models have not been run on a real machine from here.

## M-9 Anki
**Needs:** Anki open with the AnkiConnect add-on (code `2055492159`); Settings → Anki → `enabled`; for audio, M-8 working.
| Step | Expect |
|---|---|
| Settings → Anki → **Check Anki** | connection, decks found, the note type exists (created if missing) |
| Deep dive → **Add to Anki**, pick or type a deck | a note appears with Korean, meaning, example, and tags `maru`, `topik-N`, `pos::…`, `app::<program>` |
| Look at the note's cards | an English→Korean card in `<deck>::Reverse` (unless `reverse = "off"`) |
| Open the card | spoken word and sentence play |
| Add the same word again | no duplicate, no silent failure |
| **Sync now**, or add with `sync_after_add` on | Anki starts its own AnkiWeb sync |
| Review the card several times, wait `refresh_minutes`, look the word up again | its gloss fades (intervals are read back from Anki) |
| Close Anki, press **Check Anki**, then try **Add to Anki** | a clear message, no hang or crash |
**Bring:** for each row "worked" or the exact message; a screenshot of the note in Anki's Browse view (fields, tags, deck); Anki and
AnkiConnect versions; the log lines around a failure.
**Run without a person (2026-11-25, see the results log):** with Anki open on a test profile,
`MARU_REAL_ANKI=1 MARU_REAL_ANKI_PROFILE="<profile>" cargo test -p maru-engine --test real_anki`
does rows 1, 2, 3 and 5 through the engine in a throwaway deck (Check Anki, a word with its fields, tags and both sounds,
the reverse card in `<deck>::Reverse`, no duplicate) and then deletes everything; row 8 is checked with nothing listening
where Anki should be. Every change goes through a guard that refuses it unless the named profile is open and the change
stays in the throwaway deck, and that never lets a sync through.
**Still for a person:** hearing the sounds on the card (row 4), **Sync now** and AnkiWeb (row 6: the automated run never
syncs), reviewing a card until its gloss fades (row 7), really closing Anki (row 8), and the Add to Anki popover itself.

## M-10 Settings page and lite mode
**Steps:** change a shortcut (hotkeys apply at once), a panel option, and a model setting (needs a restart: use the page's restart button);
set `lite = true` (or run with `MARU_LITE=1`) and check the footprint (smaller model, no OCR reading model) beside a game.
**Expect:** invalid values are refused with a reason; comments in `config.toml` survive saves; the restart notice appears only when needed.
**Bring:** anything that was saved wrongly (the before/after of `config.toml`).

## M-11 Live speed experiments with the same translation model
**Needs:** existing PP-OCR and translation model; use the build containing `continuous.cpu_ocr` and `continuous.max_in_flight`.
**Steps:** replay the same cutscene after a fresh Maru start for each condition: CPU option off / active translations `0` (baseline),
CPU option off / `1` (original OCR, ordered requests), then optionally CPU option on / `0`, `/1`, `/2`. Save and restart after each change. Keep the model, context,
subtitle area, game resolution and other settings fixed. Repeat the baseline last to catch changing game load or server warm-up.
**Expect:** translations arrive quickly enough to read, every captured line finishes and reaches history, and Live subtitles lists lines
in appearance order even if answers finish in another order. With `1`, later requests have preceding completed answers as context.
Check names and negation against the original Korean: CPU recognition is an experiment, not a promise of equal quality on every font.
Open a manual region translation and a scan afterwards to confirm they still use the original screen reader.
**Bring:** the log from each fresh run, History → Live subtitles and any missed/misread Korean; game name/resolution and GPU memory use.
New `live request` fields show waiting/active requests and admission/model/first-text timing. Model-only `maru bench --live` excludes
screen capture, OCR and the app's other model resources, so its figures are not full live latency.
**Why unverified:** fake-server tests establish scheduling and OCR isolation; the optional real-model recording replay can establish
timings on that video, but neither exercises physical-screen capture, the overlay or game rendering during a real cutscene.

## M-48 Save what you read (raw material for a test set)
**Needs:** live subtitles working (M-3) and a cutscene with Korean subtitles.
**Steps:** Settings → Live subtitles → Advanced → **Save the subtitles it reads**, restart live mode, watch a minute or two of a cutscene, then open
`<data folder>\evalset\raw\<program name>\`.
**Expect:** one `.png` and one `.json` per subtitle line the band showed (no repeats of the same line); the picture is the subtitle strip; the `.json`
`draft` is what the band translated from; with the setting off, nothing new appears; playback does not stutter more than without it.
**Bring:** the folder's file count against the number of lines you saw, one sample picture with its `.json`, and any frame drops you noticed.
**Why unverified:** tested against a fake screen and fake reader only (pictures and drafts match the reported lines); not run on the real PC or with a real game.

## M-49 Labelling saved pictures (the review page)
**Needs:** a folder of pictures from M-48 on the real PC, Edge or Chrome, and (optional) PaddleOCR-VL installed (`maru setup ocr-vl`).
**Steps:** `maru eval-ocr <data>\evalset\raw --engine paddle-vl --write-drafts`; open `tools\label_review.html`, press **Open the folder…**, pick `evalset\raw`
(allow editing when the browser asks); label ten pictures with Enter, mark one with Alt+U, close the tab, reopen the folder.
**Expect:** pictures where the readers differ come first; after reopening, the ones you did are counted as done and `.txt`, `.unreadable` files sit beside the pictures;
the Korean typing box takes IME input without Enter confirming half a syllable; `maru eval-ocr <data>\evalset\raw --engine paddle` scores the labelled ones.
**Bring:** the count line at the top of the page after ten pictures, and anything that was awkward to type or read.
**Why unverified:** checked in Chromium on Linux against a stand-in for the folder picker (reading, writing and removing real files) and its `?demo` mode; the real folder picker, Windows paths and IME typing were not run.

## M-50 Does enlarging the subtitle picture help PP-OCR on your game?
**Needs:** labelled pictures of your game's subtitles (M-48 and M-49, or `tools/sample_subtitle_frames.py` on a recording), and the PP-OCR models.
**Steps:** `maru eval-ocr <set> --engine paddle --save-report base.json`, then `maru eval-ocr <set> --engine paddle --scale 2 --compare base.json`
(also `--scale 1.5`). Use `--split dev` to try things and `--split held-out` to judge.
**Expect:** the comparison says "better on" clearly more subtitles than "worse on", with a sign-test p below 0.05, before you set `[continuous] ocr_scale = 2`.
**Bring:** both summaries, the comparison, and what the game and its subtitle font look like (outlined, shadowed, plain).
**Why unverified:** Claude measured it once, on one recorded cutscene (13 distinct subtitles, 49 pictures, Linux CPU, ONNX Runtime 1.31): letter error
1.4% at scale 1 and 1.8% at scale 2 (better on 2 subtitles, worse on 4, the same on 7; p = 0.69). Earlier numbers that looked like a gain (4.5% down to
3.9%) counted missing commas and question marks as reading errors and moved with the resize method; they are withdrawn. Not run on Windows, a real game or the live path.

---

## M-12 Live OCR duplicates and subtitle area
**Needs:** a build with `continuous.filter_ocr_jitter`, Korean subtitles, CPU PP-OCR off and active live translations `1`.
**Steps:** play the same 2–3 minute cutscene with **Check subtitle lettering for OCR duplicates** off, then on (save/restart).
Keep the model, context, resolution and selected area fixed. Include two-line dialogue, a fade, a repeated sentence after a gap,
short answers, changed numbers and negation. Compare History → Live subtitles with the actual Korean frames: `SP`/`IOY` variants of
one visible line should not create extra requests, and genuine dialogue changes must still appear. Return the filter to off and select
a tighter subtitle box with `Ctrl+Alt+Shift+C`, leaving a margin for both rows and excluding nearby controls. Compare off/on again
with that area fixed. Repeat in another game.
**Expect:** fewer extra translations where lettering confirms OCR jitter, no lost dialogue, and every genuine repeat after a visible
gap still captured. Low-contrast fonts may yield extra readings; the visual check is conservative and does not promise a speed gain.
**Bring:** `maru-app.log` from each run (copy before restarting), History → Live subtitles, game name/resolution, the chosen box,
and timestamps/screenshots or a short recording of any missed line, duplicate, or bad translation. Report whether delay grows over time.
**Why unverified:** synthetic frames establish the guards and a recording replay can measure that clip, but neither establishes
physical-screen capture, live overlay behavior, or recognition quality across other games and fonts.
**Recording observation:** the supplied 75-second clip was replayed six times with installed hybrid OCR and local translation.
All runs completed without model errors. The tighter area excluded controls and produced fewer noisy readings; the filter did not
consistently reduce duplicates or improve latency against a repeated warm baseline. Keep it off unless an A/B test on your game helps.
Separate stand-in-OCR replays captured all 17 labelled subtitles before disappearance with the filter off and on. They test watcher
timing, not OCR correctness or translation quality. These are automated checks, not entries in the human results log below.

---

## M-16 Prepared subtitle evidence and quality review

**Needs:** the local `data/live-diagnostics/benchmark-v2.2` pack, its original videos, a bilingual reviewer, installed OCR
and approved local candidate models. Follow `docs/LIVE_BENCHMARK.md` for suite and timing limits.
**Steps:** independently compare Korean transcriptions with evidence, approve or correct meaning aids in a new pack version,
and mark uncertain fragments rather than forcing a localization match. Run actual OCR on evidence and preserve its raw text;
keep recognition errors separate from translation errors. Develop prompts on development scenes; use Sekiro validation without
tuning on its answers. Then compare identical playing cutscenes with/without inference, recording frame times and first-text/done
latency. Only earlier same-scene lines may be supplied as context. Check native prompts/decoding before ranking new models.
**Expect:** reproducible hashes, no unverified drafts in runnable suites, no duplicate weighting of restored-context cases,
and meaningful translations arriving before disappearance without unacceptable game frame-time cost. Timing brackets stay ranges.
**Bring:** the report folder, pack/model hashes, bilingual review notes with case IDs, raw OCR outputs, runtime/thread/offload
settings, game/resolution and matched game frame-time recordings. Preserve prompt/context/decoding details.
**Why unverified:** agent visual inspection and Python integrity tests verify preparation only. No independent bilingual review,
actual OCR run on new evidence, exhaustive video annotation or new-candidate real-game performance trial has been observed.
The selected timing manifest cannot establish full-scene OCR precision/recall; the existing replay test uses stand-in OCR.

## M-15 Live translation model comparison
**Additional native runner:** `tools/bench_live_candidates.py` supports Tower+, official TranslateGemma, Seed-X-PPO,
EXAONE, Konan and Kanana on the installed local runtime. Follow the README with independently approved weights and the
frozen corpus. Preserve its `records.jsonl`, `report.json`, owned server logs and `gpu.csv`; TranslateGemma additionally
requires a correct `native-template-check.json`. CPU trials use `--gpu-layers 0 --threads 4` and disable all offload.
Scripted stream/prompt checks do not verify converted-model equivalence, translation quality or gameplay frame times.
**Needs:** existing Gemma GGUF, an independently approved/downloaded HY-MT1.5-1.8B Q8 GGUF, installed llama-server and the
diagnostic CLI build. Quit Maru from its tray to unload its models. Leave the game open for the game-load condition.
**Steps:** run `bench-live-models` as documented in the README (or `tools/compare_live_models.ps1` for GPU telemetry), first with
the game open, then closed using a new output path. Keep server, context and corpus fixed. Compare both Gemma passes to detect
drift. Read `review.md` with the Korean; mark changed meaning, numbers, negation, names, pronouns or extra explanations. Re-run with
`--rolling-context` to check whether a bad prior answer propagates; include a custom JSONL corpus from a second game.
**Expect:** three passes, zero errors, first-text and done percentiles for successful translations, and every output available for
review. Faster completion alone is not a quality pass. This command does not select the candidate for the overlay.
**Bring:** report folder, sibling GPU/full log files if using the wrapper, game name/resolution, whether a cutscene was playing or
the game was idle, and case IDs with incorrect translations and the intended meaning.
**Why unverified:** scripted tests establish benchmark behavior. Real model-only runs exclude capture, OCR, other app models and
the overlay; neither proves end-to-end subtitle latency or quality in another game. An overlay trial needs a separate live-only
model selection change and a real cutscene after the model comparison has been reviewed.
**LFM/Index follow-up:** with independently approved local models, use `--candidate-prompt lfm --candidate-sampling lfm`
or `--candidate-prompt index --candidate-sampling greedy`. Run zero context first, then four fixed reference turns into another
new folder. The latter is an experimental chat-history format for these two candidates. Preserve the GGUF quantization,
dataset hash, per-game results and exact outputs; inspect meaning rather than matching the game's English localization.
Model-only runs do not establish OCR quality, watcher timing, game FPS or overlay latency. A new model remains unverified
until the installed server actually loads it and its outputs have been inspected; another game still needs its own trial.
**Translate Gemma Sub / LMT follow-up:** use independently approved local GGUFs with `--candidate-prompt translate-gemma-sub`
for E4B/E2B or `--candidate-prompt lmt`, both with `--candidate-sampling greedy`. Compare zero context, four reference lines,
and four rolling own-model lines against fresh Gemma before/after controls. Inspect the same game meanings, including
capital relocation, climbability, speaker identity, blessing negation and names; retain headings, cap failures and raw output.
Sub uses its published subtitle context blocks. LMT preceding chat pairs are experimental, and greedy streaming does not
reproduce the author's five-beam example. Report per-game results and exact GGUF hashes. These remain model-only tests;
real game/OCR/overlay trials and the proposed live-only settings selector remain unverified.

**MiLMMT dialogue-context experiment:** use `--candidate-prompt milmmt-context --candidate-sampling greedy` with
`--context-lines 1` and `4`, then `--context-lines 4 --rolling-context`. Compare fresh Gemma/MiLMMT/Gemma passes on
the same corpus with the published zero-context control. Fixed reference history is optimistic; rolling uses each model's
previous answers. Inspect ambiguous subjects, capital relocation, fictional names and meanings, and history echo or output
cap errors. Completed examples are experimental, not a published Xiaomi dialogue template. This remains a model-only
diagnostic; it cannot certify game FPS, OCR or overlay latency.

**MiLMMT/QuickMT follow-up:** use independently approved model files and isolated native Python dependencies. MiLMMT uses
`--candidate-prompt milmmt --candidate-sampling greedy --context-lines 0`; QuickMT uses `tools/compare_quickmt_cpu.ps1`
with CPU int8 beams 1/2/5 and a float32 beam-5 control. Retain batch size 1, fixed CPU threads and a new folder per condition.
Inspect the same meaning cases and compare Gemma before/after controls. QuickMT is a direct CPU call; its timing excludes HTTP,
and beam search yields completion only. A fast CPU result does not establish game FPS, OCR-to-overlay latency, contextual
quality or behavior with noisy OCR. No overlay selection/default changes are part of these diagnostics.
**MT2 follow-up:** use an independently approved/downloaded local MT2 Q8 file with `--candidate-prompt hy-mt2`. Compare
separate matched and `--candidate-sampling tencent` runs on the same corpus/context; the latter changes only the HY-MT
candidate's core sampling parameters and retains the live output cap. Review the MT1.5 failure cases explicitly. The installed
llama-server must load the new GGUF successfully before making compatibility claims. Neither profile is an overlay selection.
**Automated observation (2026-10-11):** with Maru closed and the game left open on the RTX 5070 Ti, two local A/B/A comparisons
completed 480 requests without execution errors. The three-repetition native-prompt run had median first text/done of 22/81 ms
for HY-MT Q8 and 167–170/340–341 ms for Gemma. Total GPU memory, including the game, was about 5 GiB lower with HY-MT.
Text inspection found changed meanings, including World Tree → World Water and a reversed climbing meaning; a one-repetition
Maru-prompt follow-up also changed dismissal into appointment. Keep the installed model for now. These are model-only automated
observations with fixed reference context, not a human overlay quality result; game activity was unspecified and rolling context
was only unit-tested. The reports distinguish startup/warm-up from steady requests. This does not add a human results-log entry.
**Automated MT2 follow-up (2026-10-11):** with Maru and the game closed on the same RTX 5070 Ti, three three-repetition A/B/A
comparisons tested MT2 Q8 with matched and Tencent core sampling, then MT1.5 Q8 with Tencent sampling. A one-repetition
MT2 zero-context probe followed. All 1,200 measured requests completed without execution errors; installed llama.cpp loaded
MT2, the official model checksum matched, and the installed config hash was unchanged. Median MT2 first text/done was
18/63 ms matched and 20/68 ms Tencent, versus Gemma's 139–149/268–285 ms with dialogue context. Total GPU memory was
about 4.6–5.0 GiB lower in MT2 passes. MT2 retained changed meanings (dismissal → promotion; World Tree → waterfall/mountains)
and echoed prompt/background text in 38/120 matched and 34/120 Tencent answers. Removing context stopped those echoes but
did not fix the meaning errors. MT2 improved the climbing sentence over MT1.5; neither candidate passed text review across
the corpus. Keep Gemma selected. Full local artifacts: `data/live-diagnostics/model-comparison/MT2-REPORT.md`. These are
agent text inspections and model-only timings, not a certified quality score, game-open MT2 test or human overlay result.
**Automated expanded follow-up (2026-10-12):** 30 visually checked Elden Ring lines with separate agent-created meaning
references were added to the original 40-line corpus. Eight three-repetition A/B/A comparisons tested MT1.5 Q8, MT2 Q8,
LFM2 KoEn v8 Q4 and official Index Translate 2B Q8, each with zero and four reference-context lines. A one-repetition Index
follow-up used Maru's existing live prompt. All 5,250 requests completed without execution errors on the RTX 5070 Ti with
Maru and known game processes closed. With four prior lines, median completion over all 70 lines was 60/63/27/87 ms for
MT1.5/MT2/LFM/Index versus 233–238 ms for Gemma controls. Faster candidates retained substantial meaning errors; Index's
Maru-prompt follow-up improved some phrases but still changed dismissal into injury. Keep Gemma selected. Config and source
dataset hashes were unchanged and test processes exited. Full reports: `data/live-diagnostics/elden-ring-comparison/RESULTS.md`.
These are model-only timings and agent output inspections; no certified accuracy score, game FPS, OCR or overlay claim.
Independent frames also disproved revised subtitle onsets for Elden Ring entries 5, 7 and 8; refine timing annotations before
using the dataset to score OCR/watcher delays or misses. Translation-only inputs did not use those timestamps.
**Automated MiLMMT/QuickMT follow-up (2026-10-12):** the same 70 lines were tested with zero context and three repetitions.
Gemma/MiLMMT 4B v1.0 Q8/Gemma completed 630 requests; QuickMT CPU int8 beams 1/2/5 and float32 beam 5 completed 840.
All 1,470 measured requests had zero execution errors, with Maru/game closed on the RTX 5070 Ti and Ryzen 9 7900X3D.
Median completion was 104 ms for MiLMMT versus 200–203 ms for Gemma; QuickMT used four CPU threads and completed in
29/46/49 ms for int8 beams 1/2/5 and 78 ms for float32 beam 5. MiLMMT fixed specific earlier candidate failures but still
changed capital relocation and game terms. QuickMT retained serious semantic errors across beams/compute types, including
reversed blessing negation. Keep Gemma selected. Full raw outputs, provenance and limitations:
`data/live-diagnostics/quickmt-milmmt/RESULTS.md`. These are agent text reviews and model-only timings, not certified accuracy,
contextual MiLMMT quality, game FPS, OCR or human overlay observations. Installed config, corpus and server hashes unchanged.

**Automated MiLMMT dialogue-context follow-up (2026-10-12):** four fresh Gemma/MiLMMT Q8/Gemma comparisons completed
2,520 measured requests on the same 70 lines, three repetitions: no context, one reference turn, four reference turns and
four rolling own-model turns. No execution or output-cap errors. MiLMMT median completion was 100/102/112/111 ms;
paired Gemma controls were 192-197/206-207/227-233/230-231 ms. All ten authored meaning checks retained their core
meaning in MiLMMT outputs on agent inspection, but context introduced climbability and speaker-identity errors; rolling
history propagated wrong names and identities. Four references helped direction and fictional terms, but are an optimistic
control rather than real live history. Keep Gemma selected. This completed-example prompt is experimental, not Xiaomi's
published dialogue template. Full outputs, per-game timings, preserved context and limitations:
`data/live-diagnostics/milmmt-context/RESULTS.md`. Config, original corpus, server and both models retained their hashes.
These are model-only timings with game/Maru closed, not certified translation accuracy, game FPS, OCR or human overlay results.

**2026-10-12 automated Translate Gemma Sub / LMT comparison (not an M-15 human pass):** approved E4B/E2B Q4_K_XL
and LMT-60-4B Q8_0 models completed nine Gemma/candidate/Gemma comparisons on the same frozen 70 lines, with no history,
four reference turns and four own-model turns. All 5,670 requests completed with zero reported execution errors; all warm-ups
succeeded. Candidate median completion was 114-116 ms (E4B), 75-78 ms (E2B), and 72-87 ms (LMT), against paired Gemma
medians of 180-229 ms. An E4B no-context request took 2,678 ms and is retained. E4B/E2B change climbability and fictional
entities; LMT reverses the poisoned cup and sometimes blessing negation without history. Its experimental chat context
often returns earlier answers and rolling history repeats scene-opening answers. Keep Gemma selected; none of these tested
configurations established comparable translation quality across the samples. LMT greedy streaming is not its published
five-beam decoder. Corpus/model/runtime/configuration and diagnostic source/binary hashes were verified; exact answers,
per-game timings, contexts, decoding limits and telemetry are in `data/live-diagnostics/translate-gemma-lmt/RESULTS.md`.
Workspace tests (424), app tests (21), UI tests (81), Python diagnostic tests (4), and strict LLM/app Clippy passed. Strict
workspace Clippy retains three existing warnings in untouched Windows code; it passes with only those warning categories
allowed. These tests ran with Maru/game closed and do not verify OCR, game FPS, overlay timing or the future live-only selector.

**2026-10-12 expanded automated comparison (not an M-15/M-16 human pass):** the approved ten missing weights were
downloaded and hash-verified. Fifteen candidate variants plus Gemma loaded on the RTX 5070 Ti / Ryzen 9 7900X3D.
Twenty-five paired conditions completed 27,000 measured requests on 120 distinct texts, with zero execution errors;
75 warm-ups were excluded. Native MT2 7B Q4 and MiLMMT 4B Q6 median completion was 83/82 ms, with p95 142/131 ms.
Their total device memory medians were 8.15/6.42 GiB. Both retained meaning errors; own-answer history also introduced
priority/climbability failures. Four-thread CPU rolling completion medians were 932 ms (MiLMMT Q6) and 854 ms (Kanana),
with p95 1,749/2,880 ms. Kanana with the existing Maru live prompt remained semantically unreliable on several scenes.
These are local model-only results with no playing-game FPS, OCR, overlay, NAS or independent bilingual-quality pass.
Keep the installed selection unchanged pending a real-game comparison. Tables, exact answers and limits are in
`data/live-diagnostics/benchmark-expanded-2026-10-12/RESULTS.md` and `review-by-case.md`.
The final audit verified configuration, original runtime/baseline/CLI, all candidate weights, frozen pack outputs and
both source videos; owned model ports were closed. Python (26), app (21), UI without isolation (81), dynamic-ONNX
workspace tests (424 passed, two ignored) and app Clippy passed. Standard workspace Clippy/static linking and default
Node isolation retained the separately recorded existing lint/link/sandbox blockers. No production Rust/UI change,
new application dependency or installed setting change was made in this expanded round.

## M-17 Live model settings, snapshot routing and floating controls

**Status:** unverified in a real game and NAS; Chromium screenshot verification is also pending because this session
has no connected browser and the Windows Computer Use helper reports its native pipe is unavailable. Fake-server tests
verify routing, native-only requests, failure fallback, toggling off, context opt-in and cache separation. Owned-process
lease tests verify waiting/cancellation/failure. Do not treat those checks as a visual or gameplay pass.

**Steps:** build the overlay, quit the old Maru from its tray, then launch the new executable. Open Settings -> Live subtitles,
choose MiLMMT 4B Q6, verify the discovered file (or paste its path), Save and Restart. Leave dialogue context off and use
GPU first. Repeat the same cutscene with HY-MT2 7B Q4, and optionally MiLMMT CPU. Record GPU memory/utilization, game FPS,
translation correctness, omissions, first-text/completion times from History, and `maru-app.log`.

Use the floating 마 button to toggle live, choose a tight subtitle area, take a snapshot and open History/Settings.
Check that transparent space passes game clicks through, clicking a control does not clear a selection, and controls never
appear in OCR. Drag, relaunch, check the saved position; hide and restore with Ctrl+Alt+M / tray, including a changed shortcut.
Check an offset/high-DPI second monitor, narrower settings window, long paths, keyboard focus, search, staged card edits,
Save/Discard and attempts to leave/restart with unsaved changes. Ensure the launcher does not intercept area selection.

With live on and no snapshot endpoint, take a Korean snapshot: translation + offline breakdown, no new local Gemma process.
Click a word: dictionary/formation remains, richer explanation gives a clear note. Turn live off: next snapshot restores Gemma,
waiting for any active stream. Test an English snapshot for the clear unsupported-direction note during local specialist mode.
Configure the optional NAS address and exact Gemma model name only when willing to send snapshot/context text there; repeat
snapshot/deep-dive. Stop/unplug the NAS and retry: local translation/offline meanings, no local Gemma reload. Restore NAS and
retry the same text: richer analysis is requested again. Expect load delay when switching local models; pending subtitles remain
in history. OCR reader / speech models are separate processes and do not constitute duplicate translation servers.

**Automated ownership smoke (2026-10-12):** both approved HY-MT2 7B Q4 and MiLMMT 4B Q6 streamed a Korean translation on the RTX 5070 Ti, switched to the original local Gemma 12B and streamed a second translation, then released the owned server/port. These are loading/routing checks, not an end-to-end latency, NAS, game FPS or quality benchmark.

**Automated regression checks:** dynamic-ONNX workspace tests passed (436, three ignored), app tests passed (22),
UI tests passed with Node isolation disabled (85), and Python diagnostics passed (26). Strict app and core/LLM Clippy
passed; workspace Clippy passed allowing the two warning categories from three pre-existing Windows warnings.
The standard workspace static ONNX link and default Node subprocess isolation remain blocked by the previously recorded
link/sandbox errors. The release executable was built with embedded UI assets. Installed configuration was unchanged.

**Give back:** chosen model/device, game, two or three bad/missed examples, History timing entries, GPU/FPS observations,
log path/file, and screenshots of any clipped controls or confusing status. This entry is not a results-log claim.

## M-18 Resource monitor and VRAM menu bar

**Status:** automated Windows counter smoke succeeded on the RTX 5070 Ti: adapter capacity/usage and current-process
CPU/RAM readings were available; the observed second sample took about 1.9 ms to collect. This single observation is not
a monitoring-overhead benchmark, model-attribution/gameplay pass or visual pass. Chromium/screenshots remain unavailable
because the browser inventory is empty and the Windows Computer Use native pipe is missing.

Automated regression checks: workspace tests with dynamic ONNX passed (441, four ignored), app tests passed (22), UI
logic tests passed with Node isolation disabled (89), and strict app Clippy passed. Workspace Clippy passes allowing only
the two warning categories from the three existing Windows warnings; standard workspace static linking still fails on
`DMLCreateDevice1`, and default Node subprocess isolation still fails with `spawn EPERM`. The new resource-session tests
also verify cached reads do not keep hidden windows sampling. Main and installed configuration remain unchanged.

**Steps:** launch the updated build; open floating controls and Resources. Compare adapter VRAM with Task Manager's
Performance page, and owned PID CPU/RAM/GPU readings with Details (values need not match between different sample times).
Repeat with MiLMMT, HY-MT2 and a snapshot after live off: check the selected live/main rows follow the actual owned server,
and the inactive model has no fabricated zero readings. Loading has no owned PID until the supervisor returns. Check the
separate OCR reader/speech processes if enabled and that in-process OCR is explicitly included in Maru. External/NAS
models must say local readings unavailable. Do not add process GPU percentages or overlapping memory allocations.

Check the quick VRAM bar names the most occupied measured adapter and includes game usage. High starts at 80%, critical
at 95%; no need to deliberately fill VRAM. Browser demos `launcher.html?demo=critical`, `hub.html?demo=critical#resources`
and `?demo=missing` exercise visual states without allocating GPU memory. Verify readable warning text, keyboard focus,
scrolling in a small/high-DPI menu, progress-bar accessibility, click-through space and capture exclusion.

Record 60-120 seconds of the same cutscene; close the menu and confirm recording continued, then reopen Resources and
Stop. Keep the shown JSONL path and `maru-app.log`. Pause must stop sampling and recording; Resume must start readings
again when a view is visible. Check normal Quit flushes an active log. Recording is off after restart. Repeat the same
scene paused, visible without recording, and recording; compare GPU/FPS and History timing tails. Logs contain numeric
timings and IDs but no dialogue/error text. Size caps stop recording with a clear message and retain old logs.

**Give back:** model/device, game, monitoring mode, GPU/FPS observations, History timings, resource JSONL, `maru-app.log`,
and screenshots of any missing or clipped readings. Automated tests are not entries in the human results log below.

## M-19 Startup without GPU models and Windows window opening

1. Quit Maru, then start the rebuilt app with the game open. Before toggling any feature, open Settings from
   the floating menu and tray. Browse categories, close the hub, and reopen it. Settings must remain responsive.
2. Inspect Resources and Task Manager: no owned translation, vision-reader, or speech server should be running
   solely because Maru or Settings opened. The VRAM bar includes every application, so record the baseline first.
3. Select MiLMMT or HY-MT2, save, restart, and toggle live subtitles. Only the selected translator should load for
   translation. Enable **Use CPU PP-OCR for live subtitles** to bypass the vision model; otherwise normal live OCR can
   load it. Record cold-start time separately from later subtitle timings.
4. Try a snapshot. Its configured OCR reader loads on first use; snapshot routing still follows M-17. Turn live off
   and try another snapshot: Gemma loads for normal translation when needed. Prompts and model quality settings stay the same.
5. Request speech and try the Settings voice test. The local voice loads on demand and subsequent calls reuse it.
   Opening Settings while a model loads must not freeze the app. Quit and check that owned server processes exit.
6. Turn live off while its initial model load is pending: the translation status must return to unloaded/on-demand
   instead of remaining stuck at Loading. Retest in a borderless game using real clicks and hotkeys.

**Automated verification:** interactive bootstrap and deferred-loader tests cover idle startup, independent requests,
concurrent/cancelled callers, failure and OCR readiness; runtime cancellation resets status. A local Windows check using
the app's embedded Chromium debugging interface opened Settings from the floating menu and reopened its tab, rendered
Resources, and confirmed no model server at idle startup. RTX 5070 Ti total VRAM was about 19% (2.92 GiB) during that
idle check; this is a reading, not a controlled comparison. A real local speech request loaded only the speech server
and returned cached WAV audio in about 12.2 seconds including cold startup. The native computer-use helper was unavailable.
Settings styles are bundled rather than dynamically inserted so the app CSP permits them.

**Still unverified:** physical tray/hotkey input, borderless-game behavior and FPS impact, cold snapshot OCR with the
real vision weights, and user-observed cold/warm cutscene timings. No human result has been added to the log below.

## M-20 External model connection checks in Settings

1. In Translation model, enter an external address and its exact model name without saving; press **Test connection**.
   Confirm that the model replies and the elapsed time appears. Repeat in Live subtitles for its snapshot quality server.
2. Try an unavailable port, a nonexistent model name and a server that returns no streamed text. Each must show an error,
   rather than a successful health check. Blank address/model fields disable the button.
3. Edit the address or model after a successful check: the old result must disappear. Edit while a check runs;
   its eventual result must not confirm different fields. Repeated clicks while waiting must not duplicate the request.
4. Discard the draft and reopen Settings. The connection test must not save fields, start local model processes,
   or change active translation routing. A cold external model may take up to 60 seconds; retry after it loads.

**Automated verification (2026-10-12):** scripted-server tests cover request fields, a missing model, an empty answer,
invalid input and a bounded slow stream; pure UI tests cover draft selection and result matching. The rebuilt app's
embedded Chromium interface verified both buttons against a loopback scripted server (Hello, about 1.0 s), an
unavailable port, a missing model, blank-field disabling, clearing previous results and suppressing a result after
editing during its request. Screenshots were inspected at normal width and a 720 px viewport. The installed config
hash stayed unchanged and no local model server started. This is automated app verification, not a human game result.

The real NAS at `192.168.18.217:11434` listed the expected Gemma 12B model, but its fixed greeting request did not
answer within 60 seconds; Settings remained responsive and showed the timeout. Real NAS inference success remains
unverified. Physical clicks and translation quality/game latency require user testing. No human result was added below.

**Build checks:** 452 workspace tests passed (four optional tests ignored), 22 app tests and 93 UI tests passed.
Windows workspace tests used the existing `maru-ocr/ort-dynamic` feature because static ONNX linking is blocked on this
machine. App strict Clippy passed. Workspace strict Clippy still reports three existing warnings in untouched Windows
capture/UIA code; the check passed with only those two lint categories allowed. Release app build passed offline.

## M-21 Server discovery and model selection

1. In Translation model, enter the NAS IP without a scheme or port and press **Detect server**. The completed search
   should find its model API, stage the complete endpoint and populate a dropdown. Choose the exact translation model,
   then **Test connection**. Metadata discovery and inference results must be distinguished. Repeat in Live subtitles
   for its snapshot quality server. Opening Settings and merely typing must not start a network search or local model.
2. Repeat with a hostname, explicit custom port, `/v1` or a reverse-proxy prefix. Explicit destinations take priority.
   Try a NAS dashboard URL: HTML must not count as a model API. Multiple servers must require an explicit choice.
3. **Find LAN servers** must only list interfaces. Select the shown private network, then **Search this network**.
   Verify progress, cancellation and selecting partial results. A /16 must clearly label the searched /24 portion.
   Searches must never enumerate all 65,536 ports or invoke generation. A single search runs at a time.
4. Edit an address during host discovery, cancel, leave Settings, or quit the app: pending work must stop and stale
   results must not stage fields. Select a model, then Discard; the saved config and active model routing must stay unchanged.

**Automated verification:** metadata/normalization tests cover common ports, explicit default/custom ports and API
routes, model-list deduplication, Ollama fallback, HTML and authentication failures. Engine tests cover range limits,
/31 and /32, larger-network labels, progress, overlapping searches, cancellation and drop cleanup. UI tests cover
draft paths, matching, detection decisions and progress text. Real app/NAS observations are added after checking the build;
whole-LAN discovery, authenticated servers and reverse-proxy deployments require user verification.

**App verification (2026-10-12):** the rebuilt Windows app detected a loopback fixture from a bare address, staged
its port, offered two models and requested no inference until a model was explicitly tested. Its fixed test replied
in about 0.5 s. A scripted proxy prefix was preserved; two servers on different common ports required selection.
Host cancellation and editing during a search did not apply stale results. Real Windows inventory listed Ethernet,
the VirtualBox network and WSL; the /20 WSL entry clearly showed its /24 portion. A real Ethernet search showed
16/762 checked ports and was cancelled. Direct NAS detection found `192.168.18.217:11434` and five model names,
including the exact Gemma name. No local model server started, and the saved config hash remained unchanged.

The browser demo was checked in embedded Chromium with mocked commands: cancelled LAN searches retained a found
server, model selection staged edits, and the mock translation replied. Screenshots were inspected at normal and
720 px widths. Standalone browser/native automation was unavailable; the app's Chromium debugging interface was used.
These are automated observations, not human game results. Full LAN discovery and real proxy/auth deployments remain
unverified; the NAS generation timeout recorded in M-20 was not retested by metadata discovery.

**Build checks:** 462 workspace tests passed (four optional tests ignored), 22 app tests and 95 UI tests passed.
Release build and strict app Clippy passed offline. Workspace tests used the existing `maru-ocr/ort-dynamic` feature;
strict workspace Clippy still reports the three pre-existing Windows capture/UIA warnings, with the compatible lint
run passing. No dependency, download, configuration key, installed setting or model choice was added by discovery.

## M-22 Small live models and remote deep-dive recovery
**Needs:** the build with HY-MT2 1.8B and LFM2 KoEn choices, their downloaded GGUF files, a heavy game, and the configured NAS.

1. Start with HY-MT2 1.8B, GPU, experimental context off. Save and restart. Enable Resources recording before turning live on.
2. Replay the same subtitle area and scene, then repeat with LFM2 KoEn. Include a longer scene, names, numbers, negation and
   instructions; report incorrect meanings as well as missed/duplicate OCR. Record game FPS/frame times and the translator,
   OCR-reader, Maru and total adapter memory separately. A small GGUF is not a guarantee of low combined VRAM.
3. If memory is still too high, repeat with Use CPU PP-OCR for live subtitles. Recognition can change; compare the Korean
   source as well as the English output. Other features retain their existing reader, which can still load later.
4. With the NAS as the main translation model, quit through the tray, reopen Maru, scan fresh Korean text and request a deep dive.
   Repeat before enabling live, with live on, and after turning it off. An unloaded/failed local live model must not cause
   `maru setup llm` for the configured remote main model. The floating status must describe the selected live model separately.
5. Try an unavailable NAS address and a wrong model name in an isolated test configuration, then restore the valid values.
   Expect a real connection/model error with offline analysis and successful retry after recovery. Do not alter the NAS itself.

**Bring:** `maru-app.log`, the resource-session JSONL, model/device/OCR selections, game/resolution, first-text/completion
times and examples of bad source readings or translations with scene context. Include whether memory or delay grows over time.

**Automated observations (2026-10-12):** eight working model conditions completed 360 requests each on a 120-source pack;
the two experimental STQ formats failed to load in the installed runtime. Owned-PID dedicated VRAM peaked at 1.44 GB
for HY-MT2 Q4 with 1,024 context tokens and 1.13 GB for LFM2 Q5 with 2,048. A supplied 1,280×180 subtitle crop read by
PaddleOCR-VL used another 2.36 GB, with combined owned-server totals of 3.80/3.48 GB. These are decimal GB, exclude
the game/desktop/Maru, and do not establish other image-size peaks, accuracy, game FPS or overlay latency.
Agent inspection found meaning errors in both small translators and more serious role/name/instruction errors in LFM2.

Fake-platform regression tests cover restart, local-load failure and remote HTTP failure/recovery. The explicit-endpoint
test against the real Ollama NAS passed cold boot, restart with `/v1`, live-on/local-load failure, wrong-model 404 and recovery,
using fixed synthetic text. No NAS settings or installed Maru config were changed. Chromium checks cover the new cards
and staged paths. A physical UI restart and heavy-game comparison remain unverified; no human result is entered below.

## M-23 Explicit model preparation, release and live status

**Needs:** updated build, existing local live/OCR/voice models, optionally the configured NAS and a borderless game.

1. With live off and no queued lines, open floating controls. Check live translator, device, OCR mode/readiness and the
   always-visible queue against Settings and Resources. Choose Prepare live session: progress changes to translator,
   OCR, then Prepared. The menu stays open/responsive. No history entry, speech load or live activation is produced.
2. Start live and translate a scene. Release idle models while it is running: the live translator and hybrid reader
   are kept. Busy speech/other model requests must finish normally. Turn live off, allow the queue to drain, and release:
   owned idle servers disappear from Resources/Task Manager and VRAM falls. Prepare again; normal live translation works.
3. With CPU PP-OCR live selected, take a snapshot to load its separate vision reader. The menu should report that unused
   reader. Release it during live: the translator remains, CPU live OCR continues, and the next snapshot reloads its reader.
4. Use speech and an Anki/voice-test request, then release while generation is active. The voice is kept until the request
   finishes; release afterwards and confirm the next sentence reloads it. No audio cache/model files are deleted.
5. Prepare with an unavailable/moved GGUF or unreachable selected external translator in an isolated config. A clear
   error ends progress. Release/reset and retry after recovery. Repeated clicks do not start duplicate workers. Hide/reopen
   controls during preparation; Quit remains usable. Try live on/off while loading and check there is no false Ready.
6. Inspect the menu on a small/high-DPI monitor. Status/error text wraps, actions scroll into view, disabled preparation
   explains live/queued work, transparent space passes clicks through, and controls remain excluded from OCR/captures.

**Automated observations (2026-10-12):** two real local lifecycle cycles with HY-MT2 1.8B, PaddleOCR-VL and the installed
voice passed preparation, preservation of live/leased models, idle voice release, translator/reader release and reload
with new PIDs. Preparation did not add history. Idle GPU memory after cleanup was close to the observed pre-test desktop
level. These tests did not measure game FPS or a precise per-model memory reduction. Synthetic OCR/translation and fake
server tests cover cancellation, concurrent callers, request leases, retry and unchanged remote status; the release action
does not contact external servers. Chromium screenshots at 350 and 320 px confirmed wrapping, scrolling to the footer,
preparation progress and disabled maintenance actions in the floating browser demo. The isolated real app showed a
missing-GGUF preparation error, cleared it through release and quit successfully. Native Windows input automation was
unavailable; these interactions used the app's Chromium debugging interface. The final local cycles prepared in
11.0 and 10.8 seconds; this includes model startup and synthetic OCR, not subtitle latency.
Physical game clicks, prolonged resource recovery, snapshot/CPU-OCR switching in a game and Anki playback remain unverified.

**Build checks:** 475 workspace tests passed (five optional tests ignored), 22 app tests and 99 UI tests passed.
Workspace tests used the existing `maru-ocr/ort-dynamic` feature. Release build and strict app Clippy passed offline.
Strict workspace Clippy still reports the three existing Windows capture/UIA warnings; the run allowing only their
two lint categories passed. The installed config hash stayed unchanged and no owned test model process remained.

**Bring:** resource-session JSONL and `maru-app.log`, selections/game/resolution, before/after memory, queue/timing entries,
screenshots of confusing status or clipped controls, and any request interrupted by release. No human result is entered below.

## M-24 Anki add recovery and learning progress

**Needs:** Anki desktop with AnkiConnect, your normally synced AnkiWeb account, updated Maru, and a separate test deck.

1. Enable Anki in Maru, save and restart, then Check Anki. The app creates its own Maru Korean note type. Keep credentials
   and first-sync decisions in Anki; follow its official syncing instructions. These tests do not require any Maru login.
2. Add a word to a test deck with audio unchecked, then press Enter repeatedly while a request is running. Only one add
   should run. Repeat a known word; it should report its existing deck without loading the voice. A genuinely new word
   with audio checked should have word/sentence attachments; it can load the configured voice and use GPU memory.
3. Inspect forward/reverse cards, tags and examples, sync from Anki/Maru, and inspect the note and media on AnkiWeb.
   Sync requested is not proof that Anki finished uploading; confirm inside Anki itself.
4. After a review, suspension or deleting a test note yourself, press Refresh progress. Deleted/suspended cards must not
   retain old Anki-based known status. Suspension retains tracked membership; deletion removes it. Manual known marks,
   local saved words and FSRS progress survive. Open a fresh translation to see the refreshed fading.
5. Observe recovery from an interrupted card-setup request in an isolated test configuration. The result must say that
   the note exists, then retry completes setup without a new note/audio. A pending record survives restart and belongs
   only to its original endpoint/note type. Do not deliberately disrupt or delete your real collection for this check.

**Automated verification:** fake-Anki tests cover partial reverse-move failure/retry, pending persistence, duplicate/busy
preflight, membership distinct from local saves, successful reconciliation and failed-refresh preservation. Browser demo
checks cover Refresh progress and the editor widgets. On 2026-11-25 steps 4 and 5 also ran against a real Anki (test
profile, throwaway deck, `MARU_REAL_ANKI=1 MARU_REAL_ANKI_PROFILE="<profile>" cargo test -p maru-engine --test real_anki`):
a card setup made to fail at the reverse-card move reported the unfinished note and the retry finished that same note
without a second one; after a card was suspended and a note deleted in Anki, Refresh progress kept the suspended word's
membership without progress and forgot the deleted one. AnkiWeb sync, media playback, physical keyboard behavior, a
pending record surviving a real restart and the user's own collection remain unverified; the agent never syncs.

**Bring:** app log, Anki/AnkiConnect versions, selected deck and audio choice, card/deck/field screenshots, any duplicate
or unfinished-setup message, and whether the word/note/media appeared on AnkiWeb. Do not send credentials or API keys.

## M-25 Snapshot OCR corrections

1. After a region snapshot or full screen scan, right-click the panel/label and choose Correct Korean OCR…. Correct
   `ㅅ컵` to `스킵` and retranslate. The Korean source must change, use the same region/program and discard old word boxes.
2. Leave Remember unchecked: a later fresh OCR reading is unaffected. Check Remember: the exact same detected text in
   that executable is corrected on later snapshots and after restart. Other text/apps are unchanged; games in the same
   emulator executable share this scope. Continuous subtitles must retain their existing recognition behavior.
3. Reopen the correction and use Forget saved rule. It removes future reuse of that rule, leaving the current edited
   snapshot intact. Unchecking Remember alone does not delete a previous rule.
4. Open an editor, make a newer snapshot/scan, then submit the older form. It must refuse the stale target and save no
   rule. Correct an item while an original model answer or join is pending; late answers must not overwrite the new label.
5. Check borderless-game mouse access, Korean IME typing, cancel/reopen, narrow/high-DPI layout and capture exclusion.
   The editor is a normal focusable window; the scan remains click-through outside its reported label/button rectangles.

**Automated observations (2026-10-13):** pipeline tests cover one-time and remembered scan fixes, program isolation,
forgetting, new item IDs and stale snapshot/scan refusal. The real Windows app opened its editor from injected synthetic
OCR text, applied `ㅅ컵` → `스킵`, and saved the exact rule in its isolated labels.db. That app had models disabled: the
panel showed corrected Korean plus offline analysis, not a real-model translation. Scripted model tests cover translation.
Chromium screenshots were inspected at 640/420 px, with apply/forget/stale feedback and the scan pencil on hover.
Native input automation was unavailable; Chromium debugging was used. Real game recognition, IME/mouse behavior and
AnkiWeb remain user checks. The installed config hash was unchanged; no human result was entered.

**Bring:** original/corrected Korean and English, selected checkbox/program, behavior after restart, app log, screenshots
of any clipping or stale-answer replacement, and whether live subtitles changed unexpectedly.

**Build checks:** 484 workspace tests passed (five optional tests ignored), 22 app tests and 104 UI tests passed.
Workspace tests used the existing `maru-ocr/ort-dynamic` feature. Strict app Clippy and the release build passed offline.
Strict workspace Clippy still reports the three pre-existing Windows capture/UIA warnings; allowing only those two
lint categories passed. No installed settings, model files, dependency versions or real Anki data were changed.

## M-26 Translation completion, retries and overlapping requests

1. Translate English into Korean with the normal model. Check the requested register and its word breakdown. If the
   model fails or omits that register, an error must appear and the request must finish rather than keep waiting.
2. Select a Korean line again after a temporary word-meaning failure. The completed translation should appear from
   cache, and missing meanings should retry. A completed translation must not create another history entry on replay.
3. Start a long snapshot request, immediately select another line, and check that late events cannot replace the
   newer answer. Keep a deep dive open when checking delayed events; it must remain attached to the current answer.
4. Test live subtitles with `max_in_flight = 2` in an isolated configuration. Even when a later line finishes first,
   subsequent dialogue context must follow capture order. Header stalls must release the slot after the configured
   timeout; capped/malformed/incomplete responses must report errors and stay out of history and reusable context.
5. Retest the configured TrueNAS server, including English-to-Korean, repeated Korean snapshots and a short cutscene.
   This checks its real streaming format and normal translations after the stricter completion validation.

**Regression reproductions (2026-10-13):** before the fixes, scripted servers reproduced the missing-register hang,
header stall, acceptance of incomplete/capped streams, cached failed glosses and completion-ordered live context.
Pure panel tests reproduced stale and duplicate starts replacing an answer. These use synthetic data, not game results.
Real game timing and model quality still require the user's cutscene check. No result is added to the human results log.

**Verified after the fixes:** 503 workspace tests passed (five optional tests ignored), 22 app tests and 106 UI tests
passed. Strict app Clippy and the release build passed offline. Strict workspace Clippy still reports the same three
pre-existing Windows capture/UIA warnings; allowing only those two lint categories passed. Chromium screenshots showed
stale/duplicate starts preserving the current answer and its open deep dive. Native input automation was unavailable;
the check used the local browser demo through Chromium debugging.

The real TrueNAS Ollama model completed fixed Korean-to-English and English-to-Korean requests with word meanings,
all three Korean registers, `Done` and no errors. A one-sentence live smoke test also completed without errors
(1,954 ms to first word, 2,357 ms total); this is a protocol check, not a game benchmark. All three used an isolated home.
The installed configuration's SHA256 remained unchanged. No model files, dependencies or user Anki data were changed.

**Bring:** Korean/English text, selected model and endpoint, whether the panel finished, app log, cached/retry behavior
and a screenshot or recording if a newer answer was overwritten. Do not send credentials.

## M-27 Right-click corrections and floating scan controls

1. In a borderless game, use floating **Label screen**. Its button turns green; press it again and verify that labels
   disappear rather than a new scan starting. Repeat while OCR/model work is pending; late results must stay hidden.
   Turn it back on. Join words and Refresh labels appear only while labels are active. Refresh labels explicitly rescans;
   the tray and `Ctrl+Alt+S` retain their existing rescan behavior.
2. Left-click anywhere on a scan label to open its explanation (single words open deep dive). Right-click must keep the
   label and offer Correct Korean OCR…, Hide this label and Cancel. Hide removes only the current displayed label.
   Check menus at screen corners/high DPI and that the game still takes clicks outside labels/menu/chip.
3. Choose Correct Korean OCR… from a scan label and a region snapshot panel. Confirm the current Korean appears in
   the editor; correct it and retranslate. The former hover pencil, delete corner and panel correction button are gone.
   The menu must stay usable while model output resizes the panel. Remember/Forget retain M-25's exact per-app behavior.
4. With labels active, press floating Join words, left-click two or more pieces of a sentence and press Join. They become
   one label and ordinary clicks return. Cancel or press Join words again to leave selection mode. Shift+click remains
   available; select a joined label to Separate it. Refresh/hide while a menu, selection or join is pending: old state must
   not replace the new scan. Live subtitle capture must continue with its existing routing/settings.

**Automated observations (2026-10-13):** Chromium demos verified right-click correction without removal, explicit Hide,
whole-label open actions, join selection/completion and return to normal clicks, menu hit rectangles, edge placement,
small-panel correction, green floating scan/join states and hiding join/refresh controls when scan is off. Scripted
browser events also verified stale starts/clear notifications and delayed mode responses. These are browser demos,
not game/physical-input observations; the native Computer Use pipe was unavailable. Borderless-game focus, Windows
click-through and live/snapshot interaction still require the user's check. No human result is entered below.

**Bring:** whether the toggle hid or rescanned, labels chosen for a join and the resulting sentence, corrected Korean
and translation, app log, and a screenshot/clip of any stale label, menu clipping or click that reached the game.

**Build checks:** 503 workspace tests passed (five optional tests ignored), 23 app tests and 112 UI tests passed.
Strict app Clippy and the release build passed offline. Strict workspace Clippy still reports the same three existing
Windows capture/UIA warnings; allowing only those two lint categories passed. The existing dynamic ORT feature was
used for workspace tests. No installed configuration, model files, dependencies or user data were changed.

## M-28 Screen-label loading, failed and unavailable indicators

1. With the configured TrueNAS model, label a menu with uncached text. Empty labels must show softly pulsing dots
   while waiting, with "Waiting for translation…" on hover. Each label stops pulsing as its translation arrives.
   Dictionary/remembered labels remain readable while other labels wait. Check dense labels that collapse to dots.
2. Check an actual connection failure or timeout in an isolated test configuration. Unanswered labels must settle
   to an amber **!** with a persistent failure reason on hover; existing translations must remain visible. Refresh
   labels after restoring the connection. A completed response omitting a label must show a static **?** instead.
3. Join/correct a label while other labels are waiting. One operation finishing must not settle another operation's
   items. Hide/refresh before a reply arrives: no old status or animation may restore the previous scan.
4. With Windows reduced-motion preferences enabled, loading dots remain visible but do not pulse. Check ordinary
   left-click deep dive, right-click correction/hide, join selection and click-through outside labels.

**Verification:** Pure UI and scripted-engine tests plus Chromium demo checks are recorded in the local diagnostics.
Actual NAS timing, borderless-game input and Windows accessibility preferences still require the user's check; browser
emulation does not verify these. The joined-sentence spacing bug remains a separate, diagnosed issue.

**Bring:** screenshot/short recording, whether the NAS was reachable, pending/failed/unavailable label text and hover
reason, and the relevant app log. Do not send credentials.

## M-29 Reversible word/sentence joins and per-word deep dives

1. Rescan the game's affected menu with the rebuilt app. A saved `카메라설정` join must display word boundaries.
   Click it: the panel should have separate `카메라` and `설정` word controls. Try the earlier four-word camera sentence
   and open each word's deep dive. Verify the game profile applies even after clicking Maru changes foreground focus.
2. Right-click → Separate joined label. The original boxes return without recapture, including after restart/rescan.
   Refresh labels: the join must stay forgotten. The selection chip's Separate must do the same. Test while the
   joined translation is still waiting or has failed; late answers must not restore the removed combined label.
3. Join adjacent words whose boxes almost touch. Join adds spaces; One word explicitly joins fragments (`게` + `임`).
   Refresh/restart to check both choices. Join an already joined word with another word, then Separate: restore all
   original pieces and forget the nested rules. A join in another program must remain intact.
4. Correct a joined label's Korean, then Separate. Original pieces remain recoverable; correction/Remember/Forget on
   a previously corrected piece must still show its original OCR. Try joining twice quickly, refreshing during a command
   and hiding during separation: stale results must not replace new labels or reuse another operation's item IDs.

**Verification:** Scripted-engine/pure UI tests and isolated Chromium demos are recorded in the local diagnostics.
Real game/NAS translation quality, physical click-through/high DPI and restart behavior in the user's installed
profile still need this manual check. No model, dependency or user database was downloaded or altered by these tests.

**Bring:** Korean joined text, selected Join/One word action, a screenshot of the word controls and Separate menu,
which word fails to open, and the app log. Do not send credentials.

## M-30 Saved game reading areas

1. In Hollow Knight (or another game with a fixed dialogue box), use floating **Save a new area**. Select the entire
   paragraph and exclude decorative borders/speaker labels unless you want them read too. Rename it in Reading areas.
   Return to the game and press **Read again** on successive pages without redrawing. Verify full paragraph spacing,
   translation, word meanings, individual deep dives, and the existing snapshot OCR correction menu.
2. Assign Read saved area again in Settings → Shortcuts and repeat with that key. Add a second named area; select it,
   rename it, cancel removal, and remove it. Restart Maru: saved names and the selected area survive; Auto-read is off.
   Different executables keep separate selections. Games inside one emulator require explicit distinct names/selection.
3. Enable **Auto-read**. Check typewriter/fade-in dialogue, animated backgrounds, repeated pages and empty dialogue boxes.
   It should confirm settled OCR text, avoid repeated model requests for unchanged text, and retain the last reading
   after a blank box. Quickly advance several pages while a NAS request is pending: only the latest pending paragraph
   should be read next; older output/errors must not replace it. Settling/queued status identifies the previous answer.
4. Press **Hold**, inspect several words, and press **Resume** on a later page. Deep dive, snapshot correction and opening
   another snapshot should also hold Auto-read. Check focus, resizing and click-through with the floating menu in a
   borderless game. Switch to another program: capture pauses; returning confirms the game text again.
5. Move the game window, including to a monitor left of the primary display, and test high DPI. The crop follows it.
   Resize/change resolution: reading must pause/refuse the old size. **Check scaled area** or **Reposition** outlines the
   area; Enter retains it, dragging replaces it, Esc cancels. Confirmed positioning returns to manual mode. Minimize/close
   the game during capture/selection and ensure no other application's pixels are submitted.
6. Try an actual configured NAS failure in an isolated setup, restore it, then Read again. Verify clear failure/retry
   status and usable offline breakdown. Check the Resources page for watcher/OCR/model cost beside the game.

**Verification:** Scripted-engine tests cover full snapshot/gloss/deep-dive flow and cache, per-executable persistence,
movement/size validation, superseded jobs/errors, blank/duplicate confirmation and game switching. Chromium demos
cover the manager, floating controls and picker/status layout. These do not verify actual game OCR quality, Windows
foreground/physical input, high-DPI capture or NAS timing. Native Computer Use was unavailable; no human result is entered.

**Bring:** area name/game, manual or Auto-read mode, resolution/DPI, screenshot or short clip of the original paragraph
and panel, app log, and Resources readings if latency/load is high. Note skipped/partial/duplicate paragraphs and whether
Hold/Resume and window movement worked. Do not send credentials.

## M-31 Borderless controls wheel and saved-area registration

1. Run Hollow Knight in borderless mode with its cursor hidden. Press Ctrl+Alt+Q: the wheel should open at the pointer
   with a visible cursor. Use mouse selection, arrows/Enter and 1–8; More controls must expose the other two pages.
   Check the VRAM bar, active toggles and unavailable Join/Read controls. Esc, the shortcut again and Alt+F4 close it.
   Check that control returns to the game; a focus-loss pause is expected in games configured to pause.
2. Choose Snapshot region from the wheel. Draw anywhere across the dialogue, including beyond the game's former
   cursor confinement. Escape/right-click/Alt+F4 cancel. Repeat and complete a snapshot, then use Save an area to name
   and register a game area. It must appear in Reading areas and remain after restart. Read again must use that area.
3. Open the manager in windowed mode, switch the game to borderless before starting selection, then save. Repeat
   after moving the game and with another same-executable helper window present. Current geometry/original identity
   must be used. Move/resize/minimize/close the game *during* selection: refuse the save with a clear error, logged as
   `saved reading area:`; never register another application's rectangle.
4. Alt-tab away while the wheel/picker is open. Closing it must not steal focus or restore stale confinement. Repeat
   with a game that changes its cursor clip upon focus gain, two monitors (including negative origins), and high DPI.
   The wheel must fit its monitor and the crop must align with the physical selected pixels. Close Maru normally.
5. Change/disable Controls wheel in Settings → Shortcuts. A duplicate/taken key must report a registration problem;
   Ctrl+Alt+M and existing capture keys keep their meanings. Repeat a failed saved-area request and retry successfully.

**Verification:** Portable native-input policy tests, fake-window saved-reading regressions and real Chromium wheel
screenshots/interactions cover logic and layout. The Windows implementation compiles, but hidden/confined game cursors,
actual focus return, borderless registration and physical DPI behavior need this real-game check. No human result is entered.

**Bring:** game/mode, shortcut, the exact save error, a short clip of opening the wheel/drawing/saving, DPI/resolution,
and `maru-app.log`. Include whether the cursor/focus returned correctly. Do not send credentials.

## M-32 Wheel manager handoff and manual-reading feedback

1. Open Hollow Knight in borderless mode, then Ctrl+Alt+Q → Save an area / Reading areas, Settings, History and App status.
   Each manager must appear in front on the first action, with a visible pointer, without returning briefly to the game.
   Switch to the game or another application: the manager must lose its temporary topmost state. Retry after closing it.
   Check keyboard/mouse and controller behavior separately; a controller may keep controlling the game while Maru has
   focus. Pause the game before naming an area or inspecting a reading.
2. From the wheel, manager and assigned Read saved area again shortcut, read a saved paragraph with cold OCR, then repeat
   warm and cached reads. A small indicator must acknowledge the action immediately, show reading → waiting → translating
   as those phases occur, and finish with Reading ready. Fast phases may pass quickly. Errors must show the actual reason.
   The indicator must ignore clicks, keep game focus and stay out of captured Korean text. Check high DPI / another monitor.
   Ready feedback clears after about six seconds. The panel must survive reading and that brief completion feedback;
   use its pin button for longer inspection, and check normal idle hiding resumes when unpinned afterward.
3. Enable Auto-read and explicitly Read again while Maru controls have focus. Repeat with Hold enabled and while an older
   paragraph is translating: the manual capture must survive polling, display its real progress and retain only the latest
   pending text. An older failure must not replace current feedback. Hold during capture, then Resume: watching must recover.
   Reopen the wheel/manager while a manual or automatic answer is streaming: it must keep finishing without capturing
   Maru's controls. Return to the game and change dialogue; the latest paragraph must still queue normally.
4. Minimize the selected game before Read again to trigger an error. Restore it and retry once. Also test a changed window
   size, blank OCR region and an unavailable configured model when practical. Feedback must settle into a clear error or ready
   state; it must not stay on Reading or Waiting forever after a completed/failed operation. Do not change model quality settings.
5. Use Dismiss / Ctrl+Alt+Esc during progress and after an error. It must hide the indicator, panel and labels; it does not
   cancel OCR/model work already underway. Reopen controls and read again: a fresh indicator must work, and an older action
   must not reopen its wheel or overwrite it. After reading, click each content word for its normal explanation.

**Verification:** Slow-OCR engine regressions reproduce manual/automatic overlap and Hold/Resume recovery. Shell/UI tests
cover progress IDs, phases and manager action routing. Real borderless focus handoff, game/controller input, indicator
capture exclusion and physical DPI behavior still need this Windows/game check. No human result is entered here.

**Bring:** game/mode, keyboard/mouse versus controller, OCR cold/warm, model route, a short recording from the button press
through the result, the exact feedback/error, and `maru-app.log`. Include whether the manager appeared on the first action,
whether the game kept receiving controller input, and whether Read again needed a retry. Do not send credentials.

## M-33 Review fixes: wheel left for the game, picker without focus, hover with auto-read, floating controls

*Stand-in run (2026-10-17, a fullscreen browser window as the game): the wheel closed and the picker opened on the region
shortcut, the picker appeared while the game kept receiving key presses, and the floating controls cost about 0.7% of a
core. It found that after a snapshot focus stayed on Maru's floating controls instead of returning to the game: the hidden
picker handed the foreground to the controls, and the fullscreen game window had shrunk by a pixel, which the return
check treated as a different window. Both are fixed and focus returned to the game in three rounds out of three.
Steps 3 and 5 were not run.*

1. In a borderless game, open the wheel (Ctrl+Alt+Q), click back into the game, then press the region shortcut. The
   wheel must disappear and the area selector open; finish a snapshot and check that focus returns to the game.
2. Press the region shortcut and keep playing (keys or mouse in the game) while the selector loads. It must still
   appear; draw across the dialogue with the mouse. `maru-app.log` may show `region picker opened without keyboard control`.
3. With a saved reading area on Auto-read and Hover lookup on, rest the pointer on words in the game. Auto-read must keep
   reading new paragraphs (the status must not change to held). A Translate-selection or region snapshot still holds it.
4. With the floating controls visible, open the menu and click a control immediately; drag the button and click it at
   once. Hide them (Ctrl+Alt+M) while a game runs and compare Maru's CPU use in Task Manager with them shown.
5. With Anki open, press **Refresh progress** in Settings → Anki and, during it, Add a word from the panel. The add must
   finish after the refresh instead of reporting that another add is running.

**Verification:** each step was driven on the developer's Windows PC with scripted key presses and clicks against a stand-in
game window (a plain form), the real Maru build, a stand-in AnkiConnect and the LAN Gemma server: before the fixes the
wheel step failed with "The active window changed", the selector was destroyed when focus was refused, one dwell held
Auto-read, and the add was refused during a refresh; after them each step behaved as described. That is not a person with
a real game, controller or Anki collection, so no result is entered here.

**Bring:** game/mode, which step, what happened, and `maru-app.log`.

## M-34 Model on another computer: loaded while you play, released after

*Stand-in run (2026-10-17): with Maru closed the NAS had no model; opening Maru loaded it fully on the GPU (6.5 of 6.5 GB)
about 10 s after start; it was released 62 s after live subtitles stopped and within 0.5 s of quitting. Immich was not
running.*

**Needs:** Ollama on another PC (here a TrueNAS box with an RTX 3060 Ti, 8 GB, also used by Immich), the tuned model from the
README ("A model on another PC"), `[llm] server_keep_minutes = 15`, and, for live lines on that server, Live translation model = Gemma.
1. With Maru closed, `ollama ps` on the server shows nothing. Open Maru: within a few seconds it shows the model at `100% GPU`.
2. Play a cutscene with live subtitles. Lines should start in well under a second; note any line that takes longer and whether
   Immich was working at the time. `ollama ps` keeps renewing the unload time while lines arrive.
3. Stop live subtitles: about a minute later the model is gone from `ollama ps`. Choose a new subtitle area instead: it stays.
4. Quit Maru from the tray or the floating controls: the model is gone at once. Then run an Immich job: it must find the GPU free.
5. Start a game while Immich is indexing: Maru's first lines may fail or be slow while both want the 8 GB; say what happened.

**Verification:** on the developer PC with the stand-in game window and the real server: opening Maru loaded the tuned model at
100% GPU; nine live lines took 138–266 ms to the first word (one 803 ms) and 256–443 ms to finish, including OCR; the model was
unloaded 70 s after live stopped; quitting from the floating controls unloaded it within 317 ms. Not a person with a real game,
and Immich was not running. No result is entered here.

**Bring:** `ollama ps` before/after each step, the `live line:` and `server_hold` lines of `maru-app.log`, and what Immich was doing.

**Server change made on 2026-10-14 (the developer's TrueNAS box, `192.168.18.217:11434`, Ollama 0.40.2, RTX 3060 Ti 8 GB).**
*What was found:* the installed `hf.co/unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL` loaded as 7.4 GB at **90% GPU** with Ollama's
defaults. Six short lines through `/v1/chat/completions` took 7.9–8.2 s to the first word when cold (Ollama unloads after 5 idle
minutes) and 1.1–1.7 s warm (done 1.8–2.5 s). Ping was 0 ms. A model that fitted entirely (`qwen2.5:7b`, 100% GPU) answered in
0.04–0.07 s, so the spill to the CPU, not the network or the GPU's speed, made Gemma slow.
*Per-request trials (nothing saved):* `num_ctx` 4096 or 2048 changed nothing; `num_batch` 128 gave 94% GPU; `num_batch` 32 made
Ollama answer HTTP 500; `num_gpu` 49 (all layers) with `num_batch` 128 gave **100% GPU, 0.11–0.12 s to the first word, 0.23–0.34 s
done, 3.5 s cold load**. Prompts of 1,945 / 3,385 / 6,825 tokens at `num_ctx` 4096 / 4096 / 8192 ran without running out of memory.
*What was created:* one model tag, through the API, reusing the downloaded files (no download, almost no disk):
`POST /api/create {"model":"gemma4-12b-3060ti","from":"hf.co/unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL","parameters":{"num_gpu":49,"num_batch":128,"num_ctx":4096}}`.
Ollama gives a derived model its built-in `gemma4` renderer/parser; the GGUF's own (Jinja) template cannot be set as an Ollama
template. Through Maru's CLI, Korean→English with notes and English→Korean in three registers came out identical to the original
model; a deep dive (temperature 0.4) was worded slightly differently; no thinking text, stops worked. Nothing else on the server was
changed: no environment variables, the original model is untouched; tests only loaded and unloaded models (`keep_alive: 0`).
*Undo:* `ollama rm gemma4-12b-3060ti` in the Ollama app's shell, or
`curl -X DELETE http://192.168.18.217:11434/api/delete -d '{"model":"gemma4-12b-3060ti"}'`; it removes only the tag.
*Risk and first incident:* `num_gpu` 49 bypasses Ollama's memory safety margin. In the user's first real session (64 live lines),
one request at 03:48:44 UTC got HTTP 500 "model runner has unexpectedly stopped, this may be due to resource limitations"; Ollama
restarted the runner, the next line waited 12.7 s and three queued lines arrived 2.8–8.5 s late, then lines were normal again.
The cause is not yet confirmed (suspected: VRAM taken by another GPU user on the server); the Ollama log for that minute is needed.

## M-35 Saved external connections and Azure authentication

**Needs:** a NAS/OpenAI-compatible server and an Azure OpenAI resource with a chat-completion deployment and API key.
1. In Translation model, enter the NAS URL/model, name it NAS, Save this connection, then Save. Repeat for Azure with
   Authentication expanded, Azure OpenAI selected, resource URL, exact deployment name and key. Restart and reopen:
   both names must remain in Saved connection. Ordinary NAS entry/detection must still work without authentication.
2. Select Azure: all its connection fields must fill, with a masked key. Test connection must return a real greeting;
   Save/restart, then test a snapshot and per-word deep dive. OCR and local live selection must retain their setup.
3. Select NAS, Save/restart and repeat. Azure's key must be cleared from the active NAS fields. Neither profile is forgotten.
4. In Live subtitles, select the same registrations for Snapshot quality server. That selection must not change the main
   Translation model fields, local live model/device, OCR settings, retention or fallback decisions. Test both routes.
5. Wrong Azure key/deployment must show a failure, not Connected. Editing the key invalidates an old successful result.
   Correct and retry. Save an existing registration name to update it; other names must survive. Discard staged edits:
   saved values remain unchanged. Restart must preserve credentials without requiring them in chat or logs.

6. Gemini: choose Google Gemini with `https://generativelanguage.googleapis.com/v1beta/openai`, `gemini-2.5-flash` and an
   AI Studio key. Test connection, a snapshot, a deep dive and a live line must answer. Repeat with a Gemini 3 model (it
   refuses "no thinking"; `maru-app.log` should show one `does not accept this field` line, then answers).
7. Type an Azure URL into the connection-name box and press Save as a connection: it must be refused with what to do.
   Remove a saved connection: it disappears from both selectors after Save, and the active fields stay.

**Verification:** local fake-server tests cover actual authentication headers, Azure v1 request/deployment, missing/error
keys, disabled key-bearing redirects, no cloud load/unload calls, credential isolation and config round trips. Node/browser
checks cover registration and selectors with dummy credentials. On 2026-10-14 the real app and CLI were also driven against a
strict local stand-in for both services (exact routes, required key headers, 400s for refused fields): Azure `gpt-4.1-mini`
kept its temperature; a reasoning deployment refusing `temperature` and a Gemini 3 model refusing `reasoning_effort: "none"`
each answered after one retry; a wrong key gave a clear 401; no `keep_alive` reached either. The Settings flow (refusing
an address as a name, Test connection, Save as a connection, Remove) was driven in the real page. Real Azure and Gemini
accounts, billing, authorization and model output caps still need this manual test; no human result is entered here.

On 2026-11-25 every saved connection answered real requests through Maru's client from the command line
(`maru connection test`, results log): this PC's model, the NAS, Azure, Gemini and Claude. It found that Claude through
Anthropic's OpenAI-compatible endpoint was treated as a server of the user's own (the first request of every run was
refused for its `temperature`, and it would have been sent keep-alive and unload calls), now fixed, and that the saved
Gemini key is a free-tier key. The Settings page, the snapshot and deep-dive routes in the app and steps 1 to 7 above were
not driven against the real services.

**Bring:** selected registration/provider, deployment name, exact error and `maru-app.log`; redact credentials. Do not send keys.

## M-36 Live subtitles: server backup, OCR noise and OCR on the GPU

*Stand-in run (2026-10-17): unloading the NAS model 20 s into the stand-in cutscene: the next line came from the LFM
backup after the 2.5 s wait, the next seven from the backup at 0.25–1.25 s, then the NAS again (0.8 s); the backup
mistranslated one dialect line and cut one short. Maru's CPU during the cutscene: 256–317% of one core with PP-OCR on the
CPU, 14–17% with DirectML; read time about 229 ms either way (PaddleOCR-VL sets it).*

**Needs:** live subtitles on Gemma on another computer (M-34), `continuous.translation.backup_model = "hy_mt2_small"` with its
file, and a game with Korean subtitles.
1. Play a cutscene. Lines come from the server (`live line:` in `maru-app.log`, no `backup translator` lines).
2. On the server, unload the model (`ollama stop gemma4-12b-3060ti`) or restart the Ollama app mid-cutscene. The next line
   must still appear within about 3 s (`this line uses the backup translator`), the following ones quickly; after about
   20 s lines come from the server again. Repeat with the server switched off entirely.
3. Watch for lines made of OCR noise (a bar or dash under the subtitle): none should appear in the band or the history.
4. Set Run PP-OCR on → GPU (DirectML), save, restart. Compare Maru's CPU in Task Manager during a cutscene with the CPU
   setting, and check reading speed in the `live line:` OCR times. Then CPU with `threads = 4`.

**Verification (2026-10-14, developer PC, no game):** scripted-server tests cover failed, late, half-answered and healthy
servers and a broken backup. Against the real NAS (RTX 3060 Ti, Ollama) with the real HY-MT2 1.8B backup on this PC's CPU:
warm server 0.38 s; after unloading the server model, the line came from the backup in 2.78 s, the next in 0.32 s, and after
the rest the server answered again warm in 0.43 s. PP-OCR on the real models, one subtitle line: CPU 101–105 ms per read and
13–14 s of CPU for 21 reads; DirectML 26–32 ms and 2–3 s; 4 CPU threads 143–152 ms and 10–11 s. The engine read the line on
DirectML and fell back to the CPU for a device that cannot load. Not checked with a real game or by a person.

## M-37 Live history: each line once, kept after a restart

1. Play a cutscene with live subtitles. Open **History → Live subtitles**: each spoken line should appear once, not twice
   with different spacing, and not as a half line followed by the whole line.
2. Count model requests: in `maru-app.log`, `outcome="repeat"` marks a reading answered without a request.
3. Look for two different lines merged into one (they must stay apart, especially lines with 안/못/not). Report any.
4. Quit and reopen Maru: the same lines are still in History → Live subtitles.

**Verification:** unit and scripted-pipeline tests cover re-reads with other spacing or quotes, partial and growing readings,
misread syllables, a negation or number kept apart, other games and the window, and saving across a restart.
**First real session (2026-10-14, the user's game, OCR re-reading every ~0.3 s over a moving cutscene):** 30 of 116 saved
lines were misread re-readings of a subtitle still on screen; with Azure (≈1.2 s per request, one at a time) they queued up
to 15 deep and the first word came up to 12.4 s late. The rules were then extended to misreadings and stray marks and
replayed on those saved lines (`MARU_HISTORY_DB=… cargo test -p maru-engine --lib -- --ignored replays_a_saved_session`):
26 of the 30 merge, no two different lines do, and the 4 left are replies under four syllables kept exact on purpose.
Replaying that burst's real arrival times against a stand-in model as slow as Azure: 7 requests instead of about 20, 7
history entries instead of 17, worst wait 2.2 s instead of 12.4 s.
**Stand-in cutscene (2026-10-17, developer PC, the real app with real OCR, NAS Gemma and Azure):** a browser page in kiosk
mode showed 17 outlined Korean lines over a moving, noisy backdrop in the user's subtitle box, with a Latin `AUTO LOG`
label beside them. It found that the watcher dropped `…안 간다` and `5시에…` after `…간다` / `3시에…` (no gap) and read the
label alone as lines; that `네.`/`왜?` with the label read as English; that a typing line was listed twice; and that the
first line of a session was missed while PaddleOCR-VL loaded. After the fixes: all lines read (a single-glyph `왜?` was
still missed by PP-OCR in 2 of 5 runs), the typing line once, the first line read by PP-OCR alone. A replay of a recording
of that page (`MARU_CLIP_LABEL="AUTO LOG"`): before 2 missed and 6 extra reports, after 0 and 0, slowest read 1.07 s.
Azure on that cutscene: first word median 1.46 s (1.0–2.2 s), no backlog; NAS median about 0.4 s. A real game's
fonts and effects are still unchecked.

## M-38 Star a live line from the game

*Stand-in run (2026-10-17): real `Ctrl+Alt+B` presses and the wheel's Star this line during the stand-in cutscene above:
focus stayed on the "game" for every press, stars landed on the line shown, on a line that had just appeared and (with a
reminder) on one that had gone; a double press took the star off. Not checked in a real game.*

1. With live subtitles on in a game (borderless and windowed), press **Ctrl+Alt+B** while a line is on screen: a gold ★
   appears before the translation with a short glow, and the game keeps keyboard focus (keep moving or typing in the game).
2. Press it right after the line disappears: the band returns for about a second with the ★.
3. Press it twice: the star goes off. Press it as a new line is still appearing: the star lands on that new line.
4. Open the wheel (**Ctrl+Alt+Q**) → second page → **Star this line**.
5. **History → ★ Starred** lists the starred lines; ☆/★ in Live subtitles toggles them; they are still starred after a restart.
6. Check `Ctrl+Alt+B` does not clash with the game's own keys.

**Verification:** scripted-pipeline tests cover the line on screen, a line still translating, a repeat and a half-read
line resolving to the real line, no line on screen, and stars kept after a restart; UI tests cover the band state and the
History rows; the History view and the starred band were checked in headless Chromium. Not checked: the real hotkey in a
game, focus staying with the game, the band in the content-protected live window.

## M-39 Make cards from a starred line

*Stand-in run (2026-10-17, real Anki in a throwaway deck, real NAS meanings, real Qwen3-TTS voice, all deleted after):
meanings fit the line (억수로 → "extremely"); cards, reverse cards, sentence card, tags and sounds were right; a second
Make cards reported every card "already in Anki". It found that Maru's AnkiWeb sync started mid-batch and Anki refused the
remaining cards ("Collection lifecycle operation is active"), and that the voice sometimes made 6–14 s of sound for one
word; both are fixed (the sync waits for the batch, busy Anki is retried; overlong takes are asked for again) and the
re-run added every card with words of 0.4–1.0 s. All ten words of a line start ticked, which can mean 20 cards from one line.*

Use a test deck (or an Anki test profile) the first time.

1. History → ★ Starred → **Make cards** on a line. Within a few seconds the words appear with meanings marked "as used
   in this line". Check the meanings fit the line (not just the dictionary's first sense) and that words you know or
   Anki has are unticked.
2. Untick one, edit one meaning, keep the sentence card and reverse cards on, choose a deck, **Add**.
3. In Anki: each word card shows the edited meaning, this line (word in bold) and its translation, plays the word's and
   the line's sound; its reverse card (in `<deck>::Reverse`) shows the meaning to write the word from. The sentence card
   (`Maru Korean Sentence`) plays the line on the front and lists the picked words on the back; it has no reverse card.
4. **Make cards** on the same line again: everything is reported "already in Anki"; nothing is duplicated.
5. Try with Anki closed: the sheet says Anki cannot be reached and the button stays off.
6. The line shows **✓ Cards made** after a restart.

**Verification:** engine tests against the scripted model and a stand-in AnkiConnect cover the sheet (meanings in the
line, one model request, dictionary fallback, what Anki has), a word card carrying this line and the chosen meaning
even when the word has a newer sentence, the sentence card's fields, tags and single card, duplicate lines, and a word
without a meaning being refused; the sheet was checked in headless Chromium with mocked commands. Real Anki's search
syntax for empty and filled fields was checked read-only against a real collection. With Anki open,
`MARU_REAL_ANKI=1 MARU_REAL_ANKI_PROFILE="<profile>" cargo test -p maru-srs --test real_anki` adds a word card (with
reverse), a silent word and a sentence card to a throwaway deck, checks fields, decks, duplicates and sounds added later,
then deletes them (it creates the two Maru note types if missing; they stay, AnkiConnect cannot delete one). The profile
must be named: the test writes through a guard (`maru-srs/tests/support/guarded_anki.rs`) that refuses every change
unless that profile is open in Anki and the change stays in the throwaway deck, and that never forwards a sync. The same
command with `-p maru-engine` runs steps 1 to 4 through the engine (the sheet's ticks from what Anki has, a word card
with this line and the edited meaning, reverse cards off giving one card, the sentence card, a second run adding
nothing). On 2026-10-14 it ran against the developer's Anki (AnkiConnect 6): its first run found that
AnkiConnect attaches a sound only to a field the update also names (with `fields: {}` it answers success and attaches
nothing), which the stand-in had hidden; `add_sounds` now names the fields and the stand-in behaves the same way. The
second run passed in 3.4 s and left no deck, note or sound file. On 2026-11-25 both tests passed on a test profile
(results log), the engine one with speech files made by the installed voice (`maru tts`). These are automated runs, not
a person reviewing the cards in Anki: nobody has listened to the cards, the meanings came from a scripted model, and the
sheet itself (steps 2, 5 and 6 in the app) was not driven.

## M-40 Anki on from the sheet, and sounds added later

*Stand-in run (2026-10-17): Turn on Anki saved the setting and restarted Maru (same home); Check Anki reported both note
types; with speech off cards were added with the "no sound" message; after turning speech on, Add missing sounds filled 6
notes in 17.4 s and a second press found none left. Not checked: the restart in the user's own setup.*

1. With **Review in Anki** off, open **Make cards**: the sheet says Anki is switched off and offers **Turn on Anki**; the
   first press explains, the second restarts Maru. After the restart, Settings → Anki shows it on and **Check Anki**
   reports both note types ready.
2. Turn Settings → Speech off (or stop the voice), make cards: they are added and the sheet says they have no sound.
3. Turn speech back on, press **Add missing sounds** (in the sheet or Settings → Anki): the result counts the notes given
   sound; in Anki those cards now play the word and the sentence, and a card you gave a sound by hand keeps it.
4. Time the first sound after Maru starts (the voice loading) and later ones.

**Verification:** the search for silent notes, filling only empty fields and keeping existing sounds are tested against
the stand-in AnkiConnect; the sheet's turn-on flow was clicked through in headless Chromium with mocked commands. On
2026-11-25 step 3's engine side ran against a real Anki (results log): notes added without sound were found by the real
search, got real speech files through `updateNoteFields`, were no longer listed, and a second fill changed nothing. Not
checked: the **Add missing sounds** button itself (the shell speaks each text and fills up to 30 notes of the whole
collection, which the automated run does not do outside its throwaway deck), hearing the result, a real restart.

## M-41 Card meanings in English and Spanish from the learner's dictionary

**Needs:** the dictionary installed (`python tools/build_nikl.py <xml folder> nikl.jsonl`, `maru dict nikl nikl.jsonl`,
restart; Status lists "Learner's dictionary: 한국어기초사전"), Anki as in M-39, and a model for lines with ambiguous words.

1. History → **Make cards** on a line. Each word shows an English and a Spanish meaning and where they came from ("the
   dictionary's only sense", "the dictionary sense chosen for this line", "the dictionary's first sense: check it fits",
   "not in the dictionary: the model's words"). Judge whether the chosen senses fit the line.
2. On a word with several senses, pick another one from the list under it: both meanings change. Type over one: the list
   says it was edited by hand.
3. Add. In Anki the word card shows the Spanish meaning under the English one (front of the reverse card too), the
   sentence card lists `word — english · spanish`, and cards made before the dictionary look as they did.
4. In Anki → Tools → Manage note types → Maru Korean → Fields: `MeaningEs` is last. Sync: if Anki asks for a one-way
   sync, choose **Upload to AnkiWeb**; afterwards check a card on another device.
5. With the model stopped, open a line again: first senses are shown with the note to check them, and adding still works.
6. Rename `nikl.db` away and restart: the sheet works as in M-39 (one English meaning from the model).

**Verification:** engine tests cover every source of a meaning, the single model request and what it contains, the
fallbacks, the Spanish field on the note and the sentence card's list, and the panel add's one-sense rule; the Anki
client's tests cover adding the field to an older note type and keeping the old layout when Anki refuses; the sheet was
checked in headless Chromium through the History demo. On 2026-11-25 the real-Anki tests ran against a test profile
whose `Maru Korean` note type was the older layout: the field was added at the end, the layout stamp became v2, and a
mined card held its Spanish meaning (results log). Not checked: whether the chosen senses are right (that needs a reader
and a real model: see the blind review page of run 1), the sync question in step 4, and the owner's real collection.

## M-42 One model per task, the migration, and Settings by flow

**Needs:** the app on the owner's real `config.toml` (written before `[tasks]` existed), with its saved connections.

**Why it is unverified:** the task table, the migration and the fallbacks are tested against scripted servers, and
`maru config migrate --dry-run` was run on the real file (results log); the app itself has not been started on it, and
the Settings pages were only rendered in headless Chromium with demo data: no button was clicked by a person.

1. Before starting Maru: `maru config migrate --dry-run`. **Expect** a table of what each task will use and no change
   to the file.
2. Start Maru. **Expect** a file `config.toml.before-tasks-<date>-<time>.bak` beside `config.toml`, a `[tasks]` table
   with `version = 1` in the file, every comment and other key as it was, and the app behaving as before except that
   card meanings ask this PC's model.
3. Settings → Overview. **Expect** one card per flow (live subtitles, captures, making cards, voice) saying which
   connection and model it uses, with a state that is true: stop a server and press **Test** on its card. **Expect** a
   plain sentence ("NAS cannot be reached.", "Gemini has no quota left.", "Azure has no API key, or the key was
   refused."), never a server's raw reply.
4. Settings → Live subtitles → "Translate live lines with": choose this PC or a saved connection, Save, restart.
   **Expect** live lines to come from it (stop the other server to be sure).
5. Settings → Captures → "While live subtitles are on, use": choose a connection. **Expect** snapshots taken while
   live subtitles run to use it, and the main choice otherwise.
6. Settings → Making cards → "Choose word meanings with" = a saved connection, then stop that server and open Make
   cards on a line. **Expect** the sheet to say which model answered (this PC's, as the fallback) and why the chosen
   one did not; with this PC's model stopped as well, the dictionary's first senses with the note to check them.
7. With live subtitles running and card meanings on this PC's model: open Make cards. **Expect** no model to be loaded
   for the card (the sheet says live subtitles are running) and a button to ask when they stop.
8. Settings → Connections: add, edit, test and remove a connection; remove one a task still uses. **Expect** the
   removal to be refused with the task named. Nothing is written until Save.
9. Type in the search box ("deck", "Gemini", "voice"). **Expect** matching settings from every section.
10. Copy the `.bak` file over `config.toml` and start again. **Expect** the migration to run again with a new backup.

## M-43 Make cards: two decks, the program as a tag, the queue, and asking another model

**Needs:** Anki with AnkiConnect, a starred line, the learner's dictionary, and at least two connections.

**Why it is unverified:** the engine path ran against a real Anki in the test profile (results log) with a scripted
model; nobody has seen the sheet in the app or a resulting card in Anki's reviewer.

1. Settings → Making cards: "Deck for word cards" and "Deck for sentence cards" set to two different decks. Open Make
   cards. **Expect** both decks shown and editable for this add.
2. Add two words and the sentence. **Expect** in Anki: the words in the first deck, the sentence in the second, no
   subdeck named after the program, and the tag `maru::<program>` on each.
3. **Expect** under the decks a line per deck saying how many new cards wait there and in how many days the ones you
   add would come up at your per-day numbers (3 words, 1 sentence by default). Change the numbers in Settings and check
   the arithmetic. Anki's own per-deck limits are not read: say whether the forecast matched what Anki did.
4. Edit an English and a Spanish meaning by hand; pick another sense from a word's list. **Expect** the card to hold
   what the sheet showed.
5. **Expect** the sheet to say which model chose the meanings. Press "Retry with …" for another connection. **Expect**
   new choices from it and your hand edits kept.
6. A word with more than eight senses (하다, 보다, 치다): **expect** a second, short wait and sometimes a sense from
   beyond the first eight; judge whether it fits better.
7. From the panel, add a word that has several senses. **Expect** a Spanish meaning on the card (it was empty before).
8. With "Suggest a deck per program" on: **expect** the per-program deck offered again.

## M-44 Claude through Anthropic's own API

**Needs:** an Anthropic API key.

**Why it is only partly verified:** the client was run against the real service from the command line (results log);
the connection editor's new kind was not used in the app.

1. Settings → Connections → Add → kind **Anthropic (Claude)**. **Expect** the address `https://api.anthropic.com`
   filled in and Save refused without a key. Enter the key and a model (`claude-haiku-5-5`), Test. **Expect** "working".
2. Choose it under Making cards → "Choose word meanings with", open Make cards. **Expect** meanings, and the sheet
   naming the connection.
3. A wrong key: **expect** "… has no API key, or the key was refused." on the task's card, not the service's message.
4. `maru bench --live -n 2 --file <20 lines> --connection "<name>"`. **Expect** a table with a last column "Input read
   from cache" (0 for Maru's short prompts), and afterwards no new line in History → Live subtitles.
5. The older connection saved as "A server of yours" with Anthropic's address still works unchanged.

## M-45 YouTube captions from the browser extension

**Needs:** Chrome or Edge with the unpacked extension (`extension/youtube-captions`), Maru with `[captions] enabled`,
a translation model, and YouTube videos of four kinds: Korean captions uploaded by the author; only auto-generated
Korean; captions but no Korean; Korean burned into the picture with no caption track.

**Why it is unverified:** the receiver and the extension's logic are unit-tested, but the extension has never been
loaded in a browser and has never met YouTube. The page fields it reads are not a public API, and YouTube often
refuses caption requests that its own player did not make.

1. Settings → Live subtitles → switch on "Take YouTube captions from the browser" → Save. **Expect** a 32-character
   code in "Code for the browser extension" and a restart banner. Restart. **Expect** Status → "YouTube captions: ON ·
   port 18098 on this PC · 0 lines taken". `maru-app.log` must not contain the code.
2. Load the extension, paste the code, Test the connection. **Expect** "Maru answered: the code is right." Change one
   character of the code: **expect** "Maru refused the code." Quit Maru: **expect** "Maru is not listening on that port."
3. Play a video with author-uploaded Korean captions. **Expect** the popup to count lines, and History to list each
   line once as `live · youtube` with a translation, in the order spoken, with the text exactly as the captions show it.
   Note whether the captions had to be turned on (CC) first.
4. Wind back, jump ahead, drag the seek bar, pause, play at 2× speed. **Expect** no line twice, no line that was not
   played, none while dragging.
5. Click through to another video without reloading the page. **Expect** the new video's lines, none of the old one's.
6. A video with only auto-generated Korean: **expect** nothing sent and the popup saying so; with the switch on,
   lines arrive. A video with captions but no Korean, and one with burned-in Korean only: **expect** "No Korean
   captions on this video", nothing sent, and live subtitles (OCR) working over the same video as before.
7. A video with an advertisement first: **expect** no lines during the advertisement.
8. Star a caption line with the shortcut while the video plays, and from History. Make cards from one. **Expect** the
   star in History → Starred and the tag `maru::youtube` on the cards.
9. From any web page's console: `fetch('http://127.0.0.1:18098/captions/v1/hello', {method: 'POST', headers: {'X-Maru-Token': '<the code>'}})`.
   **Expect** it to fail (the page's origin is refused); Status counts one more refused request.
10. Another program already on the port: **expect** Status to say captions are not listening and why, and Maru
    otherwise unaffected.

**Record:** browser and version, the video kinds tried (not their addresses if private), whether the direct caption
request or the player's own was needed, and any popup message that did not match what happened.

## M-46 Two voices: reading on another server, the card voice only while cards are made

**Needs:** the local voice installed (`maru setup tts`), Python with `pip install supertonic`, Anki.

**Why it is unverified:** which voice speaks for which task is unit-tested; unloading the card voice goes through the
real `tts-server` and the app, which were not run. The Supertonic package was not installed during the run (a new
download is the owner's decision), so Maru has never spoken through it from the app.

1. `python tools/supertonic_server.py --port 18097` (the first start downloads the model). Settings → Voice: "While
   reading, use another voice server" = `http://127.0.0.1:18097`, "Speaker while reading" = `F1`; Advanced → "Model
   name sent to the reading server" = `supertonic-3`. Save, restart.
2. **Test the reading voice** and **Test the card voice**. **Expect** two different voices; the second takes a few
   seconds the first time (the model loads) and Status then shows the voice loaded.
3. Read a line aloud from the panel. **Expect** the Supertonic voice at once, and no voice model on the graphics card
   (Task Manager → GPU memory, or Status).
4. Make cards with sound on. **Expect** the Qwen voice on the cards, and a few seconds after the sheet reports the
   cards, the voice gone from the graphics card again (Status: not loaded). Make cards a second time: it loads again.
5. Stop the Supertonic server and read a line aloud. **Expect** a plain message that the voice server is not
   answering, and no fall-back to the other voice.
6. Empty the address, save, restart. **Expect** one voice for both, loaded on first use and kept, as before.
7. Listen to `%APPDATA%\maru\Maru\data\run-2\voices\index.html`: the 0.6B and 1.7B card voices and Supertonic 3
   on the same sentences. Nobody has listened to it.

## M-47 The dictionary's corrections for a card: borrowed words, long entries, corrected forms

**Needs:** the learner's dictionary, a model for card meanings, lines from a game with dialect or misread text.

**Why it is unverified:** the rules are tested and were measured on 50 of the owner's lines (results log), but whether
a meaning is *right* needs a reader. The Kiwi review page is unmarked.

1. Make cards on a line with a borrowed word whose dictionary "English" is only its sound (for example a food or a
   place name). **Expect** an English meaning taken from the definition, or the field left for you, never the Korean
   word spelled in Latin letters as if it were a translation.
2. In `config.toml` set `correct_forms = true` under `[learning.anki]` (it has no switch in Settings), restart, and put
   card meanings on Claude Sonnet. Make cards on a dialect line. **Expect** a suggested dictionary form beside the analyser's, never applied without
   you choosing it. Judge the suggestions: in the run several were not dictionary forms.
3. Mark `%APPDATA%\maru\Maru\data\run-2\kiwi-review\index.html`: is each dictionary form right for the line?

## M-51 Label the game-subtitle test set, then get final OCR numbers

**Needs:** the set run 3 left in `%APPDATA%\maru\Maru\data\evalset\raw` (544 pictures cut from recordings of Elden
Ring, Sekiro, Stellar Blade, Ori and Xenoblade), Edge or Chrome, and `maru` built (`cargo build --release -p maru-cli`).

**Why it is unverified:** only the 30 Elden Ring lines (89 pictures) carry labels a person checked. 408 pictures have a
*provisional* label (`name.provisional.txt`: a hosted reader and a local reader read the same letters) that nobody has
looked at, and 47 have none. On the 30 checked lines the rule gave no wrong label (0 of 85 pictures), which says little
about the other four games' fonts. Provisional labels also favour today's readers by construction: a picture only gets
one when PP-OCR or the hybrid already read it right, so a better candidate cannot show there.

1. Open `tools\label_review.html`, press **Open the folder…**, pick `evalset\raw` and allow editing. Pictures whose
   readers disagree come first. **Expect** the box to start with the provisional text where there is one.
2. For each picture make the box say exactly what is on screen and press **Enter**; **Alt+U** for a picture that is not
   a subtitle (Ori's map screen with "필터: 전체", an empty strip). On about ten Ori pictures the box starts with an
   English remark from the hosted reader instead of a transcription: those are the ones to mark with Alt+U.
3. `python tools\compare_ocr.py "%APPDATA%\maru\Maru\data\evalset\raw"` with no game running (about half an hour; it
   prints the same tables as the report, now on your labels only). `--tables` prints them again without re-reading.
4. **Expect** today's setup first in each table and one row per candidate. A candidate is worth switching to only if,
   in the held-out table, it is "better, unlikely to be chance" and its time and video memory are within a quarter of today's.

**Record:** how many provisional labels you had to change (the page's count line), the three tables, and the games
where a candidate wins or loses.

## M-52 Another detector, recogniser or reading model in the app

**Needs:** the candidate files run 3 downloaded under `%APPDATA%\maru\Maru\data\models\ocr-candidates\` and
`…\models\vlm\paddleocr-vl-1.6\`.

**Why it is unverified:** the `[ocr]` options `det_model`, `rec_model`, `dict_path`, `det_score_threshold`,
`det_box_threshold`, `det_unclip_ratio`, `reader_dir`, `reader_prompt`, `reader_args` and `reader_html` were exercised
only through `maru eval-ocr` and unit tests. The app has never started with any of them, so loading the reader on first
use, releasing it, and the Status notes with another reader are untested.

1. In `config.toml` under `[ocr]` add `reader_dir = "C:/Users/<you>/AppData/Roaming/maru/Maru/data/models/vlm/paddleocr-vl-1.6"`
   and restart Maru. **Expect** Status to show the reading model loading as before, and a capture to be read.
2. Point `reader_dir` at an empty folder and restart. **Expect** the note "no reading model in …; PP-OCR reads alone",
   and captures still read (by PP-OCR).
3. Add `det_model = "…/models/ocr-candidates/pp-ocrv5-mobile-det/det.onnx"`, restart, run live subtitles on a
   cutscene. **Expect** lines as before; note whether any are missed that the usual detector finds.
4. Remove the lines again. **Expect** Maru exactly as it was.

**Record:** the Status notes after each restart and anything that failed to load.

## M-53 The two test-set settings in Settings → Live subtitles

**Needs:** the app.

**Why it is unverified:** after the merge with `main` the page was only looked at in headless Edge with demo values
(the search finds "Save the subtitles it reads" and "Enlarge the subtitle area before reading it" under Live
subtitles; both are advanced settings). Nobody has clicked or saved either.

1. Settings → search "save the subtitles". Turn it on, Save, restart, run live subtitles for a minute. **Expect**
   pictures under `…\data\evalset\raw\<program>\` (M-48), and none after turning it off again.
2. Search "enlarge". **Expect** the value 1; 2 is accepted, 4 is refused with a reason.

## M-54 OCR times with no game running

**Needs:** the labelled set (M-51).

**Why it is unverified:** while run 3 measured, a game left open on the PC (`ssbu.exe`) was using a little over one
processor core and about a seventh of the graphics card. Every engine was measured under the same load, so the
comparison between them holds, but the absolute times per picture are probably a little slower than on an idle PC.

1. Close every game, then `python tools\compare_ocr.py --only "today"` (the set is now `Documents\maru-evalset\raw`).
2. **Record** the p50 and p95 columns for today's setup and the two others, next to the report's.

## M-55 Live subtitles on PP-OCR alone, captures with PaddleOCR-VL 1.6, in a real game

**Needs:** `maru setup ocr` and `maru setup ocr-vl` run again (they add the small line finder and PaddleOCR-VL 1.6, or
reuse the copies run 3 left), a game with Korean subtitles and a menu.

**Why it is unverified:** the choice was measured on pictures cut from YouTube recordings (`maru eval-ocr`), and the app
was only started on a temporary home whose live area showed no Korean (M-56). Nobody has played with it.

1. Start Maru once: `config.toml` gets `[reading]` and `config.toml.before-reading-<date>.bak` appears beside it.
   **Expect** `[reading.live] detector = "pp-ocrv6-small"`, `reader = false`; `[reading.capture] reader_model = "paddleocr-vl-1.6"`.
2. Settings → Overview → **Read the screen**: three rows (Live subtitles: small and quick, no reading model, processor;
   Captures: checked by PaddleOCR-VL 1.6, processor and graphics card; Screen scan).
3. Play a cutscene with live subtitles on. **Expect** the same or better lines than before, and Task Manager showing no
   `llama-server` holding about 2 GB while only live subtitles run.
4. Make a region capture of a menu (`Ctrl+Alt+R`). **Expect** a notice that the first capture waits for the reading model,
   then the translation; a second capture without the wait.
5. With Xenoblade (stylised lettering), try `[profile.reading.live] reader = true` for its program and compare a scene.

**Bring:** a scene's History → Live subtitles text before and after, any line read worse, the notice's wording as seen.

## M-56 The reading model takes no video memory while only live subtitles run

**Needs:** any Windows PC with an NVIDIA card; no game needed. **Run by the agent once** (see the results log), on a
temporary home whose live area showed no Korean text, so the watcher read no lines.

1. Start Maru, turn live subtitles on over a real subtitle, play a minute, and look at Task Manager → Performance → GPU and
   Details (`llama-server.exe`).
2. Make one capture. **Expect** `llama-server.exe` to appear then and not before, with about 2.2 GB of dedicated memory.

**Bring:** the dedicated GPU memory before live, during live, and after the first capture.

## M-57 Label the screen-scan set, then score captures and the scan on your labels

**Needs:** `%USERPROFILE%\Documents\maru-run-4\scanset\raw` (73 full screens of the five games; 42 with only a channel
watermark set aside in `scanset\set-aside`), Edge or Chrome.

**Why it is unverified:** 70 screens carry *provisional* labels: only the lines (189) where the outside reader and one of
today's local readers read the same letters, so they list part of each screen's text and favour today's readers.

1. Open `tools\label_review.html`, pick `scanset\raw`. For each screen write **every** Korean text on it, one per line
   (Shift+Enter), and Enter; **Alt+N** for a screen that has no game text worth reading.
2. Run the engines again on your labels (from the repository folder):
   `python tools\scan_lines.py score <set> <saved runs…>` with the runs in `maru-run-4\task3\` refreshed by
   `python %USERPROFILE%\Documents\maru-run-4\scripts\task3_runs.py` after deleting the old `.json` files (it uses
   `--labels provisional`; change it to `all` once you have labels).
3. **Expect** PaddleOCR-VL 1.6 not worse than 1.5 for captures; for the scan, whether the v6 small line finder (6x faster)
   still reads like today's.

**Bring:** the two tables.

## M-58 The label page: a Documents folder, the file name, provisional flags, "not a subtitle"

**Needs:** Edge or Chrome. Checked by the agent only in headless Edge with the page's demo data.

1. `tools\label_review.html` → **Open the folder…** → `Documents\maru-evalset\raw`. **Expect** the browser to open it
   (it refused the old folder under AppData).
2. **Expect** the header line to show `game/file.png`, and a flag on pictures with a provisional label.
3. Tick **only pictures with no label at all**: only unlabelled pictures without a provisional label remain.
4. Press **Not a subtitle** (Alt+N) on one picture: a `name.not-subtitle` file appears; the count line says "1 not a
   subtitle"; `maru eval-ocr` no longer scores it.

## M-59 Dialects for one game, and the Kiwi review page

**Needs:** a game with Gyeongsang dialect (the Sekiro lines of run 2's review are), Edge or Chrome.

**Why it is unverified:** the dialect option was measured on run 2's 50 lines; nobody judged the changed forms.

1. Open `%USERPROFILE%\Documents\maru-run-4\kiwi-review\index.html`; mark each of the 10 rows (today / with option /
   neither) and **Save marks**.
2. If the option is right more often than not, add `dialects = ["gyeongsang"]` to that game's `[[profile]]`, restart,
   and make cards from a dialect line: the word's dictionary form should follow the review.

**Bring:** `kiwi-option-marks.json`.

## M-60 The end-to-end check on your PC

**Needs:** the app built (`cargo build --release --manifest-path app\src-tauri\Cargo.toml`), Node 22 or newer.

1. Copy your `history.db` somewhere (never give the one Maru uses), then
   `node tools\e2e_app.mjs --app app\src-tauri\target\release\maru-app.exe --out C:\temp\maru-e2e --history <the copy>`.
2. **Expect** exit code 0 and `report.json` with eight sections walked, seven settings written, and Make cards opened.

**What it does not click:** connections (add, edit, test, remove), each task's Test button and the voice tests, Make
cards' Add / Retry / sounds (Anki is off there), the restart button, and every window other than the launcher and the hub
(panel, live band, scan labels, picker, wheel). Those stay for a person (M-42, M-43, M-46).

## M-61 Test a picked connection, cards from a snapshot, who answered, the grouped controls

**Needs:** Maru built from `main` with your real `config.toml`, Anki open in your normal profile, a game or video with
Korean on screen.

**Why it is unverified:** everything below was checked in Chromium with mocked commands and against a scripted model
server; nobody has clicked it in the real app, and the tray menu and the panel's "Make cards" hand-over to History are
shell code that only type-checks on Linux.

1. **Settings → Captures**, click another card under *Translate captures with* (for example NAS), and press **Test NAS**
   without saving. **Expect** an answer in a few seconds ("NAS answered in …. It is used once you save and restart.") and
   no restart. Do the same for **This PC** (it loads this PC's model unless live subtitles are on). A *Recommended* tag and
   a line of why sit under the cards; **Save and restart** appears in the bottom bar and restarts Maru with the change.
2. **Snapshot a menu** in a game. **Expect** the panel's last line to name who translated it before the text arrives
   ("Gemini · …" or "NAS · …"), and **＋ Make cards** and **✎ Fix the text** under the translation. Click a word:
   its explanation ends with "Explained by …".
3. Press **＋ Make cards**. **Expect** History to open with that line's cards sheet already open; tick a word and the
   sentence card and add them. **Check in Anki** that the sentence card is in your sentence deck with the capture's
   translation. Make a card from a word whose dictionary form is wrong: **✎**, type the right form, Enter; the card should be
   for what you typed.
4. **Fix the text** of a snapshot where two words were stuck together: add the space, apply. **Expect** the panel to split
   them anew.
5. Open the **마** floating controls: groups *Live subtitles*, *Capture*, *Saved game area*, *Models and memory*, the
   row of windows; each button shows its shortcut; the live and capture groups say who translates. **Expect** it to fit
   without scrolling on a 1080p screen. Right-click the tray icon: the same groups, shortcuts beside actions.
6. **Status → Who answers** lists the four tasks with their connection and last state.

**Bring:** a photo of anything that looks wrong, and the text of any message that does not say what to do.

## Results log

Add a line per test run. An empty log means **nothing below has been checked on real hardware**.

| Date | Test | PC / setup | Result | Notes |
|---|---|---|---|---|
| 2026-10-11 | M-3 live subtitles | Ryzen 9 7900X3D, RTX 5070 Ti, a game in an emulator, 1280×720, PP-OCR on CPU | partly | Reported by the user: 11 lines translated in order and listed in History → Live subtitles, with "to the first word" times of 1.1–5.4 s. Found by replaying the recorded clip: those times were inflated (the clock started at an earlier scene change; real reads took 0.5–1.4 s), and two short subtitles (about 0.9 s and 1.1 s on screen) were never translated. The watcher was fixed (text signal) and passes the replay; **needs a retest on the real PC**. The user also reported that the band covered the original subtitles, which they do not want; it now goes above or below the original (browser demo checked in Chromium, geometry unit-tested), **not yet seen on the real PC**. |
| 2026-11-25 | M-39/M-40 Anki client (`maru-srs --test real_anki`) | run by the agent; Anki 26.9.4, AnkiConnect API 6, profile "User 2" (empty test profile), throwaway deck | passed | 4.4 s through the new profile-and-deck guard. Word card with reverse in `<deck>::Reverse`, silent word, sentence card, duplicates refused, sound added later. First run on this profile, so it created the note types `Maru Korean` and `Maru Korean Sentence`; they remain (AnkiConnect cannot delete a note type). No deck, note or sound file left; no sync requested. Sounds were generated silence. |
| 2026-11-25 | M-9, M-24, M-39, M-40 engine path (`maru-engine --test real_anki`) | run by the agent; same Anki and profile; scripted model for word meanings; speech files made by the installed Qwen3-TTS voice with `maru tts` (0.7 s for 학교, 2.4 s for the line) | passed | 11.5 s, run twice. M-9: Check Anki, word card with fields, tags (`maru`, `topik-1`, `pos::noun`, `app::game`, `added::2026-11`), both sounds, reverse card in the Reverse subdeck, second add reports it exists; nothing listening gives "cannot reach AnkiConnect … Is Anki open" in 2 s. M-24: an add whose `changeDeck` was made to fail reported the unfinished note, the retry finished the same note; after a suspension and a deletion in Anki, Refresh progress counted 2 of 3 words. M-39: sheet ticks follow what Anki has, mined word with reverse off has one card, sentence card has one card, second time everything exists. M-40: 3 silent notes found, 2 given sound, not listed again, existing sound kept. **Not checked:** nobody listened to or looked at the cards; no AnkiWeb sync; the app's own buttons; the user's real profile and deck. No defect found in this path. |
| 2026-11-25 | M-41 note type upgrade and Spanish meanings (`maru-srs` and `maru-engine` `--test real_anki`) | run by the agent; Anki 26.9.4, AnkiConnect API 6, profile "User 2", whose `Maru Korean` note type had the older 12 fields from the run above | passed | The first add called `modelFieldAdd`: `MeaningEs` is now the 13th field and the CSS stamp is `maru-model-v2`. The client's word card held `MeaningEs`; the engine's mined card held "yo (humilde)" and its sentence card listed `저 — I (said humbly) · yo (humilde)`. Throwaway deck, notes and sounds deleted afterwards; no sync. **Not checked:** how the cards look, the one-way sync Anki may ask for after a field is added, sense choices by a real model. |
| 2026-11-25 | Learner's dictionary coverage (`maru dict coverage`) | run by the agent; an isolated copy of the data folder (copied `dict.db`, the real Kiwi analyser), the 115 different lines of a copy of the owner's `history.db`, dictionary built from the 2019-08-02 XML (48,353 words, 67,653 senses) | measured | 478 words to learn (267 different): 424 (89%) are in the dictionary (224 different, 84%); 104 (22%) have one sense and need no model (71 different, 27%); 7 of 115 lines (6%) need no model request at all. |
| 2026-11-25 | M-35 saved connections, real services (`maru connection test`: one live line and one card-meaning choice each, through Maru's client) | run by the agent; the owner's saved connections by name, RTX 5070 Ti | passed, with findings | **local** `unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL`: first word 185–190 ms, line 350 ms, card meaning 310 ms (three runs; the very first run of the session took 33 s to its first word and then lost the connection once, not seen again). **NAS** `gemma4-12b-3060ti` (Ollama): answered; 7.0 s to the first word while it loaded, card meaning 2.2 s. **Azure** `gpt-4.1-mini`: first word 1.0–1.4 s, card meaning 1.1–1.3 s. **Claude** `claude-sonnet-5-5` and `claude-haiku-5-5` (Anthropic's OpenAI-compatible endpoint): before the fix the first request of each run was refused for `temperature` and repeated; after it, first word 0.7 s with no refusal. **Gemini** `gemini-3.8-flash`: answers, but on this free-tier key between 5 s and 77 s per request with HTTP 503 "high demand" on about half of the attempts. The key lists 65 models; `gemini-4-carbon`, `gemini-4-neon`, `gemini-4-argon` and `gemini-3.1-pro-preview` refuse every request (HTTP 429, free-tier quota), `gemini-4-hydrogen` answers (32 s). Every card-meaning check chose the right sense. |
| 2026-11-26 | Live-subtitle speed by connection (`maru bench --live --connection …`, 30 lines of a copy of the owner's history × 3 passes, each with the lines before it as context, one request at a time) | run by the agent; RTX 5070 Ti PC, the owner's saved connections | measured | Median ms to the first word / whole line, failed of 90: **local** gemma-4-12B QAT Q4 237 / 370, 0; **NAS** gemma4-12b-3060ti 204 / 376, 0; **Azure** gpt-4.1-mini 854 / 944, 0; **Claude Haiku 5.5** 580 / 663, 0; **Claude Sonnet 5.5** 1336 / 1573, 0; **Gemini 3.8 Flash** (free-tier key) 11298 / 11443 on the 5 lines that were answered, 85 failed (78 × HTTP 429 quota, 5 × 503, 2 timed out). Before the Claude thinking fix the same run left 10 of 90 Haiku lines and 72 of 90 Sonnet lines empty (cut off at the length limit while thinking). Full table: docs/reports/run-1.md. Timing of the model only: no OCR, no game. |
| 2026-11-26 | Card meanings by candidate (`maru dict senses`, `maru bench --cards`, `tools/rerank_senses.py`; 50 lines of the same copy, 223 words, 171 needing a decision) | run by the agent; same PC and connections; dictionary built from the 2019-08-02 XML | recorded, not judged | Every candidate's meanings are in a blind review page for the owner; nobody has marked it. Request time per line, median / p95 ms: local 675 / 1238, NAS 838 / 1635, Azure 1058 / 1571, Claude Haiku 817 / 1343, Claude Sonnet 1350 / 2108, reranker on the CPU 2847 / 9520, dictionary only 0. No failures except Gemini on the free-tier key: 3.8 Flash failed all 48 requests (HTTP 429), 4 Hydrogen answered 9 of 48. |
| 2026-11-26 | M-8 voices compared (`maru tts-eval --voice sohee --candidate "Supertonic 3 (F1)=…"`, the 8 built-in test sentences, speech made fresh) | run by the agent; RTX 5070 Ti PC; Qwen3-TTS 1.7B CustomVoice Q8 in the installed `tts-server` (Vulkan), Supertonic 3 (`Supertone/supertonic-3`, package 1.3.1, ONNX Runtime 1.32 on the CPU) behind `tools/supertonic_server.py` | measured, not listened to | **Qwen3-TTS, voice sohee:** server ready in 10.2 s and 11.7 s (two runs); peak working set 2,447 MB, peak private memory 4,480 MB, peak dedicated GPU memory 3,996 MB; made speech at 0.10–0.26 of real time, and one take in the first run ran long and was asked for again. **Supertonic 3, voice F1:** loaded in 0.9 s (8.4 s the first time, with its download); peak working set 519 MB, peak private memory 1,246 MB, no GPU memory; 0.17–0.26 of real time. Memory was sampled once a second from Windows' process and GPU-process counters. **Nobody has listened to the page**: which voice sounds right is the owner's judgement. The default voice was not changed. |
| 2026-11-26 | Hover and screen-scan demo pages in Chromium (`index.html?replay=demo/hover.json&instant`, `scan.html?demo=menu`) | run by the agent; headless Chrome, mocked commands, a hover lookup recorded from the real engine (Kiwi, dictionary) | loaded | The panel shows the word, its romanization, pronunciation and dictionary meaning with the hover chip; the scan overlay shows its nine menu labels. No script error on either page. Screenshots in docs/reports/run-1/. This is the pages with demo data, not hover or scan on a real screen. |
| 2026-11-26 | M-43 two decks, the program tag and the queue against a real Anki (`maru-srs --test real_anki`, `maru-engine --test real_anki`) | run by the agent; Anki 26.9, AnkiConnect API 6, profile "User 2" (it held three of the owner's Maru cards and the decks `Coreano`, `Coreano::2 Media`, `Coreano::2 Media::Eden`, `Default`, which were only counted), throwaway decks under `MaruTest`; scripted model for meanings | passed | Client 4.6 s, engine 16.4 s. Words went to `MaruTest::engine::1 Palabras`, the sentence to `MaruTest::engine::2 Oraciones`, with the tag `maru::eldenring` and no deck named after the program; the queue read 2 new words and 1 new sentence, then 1 word after one was suspended. Nothing was left behind. Not seen: any card in the reviewer, the sheet in the app. |
| 2026-11-26 | M-42 migration on the real file (`maru config migrate --dry-run`) and Settings pages (`hub.html?demo=1&settings=…`) | run by the agent; the owner's `config.toml` read but not written; headless Chrome with mocked commands | dry run printed; pages loaded | The dry run mapped live subtitles and captures to the hosted connection the file already used for them (captures to a second one while live subtitles run), card meanings to this PC's model, and wrote nothing. The nine Settings pages rendered (screenshots under `docs/reports/run-2/`); no control was clicked, and the app was not started on the real file. |
| 2026-11-26 | Whole-sentence translations by candidate (`tools/build_sentence_review.py`, 50 lines of a copy of the owner's history) | run by the agent; RTX 5070 Ti PC; local, NAS, Azure, Claude Haiku, Claude Sonnet | recorded, not judged | All five answered 50 of 50. Median ms for a whole line: local 233, NAS 324, Claude Haiku 631, Claude Sonnet 812, Azure 1,039. 193 different translations are in a blind review page for the owner; nobody has marked it. One Haiku line and one Sonnet line are flagged as cut off. Gemini was left out: its key answered HTTP 429 to the first request and was not asked again. |
| 2026-11-26 | M-47 what the dictionary gives a card (`maru dict romanized`, `maru bench --cards [--forms]`, `tools/nikl_stats.py`) | run by the agent; a copy of the dictionary (48,353 entries, 67,653 senses); the same 50 lines (223 words) | measured, not judged | Romanized "English": 3,280 senses in 2,448 entries; 521 replaced by a gloss from the definition, 406 kept as borrowed words, 44 kept other words, 2,309 left without English. More than eight senses: 257 of 44,436 word groups (치다 has 49); on the 50 lines the local model's wrong share in run 1's marks was 27.6% on those words against 12.3% on words with 2 to 8 senses, and the second request changed 6 words. Corrected forms: local and NAS models offered none; Claude Haiku 7; Claude Sonnet 25 (11 on dialect lines, 14 on misread lines, none on standard text), several of them not dictionary forms. Whether any changed answer is right was not judged. |
| 2026-11-26 | M-46 card voice candidates (`maru tts-eval`, 50 lines, speech made fresh) | run by the agent; RTX 5070 Ti PC; the installed `tts-server` (Vulkan); Qwen3-TTS 1.7B CustomVoice Q8 and 0.6B CustomVoice Q8 (968,588,544 bytes, Apache-2.0) | measured, not listened to | 1.7B: ready in 1.7 s, about 3.9 GB of GPU memory, 11.0 s to make 68.2 s of speech, 3 of 50 lines needed another take. 0.6B: ready in 1.1 to 1.7 s, 2.9 to 3.2 GB, 16.7 s to make 99.4 s of speech, 10 of 50 needed another take and 2 stayed too long; its clips are about 1.5 times longer with more silence. The smaller model was not faster and saved about 1 GB. Supertonic 3 was not measured again (its package is not installed). Nobody listened. |
| 2026-11-26 | M-44 Claude through Anthropic's own API (`maru connection test "Claude+native=…"`, `maru bench --live -n 2`, 20 lines × 2 passes, one request at a time) | run by the agent; the owner's saved Claude connection asked through `/v1/messages` and through the compatible endpoint | passed; measured | Both answered 40 of 40. Median ms to the first word / whole line: Haiku 506 / 615 native against 544 / 644 compatible; Sonnet 790 / 1,249 against 762 / 1,296. Input read from the prompt cache: 0 in every run (Maru's system prompts are shorter than the service caches). The connection editor's new kind was not used in the app. |
| 2026-11-26 | M-45 caption receiver on its own (`captions_listen` example and `curl` on a spare port) | run by the agent; no browser, no YouTube | passed for the receiver only | A Korean line was accepted and printed, the same cue again was answered `duplicate`, a wrong code got 401, a request with a web page's `Origin` got 403. The extension was never loaded in a browser. |
| 2026-11-26 | M-51 a test set from recordings and the OCR candidates on it (`tools/find_subtitles.py`, `sample_subtitle_frames.py`, `maru eval-ocr`, `tools/provisional_labels.py`, `tools/compare_ocr.py`) | run by the agent; RTX 5070 Ti PC, PP-OCR on the CPU, a measuring home with default settings; a game left open (`ssbu.exe`) used about one core throughout; 544 pictures of five games: 89 with checked labels (30 Elden Ring lines), 408 with provisional labels nobody has looked at, 47 unlabelled | measured; provisional except the 30 lines | Nothing was adopted. On the 30 checked lines no candidate differs from today's hybrid beyond chance (hybrid 0.6% letter error, PP-OCR alone 0.3%). On provisional labels, held-out half (212 subtitles): PP-OCR alone better on 26 and worse on 13 than the hybrid (p = 0.053) at 98 ms against 166; the hybrid with PaddleOCR-VL-1.6 better on 17, worse on 6 (p = 0.035); PP-OCRv6 and v5 mobile detectors read like today's within chance and take 38 to 64 ms; GLM-OCR, Nanonets-OCR2 and Surya 2.5 as readers are worse. The agreement rule gave no wrong label on the checked lines (0 of 85 pictures). See `docs/reports/run-3.md`. Not checked: the provisional labels, times on an idle PC (M-54), any of this in the app (M-52) |
| 2026-11-26 | M-53 the two test-set settings in Settings (`hub.html?demo=1&settings=live&q=…`) | run by the agent; headless Edge, demo values, mocked commands | loads | The search finds "Save the subtitles it reads" (a switch, off) and "Enlarge the subtitle area before reading it" (a number, 1) under Live subtitles. Nobody clicked or saved anything |
| 2026-11-27 | Task 0 of run 4: the owner's saved runs printed again (`maru eval-report` on a copy of `Documents\maru-evalset\reports`) | run by the agent | reproduced | Letter error, median time and better/worse counts match the owner's figures for all six setups; "exact lines" differs by 0.4–0.6 points because the owner counted a subtitle exact only when every picture of it was (that count gives his 83.5 / 88.5 / 88.7 / 92.5 / 92.7 / 92.7 exactly). The held-out half agrees. |
| 2026-11-27 | Run 4 subtitle measurements (`maru eval-ocr` on a copy of the labelled set, 537 pictures, 468 subtitles; dev to choose, held-out to judge) | run by the agent; RTX 5070 Ti PC, PP-OCR on the CPU; Brave, Notion and Anki open, no game | measured (148 person labels, 389 provisional) | v6 small + Qota-OCR against v6 small: held-out 15 better / 3 worse (p = 0.008), dev 7 / 6; stays off (community model). Stray lines dropped, v6 small: held-out 5 / 0 (p = 0.062), all 11 / 0; standard detector: held-out 10 / 0 (p = 0.002); not adopted for live (the default detector misses the bar). Keep sure lines 0.90 (chosen on dev among 0.80–0.98) with reader 1.6: held-out 16 / 4 (p = 0.012), same time; adopted. Tables: docs/reports/run-4.md. |
| 2026-11-27 | Run 4 scan set (M-57): 115 full frames of the five games, 73 kept with Korean text, 70 with provisional labels (189 lines) | run by the agent; the outside reader was Claude Sonnet 5.5 through the owner's saved connection (pictures left this PC; USD 0.54); Steam open (no game) | measured, provisional | Captures: PaddleOCR-VL 1.6 against 1.5, 15 lines better / 12 worse (p = 0.70): not worse, adopted. Keep sure lines with 1.6 against 1.6 alone: 9 / 2. Scan: v6 small 1 better / 3 worse (p = 0.63) in 266 ms against 1,606 ms per screen, v6 medium the same counts in 681 ms; detector unchanged (provisional labels). |
| 2026-11-27 | M-56 video memory with live subtitles, the real app | run by the agent; `maru-app` (built at 6a5c5ec) on a temporary home linked to the real models, translation server a dead address; the live area showed no Korean, so the watcher read no lines | measured | Card total 2,467 MiB before the app, 2,482 MiB after 30 s of live subtitles: no `llama-server`, only Maru's WebView windows (about 40–60 MB). The first capture (hover mode) started `llama-server` within about 4 s: 2,254 MB dedicated, card total 4,775 MiB. Quit stopped both. The app migrated the temporary `config.toml` (`[tasks]`, `[reading]`, two backups). Not checked: live subtitles reading real Korean lines, a game running. |
| 2026-11-27 | M-60 end-to-end check (`node tools/e2e_app.mjs`) | run by the agent; release app at 904ffca, temporary home without OCR models (Windows OCR answered), Anki off, a copy of the owner's `history.db` (116 live lines, none starred) | passed | All 8 Settings sections walked; 7 settings changed by clicks in the real page and saved, each found in `config.toml`; Make cards opened on a live line (5 words; plain message that the server cannot be reached and the meanings are the dictionary's). A second launch with the models linked showed the Read the screen step with one row per task (screenshot in docs/reports/run-4/). Not clicked: see M-60. |
| 2026-11-27 | M-58 label page (`tools/label_review.html?demo`) | run by the agent; headless Edge, the page's own demo items | loaded | File name in the header, "only pictures with no label at all", "Not a subtitle" and its count are shown. Not checked: opening `Documents\maru-evalset\raw` with the folder picker, writing `name.not-subtitle`. |
| 2026-11-27 | Kiwi options on run 2's 50 review lines (`examples/kiwi_options`, `tools/build_kiwi_review.py changes`) | run by the agent; Kiwi 0.23.1 (the installed one) | measured, not judged | Today's analysis equals run 2's file in all 239 words. Gyeongsang allowed: 10 dictionary forms change (0 standard text, 4 dialect, 6 broken OCR); 주꾸마 → 주다 in all three lines, 잡그래이 unchanged. Typo correction crashed Kiwi on the first line (both kiwi-rs versions); Kiwi 0.23.2, 0.24.0, 0.24.1 and 0.25.0 crash Maru's plain self-test. |
