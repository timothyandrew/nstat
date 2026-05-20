use std::time::{Duration, Instant};

use crate::state::{AppState, Health, HttpStatus, Sample};

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub count: usize,
    pub timeouts: usize,
    pub loss_pct: f64,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub avg_ms: Option<f64>,
    pub last_ms: Option<f64>,
}

pub fn window_stats(samples: &[&Sample]) -> Stats {
    let mut stats = Stats::default();
    stats.count = samples.len();
    if samples.is_empty() {
        return stats;
    }

    let mut rtts: Vec<f64> = Vec::with_capacity(samples.len());
    let mut timeouts = 0usize;
    let mut sum = 0.0;
    for s in samples {
        match s.rtt {
            Some(d) => {
                let ms = d.as_secs_f64() * 1000.0;
                rtts.push(ms);
                sum += ms;
            }
            None => timeouts += 1,
        }
    }
    stats.timeouts = timeouts;
    stats.loss_pct = (timeouts as f64 / samples.len() as f64) * 100.0;

    if let Some(last) = samples.last() {
        stats.last_ms = last.rtt.map(|d| d.as_secs_f64() * 1000.0);
    }

    if !rtts.is_empty() {
        stats.avg_ms = Some(sum / rtts.len() as f64);
        stats.p50_ms = Some(percentile(&mut rtts, 0.50));
        stats.p95_ms = Some(percentile(&mut rtts, 0.95));
        stats.p99_ms = Some(percentile(&mut rtts, 0.99));
    }
    stats
}

/// Stats for one target inside the given trailing window.
pub fn target_window_stats(state: &AppState, target_idx: usize, window: Duration) -> Stats {
    let now = Instant::now();
    let cutoff = now.checked_sub(window).unwrap_or(now);
    let filtered: Vec<&Sample> = state
        .samples
        .iter()
        .filter(|s| s.target_idx == target_idx && s.t >= cutoff)
        .collect();
    window_stats(&filtered)
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let n = values.len();
    let idx = ((p * (n as f64 - 1.0)).round() as usize).min(n - 1);
    let (_, nth, _) = values.select_nth_unstable_by(idx, |a, b| a.partial_cmp(b).unwrap());
    *nth
}

/// Health classification for a single target's trailing 30s. ICMP-blocked /
/// Offline are still global signals — they depend on the cross-target streak
/// counter and the HTTP fallback probe, both of which describe the link as a
/// whole rather than any one destination.
pub fn classify_target(state: &AppState, target_idx: usize) -> Health {
    let now = Instant::now();
    let window_30s: Vec<&Sample> = state
        .samples
        .iter()
        .filter(|s| {
            s.target_idx == target_idx && now.duration_since(s.t) <= Duration::from_secs(30)
        })
        .collect();
    classify_samples(&window_30s, state)
}

fn classify_samples(samples: &[&Sample], state: &AppState) -> Health {
    if samples.is_empty() {
        return Health::Unknown;
    }

    let s = window_stats(samples);

    if state.icmp_consecutive_timeouts >= 5 {
        match state.http_last_status {
            Some(HttpStatus::Reachable) => return Health::IcmpBlocked,
            Some(HttpStatus::CaptivePortal) => return Health::IcmpBlocked,
            Some(HttpStatus::Failed) => return Health::Offline,
            None => return Health::Bad,
        }
    }

    // Tuned for wifi to public DNS over a 30s window. Local LAN you'd see <5ms,
    // healthy wifi 15-80ms, congested wifi 80-200ms, broken 200ms+.
    let p95 = s.p95_ms.unwrap_or(0.0);
    if s.loss_pct >= 10.0 || p95 >= 300.0 {
        return Health::Bad;
    }
    if s.loss_pct == 0.0 && p95 < 100.0 {
        return Health::Healthy;
    }
    Health::Degraded
}

/// Worst (most-alarming) of a set of healths. Drives the header badge so a
/// single glance still surfaces trouble on any target.
pub fn worst(healths: &[Health]) -> Health {
    healths.iter().copied().max_by_key(severity).unwrap_or(Health::Unknown)
}

fn severity(h: &Health) -> u8 {
    match h {
        Health::Unknown => 0,
        Health::Healthy => 1,
        Health::Degraded => 2,
        Health::IcmpBlocked => 3,
        Health::Bad => 4,
        Health::Offline => 5,
    }
}
