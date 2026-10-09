//! `maru setup …` and `maru dict download`: fetch the things Maru does not bundle.
//!
//! None of these URLs could be exercised from the build sandbox (it cannot reach
//! GitHub releases or Hugging Face), so each step prints exactly what it is doing
//! and every URL can be overridden on the command line.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use maru_core::{Config, Paths};

/// Model written by `maru config init`. Change it in config.toml; any GGUF repo on Hugging Face works.
pub const RECOMMENDED_MODEL: &str = "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL";
pub const LLAMA_RELEASES_API: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=30";
/// Wiktionary extract of Korean entries (JSON lines). Override with `--url` if kaikki.org moved it.
pub const KAIKKI_KOREAN_URL: &str = "https://kaikki.org/dictionary/Korean/kaikki.org-dictionary-Korean.jsonl";
/// Hugging Face repository with PP-OCR models converted to ONNX.
pub const OCR_REPO_BASE: &str = "https://huggingface.co/monkt/paddleocr-onnx/resolve/main";
/// Layouts that repository has used; tried in order.
const OCR_DET_CANDIDATES: &[&str] = &["detection/v5/det.onnx"];
const OCR_REC_CANDIDATES: &[&str] = &["languages/korean/rec.onnx", "korean/korean_PP-OCRv5_mobile_rec.onnx"];
const OCR_DICT_CANDIDATES: &[&str] = &["languages/korean/dict.txt", "korean/ppocrv5_korean_dict.txt"];

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().user_agent(concat!("maru/", env!("CARGO_PKG_VERSION"))).connect_timeout(Duration::from_secs(15)).build()?)
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

/// Download `url` to `dest` (via a `.part` file) with a progress line on stderr.
pub async fn download(url: &str, dest: &Path, label: &str) -> Result<u64> {
    let resp = client()?.get(url).send().await.with_context(|| format!("requesting {url}"))?;
    if !resp.status().is_success() {
        bail!("{url} answered HTTP {}", resp.status());
    }
    let total = resp.content_length();
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let part = dest.with_extension("part");
    let mut file = std::fs::File::create(&part).with_context(|| format!("creating {}", part.display()))?;
    let mut stream = resp.bytes_stream();
    let (mut done, mut last_print) = (0u64, Instant::now());
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("downloading {url}"))?;
        file.write_all(&chunk)?;
        done += chunk.len() as u64;
        if last_print.elapsed() > Duration::from_millis(400) {
            match total {
                Some(t) => eprint!("\r  {label}: {:.1} / {:.1} MB ({}%)   ", mb(done), mb(t), done * 100 / t.max(1)),
                None => eprint!("\r  {label}: {:.1} MB   ", mb(done)),
            }
            last_print = Instant::now();
        }
    }
    file.flush()?;
    drop(file);
    std::fs::rename(&part, dest)?;
    eprintln!("\r  {label}: {:.1} MB done            ", mb(done));
    Ok(done)
}

async fn reachable(url: &str) -> bool {
    match client() {
        Ok(c) => c.head(url).send().await.map(|r| r.status().is_success()).unwrap_or(false),
        Err(_) => false,
    }
}

/// First candidate path under `base` that answers.
async fn first_reachable(base: &str, candidates: &[&str]) -> Option<String> {
    for c in candidates {
        let url = format!("{}/{}", base.trim_end_matches('/'), c);
        if reachable(&url).await {
            return Some(url);
        }
    }
    None
}

// ───────────────────────────── Kiwi ─────────────────────────────

/// Kiwi releases to try, newest first. The newest one sometimes has no Windows build yet.
const KIWI_FALLBACK_RELEASES: &[&str] = &["latest", "v0.22.2", "v0.22.1", "v0.21.0", "v0.20.0"];

