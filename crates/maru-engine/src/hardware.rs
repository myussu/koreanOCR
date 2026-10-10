//! What this PC has, as far as setup needs to know: graphics cards, memory and free disk.
//!
//! Detection asks the system (`nvidia-smi`, PowerShell/CIM) and never fails: anything it cannot learn
//! stays unknown and the plan ([`crate::plan`]) treats it cautiously. The parsing is separate from the
//! process calls so it can be tested with real command output.

use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

/// Does the card have its own memory, or share the computer's? Decided from the name, because Windows
/// reports no reliable video memory size for non-NVIDIA cards (a 32-bit field that tops out at 4 GB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuClass {
    Discrete,
    Integrated,
    /// A software adapter, a remote-desktop display, or something unrecognised.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    pub name: String,
    pub vendor: Vendor,
    pub class: GpuClass,
    /// Known for NVIDIA cards only (from `nvidia-smi`).
    pub vram_mb: Option<u64>,
}

/// The newest CUDA version the installed NVIDIA driver can run, and its version string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvidiaDriver {
    pub driver: String,
    pub cuda: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hardware {
    pub gpus: Vec<Gpu>,
    pub nvidia: Option<NvidiaDriver>,
    pub ram_mb: Option<u64>,
    /// Free space on the drive that will hold the models.
    pub free_disk_mb: Option<u64>,
}

impl Hardware {
    /// The card that will run the model: the discrete one with the most memory, else the first usable one.
    pub fn best_gpu(&self) -> Option<&Gpu> {
        self.gpus
            .iter()
            .filter(|g| g.class != GpuClass::None)
            .max_by_key(|g| (g.class == GpuClass::Discrete, g.vendor == Vendor::Nvidia, g.vram_mb.unwrap_or(0)))
    }
}

pub fn vendor_of(name: &str) -> Vendor {
    let n = name.to_ascii_lowercase();
    if n.contains("nvidia") || n.contains("geforce") || n.contains("quadro") || n.contains("rtx") || n.contains("tesla") {
        Vendor::Nvidia
    } else if n.contains("amd") || n.contains("radeon") || n.contains("ati ") {
        Vendor::Amd
    } else if n.contains("intel") || n.contains(" arc") || n.contains("iris") || n.contains("uhd") {
        Vendor::Intel
    } else {
        Vendor::Other
    }
}

pub fn class_of(name: &str, vendor: Vendor) -> GpuClass {
    let n = name.to_ascii_lowercase();
    let software = ["microsoft basic", "microsoft remote", "remote display", "virtual", "vmware", "hyper-v", "citrix", "parsec", "idd"];
    if software.iter().any(|s| n.contains(s)) {
        return GpuClass::None;
    }
    match vendor {
        Vendor::Nvidia => GpuClass::Discrete,
        // "Radeon RX 7800 XT", "Radeon Pro W7800", "Radeon VII" have their own memory;
        // "Radeon Graphics", "Radeon(TM) Graphics" and "Vega" are the ones inside Ryzen processors.
        Vendor::Amd => {
            let discrete = n.contains(" rx ") || n.contains("radeon pro") || n.contains("radeon vii") || n.contains("instinct");
            if discrete { GpuClass::Discrete } else { GpuClass::Integrated }
        }
        // Arc cards (A380, A770, B580) have their own memory.
        Vendor::Intel => {
            if n.contains(" arc") { GpuClass::Discrete } else { GpuClass::Integrated }
        }
        Vendor::Other => GpuClass::None,
    }
}

/// One line of `nvidia-smi --query-gpu`: name, memory in MB, driver version.
pub type NvidiaCard = (String, Option<u64>, String);

/// `nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader,nounits`
pub fn parse_nvidia_query(out: &str) -> Vec<NvidiaCard> {
    out.lines()
        .filter_map(|line| {
            let mut parts = line.split(',').map(str::trim);
            let name = parts.next().filter(|n| !n.is_empty())?;
            let vram = parts.next().and_then(|m| m.parse::<u64>().ok());
            let driver = parts.next().unwrap_or("").to_string();
            Some((name.to_string(), vram, driver))
        })
        .collect()
}

/// The `CUDA Version: 12.4` in the banner `nvidia-smi` prints.
pub fn parse_cuda_version(banner: &str) -> Option<(u32, u32)> {
    let rest = banner.split("CUDA Version:").nth(1)?;
    let word = rest.split_whitespace().next()?;
    let (major, minor) = word.split_once('.')?;
    Some((major.trim().parse().ok()?, minor.trim().parse().ok()?))
}

