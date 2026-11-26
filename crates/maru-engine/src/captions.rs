//! Captions sent by the browser extension: a video's own Korean caption track arrives as text, so it does not have
//! to be read from the screen. This module is the receiver and its rules; `docs/YOUTUBE_CAPTIONS.md` is the design.
//!
//! The extension posts one line at a time, as it comes up in the video, to a port on this PC:
//!
//! ```text
//! POST /captions/v1/line          X-Maru-Token: <the code from the settings>
//! {"text": "안녕하세요, 여러분.", "video": "dQw4w9WgXcQ", "title": "…", "start_ms": 12340, "kind": "manual", "lang": "ko"}
//! → 200 {"ok":true,"accepted":true}      or      {"ok":true,"accepted":false,"reason":"duplicate"}
//! ```
//!
//! What is refused, and in which order: a request named for another host, another path, an `Origin` that is not a
//! browser extension's (so no web page can talk to it), another method, a missing or wrong code, a body without a
//! length or over [`MAX_BODY`]. Only then is the body read. The listener binds 127.0.0.1 and nothing else, is off
//! unless `[captions] enabled`, and never writes the code or a request's headers to the log.
//!
//! It is a deliberately small HTTP/1.1 handler (one request per connection, `Content-Length` bodies only) rather
//! than a web framework: two routes do not justify a new dependency.

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use maru_core::captions::{CaptionsConfig, is_extension_origin};
use maru_core::{AppInfo, Capture, CaptureSource};
use maru_lang::hangul::is_syllable;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use crate::continuous::LiveLine;
use crate::engine::{Engine, EventRx};

/// The program caption lines belong to: History shows it, cards get the tag `maru::youtube`, and a `[[profile]]`
/// with `exe = ["youtube"]` applies to them.
pub const PROGRAM: &str = "youtube";
pub const LINE_ROUTE: &str = "/captions/v1/line";
/// Answers "is Maru listening, and is my code right?" for the extension's popup; it takes no line.
pub const HELLO_ROUTE: &str = "/captions/v1/hello";
pub const TOKEN_HEADER: &str = "x-maru-token";
/// The largest body accepted. A caption line is a few hundred bytes; this leaves room for a long title.
pub const MAX_BODY: usize = 16 * 1024;
/// The request line and headers together.
const MAX_HEAD: usize = 8 * 1024;
/// A request must arrive whole within this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Connections served at once; the extension uses one at a time.
const MAX_CONNECTIONS: usize = 8;
/// Lines waiting for the front end. A full queue answers "busy" instead of growing.
const QUEUE: usize = 64;
/// Without a start time, the same text from the same video this soon after the last line is that line again.
const SAME_TEXT_WINDOW: Duration = Duration::from_secs(5);
/// How many cues are remembered to recognise one sent again (a reloaded page plays the video from where it was).
const REMEMBERED_CUES: usize = 200;

/// One caption line that passed the rules.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptionLine {
    /// The line on one row, without invisible characters.
    pub text: String,
    /// The video's id, when the extension sent a plausible one.
    pub video: String,
    pub title: String,
    /// Where the cue starts in the video.
    pub start_ms: Option<u64>,
    /// From the auto-generated track (the extension only sends those with its switch on).
    pub auto_generated: bool,
    pub received: Instant,
}

impl CaptionLine {
    /// The line as the live lane takes it: exact text, marked as a caption, from the program [`PROGRAM`].
    pub fn into_live_line(self) -> LiveLine {
        LiveLine {
            capture: Capture {
                text: self.text,
                source: CaptureSource::Caption,
                rect: None,
                words: Vec::new(),
                cursor: None,
                app: Some(AppInfo { exe: PROGRAM.into(), title: self.title }),
                confidence: None,
            },
            appeared: self.received,
            ready_ms: 0,
            ocr_ms: 0,
        }
    }
}

impl Engine {
    /// Translate a caption line and list it in the history, exactly as a live subtitle line is.
    pub fn translate_caption(&self, line: CaptionLine) -> (u64, EventRx) {
        self.translate_live(line.into_live_line())
    }
}

/// Why a well-formed line was not taken. The extension is told, so its popup can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    Empty,
    NotKorean,
    /// An auto-translated track: never accepted, whatever the extension's switches say.
    Translated,
    Duplicate,
}

impl Dropped {
    pub fn reason(self) -> &'static str {
        match self {
            Dropped::Empty => "empty",
            Dropped::NotKorean => "not_korean",
            Dropped::Translated => "translated",
            Dropped::Duplicate => "duplicate",
        }
    }
}

/// The body of a line request. Fields this version does not know are ignored, so the extension can add some.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Wire {
    pub text: String,
    pub video: Option<String>,
    pub title: Option<String>,
    pub start_ms: Option<u64>,
    /// `"asr"` for the auto-generated track; anything else is a track someone uploaded.
    pub kind: Option<String>,
    /// The track's language code, when the extension knows it.
    pub lang: Option<String>,
    pub translated: Option<bool>,
}

