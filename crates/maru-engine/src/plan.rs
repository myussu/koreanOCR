//! What to install on a given PC: the choice the setup wizard makes (and the app's first-run page will).
//!
//! [`recommend`] turns a [`Hardware`] description into a [`Plan`] with the reasons spelled out, so the
//! person can see why and override it. No I/O here.

use serde_json::{Map, Value};

use crate::hardware::{Gpu, GpuClass, Hardware, Vendor};

/// Which llama.cpp build runs the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cuda,
    Vulkan,
    Cpu,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::Cuda => "CUDA (NVIDIA)",
            Backend::Vulkan => "Vulkan (AMD, Intel, or NVIDIA without CUDA)",
            Backend::Cpu => "CPU only",
        }
    }
}

/// Video memory the full 12B model needs next to the OCR reading model.
pub const FULL_MODEL_VRAM_MB: u64 = 10_000;
/// Below this the small model does not fit either, and the CPU is the safer place for it.
pub const SMALL_MODEL_VRAM_MB: u64 = 4_000;
/// The oldest CUDA llama.cpp publishes builds for.
pub const OLDEST_CUDA_BUILD: (u32, u32) = (12, 4);
/// RTX 50-series (Blackwell) cards need a build made with at least this CUDA.
pub const BLACKWELL_CUDA: (u32, u32) = (12, 8);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub backend: Backend,
    /// The smaller translation model and no OCR reading model (`lite = true`).
    pub lite: bool,
    /// Install PaddleOCR-VL, the reading model that fixes misread letters.
    pub ocr_vl: bool,
    /// Why, one line per decision.
    pub reasons: Vec<String>,
    /// Things that may go wrong on this PC.
    pub warnings: Vec<String>,
}

fn at_least(have: (u32, u32), need: (u32, u32)) -> bool {
    have >= need
}

fn is_blackwell(gpu: &Gpu) -> bool {
    let n = gpu.name.to_ascii_lowercase();
    ["rtx 50", "rtx pro 6000", "rtx pro 5000", "rtx pro 4500", "rtx pro 4000", "b200", "gb200"].iter().any(|p| n.contains(p))
}

