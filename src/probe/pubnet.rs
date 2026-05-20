use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::Deserialize;
use tokio::sync::{Notify, RwLock};
use tokio::time::{interval, sleep};
use tracing::{debug, warn};

use crate::state::AppState;

const URL: &str = "https://ipinfo.io/json";
const TIMEOUT: Duration = Duration::from_secs(4);
const POLL_INTERVAL: Duration = Duration::from_secs(300);
const RETRY_INTERVAL: Duration = Duration::from_secs(5);
const SETTLE_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRIES: u32 = 8;

#[derive(Debug, Deserialize)]
struct IpInfo {
    ip: Option<String>,
    org: Option<String>,
}

pub async fn spawn(
    state: Arc<RwLock<AppState>>,
    network_change: Arc<Notify>,
) -> anyhow::Result<()> {
    tokio::spawn(async move {
        run(state, network_change).await;
    });
    Ok(())
}

async fn run(state: Arc<RwLock<AppState>>, network_change: Arc<Notify>) {
    let mut ticker = interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // After a network change the route/DNS often isn't usable for ~10-30s —
    // the first one or two fetches commonly fail. Carry a small retry budget
    // so we keep trying instead of going dormant for 5 minutes.
    let mut retries_left: u32 = 0;

    loop {
        let triggered_by_net_change = if retries_left > 0 {
            tokio::select! {
                _ = sleep(RETRY_INTERVAL) => false,
                _ = network_change.notified() => true,
            }
        } else {
            tokio::select! {
                _ = ticker.tick() => false,
                _ = network_change.notified() => true,
            }
        };
        debug!(triggered_by_net_change, retries_left, "pubnet wake");
        if triggered_by_net_change {
            debug!("pubnet sleeping 5s for DHCP settle");
            sleep(SETTLE_DELAY).await;
            retries_left = MAX_RETRIES;
        }
        debug!(url = URL, "pubnet fetch starting");
        // Build a fresh client every fetch: reqwest/hyper pools both DNS and
        // TCP connections, and after a network change those cached entries
        // point at the previous route and fail immediately. Building is cheap.
        let client = match build_client() {
            Ok(c) => c,
            Err(e) => {
                debug!(error = %e, "pubnet client build failed");
                retries_left = retries_left.saturating_sub(1);
                continue;
            }
        };
        match fetch(&client).await {
            Ok(info) => {
                debug!(?info, "pubnet fetch succeeded");
                let mut s = state.write().await;
                s.pubnet.ip = info.ip;
                s.pubnet.isp = info.org.map(strip_asn_prefix);
                s.pubnet.last_check = Some(Instant::now());
                retries_left = 0;
            }
            Err(e) => {
                let chain = error_chain(&e);
                debug!(error = %e, chain = %chain, retries_left, "pubnet fetch failed");
                retries_left = retries_left.saturating_sub(1);
            }
        }
    }
}

fn build_client() -> reqwest::Result<Client> {
    Client::builder()
        .timeout(TIMEOUT)
        .user_agent("nstat/0.1")
        // Belt-and-suspenders: even though we rebuild the client per fetch,
        // disable connection pooling so the underlying hyper pool can't hold
        // a stale socket across attempts.
        .pool_max_idle_per_host(0)
        .build()
}

async fn fetch(client: &Client) -> anyhow::Result<IpInfo> {
    let resp = client.get(URL).send().await?;
    let status = resp.status();
    if !status.is_success() {
        warn!(?status, "pubnet non-success");
        anyhow::bail!("pubnet status {status}");
    }
    let info = resp.json::<IpInfo>().await?;
    Ok(info)
}

fn error_chain(e: &anyhow::Error) -> String {
    e.chain().map(|c| c.to_string()).collect::<Vec<_>>().join(" | ")
}

/// `ipinfo.io` returns `org` like `"AS13335 Cloudflare, Inc."`. The AS number
/// is noise in a status header, so trim it when present.
fn strip_asn_prefix(org: String) -> String {
    if let Some(rest) = org.strip_prefix("AS")
        && let Some((num, name)) = rest.split_once(' ')
        && num.chars().all(|c| c.is_ascii_digit())
    {
        return name.to_string();
    }
    org
}
