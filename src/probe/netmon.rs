use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::Notify;
use tokio::time::interval;
use tracing::debug;

// Cheap (~ms) ifconfig diff. system_profiler is too slow (~7s) to be the only
// network-change signal, so this runs alongside it and triggers the faster
// downstream refreshes (pubnet ISP/IP, wifi info on next tick).
const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub async fn spawn(network_change: Arc<Notify>) -> anyhow::Result<()> {
    tokio::spawn(async move {
        run(network_change).await;
    });
    Ok(())
}

async fn run(network_change: Arc<Notify>) {
    let mut ticker = interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last: Option<String> = None;
    loop {
        ticker.tick().await;
        let current = match snapshot().await {
            Some(s) => {
                debug!(len = s.len(), "ifconfig check performed");
                s
            }
            None => {
                debug!("ifconfig check failed (command error)");
                continue;
            }
        };
        if let Some(prev) = &last {
            if prev != &current {
                debug!("ifconfig change detected, notifying network_change");
                network_change.notify_one();
            } else {
                debug!("ifconfig check: no change");
            }
        } else {
            debug!("ifconfig check: priming first snapshot");
        }
        last = Some(current);
    }
}

async fn snapshot() -> Option<String> {
    let output = Command::new("ifconfig")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(filter(&text))
}

/// Keep only the lines that flip on a real network change: interface headers
/// (carries UP/RUNNING flags), IPv4 `inet`, and `status:`. Drops `inet6`
/// (privacy-extension temp addresses rotate periodically), `nd6`, `ether`,
/// `media`, and `options` — those are either stable or noisy.
fn filter(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let is_header = !line.starts_with([' ', '\t']);
        let trimmed = line.trim_start();
        if is_header || trimmed.starts_with("inet ") || trimmed.starts_with("status:") {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
