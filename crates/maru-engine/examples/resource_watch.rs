//! Isolated benchmark telemetry using the same counters as Maru's Resources page.
use maru_core::resources::ProcessTarget;
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pids = std::env::args()
        .skip(1)
        .map(|s| s.parse::<u32>())
        .collect::<Result<Vec<_>, _>>()?;
    if pids.is_empty() {
        return Err("Pass one or more owned benchmark process IDs.".into());
    }
    let mut monitor = maru_platform::resources::native();
    loop {
        let targets = pids
            .iter()
            .map(|&pid| ProcessTarget {
                role: "benchmark".into(),
                name: format!("owned process {pid}"),
                state: "running".into(),
                pid: Some(pid),
            })
            .collect();
        println!("{}", serde_json::to_string(&monitor.sample(targets))?);
        std::io::stdout().flush()?;
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