pub fn recommend(hw: &Hardware) -> Plan {
    let mut reasons = Vec::new();
    let mut warnings = Vec::new();

    let gpu = hw.best_gpu();
    let (backend, mut lite) = match gpu {
        Some(g) if g.vendor == Vendor::Nvidia => match &hw.nvidia {
            Some(driver) => {
                let cuda = driver.cuda;
                if cuda.is_some_and(|c| !at_least(c, OLDEST_CUDA_BUILD)) {
                    reasons.push(format!("{}: the driver ({}) only supports CUDA {}.{}, older than the oldest build ({}.{}).", g.name, driver.driver, cuda.unwrap().0, cuda.unwrap().1, OLDEST_CUDA_BUILD.0, OLDEST_CUDA_BUILD.1));
                    warnings.push("Updating the NVIDIA driver would let Maru use CUDA, which is faster than Vulkan.".to_string());
                    (Backend::Vulkan, false)
                } else if is_blackwell(g) && cuda.is_some_and(|c| !at_least(c, BLACKWELL_CUDA)) {
                    reasons.push(format!("{} needs CUDA {}.{} or newer and the driver ({}) is older.", g.name, BLACKWELL_CUDA.0, BLACKWELL_CUDA.1, driver.driver));
                    warnings.push("Updating the NVIDIA driver would let Maru use CUDA, which is faster than Vulkan.".to_string());
                    (Backend::Vulkan, false)
                } else {
                    reasons.push(format!("{} with driver {}: CUDA is the fastest way to run the model.", g.name, driver.driver));
                    (Backend::Cuda, false)
                }
            }
            None => {
                reasons.push(format!("{} found, but `nvidia-smi` does not answer, so the CUDA driver cannot be confirmed: Vulkan works without it.", g.name));
                (Backend::Vulkan, false)
            }
        },
        Some(g) if g.class == GpuClass::Discrete => {
            reasons.push(format!("{}: Vulkan runs on it with nothing extra installed.", g.name));
            (Backend::Vulkan, false)
        }
        Some(g) => {
            reasons.push(format!("{} shares the computer's memory, which makes it no faster than the processor for this: using the CPU.", g.name));
            (Backend::Cpu, true)
        }
        None => {
            reasons.push("No graphics card that can run the model was found: using the CPU.".to_string());
            (Backend::Cpu, true)
        }
    };

    // How big a model fits.
    match (backend, gpu.and_then(|g| g.vram_mb)) {
        (Backend::Cpu, _) => {
            reasons.push("The small model (`lite = true`) keeps answers to a few seconds on a processor.".to_string());
        }
        (_, Some(vram)) if vram >= FULL_MODEL_VRAM_MB => {
            reasons.push(format!("{:.0} GB of video memory is enough for the full 12B model and the OCR reading model.", vram as f64 / 1024.0));
        }
        (_, Some(vram)) if vram >= SMALL_MODEL_VRAM_MB => {
            lite = true;
            reasons.push(format!("{:.0} GB of video memory is not enough for the full 12B model: using the small one.", vram as f64 / 1024.0));
        }
        (_, Some(vram)) => {
            lite = true;
            reasons.push(format!("{:.0} GB of video memory is too little even for the small model: it will spill into system memory.", vram as f64 / 1024.0));
            warnings.push("Expect slow answers; a model on another PC (Settings → Translation model) would be faster.".to_string());
        }
        (_, None) => {
            lite = true;
            reasons.push("Windows does not report this card's memory, so the small model is the safe choice.".to_string());
            warnings.push("If the card has 12 GB or more, turn `lite` off in Settings for the full model.".to_string());
        }
    }

    let (mut backend, mut ram_note) = (backend, None);
    if let Some(ram) = hw.ram_mb {
        if ram < 6_000 {
            ram_note = Some(format!("Only {:.0} GB of memory: the model may not fit, and the computer will swap. Consider a model on another PC (Settings → Translation model).", ram as f64 / 1024.0));
            lite = true;
            if backend != Backend::Cpu && gpu.and_then(|g| g.vram_mb).is_none() {
                backend = Backend::Cpu;
            }
        } else if ram < 12_000 && backend == Backend::Cpu {
            ram_note = Some(format!("{:.0} GB of memory is tight for a model plus Windows; close other programs while translating.", ram as f64 / 1024.0));
        }
    }
    warnings.extend(ram_note);

    let ocr_vl = !lite;
    Plan { backend, lite, ocr_vl, reasons, warnings }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// llama.cpp and the translation model.
    Llm,
    Kiwi,
    Ocr,
    OcrVl,
    Dictionary,
    Tts,
}

impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Step::Llm => "Translation model (llama.cpp and the model)",
            Step::Kiwi => "Korean grammar analyser (Kiwi)",
            Step::Ocr => "OCR models",
            Step::OcrVl => "OCR reading model (PaddleOCR-VL)",
            Step::Dictionary => "Offline dictionary (Wiktionary)",
            Step::Tts => "Korean voice (Qwen3-TTS)",
        }
    }

    /// An estimate of what the step downloads, in MB. Shown as "about", never relied on.
    pub fn download_mb(self, plan: &Plan) -> u64 {
        match self {
            Step::Llm => {
                let runtime = match plan.backend {
                    Backend::Cuda => 900,
                    Backend::Vulkan => 80,
                    Backend::Cpu => 40,
                };
                runtime + if plan.lite { 5_000 } else { 7_500 }
            }
            Step::Kiwi => 150,
            Step::Ocr => 150,
            Step::OcrVl => 1_800,
            Step::Dictionary => 600,
            Step::Tts => 2_500,
        }
    }
}

/// The steps to run, in order. The model comes first so a failure there stops the wizard before the
/// smaller downloads.
pub fn steps(plan: &Plan, ocr_vl: bool, tts: bool) -> Vec<Step> {
    let mut out = vec![Step::Llm, Step::Kiwi, Step::Ocr];
    if ocr_vl && !plan.lite {
        out.push(Step::OcrVl);
    }
    out.push(Step::Dictionary);
    if tts {
        out.push(Step::Tts);
    }
    out
}