/// What the PowerShell probe prints: `{"gpus":["NVIDIA GeForce RTX 4070"],"ram":34246...,"free":1.2e11}`.
/// A single card arrives as a plain string, and numbers may be written in exponent form.
pub fn parse_probe(json: &str) -> (Vec<String>, Option<u64>, Option<u64>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json.trim().trim_start_matches('\u{feff}')) else {
        return (Vec::new(), None, None);
    };
    let gpus = match &v["gpus"] {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    };
    let mb = |x: &serde_json::Value| x.as_f64().filter(|b| *b > 0.0).map(|b| (b / 1_048_576.0) as u64);
    (gpus, mb(&v["ram"]), mb(&v["free"]))
}

/// Combine what the two probes found.
pub fn assemble(nvidia: Vec<NvidiaCard>, cuda: Option<(u32, u32)>, adapters: Vec<String>, ram_mb: Option<u64>, free_disk_mb: Option<u64>) -> Hardware {
    let mut gpus: Vec<Gpu> = nvidia
        .iter()
        .map(|(name, vram, _)| Gpu { name: name.clone(), vendor: Vendor::Nvidia, class: GpuClass::Discrete, vram_mb: *vram })
        .collect();
    for name in adapters {
        let vendor = vendor_of(&name);
        // An NVIDIA card nvidia-smi already told us about (with its memory) is not listed twice.
        if vendor == Vendor::Nvidia && !nvidia.is_empty() {
            continue;
        }
        gpus.push(Gpu { class: class_of(&name, vendor), vendor, vram_mb: None, name });
    }
    let nvidia = nvidia.first().map(|(_, _, driver)| NvidiaDriver { driver: driver.clone(), cuda });
    Hardware { gpus, nvidia, ram_mb, free_disk_mb }
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn drive_letter(dir: &Path) -> Option<char> {
    let text = dir.to_string_lossy();
    let mut chars = text.trim_start_matches(r"\\?\").chars();
    let letter = chars.next()?;
    (letter.is_ascii_alphabetic() && chars.next() == Some(':')).then_some(letter.to_ascii_uppercase())
}

fn nvidia_probe() -> (Vec<NvidiaCard>, Option<(u32, u32)>) {
    let query = run("nvidia-smi", &["--query-gpu=name,memory.total,driver_version", "--format=csv,noheader,nounits"]).map(|o| parse_nvidia_query(&o)).unwrap_or_default();
    let cuda = if query.is_empty() { None } else { run("nvidia-smi", &[]).and_then(|b| parse_cuda_version(&b)) };
    (query, cuda)
}

/// The installed NVIDIA driver and the newest CUDA it can run, if `nvidia-smi` answers.
pub fn nvidia_driver() -> Option<NvidiaDriver> {
    let (query, cuda) = nvidia_probe();
    query.first().map(|(_, _, driver)| NvidiaDriver { driver: driver.clone(), cuda })
}

/// Look at this PC. `models_dir` says which drive's free space matters.
pub fn detect(models_dir: &Path) -> Hardware {
    let (query, cuda) = nvidia_probe();

    let (adapters, ram_mb, free_mb) = if cfg!(windows) {
        let free = match drive_letter(models_dir) {
            Some(d) => format!("(Get-PSDrive -Name {d}).Free"),
            None => "$null".to_string(),
        };
        let script = format!(
            "$ErrorActionPreference='SilentlyContinue'; ConvertTo-Json -Compress @{{ gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object {{ $_.Name }}); ram = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory; free = {free} }}"
        );
        run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script]).map(|o| parse_probe(&o)).unwrap_or_default()
    } else {
        (Vec::new(), linux_ram_mb(), None)
    };
    assemble(query, cuda, adapters, ram_mb, free_mb)
}

