//! Windows PDH counters use English paths even on localized installations; DXGI supplies adapter capacities.
use crate::resources::{ResourceMonitor, cpu_percent, instance_adapter, instance_pid, missing};
use maru_core::resources::{GpuUsage, ProcessTarget, ResourceSample, now_ms};
use std::{collections::HashMap, time::Instant};
use windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME},
        Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1},
        System::{
            Performance::*,
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{ALL_PROCESSOR_GROUPS, GetActiveProcessorCount, GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
        },
    },
    core::PCWSTR,
};

pub struct WinResources {
    query: PDH_HQUERY,
    counters: [Option<PDH_HCOUNTER>; 4],
    adapters: Vec<GpuUsage>,
    previous: HashMap<u32, (u64, u64)>,
    last: Instant,
    cpus: usize,
}

impl WinResources {
    pub fn new() -> Self {
        let mut query = PDH_HQUERY::default();
        let mut counters = [None; 4];
        unsafe {
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) == 0 {
                for (slot, path) in counters.iter_mut().zip([
                    "\\GPU Adapter Memory(*)\\Dedicated Usage",
                    "\\GPU Process Memory(*)\\Dedicated Usage",
                    "\\GPU Process Memory(*)\\Shared Usage",
                    "\\GPU Engine(*)\\Utilization Percentage",
                ]) {
                    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
                    let mut counter = PDH_HCOUNTER::default();
                    if PdhAddEnglishCounterW(query, PCWSTR(wide.as_ptr()), 0, &mut counter) == 0 {
                        *slot = Some(counter);
                    }
                }
                PdhCollectQueryData(query);
            }
        }
        Self {
            query,
            counters,
            adapters: adapters(),
            previous: HashMap::new(),
            last: Instant::now(),
            cpus: (unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) } as usize).max(1),
        }
    }
}

impl Drop for WinResources {
    fn drop(&mut self) {
        if !self.query.is_invalid() {
            unsafe {
                PdhCloseQuery(self.query);
            }
        }
    }
}

fn adapters() -> Vec<GpuUsage> {
    let mut out = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        for i in 0..32 {
            let Ok(adapter) = factory.EnumAdapters1(i) else { break };
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if desc.Flags & 2 != 0 {
                continue;
            }
            let len = desc.Description.iter().position(|v| *v == 0).unwrap_or(desc.Description.len());
            out.push(GpuUsage {
                id: format!("luid_0x{:08x}_0x{:08x}", desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart),
                name: String::from_utf16_lossy(&desc.Description[..len]),
                dedicated_total_bytes: desc.DedicatedVideoMemory as u64,
                dedicated_used_bytes: None,
                gpu_percent: None,
            });
        }
    }
    out
}

fn values(counter: Option<PDH_HCOUNTER>) -> Option<Vec<(String, f64)>> {
    let counter = counter?;
    let mut bytes = 0;
    let mut count = 0;
    unsafe {
        let status = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, None);
        if status != PDH_MORE_DATA {
            return if status == 0 { Some(Vec::new()) } else { None };
        }
        // Instances can appear between the size and data calls. Retry with a bounded, aligned buffer.
        for _ in 0..3 {
            if bytes > 8 * 1024 * 1024 {
                return None;
            }
            let mut buffer = vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); (bytes as usize).div_ceil(std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>())];
            let status = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut bytes, &mut count, Some(buffer.as_mut_ptr()));
            if status == PDH_MORE_DATA {
                continue;
            }
            if status != 0 || count as usize > buffer.len() {
                return None;
            }
            let mut out = Vec::new();
            for item in &buffer[..count as usize] {
                let value = item.FmtValue.Anonymous.doubleValue;
                if matches!(item.FmtValue.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA) && value.is_finite() && value >= 0.0 && !item.szName.is_null()
                {
                    out.push((item.szName.to_string().ok()?, value));
                }
            }
            return if count > 0 && out.is_empty() { None } else { Some(out) };
        }
    }
    None
}

fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}
fn process(pid: u32) -> Option<((u64, u64), Option<u64>)> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let time = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
        let mut memory = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        let ram = GetProcessMemoryInfo(handle, &mut memory, std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32)
            .ok()
            .map(|_| memory.WorkingSetSize as u64);
        let _ = CloseHandle(handle);
        time.ok()?;
        // An exited/reused PID must not be mistaken for a running owned model.
        if ticks(exited) != 0 {
            return None;
        }
        Some(((ticks(created), ticks(kernel).saturating_add(ticks(user))), ram))
    }
}

fn memory_for(values: &Option<Vec<(String, f64)>>, pid: u32) -> Option<u64> {
    Some(
        values
            .as_ref()?
            .iter()
            .filter(|(name, _)| instance_pid(name) == Some(pid))
            .map(|(_, value)| *value as u64)
            .sum(),
    )
}

impl ResourceMonitor for WinResources {
    fn sample(&mut self, targets: Vec<ProcessTarget>) -> ResourceSample {
        let began = Instant::now();
        let seconds = began.duration_since(self.last).as_secs_f64();
        self.last = began;
        let collected = !self.query.is_invalid() && unsafe { PdhCollectQueryData(self.query) == 0 };
        let readings: Vec<_> = self.counters.iter().map(|counter| if collected { values(*counter) } else { None }).collect();
        let mut out = ResourceSample {
            at_ms: now_ms(),
            gpus: self.adapters.clone(),
            ..Default::default()
        };
        for target in targets {
            let mut usage = missing(target);
            if let Some(pid) = usage.target.pid
                && let Some((time, ram)) = process(pid)
            {
                usage.cpu_percent = self.previous.insert(pid, time).and_then(|old| cpu_percent(old, time, seconds, self.cpus));
                usage.ram_bytes = ram;
                usage.dedicated_gpu_bytes = memory_for(&readings[1], pid);
                usage.shared_gpu_bytes = memory_for(&readings[2], pid);
                usage.gpu_percent = readings[3].as_ref().map(|v| {
                    v.iter()
                        .filter(|(n, _)| instance_pid(n) == Some(pid))
                        .map(|(_, v)| *v)
                        .fold(0.0, f64::max)
                        .min(100.0)
                });
            }
            out.processes.push(usage);
        }
        self.previous.retain(|pid, _| out.processes.iter().any(|p| p.target.pid == Some(*pid)));
        for gpu in &mut out.gpus {
            gpu.dedicated_used_bytes = readings[0].as_ref().and_then(|v| {
                let matching: Vec<_> = v.iter().filter(|(n, _)| instance_adapter(n) == Some(gpu.id.as_str())).collect();
                (!matching.is_empty()).then(|| matching.iter().map(|(_, v)| *v as u64).sum())
            });
            gpu.gpu_percent = readings[3].as_ref().map(|v| {
                let mut engines = HashMap::<String, f64>::new();
                for (name, value) in v.iter().filter(|(n, _)| instance_adapter(n) == Some(gpu.id.as_str())) {
                    if let Some((_, engine)) = name.split_once("_eng_") {
                        *engines.entry(engine.into()).or_default() += value;
                    }
                }
                engines.values().copied().fold(0.0, f64::max).min(100.0)
            });
        }
        if readings[0].is_none() {
            out.notes
                .push("Windows GPU memory counters are unavailable; VRAM readings cannot be shown.".into());
        }
        if readings[3].is_none() {
            out.notes.push("GPU activity needs two valid samples or is unsupported by this driver.".into());
        }
        out.collection_ms = began.elapsed().as_secs_f64() * 1000.0;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "reads real Windows resource counters; run explicitly for diagnostics"]
    fn real_windows_counters_read_the_current_process_without_starting_a_model() {
        let mut sampler = WinResources::new();
        let target = ProcessTarget {
            role: "app".into(),
            name: "test".into(),
            state: "running".into(),
            pid: Some(std::process::id()),
        };
        sampler.sample(vec![target.clone()]);
        std::thread::sleep(std::time::Duration::from_secs(1));
        let sample = sampler.sample(vec![target]);
        assert!(sample.processes[0].ram_bytes.is_some_and(|ram| ram > 0));
        assert!(sample.processes[0].cpu_percent.is_some());
        println!("{sample:?}");
    }
}