pub fn total_download_mb(steps: &[Step], plan: &Plan) -> u64 {
    steps.iter().map(|s| s.download_mb(plan)).sum()
}

/// A warning if the downloads (plus room to unpack and work) do not fit in the free space.
pub fn disk_warning(total_mb: u64, free_mb: Option<u64>) -> Option<String> {
    let free = free_mb?;
    let needed = total_mb + total_mb / 5 + 1_000;
    (free < needed).then(|| format!("About {:.1} GB is needed and only {:.1} GB is free on the drive that will hold the models.", needed as f64 / 1024.0, free as f64 / 1024.0))
}

/// The config settings the plan changes, as `settings::patch` takes them.
pub fn config_changes(plan: &Plan) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("lite".into(), Value::Bool(plan.lite));
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{Gpu, NvidiaDriver};

    fn nvidia(name: &str, vram: u64, cuda: Option<(u32, u32)>) -> Hardware {
        Hardware {
            gpus: vec![Gpu { name: name.into(), vendor: Vendor::Nvidia, class: GpuClass::Discrete, vram_mb: Some(vram) }],
            nvidia: Some(NvidiaDriver { driver: "552.44".into(), cuda }),
            ram_mb: Some(32_000),
            free_disk_mb: Some(500_000),
        }
    }

    fn other(name: &str, vendor: Vendor, class: GpuClass, ram: u64) -> Hardware {
        Hardware { gpus: vec![Gpu { name: name.into(), vendor, class, vram_mb: None }], nvidia: None, ram_mb: Some(ram), free_disk_mb: Some(500_000) }
    }

    #[test]
    fn a_big_nvidia_card_gets_cuda_the_full_model_and_the_reading_model() {
        let plan = recommend(&nvidia("NVIDIA GeForce RTX 4070", 12_282, Some((12, 4))));
        assert_eq!((plan.backend, plan.lite, plan.ocr_vl), (Backend::Cuda, false, true));
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn a_smaller_nvidia_card_keeps_cuda_but_gets_the_small_model() {
        let plan = recommend(&nvidia("NVIDIA GeForce RTX 3060 Laptop GPU", 6_144, Some((12, 6))));
        assert_eq!((plan.backend, plan.lite, plan.ocr_vl), (Backend::Cuda, true, false));
    }

    #[test]
    fn a_tiny_nvidia_card_is_warned() {
        let plan = recommend(&nvidia("NVIDIA GeForce GTX 1050", 2_048, Some((12, 4))));
        assert!(plan.lite);
        assert!(plan.warnings.iter().any(|w| w.contains("another PC")));
    }

    #[test]
    fn an_old_driver_falls_back_to_vulkan_and_says_to_update() {
        let plan = recommend(&nvidia("NVIDIA GeForce GTX 1660", 6_144, Some((11, 8))));
        assert_eq!(plan.backend, Backend::Vulkan);
        assert!(plan.warnings.iter().any(|w| w.contains("driver")));
    }

    #[test]
    fn a_new_card_needs_a_new_enough_driver() {
        assert_eq!(recommend(&nvidia("NVIDIA GeForce RTX 5070", 12_227, Some((12, 6)))).backend, Backend::Vulkan);
        assert_eq!(recommend(&nvidia("NVIDIA GeForce RTX 5070", 12_227, Some((12, 8)))).backend, Backend::Cuda);
        assert_eq!(recommend(&nvidia("NVIDIA GeForce RTX 5070", 12_227, Some((13, 0)))).backend, Backend::Cuda);
        // an unknown CUDA version is not held against the card
        assert_eq!(recommend(&nvidia("NVIDIA GeForce RTX 4070", 12_282, None)).backend, Backend::Cuda);
    }

    #[test]
    fn an_nvidia_card_without_a_working_nvidia_smi_gets_vulkan() {
        let hw = Hardware { gpus: vec![Gpu { name: "NVIDIA GeForce RTX 2060".into(), vendor: Vendor::Nvidia, class: GpuClass::Discrete, vram_mb: None }], nvidia: None, ram_mb: Some(16_000), free_disk_mb: None };
        let plan = recommend(&hw);
        assert_eq!(plan.backend, Backend::Vulkan);
        assert!(plan.lite, "memory unknown");
    }

    #[test]
    fn an_amd_or_intel_graphics_card_gets_vulkan_and_the_small_model() {
        let plan = recommend(&other("AMD Radeon RX 7800 XT", Vendor::Amd, GpuClass::Discrete, 32_000));
        assert_eq!((plan.backend, plan.lite), (Backend::Vulkan, true));
        assert!(plan.warnings.iter().any(|w| w.contains("lite")), "it says how to get the full model");
        assert_eq!(recommend(&other("Intel(R) Arc(TM) A770 Graphics", Vendor::Intel, GpuClass::Discrete, 32_000)).backend, Backend::Vulkan);
    }

    #[test]
    fn integrated_graphics_or_no_card_means_the_cpu_and_the_small_model() {
        for (name, vendor, class) in [("Intel(R) UHD Graphics 770", Vendor::Intel, GpuClass::Integrated), ("AMD Radeon(TM) Graphics", Vendor::Amd, GpuClass::Integrated)] {
            let plan = recommend(&other(name, vendor, class, 16_000));
            assert_eq!((plan.backend, plan.lite, plan.ocr_vl), (Backend::Cpu, true, false), "{name}");
        }
        let none = recommend(&Hardware { ram_mb: Some(16_000), ..Hardware::default() });
        assert_eq!((none.backend, none.lite), (Backend::Cpu, true));
        let basic = recommend(&other("Microsoft Basic Display Adapter", Vendor::Other, GpuClass::None, 16_000));
        assert_eq!(basic.backend, Backend::Cpu);
    }

    #[test]
    fn very_little_memory_is_warned_about() {
        let plan = recommend(&other("Intel(R) UHD Graphics 600", Vendor::Intel, GpuClass::Integrated, 4_096));
        assert!(plan.warnings.iter().any(|w| w.contains("4 GB")));
        let tight = recommend(&other("Intel(R) UHD Graphics 600", Vendor::Intel, GpuClass::Integrated, 8_192));
        assert!(tight.warnings.iter().any(|w| w.contains("tight")));
        assert!(recommend(&other("Intel(R) UHD Graphics 600", Vendor::Intel, GpuClass::Integrated, 16_384)).warnings.is_empty());
    }

    #[test]
    fn the_steps_follow_the_plan() {
        let full = recommend(&nvidia("NVIDIA GeForce RTX 4070", 12_282, Some((12, 4))));
        assert_eq!(steps(&full, true, false), [Step::Llm, Step::Kiwi, Step::Ocr, Step::OcrVl, Step::Dictionary]);
        assert_eq!(steps(&full, false, true), [Step::Llm, Step::Kiwi, Step::Ocr, Step::Dictionary, Step::Tts]);
        let lite = recommend(&other("Intel(R) UHD Graphics 770", Vendor::Intel, GpuClass::Integrated, 16_000));
        assert!(!steps(&lite, true, false).contains(&Step::OcrVl), "the reading model is not part of a lite install");
    }

    #[test]
    fn download_sizes_add_up_and_the_disk_is_checked() {
        let full = recommend(&nvidia("NVIDIA GeForce RTX 4070", 12_282, Some((12, 4))));
        let all = steps(&full, true, true);
        let total = total_download_mb(&all, &full);
        assert_eq!(total, 900 + 7_500 + 150 + 150 + 1_800 + 600 + 2_500);
        assert!(disk_warning(total, Some(500_000)).is_none());
        assert!(disk_warning(total, Some(8_000)).unwrap().contains("free"));
        assert!(disk_warning(total, None).is_none(), "unknown free space is not a warning");
    }

    #[test]
    fn the_plan_turns_lite_on_or_off_in_the_config() {
        let full = recommend(&nvidia("NVIDIA GeForce RTX 4070", 12_282, Some((12, 4))));
        assert_eq!(config_changes(&full)["lite"], Value::Bool(false));
        let lite = recommend(&Hardware::default());
        assert_eq!(config_changes(&lite)["lite"], Value::Bool(true));
    }
}
