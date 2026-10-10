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

/// Download `file` from a Hugging Face repo into `dir` unless it is already there.
pub async fn fetch_hf(repo: &str, file: &str, dir: &Path) -> Result<PathBuf> {
    let dest = dir.join(file);
    if dest.metadata().is_ok_and(|m| m.len() > 0) {
        return Ok(dest);
    }
    std::fs::create_dir_all(dir)?;
    download(&format!("https://huggingface.co/{repo}/resolve/main/{file}"), &dest, file).await?;
    Ok(dest)
}

// ───────────────────────────── Kiwi ─────────────────────────────

const KIWI_RELEASES_API: &str = "https://api.github.com/repos/bab2min/Kiwi/releases?per_page=20";

/// Name of Kiwi's native-library archive for this platform and release (e.g. `0.22.2`).
fn kiwi_library_asset(version: &str) -> Result<String> {
    let name = if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86") { format!("kiwi_win_Win32_v{version}.zip") } else { format!("kiwi_win_x64_v{version}.zip") }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { format!("kiwi_mac_arm64_v{version}.tgz") } else { format!("kiwi_mac_x86_64_v{version}.tgz") }
    } else if cfg!(target_arch = "aarch64") {
        format!("kiwi_lnx_aarch64_v{version}.tgz")
    } else {
        format!("kiwi_lnx_x86_64_v{version}.tgz")
    };
    Ok(name)
}

/// A Kiwi release that has everything we need: `(version, library url, model url)`.
pub type KiwiRelease = (String, String, String);

/// The `kiwi-rs` bindings match Kiwi's C ABI from 0.23.0 on; older libraries crash.
pub const KIWI_MIN_VERSION: (u32, u32, u32) = (0, 23, 0);

fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim_start_matches('v').split('.').map(|p| p.split(['-', '+']).next().unwrap_or("").parse::<u32>());
    Some((it.next()?.ok()?, it.next()?.ok()?, it.next().unwrap_or(Ok(0)).ok()?))
}

/// Every release (in the order given) that has both the library and the model for this
/// platform and is new enough for the bindings. `wanted` pins one tag (`v0.23.1` or `0.23.1`).
pub fn kiwi_candidates(releases: &serde_json::Value, wanted: Option<&str>, library_asset: &dyn Fn(&str) -> Result<String>) -> Result<Vec<KiwiRelease>> {
    let wanted = wanted.map(|w| w.trim_start_matches('v'));
    let (mut found, mut skipped) = (Vec::new(), Vec::new());
    for r in releases.as_array().into_iter().flatten() {
        let tag = r["tag_name"].as_str().unwrap_or_default();
        let version = tag.trim_start_matches('v');
        if version.is_empty() || wanted.is_some_and(|w| w != version) {
            continue;
        }
        if wanted.is_none() && parse_version(version).is_none_or(|v| v < KIWI_MIN_VERSION) {
            skipped.push(format!("{tag} (too old for the bindings)"));
            continue;
        }
        let url_of = |name: &str| -> Option<String> {
            r["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(name))?["browser_download_url"].as_str().map(str::to_string)
        };
        let (lib_name, model_name) = (library_asset(version)?, format!("kiwi_model_v{version}_base.tgz"));
        match (url_of(&lib_name), url_of(&model_name)) {
            (Some(lib), Some(model)) => found.push((version.to_string(), lib, model)),
            _ => skipped.push(format!("{tag} (no files for this platform)")),
        }
    }
    if found.is_empty() {
        bail!(
            "no usable Kiwi release{} (looked at: {}). See https://github.com/bab2min/Kiwi/releases and install it by hand, then set KIWI_LIBRARY_PATH and KIWI_MODEL_PATH.",
            wanted.map(|w| format!(" (wanted {w})")).unwrap_or_default(),
            if skipped.is_empty() { "nothing".to_string() } else { skipped.join(", ") }
        );
    }
    Ok(found)
}

fn kiwi_library_filename() -> &'static str {
    if cfg!(windows) {
        "kiwi.dll"
    } else if cfg!(target_os = "macos") {
        "libkiwi.dylib"
    } else {
        "libkiwi.so"
    }
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}