/// What the receiver remembers of the lines it took, to drop one that is sent again.
#[derive(Default)]
pub struct Seen {
    cues: VecDeque<(String, u64, String)>,
    last: Option<(String, String, Instant)>,
}

/// The line on one row: caption rows are joined, runs of spaces collapsed, invisible characters removed.
fn clean(text: &str) -> String {
    let visible: String = text.chars().filter(|c| !matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}')).collect();
    visible.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Apply the rules to one line: what a valid line is, and whether it was already taken.
pub fn accept(wire: Wire, seen: &mut Seen, now: Instant) -> Result<CaptionLine, Dropped> {
    if wire.translated == Some(true) {
        return Err(Dropped::Translated);
    }
    if wire.lang.as_deref().is_some_and(|lang| !(lang.eq_ignore_ascii_case("ko") || lang.to_ascii_lowercase().starts_with("ko-"))) {
        return Err(Dropped::NotKorean);
    }
    let text = clean(&wire.text);
    if text.is_empty() {
        return Err(Dropped::Empty);
    }
    if !text.chars().any(is_syllable) {
        return Err(Dropped::NotKorean);
    }
    // A video id is 11 letters, digits, `-` or `_`; anything else is not kept (it only groups duplicates).
    let video = wire.video.filter(|v| v.len() <= 32 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).unwrap_or_default();
    match wire.start_ms {
        // The same cue of the same video: sent again after the page was reloaded or the video was wound back.
        Some(start) => {
            if seen.cues.iter().any(|(v, s, t)| *v == video && *s == start && *t == text) {
                return Err(Dropped::Duplicate);
            }
            seen.cues.push_back((video.clone(), start, text.clone()));
            if seen.cues.len() > REMEMBERED_CUES {
                seen.cues.pop_front();
            }
        }
        // No start time to tell two cues apart: only an immediate repeat is taken for the same line.
        None => {
            if seen.last.as_ref().is_some_and(|(v, t, at)| *v == video && *t == text && now.duration_since(*at) <= SAME_TEXT_WINDOW) {
                return Err(Dropped::Duplicate);
            }
        }
    }
    seen.last = Some((video.clone(), text.clone(), now));
    let title: String = clean(wire.title.as_deref().unwrap_or_default()).chars().take(200).collect();
    Ok(CaptionLine { text, video, title, start_ms: wire.start_ms, auto_generated: wire.kind.as_deref() == Some("asr"), received: now })
}

/// A new code for the extension: 32 hexadecimal characters carrying 128 random bits.
///
/// The workspace depends on no random-number crate, and one must not be added for this. The standard library
/// seeds every `RandomState` from the operating system's random source (two secret 64-bit keys), so hashing with
/// fresh ones yields values nobody can predict without those keys. That is enough for a code that only guards a
/// port on this PC; it is not a general-purpose key generator.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    (0..2u64)
        .map(|round| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(round);
            hasher.write_u128(nanos);
            hasher.write_u32(std::process::id());
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

/// Compare the code a request carries with the right one without stopping at the first wrong character, so the
/// time taken says nothing about how much of a guess was right. An empty code never matches.
pub fn same_token(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    let mut differs = given.len() ^ expected.len();
    for (i, byte) in expected.iter().enumerate() {
        differs |= usize::from(given.get(i).copied().unwrap_or(0) ^ byte);
    }
    differs == 0 && !expected.is_empty()
}

/// How many requests were taken, dropped by the rules, or refused, since the listener started.
#[derive(Debug, Default)]
pub struct Counters {
    accepted: AtomicU64,
    dropped: AtomicU64,
    refused: AtomicU64,
    wrong_code_logged: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Counts {
    pub accepted: u64,
    pub dropped: u64,
    pub refused: u64,
}

impl Counters {
    pub fn counts(&self) -> Counts {
        Counts { accepted: self.accepted.load(Ordering::Relaxed), dropped: self.dropped.load(Ordering::Relaxed), refused: self.refused.load(Ordering::Relaxed) }
    }
}

struct Shared {
    token: String,
    /// Empty: any browser extension. Otherwise the one origin that may send.
    allowed_origin: String,
    lines: mpsc::Sender<CaptionLine>,
    seen: std::sync::Mutex<Seen>,
    counters: Arc<Counters>,
}

/// The running listener. Dropping it (or [`CaptionServer::stop`]) closes the port.
pub struct CaptionServer {
    rx: mpsc::Receiver<CaptionLine>,
    addr: SocketAddr,
    stop: CancellationToken,
    counters: Arc<Counters>,
}

impl CaptionServer {
    /// Where it listens: always 127.0.0.1.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    pub fn counters(&self) -> Arc<Counters> {
        self.counters.clone()
    }
    /// The next line that passed the rules; `None` once the listener has stopped.
    pub async fn next(&mut self) -> Option<CaptionLine> {
        self.rx.recv().await
    }
    pub fn stop(&self) {
        self.stop.cancel();
    }
}

impl Drop for CaptionServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// Start listening on 127.0.0.1 as `[captions]` says. Must be called from within a Tokio runtime.
/// Refuses to start switched off or without a code, and says in plain words why the port cannot be had.
pub async fn listen(cfg: &CaptionsConfig) -> Result<CaptionServer, String> {
    if !cfg.enabled {
        return Err("YouTube captions are switched off.".into());
    }
    let token = cfg.token().ok_or("YouTube captions have no code for the extension yet.")?.to_string();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, cfg.port))
        .await
        .map_err(|e| format!("YouTube captions cannot listen on port {} of this PC ({e}). Another program may be using it: choose another port in Settings.", cfg.port))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let (lines, rx) = mpsc::channel(QUEUE);
    let counters = Arc::new(Counters::default());
    let shared = Arc::new(Shared { token, allowed_origin: cfg.allowed_origin.trim().to_string(), lines, seen: Default::default(), counters: counters.clone() });
    let stop = CancellationToken::new();
    tokio::spawn(serve_all(listener, shared, stop.clone()));
    tracing::info!("YouTube captions: listening on {addr}");
    Ok(CaptionServer { rx, addr, stop, counters })
}