/// Download Kiwi (library + model) into Maru's own folder and prove it loads.
/// `KIWI_RS_CACHE_DIR` must already point at that folder (set in `main` before threads start).
pub async fn kiwi(paths: &Paths, release: Option<String>) -> Result<()> {
    let dir = maru_engine::bootstrap::kiwi_cache_dir(paths);
    std::fs::create_dir_all(&dir)?;
    let candidates: Vec<String> = match release {
        Some(r) => vec![r],
        None => KIWI_FALLBACK_RELEASES.iter().map(|s| s.to_string()).collect(),
    };
    let mut last_error = String::new();
    for rel in candidates {
        eprintln!("Downloading Kiwi ({rel}) into {} …", dir.display());
        let attempt = {
            let rel = rel.clone();
            tokio::task::spawn_blocking(move || maru_lang::kiwi::KiwiAnalyzer::spawn_with_release(&rel)).await?
        };
        match attempt {
            Ok(analyzer) => {
                use maru_lang::Morphology;
                let tokens = analyzer.tokenize("안녕하세요, 한국어를 공부해요.").map_err(|e| anyhow::anyhow!("{e}"))?;
                let shown = tokens.iter().map(|t| format!("{}/{}", t.form, t.tag)).collect::<Vec<_>>().join(" ");
                println!("Kiwi {rel} works: {shown}");
                match maru_engine::bootstrap::find_kiwi(paths) {
                    Some((lib, _)) => println!("Installed at {}", lib.display()),
                    None => println!("Note: Kiwi loaded, but not from {} (found elsewhere on this machine); that is fine.", dir.display()),
                }
                return Ok(());
            }
            Err(e) => {
                eprintln!("  {rel} did not work: {e}");
                last_error = e.to_string();
            }
        }
    }
    bail!("none of the Kiwi releases could be installed. Last error: {last_error}\nYou can try a specific one with `maru setup kiwi --release vX.Y.Z` (see https://github.com/bab2min/Kiwi/releases).")
}

// ───────────────────────────── OCR models ─────────────────────────────

#[derive(Debug, Default)]
pub struct OcrSources {
    pub from_dir: Option<PathBuf>,
    pub base_url: Option<String>,
    pub det_url: Option<String>,
    pub rec_url: Option<String>,
    pub dict_url: Option<String>,
}

fn find_in(dir: &Path, predicate: impl Fn(&str) -> bool) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(&predicate))
}

pub async fn ocr(paths: &Paths, cfg: &Config, src: OcrSources) -> Result<()> {
    let out = paths.models_dir.join("ocr");
    std::fs::create_dir_all(&out)?;
    let (det, rec, dict) = (out.join("det.onnx"), out.join("rec.onnx"), out.join("dict.txt"));

    if let Some(dir) = &src.from_dir {
        let pick = |what: &str, found: Option<PathBuf>, dest: &Path| -> Result<()> {
            let from = found.with_context(|| format!("no {what} file in {}", dir.display()))?;
            std::fs::copy(&from, dest).with_context(|| format!("copying {}", from.display()))?;
            println!("  {what}: {}", from.display());
            Ok(())
        };
        pick("detection model", find_in(dir, |n| n.contains("det") && n.ends_with(".onnx")), &det)?;
        pick("recognition model", find_in(dir, |n| n.contains("rec") && n.ends_with(".onnx")), &rec)?;
        pick("dictionary", find_in(dir, |n| n.contains("dict") && n.ends_with(".txt")), &dict)?;
    } else {
        let base = src.base_url.as_deref().unwrap_or(OCR_REPO_BASE);
        eprintln!("Looking for PP-OCRv5 Korean models under {base} …");
        for (label, explicit, candidates, dest) in [
            ("detection model", &src.det_url, OCR_DET_CANDIDATES, &det),
            ("recognition model (Korean)", &src.rec_url, OCR_REC_CANDIDATES, &rec),
            ("character dictionary (Korean)", &src.dict_url, OCR_DICT_CANDIDATES, &dict),
        ] {
            let url = match explicit {
                Some(u) => u.clone(),
                None => first_reachable(base, candidates).await.with_context(|| {
                    format!(
                        "could not find the {label} under {base} (tried: {}).\n\
                         Open the repository's file list in a browser, then pass explicit links, e.g.\n  \
                         maru setup ocr --det-url <link> --rec-url <link> --dict-url <link>\n\
                         or download the files yourself and use `maru setup ocr --from-dir <folder>`.",
                        candidates.join(", ")
                    )
                })?,
            };
            download(&url, dest, label).await?;
        }
    }

    // Prove the files work together before declaring success.
    let mut ocr_cfg = cfg.ocr.clone();
    ocr_cfg.det_model = Some(det.clone());
    ocr_cfg.rec_model = Some(rec.clone());
    ocr_cfg.dict_path = Some(dict.clone());
    let pc = maru_ocr::paddle::PaddleConfig::resolve(&ocr_cfg, paths);
    match maru_ocr::paddle::PaddleOcr::load(&pc) {
        Ok(_) => println!("OCR models installed in {} and load correctly.", out.display()),
        Err(e) => bail!(
            "files were saved to {} but could not be loaded: {e}\n\
             (if this mentions ONNX Runtime, make sure onnxruntime.dll sits next to maru.exe or set ORT_DYLIB_PATH)",
            out.display()
        ),
    }
    Ok(())
}

