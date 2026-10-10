//! `maru setup tts`, `maru tts` and `maru tts-eval`: install, try and compare the voices.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use maru_core::config::{TtsConfig, TtsProvider};
use maru_core::{Config, Paths};
use maru_tts::local::{self, LocalTts};
use maru_tts::{Tts, wav};

use crate::setup;

/// Where `tts-server` builds are published (see `.github/workflows/tts-server.yml`).
const SERVER_RELEASES_API: &str = "https://api.github.com/repos/myussu/koreanOCR/releases?per_page=30";
const SERVER_ASSET: &str = "tts-server-windows-vulkan-x64.zip";
/// A smaller model (about 1 GB instead of 2 GB), a little less natural.
const SMALL_TALKER: &str = "qwen-talker-0.6b-customvoice-Q8_0.gguf";

/// The newest release that carries the server zip: `(tag, url)`.
pub fn pick_server_release(releases: &serde_json::Value) -> Option<(String, String)> {
    releases.as_array()?.iter().find_map(|r| {
        let tag = r["tag_name"].as_str()?;
        if !tag.starts_with("tts-server-") {
            return None;
        }
        let url = r["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(SERVER_ASSET))?["browser_download_url"].as_str()?;
        Some((tag.to_string(), url.to_string()))
    })
}

pub struct SetupOptions {
    pub small: bool,
    pub server_url: Option<String>,
    pub no_test: bool,
}

pub async fn setup(paths: &Paths, cfg: &Config, opts: SetupOptions) -> Result<()> {
    paths.ensure()?;
    let tts = &cfg.tts;

    // 1. the server
    let binary = local::server_binary(tts, paths);
    if binary.is_file() {
        println!("tts-server found: {}", binary.display());
    } else if cfg!(windows) {
        let url = match opts.server_url {
            Some(u) => u,
            None => {
                let releases: serde_json::Value = reqwest::Client::builder()
                    .user_agent("maru")
                    .build()?
                    .get(SERVER_RELEASES_API)
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                    .context("asking GitHub for tts-server releases")?
                    .error_for_status()?
                    .json()
                    .await?;
                let (tag, url) = pick_server_release(&releases).context(
                    "no tts-server build has been published yet. Run the `tts-server` workflow in the repository's Actions tab, or build https://github.com/ServeurpersoCom/qwentts.cpp yourself and set [tts] server_path",
                )?;
                eprintln!("Using {tag}");
                url
            }
        };
        let zip_path = paths.bin_dir.join(SERVER_ASSET);
        setup::download(&url, &zip_path, SERVER_ASSET).await?;
        zip::ZipArchive::new(std::fs::File::open(&zip_path)?)?.extract(&paths.bin_dir).context("extracting the server")?;
        std::fs::remove_file(&zip_path).ok();
        if !binary.is_file() {
            bail!("the archive did not contain {}", binary.display());
        }
        println!("tts-server installed in {}", paths.bin_dir.display());
    } else {
        bail!("tts-server was not found. Build https://github.com/ServeurpersoCom/qwentts.cpp (buildvulkan.sh / buildcuda.sh), then set [tts] server_path in {}", paths.config_file.display());
    }

    // 2. the models
    let dir = paths.tts_models_dir();
    let talker = if opts.small { SMALL_TALKER } else { local::TALKER_FILE };
    setup::fetch_hf(local::REPO, talker, &dir).await.with_context(|| format!("downloading {talker}"))?;
    setup::fetch_hf(local::REPO, local::CODEC_FILE, &dir).await.with_context(|| format!("downloading {}", local::CODEC_FILE))?;

    // 3. remember it in the settings (comments in the file are kept)
    let mut changes = serde_json::Map::new();
    changes.insert("tts.enabled".into(), true.into());
    changes.insert("tts.provider".into(), "local".into());
    changes.insert("tts.model_path".into(), if opts.small { dir.join(SMALL_TALKER).display().to_string().into() } else { serde_json::Value::Null });
    let text = std::fs::read_to_string(&paths.config_file).unwrap_or_default();
    let patched = maru_core::settings::patch(&text, &changes)?;
    std::fs::write(&paths.config_file, &patched.text)?;
    println!("Speech is switched on in {}", paths.config_file.display());

    // 4. prove it speaks
    if opts.no_test {
        return Ok(());
    }
    let cfg = patched.config;
    println!("\nStarting the speech server once to try it (it loads the model; this can take a minute) …");
    let started = Instant::now();
    let server = LocalTts::start(&cfg.tts, paths).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("Ready after {:.1}s.", started.elapsed().as_secs_f32());
    let speech = Tts::new(&cfg.tts, paths).map_err(|e| anyhow::anyhow!("{e}"))?;
    let t = Instant::now();
    let (audio, _) = speech.speak("안녕하세요. 만나서 반갑습니다.").await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let out = paths.data_dir.join("tts-test.wav");
    std::fs::write(&out, &audio.bytes)?;
    let secs = wav::info(&audio.bytes).map_or(0.0, |i| i.seconds);
    println!("Spoke {secs:.1}s of audio in {:.1}s: {}\nOpen it and listen; `maru tts-eval` compares voices on harder sentences.", t.elapsed().as_secs_f32(), out.display());
    drop(server);
    Ok(())
}

/// The speech provider from the config, with the local server started when it is the provider.
async fn open(paths: &Paths, cfg: &TtsConfig) -> Result<(Tts, Option<LocalTts>)> {
    let server = if cfg.provider == TtsProvider::Local {
        eprintln!("Starting the local speech server …");
        Some(LocalTts::start(cfg, paths).await.map_err(|e| anyhow::anyhow!("{e}"))?)
    } else {
        None
    };
    let tts = Tts::new(cfg, paths).map_err(|e| anyhow::anyhow!("{e}"))?;
    tts.client().check().await.map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok((tts, server))
}

/// `maru tts "text" -o out.wav`: speak text (sentence by sentence) into a WAV file.
pub async fn speak(paths: &Paths, cfg: &Config, text: &str, out: &Path) -> Result<()> {
    let (tts, _server) = open(paths, &cfg.tts).await?;
    let sentences = tts.sentences(text);
    if sentences.is_empty() {
        bail!("nothing to say");
    }
    let (mut parts, started) = (Vec::new(), Instant::now());
    for (i, s) in sentences.iter().enumerate() {
        let t = Instant::now();
        let (audio, cached) = tts.speak(s).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let secs = wav::info(&audio.bytes).map_or(0.0, |w| w.seconds);
        println!("{:>2}. {:.1}s audio in {:.1}s{}  {s}", i + 1, secs, t.elapsed().as_secs_f32(), if cached { " (cached)" } else { "" });
        parts.push(audio.bytes);
    }
    let joined = wav::concat_pcm16(&parts, 0.35);
    match (joined, parts.len()) {
        (Some(all), _) => std::fs::write(out, all)?,
        (None, 1) => std::fs::write(out, &parts[0])?,
        (None, _) => {
            // not plain 16-bit mono WAV (an MP3 from a paid service, say): one file per sentence
            for (i, p) in parts.iter().enumerate() {
                let one = out.with_file_name(format!("{}-{:02}.{}", out.file_stem().and_then(|s| s.to_str()).unwrap_or("speech"), i + 1, out.extension().and_then(|e| e.to_str()).unwrap_or("wav")));
                std::fs::write(one, p)?;
            }
        }
    }
    println!("Wrote {} ({:.1}s)", out.display(), started.elapsed().as_secs_f32());
    Ok(())
}

/// Sentences chosen to show whether a voice sounds like a native speaker: sound changes that
/// spelling does not show, numbers, intonation and a longer passage.
pub const EVAL_SENTENCES: &[(&str, &str)] = &[
    ("greeting", "안녕하세요. 만나서 반갑습니다. 저는 한국어를 공부하고 있어요."),
    ("palatalisation", "같이 가요. 굳이 말할 필요 없어요. 해돋이를 보러 갈까요?"),
    ("nasalisation", "독립 선언문을 읽었어요. 국물이 맛있어요. 십만 원이에요."),
    ("liaison", "닭이 앞마당에서 먹이를 찾고 있어요. 꽃이 예쁘게 피었어요. 값이 얼마예요?"),
    ("tensification", "학교에 갔어요. 어젯밤에 잠을 못 잤어요. 믿을 수 없어요."),
    ("numbers", "오늘은 3월 5일이고, 저는 사과 3개와 커피 2잔을 샀어요. 전화번호는 010-1234-5678이에요."),
    ("intonation", "정말요? 진짜 그랬어요? 믿을 수 없어요! 그럼 어떻게 해요?"),
    (
        "paragraph",
        "한국어를 공부하는 것은 쉽지 않지만, 매일 조금씩 연습하면 분명히 늘어요. 처음에는 한글을 읽는 것도 어렵게 느껴지지만, 며칠만 지나면 간판과 메뉴판이 눈에 들어오기 시작해요. 그때부터는 모든 것이 공부가 돼요. 포기하지 말고 천천히 계속해 보세요.",
    ),
];

pub struct EvalOptions {
    pub out: Option<PathBuf>,
    pub voices: Vec<String>,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub language: Option<String>,
    pub instructions: Option<String>,
    pub sentences_file: Option<PathBuf>,
}

fn slug(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    t.trim_matches('-').to_string()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub async fn eval(paths: &Paths, cfg: &Config, opts: EvalOptions) -> Result<()> {
    let mut tts_cfg = cfg.tts.clone();
    if let Some(e) = &opts.endpoint {
        tts_cfg.provider = TtsProvider::Endpoint;
        tts_cfg.endpoint = Some(e.clone());
    }
    if let Some(k) = opts.api_key {
        tts_cfg.api_key = Some(k);
    }
    if let Some(m) = opts.model {
        tts_cfg.model = m;
    }
    if let Some(l) = opts.language {
        tts_cfg.language = l;
    }
    if let Some(i) = opts.instructions {
        tts_cfg.instructions = Some(i);
    }
    // each voice gets its own cache identity, so voices never reuse each other's audio
    let voices = if opts.voices.is_empty() { vec![tts_cfg.voice.clone()] } else { opts.voices };
    let sentences: Vec<(String, String)> = match &opts.sentences_file {
        Some(f) => std::fs::read_to_string(f)
            .with_context(|| format!("reading {}", f.display()))?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, l)| (format!("line{:02}", i + 1), l.trim().to_string()))
            .collect(),
        None => EVAL_SENTENCES.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect(),
    };
    let out_dir = opts.out.unwrap_or_else(|| paths.data_dir.join("tts-eval"));
    std::fs::create_dir_all(&out_dir)?;

    let server = if tts_cfg.provider == TtsProvider::Local {
        eprintln!("Starting the local speech server …");
        Some(LocalTts::start(&tts_cfg, paths).await.map_err(|e| anyhow::anyhow!("{e}"))?)
    } else {
        None
    };

    let mut html = String::from("<!doctype html><meta charset=utf-8><title>Maru voices</title><style>body{font:16px system-ui;max-width:860px;margin:2em auto;padding:0 1em}h2{margin-top:2em}.s{margin:1.2em 0;padding-bottom:1em;border-bottom:1px solid #ddd}.k{font-size:20px}.m{color:#777;font-size:13px}audio{width:100%;margin-top:.4em}</style><h1>Maru voices</h1><p>Listen and note which voice sounds most like a native speaker; the sentences test sound changes that the spelling does not show.</p>");
    println!("{:<18} {:<16} {:>8} {:>8} {:>6}", "voice", "sentence", "audio", "took", "RTF");
    for voice in &voices {
        let mut c = tts_cfg.clone();
        c.voice = voice.clone();
        let tts = Tts::new(&c, paths).map_err(|e| anyhow::anyhow!("{e}"))?;
        tts.client().check().await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let dir = out_dir.join(slug(voice));
        std::fs::create_dir_all(&dir)?;
        html.push_str(&format!("<h2>{}</h2>", escape(voice)));
        let mut all = Vec::new();
        for (name, text) in &sentences {
            // a paragraph is spoken the way the reader will: one sentence at a time, joined
            let (mut parts, mut took) = (Vec::new(), 0.0f32);
            for s in tts.sentences(text) {
                let t = Instant::now();
                let (audio, cached) = tts.speak(&s).await.map_err(|e| anyhow::anyhow!("{voice}: {e}"))?;
                if !cached {
                    took += t.elapsed().as_secs_f32();
                }
                parts.push(audio.bytes);
            }
            let bytes = wav::concat_pcm16(&parts, 0.35).or_else(|| parts.first().cloned()).context("no audio")?;
            let secs = wav::info(&bytes).map_or(0.0, |i| i.seconds);
            let file = format!("{}.wav", slug(name));
            std::fs::write(dir.join(&file), &bytes)?;
            println!("{voice:<18} {name:<16} {secs:>7.1}s {took:>7.1}s {:>6.2}", if secs > 0.0 { took / secs } else { 0.0 });
            html.push_str(&format!("<div class=s><div class=k>{}</div><div class=m>{} · {secs:.1}s · made in {took:.1}s</div><audio controls preload=none src=\"{}/{file}\"></audio></div>", escape(text), escape(name), slug(voice)));
            all.push(bytes);
        }
        if let Some(joined) = wav::concat_pcm16(&all, 0.8) {
            std::fs::write(dir.join("all.wav"), joined)?;
        }
    }
    let index = out_dir.join("index.html");
    std::fs::write(&index, html)?;
    println!("\nOpen {} in a browser and listen. RTF below 1 means it speaks faster than real time.", index.display());
    drop(server);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_newest_published_server() {
        let releases = serde_json::json!([
            {"tag_name": "v0.9", "assets": []},
            {"tag_name": "tts-server-abc", "assets": [{"name": "notes.txt", "browser_download_url": "https://x/n"}]},
            {"tag_name": "tts-server-def", "assets": [{"name": SERVER_ASSET, "browser_download_url": "https://x/def.zip"}]},
            {"tag_name": "tts-server-old", "assets": [{"name": SERVER_ASSET, "browser_download_url": "https://x/old.zip"}]},
        ]);
        assert_eq!(pick_server_release(&releases), Some(("tts-server-def".into(), "https://x/def.zip".into())));
        assert_eq!(pick_server_release(&serde_json::json!([])), None);
    }

    #[test]
    fn the_listening_sentences_all_split_into_something_sayable() {
        for (name, text) in EVAL_SENTENCES {
            assert!(!maru_tts::split::sentences(text).is_empty(), "{name}");
        }
        assert!(maru_tts::split::sentences(EVAL_SENTENCES.last().unwrap().1).len() >= 4, "the paragraph really is several sentences");
        assert_eq!(slug("sohee / v2"), "sohee---v2");
    }
}
