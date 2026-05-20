use std::collections::VecDeque;
use std::net::IpAddr;
use std::time::{Duration, Instant};

pub const MAX_SAMPLES: usize = 3600;

#[derive(Clone, Debug)]
pub struct Target {
    pub label: String,
    pub addr: IpAddr,
}

impl Target {
    pub fn new(label: impl Into<String>, addr: IpAddr) -> Self {
        Self {
            label: label.into(),
            addr,
        }
    }
}

pub fn default_targets() -> Vec<Target> {
    vec![
        Target::new("1.1.1.1", "1.1.1.1".parse().unwrap()),
        Target::new("8.8.8.8", "8.8.8.8".parse().unwrap()),
    ]
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub t: Instant,
    pub target_idx: usize,
    pub rtt: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpStatus {
    Reachable,
    CaptivePortal,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Healthy,
    Degraded,
    Bad,
    IcmpBlocked,
    Offline,
    Unknown,
}

impl Health {
    pub fn label(self) -> &'static str {
        match self {
            Health::Healthy => "HEALTHY",
            Health::Degraded => "DEGRADED",
            Health::Bad => "BAD",
            Health::IcmpBlocked => "ICMP-BLOCKED",
            Health::Offline => "OFFLINE",
            Health::Unknown => "—",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeWindow {
    OneMinute,
    TenMinutes,
    OneHour,
    Recent,
}

impl TimeWindow {
    /// Time range covered by this view. For `Recent` (a scrolling list of the
    /// newest samples, not a time-bounded view) the duration is only used to
    /// scope the footer stats, so 1m matches what the user is looking at.
    pub fn duration(self) -> Duration {
        match self {
            TimeWindow::OneMinute | TimeWindow::Recent => Duration::from_secs(60),
            TimeWindow::TenMinutes => Duration::from_secs(600),
            TimeWindow::OneHour => Duration::from_secs(3600),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TimeWindow::OneMinute => "1m",
            TimeWindow::TenMinutes => "10m",
            TimeWindow::OneHour => "1h",
            TimeWindow::Recent => "list",
        }
    }

    pub fn next(self) -> Self {
        match self {
            TimeWindow::OneMinute => TimeWindow::TenMinutes,
            TimeWindow::TenMinutes => TimeWindow::OneHour,
            TimeWindow::OneHour => TimeWindow::Recent,
            TimeWindow::Recent => TimeWindow::OneMinute,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PublicNet {
    pub ip: Option<String>,
    pub isp: Option<String>,
    pub last_check: Option<Instant>,
}

#[derive(Clone, Debug, Default)]
pub struct WifiInfo {
    pub interface: Option<String>,
    pub interface_label: Option<String>,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub rssi_dbm: Option<i32>,
    pub noise_dbm: Option<i32>,
    pub channel: Option<String>,
    pub phy_mode: Option<String>,
    pub tx_rate_mbps: Option<f64>,
}

pub struct AppState {
    pub started_at: Instant,
    pub targets: Vec<Target>,
    pub samples: VecDeque<Sample>,
    pub network_markers: VecDeque<Instant>,
    pub icmp_consecutive_timeouts: u32,
    pub http_fallback_active: bool,
    pub http_last_status: Option<HttpStatus>,
    pub http_last_check: Option<Instant>,
    pub wifi: WifiInfo,
    pub pubnet: PublicNet,
    pub health: Health,
    pub window: TimeWindow,
}

impl AppState {
    pub fn new(targets: Vec<Target>) -> Self {
        Self {
            started_at: Instant::now(),
            targets,
            samples: VecDeque::with_capacity(MAX_SAMPLES),
            network_markers: VecDeque::with_capacity(16),
            icmp_consecutive_timeouts: 0,
            http_fallback_active: false,
            http_last_status: None,
            http_last_check: None,
            wifi: WifiInfo::default(),
            pubnet: PublicNet::default(),
            health: Health::Unknown,
            window: TimeWindow::OneMinute,
        }
    }

    pub fn push_sample(&mut self, sample: Sample) {
        if self.samples.len() >= MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn push_marker(&mut self, t: Instant) {
        if self.network_markers.len() >= 16 {
            self.network_markers.pop_front();
        }
        self.network_markers.push_back(t);
    }

    pub fn cycle_window(&mut self) {
        self.window = self.window.next();
    }

    /// Wipe ping history and derived counters, restart the uptime clock.
    /// Leaves network identity (wifi, pubnet) intact since those are still
    /// describing the same connection.
    pub fn reset_data(&mut self) {
        self.started_at = Instant::now();
        self.samples.clear();
        self.network_markers.clear();
        self.icmp_consecutive_timeouts = 0;
        self.http_fallback_active = false;
        self.http_last_status = None;
        self.http_last_check = None;
        self.health = Health::Unknown;
    }

    pub fn uptime(&self) -> Duration {
        self.started_at.elapsed()
    }
}
