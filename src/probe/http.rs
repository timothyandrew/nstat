use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Client;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, warn};

use crate::state::{AppState, HttpStatus};

const HTTP_URL: &str = "http://captive.apple.com/hotspot-detect.html";
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const ICMP_FAIL_THRESHOLD: u32 = 5;

pub async fn spawn(state: Arc<RwLock<AppState>>) -> anyhow::Result<()> {
    let client = Client::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
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
        let should_probe = {
            let s = state.read().await;
            s.icmp_consecutive_timeouts >= ICMP_FAIL_THRESHOLD
        };
        if !should_probe {
            let mut s = state.write().await;
            if s.http_fallback_active {
                s.http_fallback_active = false;
            }
            continue;
        }

        let status = probe(&client).await;
        debug!(?status, "http fallback probed");

        let mut s = state.write().await;
        s.http_fallback_active = true;
        s.http_last_status = Some(status);
        s.http_last_check = Some(Instant::now());
    }
}

async fn probe(client: &Client) -> HttpStatus {
    match client.get(HTTP_URL).send().await {
        Ok(resp) => {
            let status = resp.status();
            if !status.is_success() {
                debug!(?status, "captive-portal non-200");
                return HttpStatus::CaptivePortal;
            }
            match resp.text().await {
                Ok(body) if body.contains("Success") => HttpStatus::Reachable,
                Ok(_) => HttpStatus::CaptivePortal,
                Err(e) => {
                    warn!(error = %e, "http body read failed");
                    HttpStatus::Failed
                }
            }
        }
        Err(e) => {
            debug!(error = %e, "http request failed");
            HttpStatus::Failed
        }
    }
}
