use std::sync::Arc;
use std::time::{Duration, Instant};

use plist::Value;
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::state::{AppState, WifiInfo};

// system_profiler SPAirPortDataType is ~7s on macOS 15+, so polling faster than
// that would just queue up redundant calls. Signal/channel rarely change.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

pub async fn spawn(state: Arc<RwLock<AppState>>) -> anyhow::Result<()> {
    tokio::spawn(async move {
        run(state).await;
    });
    Ok(())
}

pub async fn probe_once() -> anyhow::Result<WifiInfo> {
    query_wifi().await
}

async fn run(state: Arc<RwLock<AppState>>) {
    info!("wifi worker started");
    let mut ticker = interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        match query_wifi().await {
            Ok(info) => {
                debug!(?info, "wifi worker got info");
                let (prev_ssid, had_any) = {
                    let s = state.read().await;
                    (s.wifi.ssid.clone(), s.wifi.interface.is_some())
                };
                // Roam proxy: SSID value change. macOS redacts SSID without Location
                // entitlement, so same-SSID roams (different BSSID) won't trigger this.
                let ssid_changed = match (&prev_ssid, &info.ssid) {
                    (Some(a), Some(b)) => a != b,
                    _ => false,
                };
                let mut s = state.write().await;
                s.wifi = info;
                if ssid_changed && had_any {
                    s.push_marker(Instant::now());
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
    parse_plist(&output.stdout)
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
}