fn find_dir(dir: &Path, parent: &str, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some(name) && dir.file_name().and_then(|n| n.to_str()) == Some(parent) {
                return Some(path);
            }
            if let Some(found) = find_dir(&path, parent, name) {
                return Some(found);
            }
        }
    }
    None
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Extract Kiwi's two archives into `cache_dir` and make sure the layout is
/// `lib/<library>` and `models/cong/base`, whatever folder nesting the archives used.
pub fn install_kiwi_archives(cache_dir: &Path, lib_archive: &Path, model_archive: &Path) -> Result<(PathBuf, PathBuf)> {
    let staging = cache_dir.join("extracted");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;

    let lib_name = lib_archive.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if lib_name.ends_with(".zip") {
        zip::ZipArchive::new(std::fs::File::open(lib_archive)?)?.extract(&staging).context("extracting the Kiwi library")?;
    } else {
        tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(lib_archive)?)).unpack(&staging).context("extracting the Kiwi library")?;
    }
    tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(model_archive)?)).unpack(&staging).context("extracting the Kiwi model")?;

    let library = find_file(&staging, kiwi_library_filename()).with_context(|| format!("{} is not inside the Kiwi library archive", kiwi_library_filename()))?;
    let model = find_dir(&staging, "cong", "base").context("models/cong/base is not inside the Kiwi model archive")?;

    let (lib_dest, model_dest) = (cache_dir.join("lib"), cache_dir.join("models").join("cong").join("base"));
    std::fs::create_dir_all(&lib_dest)?;
    // The library may need its sibling DLLs, so copy the whole folder it came from.
    copy_dir(library.parent().unwrap_or(&staging), &lib_dest)?;
    copy_dir(&model, &model_dest)?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok((lib_dest.join(kiwi_library_filename()), model_dest))
}