fn linux_ram_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = text.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.split_whitespace().next()?.parse().ok()?;
    Some(kb / 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_smi_query_lines_are_read() {
        let out = "NVIDIA GeForce RTX 4070, 12282, 552.44\nNVIDIA GeForce GTX 1050, 2048, 552.44\n";
        let q = parse_nvidia_query(out);
        assert_eq!(q, vec![("NVIDIA GeForce RTX 4070".into(), Some(12282), "552.44".into()), ("NVIDIA GeForce GTX 1050".into(), Some(2048), "552.44".into())]);
        assert!(parse_nvidia_query("NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver.").len() <= 1, "an error text is at worst one bogus name, never a crash");
        assert_eq!(parse_nvidia_query("").len(), 0);
        assert_eq!(parse_nvidia_query("GeForce RTX 3060, [N/A], 551.23")[0].1, None, "no number, no memory");
    }

    #[test]
    fn the_cuda_version_is_read_from_the_banner() {
        let banner = "| NVIDIA-SMI 552.44                 Driver Version: 552.44         CUDA Version: 12.4     |";
        assert_eq!(parse_cuda_version(banner), Some((12, 4)));
        assert_eq!(parse_cuda_version("| Driver Version: 581.15  CUDA Version: 13.0 |"), Some((13, 0)));
        assert_eq!(parse_cuda_version("no banner here"), None);
    }

    #[test]
    fn the_probe_output_is_read_with_one_card_or_many() {
        let many = r#"{"gpus":["Intel(R) UHD Graphics 770","NVIDIA GeForce RTX 4070"],"ram":34262933504,"free":1.2e11}"#;
        let (gpus, ram, free) = parse_probe(many);
        assert_eq!(gpus.len(), 2);
        assert_eq!(ram, Some(32675));
        assert_eq!(free, Some(114440));
        let one = parse_probe("\u{feff}{\"gpus\":\"AMD Radeon RX 7800 XT\",\"ram\":17179869184,\"free\":null}");
        assert_eq!(one.0, vec!["AMD Radeon RX 7800 XT".to_string()]);
        assert_eq!((one.1, one.2), (Some(16384), None));
        assert_eq!(parse_probe("garbage"), (Vec::new(), None, None));
    }

    #[test]
    fn cards_are_classified_by_name() {
        let class = |n: &str| class_of(n, vendor_of(n));
        assert_eq!(class("NVIDIA GeForce RTX 4070"), GpuClass::Discrete);
        assert_eq!(class("AMD Radeon RX 7800 XT"), GpuClass::Discrete);
        assert_eq!(class("AMD Radeon(TM) Graphics"), GpuClass::Integrated, "the one inside Ryzen");
        assert_eq!(class("AMD Radeon(TM) Vega 8 Graphics"), GpuClass::Integrated);
        assert_eq!(class("Intel(R) UHD Graphics 770"), GpuClass::Integrated);
        assert_eq!(class("Intel(R) Iris(R) Xe Graphics"), GpuClass::Integrated);
        assert_eq!(class("Intel(R) Arc(TM) A770 Graphics"), GpuClass::Discrete);
        assert_eq!(class("Microsoft Basic Display Adapter"), GpuClass::None);
        assert_eq!(class("Parsec Virtual Display Adapter"), GpuClass::None);
        assert_eq!(class("Some Unknown Adapter"), GpuClass::None);
    }

    #[test]
    fn nvidia_is_listed_once_and_the_discrete_card_is_preferred_over_the_integrated_one() {
        let hw = assemble(
            vec![("NVIDIA GeForce RTX 4070".into(), Some(12282), "552.44".into())],
            Some((12, 4)),
            vec!["Intel(R) UHD Graphics 770".into(), "NVIDIA GeForce RTX 4070".into()],
            Some(32768),
            Some(500_000),
        );
        assert_eq!(hw.gpus.len(), 2);
        let best = hw.best_gpu().unwrap();
        assert_eq!((best.vendor, best.vram_mb), (Vendor::Nvidia, Some(12282)));
        assert_eq!(hw.nvidia, Some(NvidiaDriver { driver: "552.44".into(), cuda: Some((12, 4)) }));
    }

    #[test]
    fn software_adapters_are_never_chosen() {
        let hw = assemble(vec![], None, vec!["Microsoft Basic Display Adapter".into()], Some(8192), None);
        assert!(hw.best_gpu().is_none());
    }

    #[test]
    fn the_drive_is_taken_from_the_models_folder() {
        assert_eq!(drive_letter(Path::new(r"d:\Maru\models")), Some('D'));
        assert_eq!(drive_letter(Path::new(r"\\?\C:\Users\me")), Some('C'));
        assert_eq!(drive_letter(Path::new("/home/me")), None);
    }
}
