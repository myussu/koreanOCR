//! Resource readings describe processes, not model weights alone. Missing measurements stay missing.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessTarget {
    pub role: String,
    pub name: String,
    pub state: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessUsage {
    pub target: ProcessTarget,
    pub cpu_percent: Option<f64>,
    pub ram_bytes: Option<u64>,
    pub dedicated_gpu_bytes: Option<u64>,
    pub shared_gpu_bytes: Option<u64>,
    /// Busiest engine for this process, as in Task Manager; not an additive share of the GPU.
    pub gpu_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuUsage {
    pub id: String,
    pub name: String,
    pub dedicated_total_bytes: u64,
    pub dedicated_used_bytes: Option<u64>,
    pub gpu_percent: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ResourceSample {
    pub at_ms: u64,
    pub collection_ms: f64,
    pub processes: Vec<ProcessUsage>,
    pub gpus: Vec<GpuUsage>,
    pub notes: Vec<String>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
