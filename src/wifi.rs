use std::sync::Arc;
use std::time::{Duration, Instant};

use plist::Value;
use tokio::process::Command;
use tokio::sync::{Notify, RwLock};
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::state::{AppState, WifiInfo};

// system_profiler SPAirPortDataType is ~7s on macOS 15+, so polling faster than
// that would just queue up redundant calls. Signal/channel rarely change.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

pub async fn spawn(
    state: Arc<RwLock<AppState>>,
    network_change: Arc<Notify>,
) -> anyhow::Result<()> {
    tokio::spawn(async move {
        run(state, network_change).await;
    });
    Ok(())
}

pub async fn probe_once() -> anyhow::Result<WifiInfo> {
    query_wifi().await
}

async fn run(state: Arc<RwLock<AppState>>, network_change: Arc<Notify>) {
    info!("wifi worker started");
    let mut ticker = interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        match query_wifi().await {
            Ok(info) => {
                debug!(?info, "wifi worker got info");
                let (prev_ssid, prev_iface, had_any) = {
                    let s = state.read().await;
                    (
                        s.wifi.ssid.clone(),
                        s.wifi.interface.clone(),
                        s.wifi.interface.is_some(),
                    )
                };
                // Any SSID transition counts as a network change (Some↔None or
                // Some(a)→Some(b)). Interface flips (en0→en1) are also a change.
                // macOS redacts SSID without Location entitlement, so same-SSID
                // roams (different BSSID) won't be caught here.
                let ssid_changed = prev_ssid != info.ssid;
                let iface_changed = prev_iface != info.interface;
                let net_changed = had_any && (ssid_changed || iface_changed);

                let mut s = state.write().await;
                s.wifi = info;
                if net_changed {
                    s.push_marker(Instant::now());
                }
                drop(s);
                if net_changed {
                    debug!("network change detected, notifying");
                    network_change.notify_waiters();
                }
            }
            Err(e) => {
                debug!(error = %e, "wifi query failed");
            }
        }
    }
}

async fn query_wifi() -> anyhow::Result<WifiInfo> {
    let output = Command::new("system_profiler")
        .args(["-xml", "-detailLevel", "basic", "SPAirPortDataType"])
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!("system_profiler failed: {}", output.status);
    }
    let mut info = parse_plist(&output.stdout)?;
    if let Some(dev) = info.interface.as_deref() {
        info.interface_label = lookup_hardware_port(dev).await;
    }
    Ok(info)
}

// Maps a BSD device (e.g. "en0") to its System Preferences hardware-port label
// (e.g. "Wi-Fi"). Returns None if `networksetup` fails or the device isn't
// listed — the caller falls back to the BSD name alone.
async fn lookup_hardware_port(device: &str) -> Option<String> {
    let output = Command::new("networksetup")
        .arg("-listallhardwareports")
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = std::str::from_utf8(&output.stdout).ok()?;
    parse_hardware_port(text, device)
}

fn parse_hardware_port(text: &str, device: &str) -> Option<String> {
    let mut current_port: Option<&str> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Hardware Port:") {
            current_port = Some(rest.trim());
        } else if let Some(rest) = line.strip_prefix("Device:") {
            if rest.trim() == device {
                return current_port.map(str::to_string);
            }
        }
    }
    None
}

