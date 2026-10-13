//! Bounded, cancellable metadata searches. No configuration or model runtime is touched.
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, MutexGuard};
use serde::Serialize;
use tokio::task::JoinSet;
use tokio_util::task::AbortOnDropHandle;
use maru_llm::discovery::{COMMON_PORTS, FoundServer, ServerProbe, candidates};

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> { value.lock().unwrap_or_else(|e| e.into_inner()) }

#[derive(Clone, Debug, Serialize)]
pub struct DiscoveryProgress {
    pub checked: usize, pub total: usize, pub done: bool, pub cancelled: bool,
    pub servers: Vec<FoundServer>, pub errors: Vec<String>,
}
struct Job { id: u32, progress: Arc<Mutex<DiscoveryProgress>>, task: AbortOnDropHandle<()> }
#[derive(Default)]
pub struct DiscoveryService { job: Mutex<Option<Job>> }

impl DiscoveryService {
    pub fn start(&self, kind: &str, target: &str) -> Result<u32, String> {
        let endpoints = match kind {
            "host" => candidates(target)?,
            "lan" => lan_hosts(target)?.into_iter().flat_map(|ip| COMMON_PORTS.map(|port| format!("http://{ip}:{port}"))).collect(),
            _ => return Err("Unknown discovery mode.".into()),
        };
        self.start_endpoints(endpoints, kind == "host")
    }

    fn start_endpoints(&self, endpoints: Vec<String>, keep_errors: bool) -> Result<u32, String> {
        let probe = ServerProbe::new()?;
        let mut current = lock(&self.job);
        if current.as_ref().is_some_and(|j| !lock(&j.progress).done) { return Err("A server search is already running. Cancel it or wait for it to finish.".into()); }
        let id = current.as_ref().map_or(1, |j| j.id.wrapping_add(1));
        let progress = Arc::new(Mutex::new(DiscoveryProgress { checked: 0, total: endpoints.len(), done: false, cancelled: false, servers: vec![], errors: vec![] }));
        let state = progress.clone();
        let task = tokio::spawn(async move {
            let mut endpoints = endpoints.into_iter();
            let mut requests = JoinSet::new();
            loop {
                while requests.len() < 16 {
                    let Some(endpoint) = endpoints.next() else { break; };
                    let probe = probe.clone();
                    requests.spawn(async move { probe.check(&endpoint).await });
                }
                let Some(result) = requests.join_next().await else { break; };
                let mut state = lock(&state);
                state.checked += 1;
                match result {
                    Ok(Ok(server)) => state.servers.push(server),
                    Ok(Err(error)) if keep_errors => state.errors.push(error),
                    Err(error) if keep_errors => state.errors.push(error.to_string()),
                    _ => {}
                }
                state.servers.sort_by(|a,b| a.endpoint.cmp(&b.endpoint));
            }
            lock(&state).done = true;
        });
        *current = Some(Job { id, progress, task: AbortOnDropHandle::new(task) });
        Ok(id)
    }

    pub fn status(&self, id: u32) -> Result<DiscoveryProgress, String> {
        let current = lock(&self.job);
        let job = current.as_ref().filter(|j| j.id == id).ok_or("This server search is no longer current.")?;
        Ok(lock(&job.progress).clone())
    }

    pub fn cancel(&self, id: u32) -> Result<(), String> {
        let current = lock(&self.job);
        let job = current.as_ref().filter(|j| j.id == id).ok_or("This server search is no longer current.")?;
        job.task.abort();
        let mut state = lock(&job.progress);
        state.done = true; state.cancelled = true;
        Ok(())
    }
}

pub fn lan_hosts(cidr: &str) -> Result<Vec<Ipv4Addr>, String> {
    let (address, prefix) = cidr.split_once('/').ok_or("Choose a local IPv4 network.")?;
    let ip: Ipv4Addr = address.parse().map_err(|_| "Choose a valid IPv4 network.".to_string())?;
    let prefix: u8 = prefix.parse().map_err(|_| "Invalid network prefix.".to_string())?;
    if !ip.is_private() || !(24..=32).contains(&prefix) { return Err("LAN discovery supports private IPv4 networks of at most 256 addresses (/24 or smaller).".into()); }
    let count = 1u32 << (32 - prefix);
    let base = u32::from(ip) & !(count - 1);
    let range = if prefix < 31 { 1..count - 1 } else { 0..count };
    Ok(range.map(|offset| Ipv4Addr::from(base + offset)).collect())
}

