use std::sync::Arc;
use std::time::{Duration, Instant};

use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence, SurgeError};
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, warn};

use crate::state::{AppState, Sample, TARGETS, Target};

const PING_INTERVAL: Duration = Duration::from_secs(1);
const PING_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn spawn_all(state: Arc<RwLock<AppState>>) -> anyhow::Result<()> {
    let config = Config::builder()
        .kind(ICMP::V4)
        .sock_type_hint(socket2::Type::DGRAM)
        .build();
    // surge-ping marks the shared reply_map as destroyed on every Client drop,
    // including clones — so we share a single Arc<Client> across tasks instead.
    let client = Arc::new(Client::new(&config)?);

    for (i, &target) in TARGETS.iter().enumerate() {
        let client = client.clone();
        let state = state.clone();
        let ident = PingIdentifier(rand::random::<u16>().wrapping_add(i as u16));
        tokio::spawn(async move {
            run_target(client, target, ident, state).await;
        });
    }
    Ok(())
}

async fn run_target(
    client: Arc<Client>,
    target: Target,
    ident: PingIdentifier,
    state: Arc<RwLock<AppState>>,
) {
    let mut pinger = client.pinger(target.addr(), ident).await;
    pinger.timeout(PING_TIMEOUT);
    let payload = [0u8; 32];
    let mut seq: u16 = 0;
    let mut ticker = interval(PING_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        let started = Instant::now();
        let result = pinger.ping(PingSequence(seq), &payload).await;
        seq = seq.wrapping_add(1);

        let rtt = match result {
            Ok((_packet, dur)) => Some(dur),
            Err(SurgeError::Timeout { .. }) => {
                debug!(target = target.label(), "icmp timeout");
                None
            }
            Err(e) => {
                warn!(target = target.label(), error = %e, "icmp error");
                None
            }
        };

        let sample = Sample {
            t: started,
            target,
            rtt,
        };
        let mut s = state.write().await;
        s.push_sample(sample);
        update_streak(&mut s, target, rtt.is_none());
    }
}

fn update_streak(state: &mut AppState, target: Target, timed_out: bool) {
    if target != Target::Cloudflare {
        return;
    }
    if timed_out {
        state.icmp_consecutive_timeouts = state.icmp_consecutive_timeouts.saturating_add(1);
    } else {
        state.icmp_consecutive_timeouts = 0;
        state.http_fallback_active = false;
    }
}
