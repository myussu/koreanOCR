//! A sampler is created and used on one background thread; no external monitoring processes.
use maru_core::resources::{ProcessTarget, ProcessUsage, ResourceSample, now_ms};

pub trait ResourceMonitor {
    fn sample(&mut self, targets: Vec<ProcessTarget>) -> ResourceSample;
}

pub fn native() -> Box<dyn ResourceMonitor> {
    #[cfg(windows)]
    {
        Box::new(crate::windows::resources::WinResources::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(Unavailable)
    }
}

pub fn missing(target: ProcessTarget) -> ProcessUsage {
    ProcessUsage {
        target,
        cpu_percent: None,
        ram_bytes: None,
        dedicated_gpu_bytes: None,
        shared_gpu_bytes: None,
        gpu_percent: None,
    }
}

pub struct Unavailable;
impl ResourceMonitor for Unavailable {
    fn sample(&mut self, targets: Vec<ProcessTarget>) -> ResourceSample {
        ResourceSample {
            at_ms: now_ms(),
            processes: targets.into_iter().map(missing).collect(),
            notes: vec!["Resource counters are unavailable on this operating system.".into()],
            ..Default::default()
        }
    }
}

/// CPU is normalized to all logical CPUs; creation time prevents spikes after PID reuse.
pub fn cpu_percent(previous: (u64, u64), current: (u64, u64), seconds: f64, cpus: usize) -> Option<f64> {
    if previous.0 != current.0 || seconds <= 0.0 || cpus == 0 {
        return None;
    }
    Some((current.1.checked_sub(previous.1)? as f64 / 10_000_000.0 / seconds / cpus as f64 * 100.0).clamp(0.0, 100.0))
}

pub fn instance_pid(name: &str) -> Option<u32> {
    name.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}
pub fn instance_adapter(name: &str) -> Option<&str> {
    let start = name.find("luid_")?;
    let part = &name[start..];
    let end = part.find("_phys_")?;
    Some(&part[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_samples_are_normalized_and_pid_reuse_is_not_a_spike() {
        assert_eq!(cpu_percent((1, 0), (1, 10_000_000), 1.0, 8), Some(12.5));
        assert_eq!(cpu_percent((1, 0), (2, 10_000_000), 1.0, 8), None);
        assert_eq!(cpu_percent((1, 20), (1, 10), 1.0, 8), None);
        assert_eq!(cpu_percent((1, 0), (1, 10), 0.0, 8), None);
    }
    #[test]
    fn gpu_instances_match_by_pid_and_adapter_instead_of_process_name() {
        let name = "pid_42_luid_0x00000000_0x00016073_phys_0_eng_1_engtype_Compute";
        assert_eq!(instance_pid(name), Some(42));
        assert_eq!(instance_adapter(name), Some("luid_0x00000000_0x00016073"));
        assert_eq!(instance_pid("unrelated"), None);
        assert_eq!(instance_adapter("pid_42"), None);
    }
}
