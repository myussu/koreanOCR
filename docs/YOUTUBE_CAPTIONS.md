# YouTube captions as text

When a YouTube video has a Korean caption track that its author uploaded, Maru can take the lines as text from the
browser instead of reading them from the screen with OCR. This note is the design: the flow, the protocol, the
security choices, exactly what is relied on in YouTube's page, what has been verified and what has not, and how to try it.

**Status: partial, and not verified against YouTube.** The receiver in Maru, its rules, the settings and the
extension's decisions are tested. The extension has never been loaded into a browser and has never met the real
site. See [What is verified](#what-is-verified-and-what-is-not).

## What you get, and what you do not

* Caption lines arrive as exact text, are translated like live subtitle lines and listed in **History** (program
  `youtube`). Starring (History, or the star shortcut) and **Make cards** work as for any live line; cards get the tag
  `maru::youtube`. A `[[profile]]` with `exe = ["youtube"]` (hint, glossary, register, Anki tags) applies to them.
* **No translation is drawn on the video.** The live band belongs to the screen area live subtitles watch; a caption
  line has no place on screen. The translation is in History only. Returning it to the extension so it can be shown
  under the video is the natural next step and is not built.
* Only the Korean track **uploaded by the author**. Auto-translated tracks are never used. Auto-generated Korean
  (YouTube's speech recognition, `kind: "asr"`) is used only with the switch in the extension's popup, which is off.
* When there is no usable track, or the text is burned into the picture, nothing is sent and the popup says
  "No Korean captions on this video." Live subtitles (OCR) work exactly as before; nothing in that path was changed.
* If live subtitles are also running over the same video, both list the line: once read from the screen (program
  `chrome.exe` or `msedge.exe`) and once as a caption (`youtube`). Turn live subtitles off for videos with captions.

## Flow

```
YouTube watch page
  page-bridge.js   (page's own JS world)  reads the player's list of caption tracks
  content.js       (extension's world)    picks the track, fetches its cues, watches the video's time
        │ chrome.runtime.sendMessage({type: "line", …})
  background.js    (service worker)       POST http://127.0.0.1:18098/captions/v1/line   X-Maru-Token: <code>
        ▼
Maru  maru-engine/src/captions.rs         loopback listener: host, path, origin, method, code, size → rules → CaptionLine
      app/src-tauri/src/captions.rs       starts it at launch, forwards each line
      Engine::translate_caption           = Engine::translate_live with source `caption`, program "youtube"
      pipeline::live                      translation, live cache, history, history.db
```

* `extension/youtube-captions/captions-core.js` holds every decision of the extension as pure functions
  (`pickTrack`, `trackUrl`, `playerUrlFor`, `parseJson3`, `activeCues`, `createSender`, `lineFor`, `resultOf`,
  `describe`), tested by `test/captions-core.test.mjs`. The other files only talk to the page and the browser.
* A line is sent once it has been up for 250 ms. Each line of a video is sent at most once: winding back does not
  send it again; a line skipped by jumping ahead is sent when it is really played; nothing is sent while the seek bar
  is dragged; a line shorter than the wait is still sent when the video plays past it.
* In the engine a caption goes through the same `pipeline::live` as a subtitle read from the screen, with two
  differences keyed on `CaptureSource::Caption`, because the text is exact: stray marks are not trimmed from its ends
  (`BTS 정국이 왔어요` keeps `BTS`), and it is not merged with a recent similar line (`이것은 물이에요` then
  `이것은 불이에요` are two lines, where two OCR readings one letter apart are one line read twice). Repeats are
  dropped by the receiver instead (below). The model is not told the text may be misread.

## Protocol

One line per request, to this PC only. Version 1 is in the path.

```
POST /captions/v1/line HTTP/1.1
Host: 127.0.0.1:18098
Origin: chrome-extension://abcdefghijklmnopabcdefghijklmnop
Content-Type: application/json
X-Maru-Token: 5f0c1d7a9b3e4c2188a6d0f4e7b2c391
Content-Length: 132

{"text":"안녕하세요, 여러분.","video":"dQw4w9WgXcQ","title":"한국어 수업 1","start_ms":12340,"kind":"manual","lang":"ko"}
```

| Field | Meaning |
|---|---|
| `text` | the line (required). Rows are joined, spaces collapsed, invisible characters removed |
| `video` | the video's id; only used to tell a repeated cue from the same words in another video |
| `title` | the video's title (kept as the capture's window title, at most 200 characters) |
| `start_ms` | where the cue starts in the video |
| `kind` | `"asr"` for the auto-generated track; anything else is an uploaded track |
| `lang` | the track's language code; a line whose track is not `ko` / `ko-…` is dropped |
| `translated` | `true` is always dropped: Maru never takes an auto-translated line |

Unknown fields are ignored, so fields can be added later.

| Answer | When |
|---|---|
| `200 {"ok":true,"accepted":true}` | taken |
| `200 {"ok":true,"accepted":false,"reason":"…"}` | well-formed but dropped: `empty`, `not_korean` (no Hangul syllable, or another language's track), `translated`, `duplicate` |
| `400` | not a request Maru understands, a body that is not JSON, a header given twice |
| `401` | the code is missing or wrong |
| `403` | an `Origin` that is not a browser extension's (or not the one named in the settings), or a `Host` that is not this PC |
| `404` / `405` | another address / another method (`Allow: POST, OPTIONS`) |
| `411` / `413` / `431` | no `Content-Length` (chunked bodies are not accepted) / a body over 16 KiB / headers over 8 KiB |
| `503` | 64 lines are already waiting for the front end |

`POST /captions/v1/hello` with the code answers `{"ok":true,"app":"maru","protocol":1}`; the popup's
"Test the connection" uses it. `OPTIONS` on either address answers a browser's preflight for an extension origin
(and `Access-Control-Allow-Private-Network: true` when asked).

**Duplicates.** With a start time: the same cue (video + start + text) among the last 200 taken is dropped, which
covers a reloaded page and a video wound back. Without one: the same text from the same video within 5 s of the
last line. The same words at another moment of a video are another line and are taken.

Try it by hand (the code and port are the ones in your settings). Put the body in a UTF-8 file: Korean typed on a
Windows command line reaches `curl` in another encoding and is then dropped as `not_korean`.

```
line.json:   {"text":"저는 학교에서 공부했어요","video":"dQw4w9WgXcQ","start_ms":4000,"lang":"ko"}

curl -s -X POST http://127.0.0.1:18098/captions/v1/line -H "X-Maru-Token: <code>" -H "Content-Type: application/json" --data-binary @line.json
```

## Security choices

* **Off by default.** Nothing listens until `[captions] enabled = true` and Maru is restarted.
* **This PC only.** The socket is bound to `127.0.0.1` and nothing else; the peer address is checked again per
  connection. A request whose `Host` is not `127.0.0.1` or `localhost` is refused (a web page that points its own
  name at 127.0.0.1 still names itself there).
* **A code on every request**, compared without stopping at the first wrong character (`same_token`). Maru makes
  it when the feature is first switched on (saved with the switch in Settings, or written at launch when the file was
  edited by hand). It is never logged: the config type prints as `[redacted]`, the listener logs neither headers
  nor the code, and answers never repeat it. It is stored in plain text in `config.toml` (like the other keys there)
  and in the extension's `chrome.storage.local` (never synced).
* **No web page can use it.** A browser always sends `Origin` on a request like this; only `chrome-extension://…`
  or `moz-extension://…` is let through, before the code is even looked at, and a refused origin gets no CORS
  headers. `[captions] allowed_origin` narrows this to one extension (the popup shows its address). A request
  without `Origin` is a local program that is not a browser; it still needs the code. The origin rule adds nothing
  against such a program (it can send any header) and is not meant to.
* **Small and bounded.** 8 KiB of headers, 16 KiB of body (refused on the declared length before any of it is read),
  5 s per request, 8 connections at once, 64 queued lines, one request per connection. The body is only read after
  host, path, origin, method, code and length have passed.
* **The code's randomness.** The workspace has no random-number crate as a direct dependency and none was added.
  `new_token` hashes with two fresh `std::collections::hash_map::RandomState`s, whose keys the standard library
  draws from the operating system's random source: 128 secret bits, 32 hexadecimal characters. That is adequate for
  a code guarding a loopback port; it is not a general-purpose key generator, and if Maru ever gains a random-number
  dependency this function should use it.
* **The extension** asks for `storage` and for the host `http://127.0.0.1/*`; its content scripts run only on
  `https://www.youtube.com/*`. No remote code, no build step, no framework. It fetches only
  `https://www.youtube.com/api/timedtext` (checked in `trackUrl`) and posts only to `http://127.0.0.1:<port>`
  (`endpoint`). A request body is rebuilt field by field in the background worker (`lineFor`).
* **What a hostile YouTube page could do:** feed the content script false tracks, so false Korean text reaches
  History. It cannot make the extension fetch another site or post anywhere but this PC.

## What is relied on in YouTube's page

None of this is a public API. It can change without notice, and when it does the extension must send nothing.

| Relied on | Where | If it is missing or different |
|---|---|---|
| `document.getElementById('movie_player').getPlayerResponse()` | `page-bridge.js` | asked 20 times, 0.5 s apart; then the popup says the caption list could not be read; nothing is sent |
| `.videoDetails.videoId`, `.videoDetails.title` | `page-bridge.js` | the id must equal the `v` of the address, else the answer is taken for the previous video and asked again; a missing title is left out |
| `.captions.playerCaptionsTracklistRenderer.captionTracks[]` with `baseUrl`, `languageCode`, `kind` (`"asr"` = auto-generated) | `page-bridge.js`, `pickTrack` | no list = no captions: "No Korean captions on this video" |
| `baseUrl` is `https://www.youtube.com/api/timedtext?…`; adding `fmt=json3` gives JSON; `tlang` asks for an automatic translation | `trackUrl` | another address is not fetched; an address with `tlang` is refused |
| `json3`: `{ events: [{ tStartMs, dDurationMs, segs: [{ utf8 }] }] }` | `parseJson3` | anything else gives no cues: nothing is sent |
| The player's own caption request appears in the page's resource timings with `v`, `lang`, `kind` and a token (`pot`) | `content.js`, `playerUrlFor` | only the direct request is tried |
| `#movie_player video` (`currentTime`, `seeking`, `paused`, `playbackRate`) | `content.js` | no video element: nothing is sent |
| `#movie_player.ad-showing` while an advertisement plays | `content.js` | lines could be sent at the wrong moments during an advertisement |
| `yt-navigate-finish` on `document` when the video changes without a page load | `content.js` | the address is also compared every tick and every 2 s |
| a watch page is `https://www.youtube.com/watch?v=<id>` | `videoIdOf` | Shorts, embeds, `m.youtube.com` and `music.youtube.com` are not handled |

**The known obstacle.** Since 2025 YouTube often answers a caption request with an empty body (status 200) unless
the address carries a proof-of-origin token (`pot`) that only its player can make. This is reported by other
projects, not documented by YouTube. The extension therefore tries the listed address first and, when that comes
back empty, waits for the player's own request for the same track, which carries the token. The player only makes
that request when captions are turned on, so the popup then says: "Turn the video's captions (CC) on in Korean once
and they will be read." Whether either route works today is exactly what has not been verified.

## What is verified and what is not

| | State |
|---|---|
| Receiver: socket bound to 127.0.0.1, host, origin, preflight, code, sizes, methods, duplicates, empty and non-Korean lines, queue limit, stop | tested against a real local socket (`maru-engine/src/captions.rs`, 13 tests) |
| A caption line through the engine: translated, in the history as `youtube`, saved, starred, tagged `maru::youtube`, exact text kept, not merged | tested against the scripted model server (`tests/pipeline.rs`, 3 tests) |
| Config: old files parse, the example parses, the code is not printed, validation | tested (`maru-core/src/captions.rs`) |
| Making and saving the code (with the switch, or at launch for a hand-edited file) | tested (`app/src-tauri/src/captions.rs`) |
| The standalone listener with a real HTTP client | run once by the agent (2026-11-26, Windows 11): the `captions_listen` example and `curl`. `hello` answered; a Korean line from a UTF-8 file was taken and printed; the same cue again was `duplicate`; a wrong code got 401; `Origin: https://www.youtube.com` got 403. No browser was involved |
| The shell starting the listener in the running app and lines reaching History | **not verified**: it compiles and passes clippy; the app was not run |
| Settings fields and the Status row | looked at once in the hub's browser demo (headless Chrome, mocked commands, 2026-11-26): the switch and the code field show under Live subtitles and the Status row reads `ON · port 18098 on this PC · 12 lines taken · 3 repeats or empty lines dropped`. **Not verified** in the real window, and saving the switch was not exercised there |
| The extension's decisions (track choice, parsing, timing, seeking, messages) | tested with `node --test` (23 tests) against data shaped as described above, written from memory of the format, not captured from the site |
| The extension loaded in Chrome or Edge | **not verified**: never loaded; `manifest.json` only parses as JSON |
| Against YouTube: the track list, the caption request, the token fallback, navigation between videos, advertisements | **not verified against YouTube** |
| That the browser sends `Origin: chrome-extension://…` from the service worker and allows the request to 127.0.0.1 | **not verified**; it is what the design assumes |
| Firefox | not supported (the manifest uses a service worker), not tried |
| How good the lines are for study (line length, auto-generated captions) | not looked at |

## Install the extension (unpacked)

1. In Maru: Settings → Live subtitles → switch on **Take YouTube captions from the browser** → Save. The field
   **Code for the browser extension** appears with a code. Restart Maru (the banner offers it).
2. In Chrome or Edge open `chrome://extensions` (or `edge://extensions`), switch on **Developer mode**, press
   **Load unpacked** and choose the folder `extension/youtube-captions` of this repository.
3. Press the extension's button, paste the code, leave the port at 18098 unless you changed it in Maru, press
   **Save**, then **Test the connection**. It should say "Maru answered: the code is right."

Loading it unpacked from a checkout means the browser runs whatever is in that folder: after pulling changes, press
the reload arrow on the extension's card.

## Try it, step by step

1. Maru running, captions switched on and restarted. The Status tab shows `YouTube captions: ON · port 18098 on this PC · 0 lines taken`.
2. Open a YouTube video with Korean captions uploaded by its author (the captions menu lists "Korean", not
   "Korean (auto-generated)"). Open the extension's popup.
3. Expected: "Sending this video's Korean captions (uploaded by the author) to Maru: N lines so far", N growing as
   the video plays. If it says YouTube did not hand them over, turn the video's captions (CC) on in Korean.
4. Maru → History: the lines appear as `live · youtube`, each with its translation. Star one; open Make cards on one
   and check the tag `maru::youtube`.
5. Wind the video back 30 s: no line is listed twice. Jump ahead: the skipped lines are not listed.
6. Open a video without Korean captions: the popup says "No Korean captions on this video." Nothing new in History.
7. Open a video with only auto-generated Korean: the popup says so and nothing is sent; switch on "Use
   auto-generated Korean captions", and lines arrive.
8. To watch the lines without Maru: `cargo run -p maru-engine --example captions_listen -- <a code of 16+ characters>`
   prints each line it takes (Maru must not be running on the same port).

## Manual test entry

For `docs/MANUAL_TESTS.md`, as the next free number.

### M-45 YouTube captions from the browser extension

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