fn parse_plist(bytes: &[u8]) -> anyhow::Result<WifiInfo> {
    let value: Value = plist::from_bytes(bytes)?;
    let mut info = WifiInfo::default();

    // system_profiler -xml output is an array of dicts (one per data type).
    let outer = value
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_dictionary())
        .ok_or_else(|| anyhow::anyhow!("unexpected plist shape: missing outer dict"))?;

    let items = outer
        .get("_items")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("missing _items"))?;

    // Find the "AirPort" entry (controller), then dig into interfaces.
    for entry in items {
        let entry = match entry.as_dictionary() {
            Some(d) => d,
            None => continue,
        };
        if let Some(interfaces) = entry.get("spairport_airport_interfaces").and_then(|v| v.as_array()) {
            for iface in interfaces {
                let iface = match iface.as_dictionary() {
                    Some(d) => d,
                    None => continue,
                };
                let name = iface.get("_name").and_then(|v| v.as_string()).map(str::to_string);
                let status = iface.get("spairport_status_information").and_then(|v| v.as_string());
                if status == Some("spairport_status_off") {
                    continue;
                }
                if let Some(current) = iface
                    .get("spairport_current_network_information")
                    .and_then(|v| v.as_dictionary())
                {
                    info.interface = name;
                    info.ssid = current.get("_name").and_then(|v| v.as_string()).map(str::to_string);
                    info.bssid = current
                        .get("spairport_network_bssid")
                        .and_then(|v| v.as_string())
                        .map(str::to_string);
                    info.channel = current
                        .get("spairport_network_channel")
                        .and_then(|v| v.as_string())
                        .map(str::to_string);
                    info.phy_mode = current
                        .get("spairport_network_phymode")
                        .and_then(|v| v.as_string())
                        .map(str::to_string);

                    let signal_noise = current
                        .get("spairport_signal_noise")
                        .and_then(|v| v.as_string());
                    if let Some(sn) = signal_noise {
                        let (rssi, noise) = parse_signal_noise(sn);
                        info.rssi_dbm = rssi;
                        info.noise_dbm = noise;
                    }

                    if let Some(rate) = current.get("spairport_network_rate") {
                        if let Some(n) = rate.as_real() {
                            info.tx_rate_mbps = Some(n);
                        } else if let Some(n) = rate.as_signed_integer() {
                            info.tx_rate_mbps = Some(n as f64);
                        }
                    }
                    return Ok(info);
                } else {
                    // Interface present but not associated.
                    info.interface = name;
                }
            }
        }
    }

    if info.interface.is_none() {
        warn!("no wifi interface found in system_profiler output");
    }
    Ok(info)
}

fn parse_signal_noise(s: &str) -> (Option<i32>, Option<i32>) {
    // Example: "-52 dBm / -89 dBm"
    let mut parts = s.split('/').map(str::trim);
    let rssi = parts.next().and_then(parse_dbm);
    let noise = parts.next().and_then(parse_dbm);
    (rssi, noise)
}

fn parse_dbm(s: &str) -> Option<i32> {
    let s = s.trim();
    let num = s.split_whitespace().next()?;
    num.parse::<i32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_macos_15_output() {
        let bytes = include_bytes!("../tests/fixtures/airport.xml");
        let info = parse_plist(bytes).expect("parse plist");
        assert_eq!(info.interface.as_deref(), Some("en0"));
        // SSID may be "<redacted>" on macOS 15+ without Location entitlement; the
        // parser should still surface *some* string.
        assert!(info.ssid.is_some(), "SSID field should be populated");
        assert!(info.channel.is_some(), "channel should be parsed");
        assert!(info.rssi_dbm.is_some(), "RSSI should be parsed");
        assert!(info.rssi_dbm.unwrap() < 0, "RSSI must be negative dBm");
        assert!(info.phy_mode.is_some(), "phy mode should be parsed");
        assert!(info.tx_rate_mbps.is_some(), "tx rate should be parsed");
    }

    #[test]
    fn parses_signal_noise() {
        let (r, n) = parse_signal_noise("-52 dBm / -89 dBm");
        assert_eq!(r, Some(-52));
        assert_eq!(n, Some(-89));
    }

    #[test]
    fn parses_hardware_port() {
        let text = "Hardware Port: Ethernet\nDevice: en1\nEthernet Address: aa:bb:cc:dd:ee:ff\n\nHardware Port: Wi-Fi\nDevice: en0\nEthernet Address: 11:22:33:44:55:66\n";
        assert_eq!(parse_hardware_port(text, "en0").as_deref(), Some("Wi-Fi"));
        assert_eq!(parse_hardware_port(text, "en1").as_deref(), Some("Ethernet"));
        assert_eq!(parse_hardware_port(text, "en99"), None);
    }
}