/// Download Kiwi (library + model) into Maru's own folder, trying compatible releases newest
/// first, and keep the first one that passes the crash-proof self-test.
///
/// This replaces the downloader inside the `kiwi-rs` crate, which cannot read GitHub's real
/// release JSON (it looks for the download link before the asset's `uploader` block ends).
pub async fn kiwi(paths: &Paths, release: Option<String>) -> Result<()> {
    let root = maru_engine::bootstrap::kiwi_cache_dir(paths).join("kiwi-rs");
    eprintln!("Looking up Kiwi releases …");
    let releases: serde_json::Value = client()?
        .get(KIWI_RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("asking GitHub for Kiwi releases")?
        .error_for_status()?
        .json()
        .await?;
    let candidates = kiwi_candidates(&releases, release.as_deref(), &kiwi_library_asset)?;

    let mut failures = Vec::new();
    for (version, lib_url, model_url) in candidates {
        eprintln!("Trying Kiwi v{version} …");
        let cache_dir = root.join(&version);
        let downloads = cache_dir.join("downloads");
        std::fs::create_dir_all(&downloads)?;
        let _ = std::fs::remove_file(cache_dir.join(maru_engine::bootstrap::PROBE_MARKER));
        let lib_archive = downloads.join(lib_url.rsplit('/').next().unwrap_or("kiwi-lib"));
        let model_archive = downloads.join(model_url.rsplit('/').next().unwrap_or("kiwi-model.tgz"));
        download(&lib_url, &lib_archive, "Kiwi library").await?;
        download(&model_url, &model_archive, "Kiwi model").await?;
        let (library, model) = install_kiwi_archives(&cache_dir, &lib_archive, &model_archive)?;

        // Load it in a child process: a mismatched native library crashes the whole process.
        eprintln!("  testing it in a separate process …");
        let (lib2, model2) = (library.clone(), model.clone());
        match tokio::task::spawn_blocking(move || maru_engine::bootstrap::probe_kiwi(&lib2, &model2)).await? {
            Ok(()) => {
                std::fs::write(cache_dir.join(maru_engine::bootstrap::PROBE_MARKER), b"ok")?;
                let _ = std::fs::remove_dir_all(&downloads);
                println!("Kiwi v{version} installed and verified: {}", library.display());
                let analyzer = tokio::task::spawn_blocking(move || maru_lang::kiwi::KiwiAnalyzer::spawn_from_paths(library, model)).await?.map_err(|e| anyhow::anyhow!("{e}"))?;
                use maru_lang::Morphology;
                let tokens = analyzer.tokenize("안녕하세요, 한국어를 공부해요.").map_err(|e| anyhow::anyhow!("{e}"))?;
                println!("It works: {}", tokens.iter().map(|t| format!("{}/{}", t.form, t.tag)).collect::<Vec<_>>().join(" "));
                return Ok(());
            }
            Err(why) => {
                eprintln!("  v{version} is not usable here: {why}");
                failures.push(format!("v{version}: {why}"));
            }
        }
    }
    bail!(
        "no Kiwi release passed the self-test:\n  {}\nIf every attempt says the library would not load, install the Microsoft Visual C++ runtime (https://aka.ms/vs/17/release/vc_redist.x64.exe). Maru works without Kiwi, with a simpler word breakdown.",
        failures.join("\n  ")
    )
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

    /// GitHub's real asset objects: the name comes *before* a nested `uploader` object,
    /// and `browser_download_url` after it. (This broke the kiwi-rs downloader.)
    fn github_release(tag: &str, names: &[&str]) -> serde_json::Value {
        let assets: Vec<_> = names
            .iter()
            .map(|n| {
                serde_json::json!({
                    "url": format!("https://api.github.com/assets/{n}"), "id": 1, "name": n, "label": null,
                    "uploader": { "login": "bot", "id": 2, "type": "Bot" },
                    "content_type": "application/zip", "size": 10,
                    "browser_download_url": format!("https://github.com/bab2min/Kiwi/releases/download/{tag}/{n}")
                })
            })
            .collect();
        serde_json::json!({ "tag_name": tag, "assets": assets })
    }

    #[test]
    fn kiwi_candidates_come_from_realistic_github_json_newest_first_and_skip_what_cannot_work() {
        let lib = |v: &str| Ok(format!("kiwi_win_x64_v{v}.zip"));
        let releases = serde_json::json!([
            github_release("v0.25.0", &["readme.txt"]),
            github_release("v0.24.0", &["kiwi_win_x64_v0.24.0.zip", "kiwi_model_v0.24.0_base.tgz", "kiwi_lnx_x86_64_v0.24.0.tgz"]),
            github_release("v0.23.1", &["kiwi_win_x64_v0.23.1.zip", "kiwi_model_v0.23.1_base.tgz"]),
            github_release("v0.22.2", &["kiwi_win_x64_v0.22.2.zip", "kiwi_model_v0.22.2_base.tgz"]),
        ]);
        let found = kiwi_candidates(&releases, None, &lib).unwrap();
        assert_eq!(found.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["0.24.0", "0.23.1"],
            "v0.25.0 has no platform files and v0.22.2 is too old for the bindings (it crashes)");
        assert!(found[0].1.ends_with("/v0.24.0/kiwi_win_x64_v0.24.0.zip") && found[0].2.ends_with("kiwi_model_v0.24.0_base.tgz"));

        // pinning a version bypasses the age filter: the user knows what they asked for
        assert_eq!(kiwi_candidates(&releases, Some("v0.22.2"), &lib).unwrap()[0].0, "0.22.2");
        let err = kiwi_candidates(&releases, Some("v9.9.9"), &lib).unwrap_err().to_string();
        assert!(err.contains("9.9.9"), "{err}");
    }

    #[test]
    fn a_release_missing_the_model_is_not_good_enough() {
        let lib = |v: &str| Ok(format!("kiwi_win_x64_v{v}.zip"));
        let releases = serde_json::json!([github_release("v0.30.0", &["kiwi_win_x64_v0.30.0.zip"])]);
        let err = kiwi_candidates(&releases, None, &lib).unwrap_err().to_string();
        assert!(err.contains("v0.30.0") && err.contains("KIWI_LIBRARY_PATH"), "{err}");
    }

    #[test]
    fn versions_parse_and_compare() {
        assert!(parse_version("0.22.2").unwrap() < KIWI_MIN_VERSION);
        assert!(parse_version("v0.23.0").unwrap() >= KIWI_MIN_VERSION);
        assert!(parse_version("0.24").unwrap() >= KIWI_MIN_VERSION);
        assert_eq!(parse_version("nightly"), None);
    }

    fn write_tgz(path: &Path, files: &[(&str, &[u8])]) {
        let gz = flate2::write::GzEncoder::new(std::fs::File::create(path).unwrap(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, name, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn archives_are_laid_out_as_lib_and_models_cong_base_even_when_nested() {
        let dir = std::env::temp_dir().join(format!("maru-kiwi-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = kiwi_library_filename();

        // library archive with an extra top-level folder, as some releases are packaged
        let lib_archive = dir.join(if cfg!(windows) { "lib.zip" } else { "lib.tgz" });
        if cfg!(windows) {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&lib_archive).unwrap());
            z.start_file(format!("kiwi_pkg/lib/{lib}"), zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut z, b"binary").unwrap();
            z.finish().unwrap();
        } else {
            write_tgz(&lib_archive, &[(&format!("kiwi_pkg/lib/{lib}"), b"binary")]);
        }
        let model_archive = dir.join("model.tgz");
        write_tgz(&model_archive, &[("models/cong/base/sj.knlm", b"m1"), ("models/cong/base/default.dict", b"m2")]);

        let cache = dir.join("kiwi-rs").join("0.22.2");
        std::fs::create_dir_all(&cache).unwrap();
        let (library, model) = install_kiwi_archives(&cache, &lib_archive, &model_archive).unwrap();
        assert_eq!(library, cache.join("lib").join(lib));
        assert_eq!(std::fs::read(&library).unwrap(), b"binary");
        assert!(model.ends_with("models/cong/base") && model.join("sj.knlm").exists() && model.join("default.dict").exists());
        assert!(!cache.join("extracted").exists(), "staging is cleaned up");

        // and the app's own lookup finds that layout
        let paths = maru_core::Paths::under(&dir.join("home"));
        let target = maru_engine::bootstrap::kiwi_cache_dir(&paths).join("kiwi-rs").join("0.22.2");
        copy_dir(&cache, &target).unwrap();
        assert!(maru_engine::bootstrap::find_kiwi(&paths).is_none(), "not usable until the self-test has passed");
        std::fs::write(target.join(maru_engine::bootstrap::PROBE_MARKER), b"ok").unwrap();
        assert!(maru_engine::bootstrap::find_kiwi(&paths).is_some());
        std::fs::remove_dir_all(dir).ok();
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