#[derive(Serialize)]
pub struct NetworkChoice { pub cidr: String, pub label: String }
pub async fn local_networks() -> Result<Vec<NetworkChoice>, String> {
    let networks = tokio::task::spawn_blocking(|| maru_platform::network::native().ipv4()).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    Ok(network_choices(networks))
}

fn network_choices(networks: Vec<maru_platform::network::LocalNetwork>) -> Vec<NetworkChoice> {
    networks.into_iter().filter(|n| n.address.is_private()).map(|n| {
        let prefix = n.prefix.max(24);
        let base = u32::from(n.address) & !((1u32 << (32 - prefix)) - 1);
        let cidr = format!("{}/{prefix}", Ipv4Addr::from(base));
        let portion = if n.prefix < 24 { format!("; portion of /{}", n.prefix) } else { String::new() };
        NetworkChoice { label: format!("{} · {cidr}{portion}", n.name), cidr }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use maru_llm::testing::{MockResponse, spawn_mock};
    #[test]
    fn lan_searches_are_small_private_ranges_and_skip_broadcast_and_network_addresses() {
        let hosts = lan_hosts("192.168.18.217/24").unwrap();
        assert_eq!(hosts.len(), 254); assert_eq!(hosts[0].to_string(), "192.168.18.1");
        assert_eq!(hosts[253].to_string(), "192.168.18.254");
        assert_eq!(lan_hosts("10.0.0.4/32").unwrap().len(), 1);
        assert_eq!(lan_hosts("10.0.0.4/31").unwrap().len(), 2);
        assert!(lan_hosts("8.8.8.0/24").is_err()); assert!(lan_hosts("10.0.0.0/16").is_err());
    }
    #[test]
    fn larger_networks_show_the_bounded_portion_and_host_routes_do_not_overflow() {
        let rows = network_choices(vec![
            maru_platform::network::LocalNetwork { address: Ipv4Addr::new(10,1,2,3), prefix: 16, name: "VPN".into() },
            maru_platform::network::LocalNetwork { address: Ipv4Addr::new(192,168,18,2), prefix: 32, name: "Ethernet".into() },
        ]);
        assert_eq!(rows[0].cidr, "10.1.2.0/24"); assert!(rows[0].label.contains("portion of /16"));
        assert_eq!(rows[1].cidr, "192.168.18.2/32");
    }
    #[tokio::test]
    async fn discovery_reports_results_and_refuses_overlapping_searches() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![br#"{"data":[{"id":"gemma"}]}"#.to_vec()], delay_ms: 40 }).await;
        let service = DiscoveryService::default();
        let id = service.start("host", &server.url).unwrap();
        assert!(service.start("host", &server.url).is_err());
        for _ in 0..100 {
            let state = service.status(id).unwrap();
            if state.done { assert_eq!(state.checked, 1); assert_eq!(state.servers[0].models, ["gemma"]); return; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("Discovery did not complete");
    }
    #[tokio::test]
    async fn cancelling_discovery_stops_its_requests_and_allows_a_new_search() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![b"{".to_vec(), b"}".to_vec()], delay_ms: 500 }).await;
        let service = DiscoveryService::default();
        let id = service.start("host", &server.url).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        service.cancel(id).unwrap();
        let state = service.status(id).unwrap(); assert!(state.done && state.cancelled);
        let next = service.start("host", &server.url).unwrap();
        assert_ne!(id,next); assert!(service.status(id).is_err()); service.cancel(next).unwrap();
    }
    #[tokio::test]
    async fn dropping_the_discovery_service_aborts_its_pending_search() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![br#"{"data":[{"id":"gemma"}]}"#.to_vec()], delay_ms: 100 }).await;
        let service = DiscoveryService::default();
        let id = service.start("host", &server.url).unwrap();
        let progress = lock(&service.job).as_ref().unwrap().progress.clone();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!service.status(id).unwrap().done);
        drop(service);
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(lock(&progress).checked, 0);
    }
}
