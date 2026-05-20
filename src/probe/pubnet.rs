use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::Deserialize;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, warn};

use crate::state::AppState;

const URL: &str = "https://ipinfo.io/json";
const TIMEOUT: Duration = Duration::from_secs(4);
const POLL_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Debug, Deserialize)]
struct IpInfo {
    ip: Option<String>,
    org: Option<String>,
}

pub async fn spawn(state: Arc<RwLock<AppState>>) -> anyhow::Result<()> {
    let client = Client::builder()
        .timeout(TIMEOUT)
        .user_agent("nstat/0.1")
        .build()?;

    tokio::spawn(async move {
        run(client, state).await;
    });
    Ok(())
}

async fn run(client: Client, state: Arc<RwLock<AppState>>) {
    let mut ticker = interval(POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        match fetch(&client).await {
            Ok(info) => {
                debug!(?info, "pubnet refreshed");
                let mut s = state.write().await;
                s.pubnet.ip = info.ip;
                s.pubnet.isp = info.org.map(strip_asn_prefix);
                s.pubnet.last_check = Some(Instant::now());
            }
            Err(e) => {
                debug!(error = %e, "pubnet fetch failed");
            }
        }
    }
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
