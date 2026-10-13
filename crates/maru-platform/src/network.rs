//! Local interface inventory behind an OS boundary. Listing does not probe the LAN.
use std::net::Ipv4Addr;
use crate::Result;

#[derive(Clone, Debug)]
pub struct LocalNetwork { pub address: Ipv4Addr, pub prefix: u8, pub name: String }

pub trait NetworkInterfaces: Send + Sync {
    fn ipv4(&self) -> Result<Vec<LocalNetwork>>;
}

pub fn native() -> Box<dyn NetworkInterfaces> {
    #[cfg(windows)]
    { Box::new(crate::windows::network::WinNetworks) }
    #[cfg(not(windows))]
    { Box::new(Unavailable) }
}

#[cfg(not(windows))]
struct Unavailable;
#[cfg(not(windows))]
impl NetworkInterfaces for Unavailable {
    fn ipv4(&self) -> Result<Vec<LocalNetwork>> { Err(crate::PlatformError::Unsupported("LAN interface discovery")) }
}

#[cfg(any(windows, test))]
pub(crate) fn parse_inventory(text: &str) -> Vec<LocalNetwork> {
    text.lines().filter_map(|line| {
        let mut parts = line.trim().splitn(3, '|');
        let address = parts.next()?.parse().ok()?;
        let prefix = parts.next()?.parse::<u8>().ok().filter(|p| *p <= 32)?;
        Some(LocalNetwork { address, prefix, name: parts.next()?.to_string() })
    }).take(256).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interface_inventory_keeps_actual_prefixes_and_ignores_invalid_lines() {
        let rows = parse_inventory("192.168.18.3|24|Ethernet\n10.1.1.2|16|VPN\nnot an address|24|bad\n10.0.0.1|33|bad");
        assert_eq!(rows.len(), 2); assert_eq!(rows[1].prefix, 16); assert_eq!(rows[0].name, "Ethernet");
    }
}
