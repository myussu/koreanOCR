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
**Why unverified:** the client was only run against a fake AnkiConnect; the real one rejects calls in ways the fake does not.

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
checks cover Refresh progress and the editor widgets. The real Anki collection, AnkiWeb sync, media playback and physical
keyboard behavior remain unverified; the agent did not open, alter or sync the user's collection.

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

## Results log

Add a line per test run. An empty log means **nothing below has been checked on real hardware**.

| Date | Test | PC / setup | Result | Notes |
|---|---|---|---|---|
| 2026-10-11 | M-3 live subtitles | Ryzen 9 7900X3D, RTX 5070 Ti, a game in an emulator, 1280×720, PP-OCR on CPU | partly | Reported by the user: 11 lines translated in order and listed in History → Live subtitles, with "to the first word" times of 1.1–5.4 s. Found by replaying the recorded clip: those times were inflated (the clock started at an earlier scene change; real reads took 0.5–1.4 s), and two short subtitles (about 0.9 s and 1.1 s on screen) were never translated. The watcher was fixed (text signal) and passes the replay; **needs a retest on the real PC**. The user also reported that the band covered the original subtitles, which they do not want; it now goes above or below the original (browser demo checked in Chromium, geometry unit-tested), **not yet seen on the real PC**. |