async fn serve_all(listener: TcpListener, shared: Arc<Shared>, stop: CancellationToken) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let accepted = tokio::select! {
            _ = stop.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, peer)) = accepted else {
            tokio::time::sleep(Duration::from_millis(50)).await; // out of handles, say: do not spin
            continue;
        };
        // The socket is bound to 127.0.0.1, so this cannot differ; it is checked because the rule is "this PC only".
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(slot) = slots.clone().try_acquire_owned() else { continue };
        let (shared, stop) = (shared.clone(), stop.clone());
        tokio::spawn(async move {
            let _slot = slot;
            // Stopping also ends requests in progress, so the port and the line queue are released at once.
            tokio::select! {
                _ = stop.cancelled() => {}
                _ = tokio::time::timeout(REQUEST_TIMEOUT, serve(stream, &shared)) => {}
            }
        });
    }
}

struct Head {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

fn parse_head(text: &str) -> Option<Head> {
    let mut lines = text.split("\r\n");
    let mut request = lines.next()?.split(' ');
    let (method, path, version) = (request.next()?, request.next()?, request.next()?);
    if request.next().is_some() || !version.starts_with("HTTP/1.") || !path.starts_with('/') {
        return None;
    }
    let mut headers = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    // Two lengths (or two codes) are how one request is made to look like another: refuse them.
    for single in ["content-length", TOKEN_HEADER, "origin", "host"] {
        if headers.iter().filter(|(n, _)| n == single).count() > 1 {
            return None;
        }
    }
    Some(Head { method: method.to_string(), path: path.split('?').next().unwrap_or(path).to_string(), headers })
}

struct Reply {
    status: u16,
    body: String,
    /// Extra header lines, each ending in `\r\n`.
    extra: String,
}

impl Reply {
    fn error(status: u16, message: &str) -> Self {
        Self { status, body: serde_json::json!({ "ok": false, "error": message }).to_string(), extra: String::new() }
    }
    fn ok(body: serde_json::Value) -> Self {
        Self { status: 200, body: body.to_string(), extra: String::new() }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Service Unavailable",
    }
}

/// The host a request names must be this PC: a web page that points its own name at 127.0.0.1 is turned away.
fn host_is_local(host: &str) -> bool {
    let name = host.rsplit_once(':').map_or(host, |(name, port)| if port.chars().all(|c| c.is_ascii_digit()) { name } else { host });
    name == "127.0.0.1" || name.eq_ignore_ascii_case("localhost")
}

fn origin_allowed(origin: &str, pinned: &str) -> bool {
    if pinned.is_empty() { is_extension_origin(origin) } else { origin == pinned }
}

async fn serve(mut stream: TcpStream, shared: &Shared) {
    let mut buffer = Vec::with_capacity(1024);
    let reply = match read_head(&mut stream, &mut buffer).await {
        Ok(Some((head, body_start))) => answer(&mut stream, shared, &head, buffer.split_off(body_start)).await,
        Ok(None) => return, // closed before a whole request arrived: nothing to answer
        Err(reply) => reply,
    };
    if !matches!(reply.status, 200 | 204) {
        shared.counters.refused.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(status = reply.status, "a caption request was refused");
    }
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n{}\r\n",
        reply.status,
        reason(reply.status),
        reply.body.len(),
        reply.extra
    );
    if stream.write_all(head.as_bytes()).await.is_err() || stream.write_all(reply.body.as_bytes()).await.is_err() {
        return;
    }
    // Say we are done, then let the sender finish: closing with its body unread would reset the connection and
    // the browser would report a network failure instead of the answer above.
    let _ = stream.shutdown().await;
    let mut rest = [0u8; 4096];
    let mut drained = 0;
    while drained < 4 * MAX_BODY {
        match stream.read(&mut rest).await {
            Ok(n) if n > 0 => drained += n,
            _ => break,
        }
    }
}

