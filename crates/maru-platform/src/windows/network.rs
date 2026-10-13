use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use crate::{PlatformError, Result};
use crate::network::{LocalNetwork, NetworkInterfaces, parse_inventory};

pub struct WinNetworks;
impl NetworkInterfaces for WinNetworks {
    fn ipv4(&self) -> Result<Vec<LocalNetwork>> {
        let script = "[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding; Get-NetIPAddress -AddressFamily IPv4 -AddressState Preferred -ErrorAction Stop | ForEach-Object { '{0}|{1}|{2}' -f $_.IPAddress,$_.PrefixLength,$_.InterfaceAlias }";
        let mut child = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(0x08000000).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|e| PlatformError::Failed(format!("Could not list network interfaces: {e}")))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() < Duration::from_secs(8) => std::thread::sleep(Duration::from_millis(25)),
                result => {
                    let _ = child.kill(); let _ = child.wait();
                    return Err(PlatformError::Failed(match result { Err(e) => e.to_string(), _ => "Listing network interfaces timed out.".into() }));
                }
            }
        }
        let output = child.wait_with_output().map_err(|e| PlatformError::Failed(e.to_string()))?;
        if !output.status.success() { return Err(PlatformError::Failed("Windows could not list network interfaces.".into())); }
        Ok(parse_inventory(&String::from_utf8_lossy(&output.stdout)))
    }
}