// ───────────────────────────── llama.cpp ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LlamaBackend {
    Cuda,
    Vulkan,
    Cpu,
}

type Asset = (String, String); // (name, url)

fn parse_cuda(name: &str) -> Option<(u32, u32)> {
    let rest = name.split("bin-win-cuda-").nth(1)?;
    let ver = rest.strip_suffix("-x64.zip")?;
    let (major, minor) = ver.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Newest CUDA build we trust. CUDA 13.2 builds were reported broken by a third-party
/// installer project, so the default stops at 13.1 unless `--cuda` asks for more.
const MAX_DEFAULT_CUDA: (u32, u32) = (13, 1);

/// Choose the Windows zips to download from a release's assets.
pub fn pick_llama_assets(assets: &[Asset], backend: LlamaBackend, cuda: Option<&str>) -> Result<Vec<Asset>> {
    let find = |pred: &dyn Fn(&str) -> bool| assets.iter().find(|(n, _)| pred(n)).cloned();
    match backend {
        LlamaBackend::Cuda => {
            let wanted = cuda.map(|c| c.trim().to_string());
            let mut builds: Vec<((u32, u32), &Asset)> = assets
                .iter()
                .filter(|(n, _)| n.starts_with("llama-") && n.contains("bin-win-cuda-"))
                .filter_map(|a| parse_cuda(&a.0).map(|v| (v, a)))
                .collect();
            builds.sort_by_key(|(v, _)| std::cmp::Reverse(*v));
            let chosen = match &wanted {
                Some(w) => builds.iter().find(|((a, b), _)| format!("{a}.{b}") == *w),
                None => builds.iter().find(|(v, _)| *v <= MAX_DEFAULT_CUDA).or(builds.first()),
            };
            let Some(((major, minor), build)) = chosen else {
                bail!("no matching CUDA build in this release; available: {}", assets.iter().map(|a| a.0.as_str()).collect::<Vec<_>>().join(", "));
            };
            let mut out = vec![(*build).clone()];
            // The CUDA zip does not include the CUDA runtime DLLs.
            let rt_name = format!("cudart-llama-bin-win-cuda-{major}.{minor}-x64.zip");
            if let Some(rt) = find(&|n| n == rt_name) {
                out.push(rt);
            }
            Ok(out)
        }
        LlamaBackend::Vulkan => find(&|n| n.starts_with("llama-") && n.contains("bin-win-vulkan-x64.zip"))
            .map(|a| vec![a])
            .context("this release has no Windows Vulkan build"),
        LlamaBackend::Cpu => find(&|n| n.starts_with("llama-") && n.contains("bin-win-cpu-x64.zip"))
            .map(|a| vec![a])
            .context("this release has no Windows CPU build"),
    }
}

/// Recent releases, newest first, as `(tag, assets)`.
async fn recent_llama_releases() -> Result<Vec<(String, Vec<Asset>)>> {
    let v: serde_json::Value = client()?
        .get(LLAMA_RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("asking GitHub for llama.cpp releases")?
        .error_for_status()?
        .json()
        .await?;
    Ok(v.as_array()
        .map(|releases| {
            releases
                .iter()
                .map(|r| {
                    let tag = r["tag_name"].as_str().unwrap_or("?").to_string();
                    let assets = r["assets"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| Some((x["name"].as_str()?.to_string(), x["browser_download_url"].as_str()?.to_string()))).collect())
                        .unwrap_or_default();
                    (tag, assets)
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The newest release that actually has a matching Windows build (some "releases" only carry
/// metadata files), with the zips to download.
pub fn pick_release(releases: &[(String, Vec<Asset>)], backend: LlamaBackend, cuda: Option<&str>) -> Result<(String, Vec<Asset>)> {
    for (tag, assets) in releases {
        if let Ok(picks) = pick_llama_assets(assets, backend, cuda) {
            return Ok((tag.clone(), picks));
        }
    }
    let seen = releases.iter().map(|(t, a)| format!("{t} ({} files)", a.len())).collect::<Vec<_>>().join(", ");
    bail!("none of the {} most recent llama.cpp releases has a matching Windows build [{seen}]. Download one yourself from https://github.com/ggml-org/llama.cpp/releases and set [llm] server_path.", releases.len())
}

fn binary_works(bin: &Path) -> bool {
    std::process::Command::new(bin).arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok()
}

pub async fn llm(paths: &Paths, cfg: &Config, backend: LlamaBackend, cuda: Option<String>) -> Result<()> {
    let binary = maru_llm::resolve_binary(&cfg.llm, paths);
    if binary_works(&binary) {
        println!("llama-server found: {}", binary.display());
    } else if cfg!(windows) {
        std::fs::create_dir_all(&paths.bin_dir)?;
        let releases = recent_llama_releases().await?;
        let (tag, picks) = pick_release(&releases, backend, cuda.as_deref())?;
        eprintln!("Using llama.cpp release {tag}");
        for (name, url) in picks {
            let zip_path = paths.bin_dir.join(&name);
            download(&url, &zip_path, &name).await?;
            let file = std::fs::File::open(&zip_path)?;
            zip::ZipArchive::new(file)?.extract(&paths.bin_dir).with_context(|| format!("extracting {name}"))?;
            std::fs::remove_file(&zip_path).ok();
        }
        println!("llama-server installed in {}", paths.bin_dir.display());
        if backend == LlamaBackend::Cuda {
            println!("Note: RTX 50-series (Blackwell) GPUs need a build made with CUDA 12.8 or newer; if loading fails, try `--backend vulkan`.");
        }
    } else {
        bail!(
            "llama-server was not found. Install llama.cpp for your platform (https://github.com/ggml-org/llama.cpp), \
             then either put llama-server on PATH or set [llm] server_path in {}",
            paths.config_file.display()
        );
    }

    let Some(source) = maru_llm::ModelSource::from_config(&cfg.llm) else {
        println!(
            "\nNo model is configured yet. Add this to {}:\n\n  [llm]\n  hf_repo = \"unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL\"\n\n\
             (or set model_path to a .gguf file you already have), then run `maru setup llm` again to download and test it.",
            paths.config_file.display()
        );
        return Ok(());
    };

    // Run the real server once with its own progress output visible: this downloads the
    // model on first use and proves the GPU build works.
    let mut args = maru_llm::build_args(&cfg.llm, &source);
    if let Some(i) = args.iter().position(|a| a == "--port") {
        args[i + 1] = cfg.llm.port.to_string();
    }
    eprintln!("Starting llama-server (first run downloads the model; this can take several minutes) …");
    let mut child = tokio::process::Command::new(&binary).args(&args).kill_on_drop(true).spawn().with_context(|| format!("running {}", binary.display()))?;
    let client = maru_llm::LlmClient::from_config(&cfg.llm);
    let deadline = Instant::now() + Duration::from_secs(60 * 60);
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("llama-server exited ({status}) before the model was ready; see its output above");
        }
        if client.health().await {
            break;
        }
        if Instant::now() > deadline {
            bail!("gave up waiting for the model after an hour");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    println!("\nModel loaded and answering on port {}. Maru will start it automatically from now on.", cfg.llm.port);
    let _ = child.kill().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets(names: &[&str]) -> Vec<Asset> {
        names.iter().map(|n| (n.to_string(), format!("https://example.invalid/{n}"))).collect()
    }

    fn release() -> Vec<Asset> {
        assets(&[
            "llama-b6000-bin-win-cuda-12.4-x64.zip",
            "llama-b6000-bin-win-cuda-13.1-x64.zip",
            "llama-b6000-bin-win-cuda-13.2-x64.zip",
            "cudart-llama-bin-win-cuda-12.4-x64.zip",
            "cudart-llama-bin-win-cuda-13.1-x64.zip",
            "llama-b6000-bin-win-vulkan-x64.zip",
            "llama-b6000-bin-win-cpu-x64.zip",
            "llama-b6000-bin-ubuntu-x64.zip",
        ])
    }

    fn names(v: &[Asset]) -> Vec<&str> {
        v.iter().map(|a| a.0.as_str()).collect()
    }

    #[test]
    fn default_cuda_pick_is_the_newest_trusted_build_with_its_runtime() {
        let picks = pick_llama_assets(&release(), LlamaBackend::Cuda, None).unwrap();
        assert_eq!(names(&picks), ["llama-b6000-bin-win-cuda-13.1-x64.zip", "cudart-llama-bin-win-cuda-13.1-x64.zip"]);
    }

    #[test]
    fn explicit_cuda_version_is_honoured_even_if_newer() {
        let picks = pick_llama_assets(&release(), LlamaBackend::Cuda, Some("12.4")).unwrap();
        assert_eq!(names(&picks), ["llama-b6000-bin-win-cuda-12.4-x64.zip", "cudart-llama-bin-win-cuda-12.4-x64.zip"]);
        let picks = pick_llama_assets(&release(), LlamaBackend::Cuda, Some("13.2")).unwrap();
        assert_eq!(names(&picks), ["llama-b6000-bin-win-cuda-13.2-x64.zip"], "no matching runtime zip: still usable if CUDA is installed");
        assert!(pick_llama_assets(&release(), LlamaBackend::Cuda, Some("9.9")).is_err());
    }

    #[test]
    fn vulkan_and_cpu_builds() {
        assert_eq!(names(&pick_llama_assets(&release(), LlamaBackend::Vulkan, None).unwrap()), ["llama-b6000-bin-win-vulkan-x64.zip"]);
        assert_eq!(names(&pick_llama_assets(&release(), LlamaBackend::Cpu, None).unwrap()), ["llama-b6000-bin-win-cpu-x64.zip"]);
        assert!(pick_llama_assets(&assets(&["llama-b1-bin-ubuntu-x64.zip"]), LlamaBackend::Vulkan, None).is_err());
    }

    #[test]
    fn only_cuda_newer_than_the_default_cap_falls_back_to_the_newest_available() {
        let only_new = assets(&["llama-b1-bin-win-cuda-13.2-x64.zip"]);
        assert_eq!(names(&pick_llama_assets(&only_new, LlamaBackend::Cuda, None).unwrap()), ["llama-b1-bin-win-cuda-13.2-x64.zip"]);
    }

    #[test]
    fn releases_without_windows_builds_are_skipped() {
        // What a real user hit: the newest "release" only carried a metadata file.
        let releases = vec![
            ("v0.6.0".to_string(), assets(&["nightly-tag.txt"])),
            ("b6000".to_string(), release()),
        ];
        let (tag, picks) = pick_release(&releases, LlamaBackend::Cuda, None).unwrap();
        assert_eq!(tag, "b6000");
        assert_eq!(names(&picks)[0], "llama-b6000-bin-win-cuda-13.1-x64.zip");

        let only_junk = vec![("v0.6.0".to_string(), assets(&["nightly-tag.txt"]))];
        let err = pick_release(&only_junk, LlamaBackend::Cuda, None).unwrap_err().to_string();
        assert!(err.contains("v0.6.0") && err.contains("server_path"), "{err}");
    }

    #[test]
    fn cuda_versions_parse() {
        assert_eq!(parse_cuda("llama-b1-bin-win-cuda-12.4-x64.zip"), Some((12, 4)));
        assert_eq!(parse_cuda("llama-b1-bin-win-vulkan-x64.zip"), None);
    }

    #[test]
    fn model_files_are_found_by_role_in_a_folder() {
        let dir = std::env::temp_dir().join(format!("maru-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["korean_PP-OCRv5_mobile_rec.onnx", "ch_PP-OCRv5_mobile_det.onnx", "ppocrv5_korean_dict.txt", "readme.md"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        assert!(find_in(&dir, |n| n.contains("det") && n.ends_with(".onnx")).unwrap().ends_with("ch_PP-OCRv5_mobile_det.onnx"));
        assert!(find_in(&dir, |n| n.contains("rec") && n.ends_with(".onnx")).unwrap().ends_with("korean_PP-OCRv5_mobile_rec.onnx"));
        assert!(find_in(&dir, |n| n.contains("dict") && n.ends_with(".txt")).is_some());
        std::fs::remove_dir_all(dir).ok();
    }
}