/// Read up to the blank line that ends the headers. `Ok(None)`: the connection closed first.
async fn read_head(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> Result<Option<(Head, usize)>, Reply> {
    let mut chunk = [0u8; 2048];
    loop {
        let end = buffer.windows(4).position(|w| w == b"\r\n\r\n");
        if end.unwrap_or(buffer.len()) > MAX_HEAD {
            return Err(Reply::error(431, "the request's headers are too long"));
        }
        if let Some(end) = end {
            let head = std::str::from_utf8(&buffer[..end]).ok().and_then(parse_head);
            return head.map(|h| Some((h, end + 4))).ok_or_else(|| Reply::error(400, "this is not a request Maru understands"));
        }
        match stream.read(&mut chunk).await {
            Ok(n) if n > 0 => buffer.extend_from_slice(&chunk[..n]),
            _ => return Ok(None),
        }
    }
}

async fn answer(stream: &mut TcpStream, shared: &Shared, head: &Head, mut body: Vec<u8>) -> Reply {
    if head.header("host").is_some_and(|host| !host_is_local(host)) {
        return Reply::error(403, "this address is only for programs on this PC");
    }
    if head.path != LINE_ROUTE && head.path != HELLO_ROUTE {
        return Reply::error(404, "there is nothing at this address");
    }
    // A program that is not a browser sends no Origin and still needs the code. A browser always sends one for
    // a request like this, and only an extension's is let through: no web page can reach the code check.
    let origin = head.header("origin");
    if origin.is_some_and(|origin| !origin_allowed(origin, &shared.allowed_origin)) {
        return Reply::error(403, "only Maru's browser extension may send captions");
    }
    let cors = origin.map(|o| format!("Access-Control-Allow-Origin: {o}\r\nVary: Origin\r\n")).unwrap_or_default();
    match head.method.as_str() {
        "POST" => {}
        // The browser asks first whether the extension may send this header; the question itself carries no code.
        "OPTIONS" => {
            let private = if head.header("access-control-request-private-network").is_some() { "Access-Control-Allow-Private-Network: true\r\n" } else { "" };
            let extra = format!("{cors}Access-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type, {TOKEN_HEADER}\r\nAccess-Control-Max-Age: 600\r\n{private}");
            return Reply { status: 204, body: String::new(), extra };
        }
        _ => return Reply { extra: "Allow: POST, OPTIONS\r\n".into(), ..Reply::error(405, "send a POST request") },
    }
    if !same_token(head.header(TOKEN_HEADER).unwrap_or_default(), &shared.token) {
        if !shared.counters.wrong_code_logged.swap(true, Ordering::Relaxed) {
            tracing::warn!("YouTube captions: a request without the right code was refused (copy the code from Settings into the extension)");
        }
        return Reply { extra: cors, ..Reply::error(401, "the code is missing or wrong: copy it from Maru's settings into the extension") };
    }
    if head.header("transfer-encoding").is_some() {
        return Reply { extra: cors, ..Reply::error(411, "send the body with a Content-Length") };
    }
    let Some(length) = head.header("content-length") else {
        return Reply { extra: cors, ..Reply::error(411, "send the body with a Content-Length") };
    };
    let Ok(length) = length.parse::<usize>() else {
        return Reply { extra: cors, ..Reply::error(400, "the Content-Length is not a number") };
    };
    if length > MAX_BODY {
        return Reply { extra: cors, ..Reply::error(413, "a caption line cannot be this long") };
    }
    let mut chunk = [0u8; 2048];
    while body.len() < length {
        match stream.read(&mut chunk).await {
            Ok(n) if n > 0 => body.extend_from_slice(&chunk[..n]),
            _ => return Reply { extra: cors, ..Reply::error(400, "the request ended before its body did") },
        }
    }
    body.truncate(length);
    if head.path == HELLO_ROUTE {
        return Reply { extra: cors, ..Reply::ok(serde_json::json!({ "ok": true, "app": "maru", "protocol": 1 })) };
    }
    let Ok(wire) = serde_json::from_slice::<Wire>(&body) else {
        return Reply { extra: cors, ..Reply::error(400, "the body must be JSON like {\"text\": \"…\"}") };
    };
    let taken = accept(wire, &mut shared.seen.lock().unwrap_or_else(|p| p.into_inner()), Instant::now());
    let reply = match taken {
        Err(why) => {
            shared.counters.dropped.fetch_add(1, Ordering::Relaxed);
            Reply::ok(serde_json::json!({ "ok": true, "accepted": false, "reason": why.reason() }))
        }
        Ok(line) => {
            tracing::debug!(text = line.text, auto_generated = line.auto_generated, "caption line");
            match shared.lines.try_send(line) {
                Ok(()) => {
                    shared.counters.accepted.fetch_add(1, Ordering::Relaxed);
                    Reply::ok(serde_json::json!({ "ok": true, "accepted": true }))
                }
                Err(_) => Reply::error(503, "Maru is busy with earlier lines; this one was not taken"),
            }
        }
    };
    Reply { extra: cors, ..reply }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "0123456789abcdef0123456789abcdef";
    const EXTENSION: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";

    fn settings() -> CaptionsConfig {
        // Port 0: the system picks a free one, so tests never collide with a running Maru or with each other.
        CaptionsConfig { enabled: true, port: 0, token: Some(CODE.to_string().into()), allowed_origin: String::new() }
    }

    struct Answer {
        status: u16,
        head: String,
        body: String,
    }

    /// Send raw bytes over a real socket and read the whole answer.
    async fn raw(addr: SocketAddr, request: &[u8]) -> Answer {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request).await.unwrap();
        let mut answer = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut answer)).await.expect("the listener answers and closes").unwrap();
        let text = String::from_utf8(answer).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let status = head.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        Answer { status, head: head.to_string(), body: body.to_string() }
    }

    fn request(method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Vec<u8> {
        let mut text = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
        for (name, value) in headers {
            text.push_str(&format!("{name}: {value}\r\n"));
        }
        if method == "POST" && !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-length")) {
            text.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        text.push_str("\r\n");
        text.push_str(body);
        text.into_bytes()
    }

    async fn post_line(addr: SocketAddr, body: &str) -> Answer {
        raw(addr, &request("POST", LINE_ROUTE, &[("X-Maru-Token", CODE), ("Origin", EXTENSION), ("Content-Type", "application/json")], body)).await
    }

    #[tokio::test]
    async fn a_line_with_the_right_code_reaches_the_receiver_as_text_from_youtube() {
        let mut server = listen(&settings()).await.unwrap();
        let answer = post_line(server.addr(), r#"{"text":"안녕하세요,\n여러분.","video":"dQw4w9WgXcQ","title":"첫 번째  영상","start_ms":12340,"kind":"manual","lang":"ko","later_field":1}"#).await;
        assert_eq!((answer.status, answer.body.as_str()), (200, r#"{"accepted":true,"ok":true}"#));
        assert!(answer.head.contains(&format!("Access-Control-Allow-Origin: {EXTENSION}")), "{}", answer.head);
        let line = server.next().await.unwrap();
        assert_eq!((line.text.as_str(), line.video.as_str(), line.title.as_str(), line.start_ms, line.auto_generated), ("안녕하세요, 여러분.", "dQw4w9WgXcQ", "첫 번째 영상", Some(12340), false));
        let live = line.into_live_line();
        assert_eq!(live.capture.source, CaptureSource::Caption);
        assert_eq!(live.capture.app.as_ref().map(|a| a.exe.as_str()), Some(PROGRAM));
        assert_eq!(live.capture.confidence, None, "nothing was recognised from pixels");
        assert_eq!(server.counters().counts(), Counts { accepted: 1, dropped: 0, refused: 0 });
    }

    #[tokio::test]
    async fn a_request_without_the_code_or_with_a_wrong_one_is_refused_before_its_body_is_read() {
        let mut server = listen(&settings()).await.unwrap();
        let addr = server.addr();
        let body = r#"{"text":"안녕하세요"}"#;
        for headers in [vec![], vec![("X-Maru-Token", "")], vec![("X-Maru-Token", "0123456789abcdef0123456789abcdee")], vec![("X-Maru-Token", "0123456789abcdef")], vec![("Authorization", CODE)]] {
            let answer = raw(addr, &request("POST", LINE_ROUTE, &headers, body)).await;
            assert_eq!(answer.status, 401, "{headers:?}");
            assert!(answer.body.contains("code"), "{}", answer.body);
            assert!(!answer.body.contains(CODE) && !answer.head.contains(CODE), "the answer never repeats the code");
        }
        // The code is checked on the headers alone: a body that never arrives does not delay the refusal.
        let answer = raw(addr, b"POST /captions/v1/line HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 500\r\n\r\n").await;
        assert_eq!(answer.status, 401);
        // The same code twice is one way to confuse a parser; it is not a request.
        let answer = raw(addr, &request("POST", LINE_ROUTE, &[("X-Maru-Token", CODE), ("X-Maru-Token", CODE)], body)).await;
        assert_eq!(answer.status, 400);
        assert!(tokio::time::timeout(Duration::from_millis(100), server.next()).await.is_err(), "none of them delivered a line");
        assert_eq!(server.counters().counts().refused, 7);
    }

    #[tokio::test]
    async fn a_web_page_is_refused_and_an_extension_is_answered_with_its_origin() {
        let server = listen(&settings()).await.unwrap();
        let addr = server.addr();
        let body = r#"{"text":"안녕하세요"}"#;
        // Even with the right code: a page on the web (or a local file, whose origin is "null") is not the extension.
        for origin in ["https://www.youtube.com", "https://evil.example", "http://127.0.0.1:18096", "null", "chrome-extension://"] {
            let answer = raw(addr, &request("POST", LINE_ROUTE, &[("X-Maru-Token", CODE), ("Origin", origin)], body)).await;
            assert_eq!(answer.status, 403, "{origin}");
            assert!(!answer.head.contains("Access-Control-Allow-Origin"), "a refused origin is not told it may read the answer");
            let preflight = raw(addr, &request("OPTIONS", LINE_ROUTE, &[("Origin", origin), ("Access-Control-Request-Method", "POST")], "")).await;
            assert_eq!(preflight.status, 403, "{origin}");
        }
        // The browser's question before the real request: answered for an extension, without needing the code.
        let preflight = raw(addr, &request("OPTIONS", LINE_ROUTE, &[("Origin", EXTENSION), ("Access-Control-Request-Method", "POST"), ("Access-Control-Request-Headers", "content-type,x-maru-token"), ("Access-Control-Request-Private-Network", "true")], "")).await;
        assert_eq!(preflight.status, 204);
        for header in [format!("Access-Control-Allow-Origin: {EXTENSION}"), "Access-Control-Allow-Methods: POST, OPTIONS".into(), "Access-Control-Allow-Headers: content-type, x-maru-token".into(), "Access-Control-Allow-Private-Network: true".into(), "Vary: Origin".into()] {
            assert!(preflight.head.contains(&header), "{header} in {}", preflight.head);
        }
        // A program on this PC that is not a browser sends no Origin; it still needs the code.
        assert_eq!(raw(addr, &request("POST", LINE_ROUTE, &[("X-Maru-Token", CODE)], body)).await.status, 200);
        assert_eq!(raw(addr, &request("POST", LINE_ROUTE, &[], body)).await.status, 401);
    }

    #[tokio::test]
    async fn only_the_named_extension_may_send_when_one_is_named() {
        let server = listen(&CaptionsConfig { allowed_origin: format!(" {EXTENSION} "), ..settings() }).await.unwrap();
        let other = "chrome-extension://ponmlkjihgfedcbaponmlkjihgfedcba";
        let body = r#"{"text":"안녕하세요"}"#;
        assert_eq!(raw(server.addr(), &request("POST", LINE_ROUTE, &[("X-Maru-Token", CODE), ("Origin", other)], body)).await.status, 403);
        assert_eq!(raw(server.addr(), &request("POST", LINE_ROUTE, &[("X-Maru-Token", CODE), ("Origin", EXTENSION)], body)).await.status, 200);
    }

    #[tokio::test]
    async fn a_body_over_the_limit_or_without_a_length_is_refused() {
        let mut server = listen(&settings()).await.unwrap();
        let addr = server.addr();
        let auth = [("X-Maru-Token", CODE), ("Origin", EXTENSION)];
        // Refused on the declared length, before any of it is read.
        let too_long = raw(addr, format!("POST {LINE_ROUTE} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Maru-Token: {CODE}\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1).as_bytes()).await;
        assert_eq!(too_long.status, 413);
        // And when the whole oversized body is really sent, the answer still arrives instead of a broken connection.
        let big = format!(r#"{{"text":"{}"}}"#, "가".repeat(MAX_BODY));
        assert_eq!(raw(addr, &request("POST", LINE_ROUTE, &auth, &big)).await.status, 413);
        let around = r#"{"text":"안녕 "}"#.len();
        let just_fits = format!(r#"{{"text":"안녕 {}"}}"#, "a".repeat(MAX_BODY - around));
        assert_eq!(just_fits.len(), MAX_BODY);
        assert_eq!(raw(addr, &request("POST", LINE_ROUTE, &auth, &just_fits)).await.status, 200);
        assert!(server.next().await.is_some());

        let no_length = raw(addr, format!("POST {LINE_ROUTE} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Maru-Token: {CODE}\r\n\r\n").as_bytes()).await;
        assert_eq!(no_length.status, 411);
        let chunked = raw(addr, format!("POST {LINE_ROUTE} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Maru-Token: {CODE}\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n").as_bytes()).await;
        assert_eq!(chunked.status, 411);
        let not_a_number = raw(addr, &request("POST", LINE_ROUTE, &[("X-Maru-Token", CODE), ("Content-Length", "many")], "")).await;
        assert_eq!(not_a_number.status, 400);
        let two_lengths = raw(addr, &request("POST", LINE_ROUTE, &[("X-Maru-Token", CODE), ("Content-Length", "2"), ("Content-Length", "20")], "{}")).await;
        assert_eq!(two_lengths.status, 400);
        let headers_too_long = raw(addr, format!("POST {LINE_ROUTE} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Filler: {}\r\n\r\n", "x".repeat(MAX_HEAD + 100)).as_bytes()).await;
        assert_eq!(headers_too_long.status, 431);
        let not_json = raw(addr, &request("POST", LINE_ROUTE, &auth, "text=안녕하세요")).await;
        assert_eq!(not_json.status, 400);
        assert!(not_json.body.contains("JSON"));
    }

    #[tokio::test]
    async fn empty_lines_lines_without_korean_translated_tracks_and_repeats_are_dropped_and_the_sender_is_told_why() {
        let mut server = listen(&settings()).await.unwrap();
        let addr = server.addr();
        for (body, why) in [
            (r#"{"text":"  \n\u200b "}"#, "empty"),
            (r#"{"video":"abc"}"#, "empty"),
            (r#"{"text":"[Music] ♪"}"#, "not_korean"),
            (r#"{"text":"ㅋㅋㅋ"}"#, "not_korean"),
            (r#"{"text":"안녕하세요","lang":"en"}"#, "not_korean"),
            (r#"{"text":"안녕하세요","lang":"ko","translated":true}"#, "translated"),
        ] {
            let answer = post_line(addr, body).await;
            assert_eq!(answer.status, 200, "{body}");
            assert_eq!(answer.body, format!(r#"{{"accepted":false,"ok":true,"reason":"{why}"}}"#), "{body}");
        }
        let cue = r#"{"text":"저는 학교에서 공부했어요","video":"abc","start_ms":4000,"lang":"ko-KR"}"#;
        assert!(post_line(addr, cue).await.body.contains(r#""accepted":true"#));
        assert!(post_line(addr, cue).await.body.contains(r#""reason":"duplicate""#), "the same cue of the same video");
        assert!(post_line(addr, r#"{"text":"저는  학교에서\n공부했어요","video":"abc","start_ms":4000}"#).await.body.contains("duplicate"), "spacing and rows do not make it another line");
        // The same words at another moment of the video, or in another video, are another line.
        assert!(post_line(addr, r#"{"text":"저는 학교에서 공부했어요","video":"abc","start_ms":9000}"#).await.body.contains(r#""accepted":true"#));
        assert!(post_line(addr, r#"{"text":"저는 학교에서 공부했어요","video":"xyz","start_ms":4000}"#).await.body.contains(r#""accepted":true"#));
        for _ in 0..3 {
            assert_eq!(server.next().await.unwrap().text, "저는 학교에서 공부했어요");
        }
        assert_eq!(server.counters().counts(), Counts { accepted: 3, dropped: 8, refused: 0 });
    }

    #[test]
    fn without_a_start_time_only_an_immediate_repeat_is_the_same_line() {
        let mut seen = Seen::default();
        let now = Instant::now();
        let line = || Wire { text: "네, 알겠습니다.".into(), video: Some("abc".into()), ..Default::default() };
        assert!(accept(line(), &mut seen, now).is_ok());
        assert_eq!(accept(line(), &mut seen, now + Duration::from_secs(2)), Err(Dropped::Duplicate));
        assert!(accept(line(), &mut seen, now + Duration::from_secs(2) + SAME_TEXT_WINDOW + Duration::from_millis(1)).is_ok(), "said again later in the video");
        assert!(accept(Wire { text: "다음 줄입니다".into(), ..line() }, &mut seen, now).is_ok());
        assert!(accept(line(), &mut seen, now).is_ok(), "another line came between");
    }

    #[test]
    fn remembered_cues_are_bounded_and_a_strange_video_id_or_a_long_title_is_not_kept() {
        let mut seen = Seen::default();
        let now = Instant::now();
        for start in 0..(REMEMBERED_CUES as u64 + 50) {
            assert!(accept(Wire { text: "가나다".into(), start_ms: Some(start), ..Default::default() }, &mut seen, now).is_ok());
        }
        assert_eq!(seen.cues.len(), REMEMBERED_CUES);
        let line = accept(Wire { text: "가나다".into(), video: Some("../../etc/passwd".into()), title: Some("제목 ".repeat(200)), kind: Some("asr".into()), ..Default::default() }, &mut Seen::default(), now).unwrap();
        assert_eq!(line.video, "");
        assert_eq!(line.title.chars().count(), 200);
        assert!(line.auto_generated);
    }

    #[tokio::test]
    async fn other_addresses_methods_and_hosts_are_refused() {
        let server = listen(&settings()).await.unwrap();
        let addr = server.addr();
        let auth = [("X-Maru-Token", CODE)];
        assert_eq!(raw(addr, &request("POST", "/", &auth, "{}")).await.status, 404);
        assert_eq!(raw(addr, &request("POST", "/captions/v2/line", &auth, "{}")).await.status, 404);
        let get = raw(addr, &request("GET", LINE_ROUTE, &auth, "")).await;
        assert_eq!(get.status, 405);
        assert!(get.head.contains("Allow: POST, OPTIONS"));
        assert_eq!(raw(addr, &request("DELETE", LINE_ROUTE, &auth, "")).await.status, 405);
        assert_eq!(raw(addr, b"hello\r\n\r\n").await.status, 400);
        assert_eq!(raw(addr, b"POST /captions/v1/line HTTP/2\r\n\r\n").await.status, 400);
        // A web page that resolves its own name to 127.0.0.1 still names itself in Host.
        let rebound = raw(addr, format!("POST {LINE_ROUTE} HTTP/1.1\r\nHost: evil.example:18096\r\nX-Maru-Token: {CODE}\r\nContent-Length: 2\r\n\r\n{{}}").as_bytes()).await;
        assert_eq!(rebound.status, 403);
        for host in ["127.0.0.1", "127.0.0.1:18096", "localhost:18096", "LOCALHOST"] {
            assert!(host_is_local(host), "{host}");
        }
        for host in ["evil.example", "127.0.0.1.evil.example", "192.168.1.20:18096", ""] {
            assert!(!host_is_local(host), "{host}");
        }
        // The popup's question "are you there, and is my code right?"
        let hello = raw(addr, &request("POST", HELLO_ROUTE, &[("X-Maru-Token", CODE), ("Origin", EXTENSION)], "")).await;
        assert_eq!((hello.status, hello.body.as_str()), (200, r#"{"app":"maru","ok":true,"protocol":1}"#));
        assert_eq!(raw(addr, &request("POST", HELLO_ROUTE, &[("X-Maru-Token", "wrong")], "")).await.status, 401);
    }

    #[tokio::test]
    async fn the_listener_is_on_this_pc_only_and_nothing_listens_when_off_or_without_a_code() {
        let server = listen(&settings()).await.unwrap();
        assert_eq!(server.addr().ip(), std::net::IpAddr::V4(Ipv4Addr::LOCALHOST));
        let off = listen(&CaptionsConfig { enabled: false, ..settings() }).await.err().unwrap();
        assert!(off.contains("switched off"), "{off}");
        for token in [None, Some(String::new().into())] {
            let no_code = listen(&CaptionsConfig { token, ..settings() }).await.err().unwrap();
            assert!(no_code.contains("no code"), "{no_code}");
        }
        // A port another program has is reported in words, with what to do.
        let taken = listen(&CaptionsConfig { port: server.addr().port(), ..settings() }).await.err().unwrap();
        assert!(taken.contains(&server.addr().port().to_string()) && taken.contains("another port"), "{taken}");
        assert!(!taken.contains(CODE));
    }

    #[tokio::test]
    async fn a_half_sent_request_does_not_hold_up_the_next_one_and_stopping_frees_the_port() {
        let mut server = listen(&settings()).await.unwrap();
        let addr = server.addr();
        let mut stalled = TcpStream::connect(addr).await.unwrap();
        stalled.write_all(b"POST /captions/v1/line HTTP/1.1\r\nHost: 127.0").await.unwrap();
        assert_eq!(post_line(addr, r#"{"text":"안녕하세요"}"#).await.status, 200);
        assert_eq!(server.next().await.unwrap().text, "안녕하세요");
        server.stop();
        assert!(server.next().await.is_none(), "no more lines once stopped");
        let mut freed = false;
        for _ in 0..50 {
            if TcpStream::connect(addr).await.is_err() {
                freed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(freed, "the port is closed");
        drop(stalled);
    }

    #[tokio::test]
    async fn lines_nobody_collects_are_refused_as_busy_instead_of_piling_up() {
        let server = listen(&settings()).await.unwrap();
        for n in 0..QUEUE {
            assert_eq!(post_line(server.addr(), &format!(r#"{{"text":"줄 {n}","start_ms":{n}}}"#)).await.status, 200);
        }
        let busy = post_line(server.addr(), r#"{"text":"하나 더","start_ms":99999}"#).await;
        assert_eq!(busy.status, 503);
        assert!(busy.body.contains("busy"));
    }

    #[test]
    fn codes_are_long_differ_every_time_and_are_compared_in_full() {
        let codes: std::collections::HashSet<String> = (0..200).map(|_| new_token()).collect();
        assert_eq!(codes.len(), 200, "no two alike");
        for code in &codes {
            assert_eq!(code.len(), 32);
            assert!(code.chars().all(|c| c.is_ascii_hexdigit()));
            let cfg = CaptionsConfig { token: Some(code.clone().into()), ..Default::default() };
            assert!(cfg.problems().is_empty(), "a code Maru makes passes the settings' own check");
        }
        assert!(same_token(CODE, CODE));
        for wrong in ["", "0", "0123456789abcdef0123456789abcdeF", "0123456789abcdef0123456789abcde", "0123456789abcdef0123456789abcdef0", "x123456789abcdef0123456789abcdef"] {
            assert!(!same_token(wrong, CODE), "{wrong:?}");
        }
        assert!(!same_token("", ""), "no code set means nothing matches");
    }
}
