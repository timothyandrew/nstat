mod app;
mod probe;
mod state;
mod stats;
mod ui;
mod wifi;

use std::net::IpAddr;
use std::sync::Arc;

use tokio::net::lookup_host;
use tokio::sync::{Notify, RwLock};
use tracing_subscriber::EnvFilter;

use crate::state::{AppState, Target, default_targets};

fn init_logging(debug: bool) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = dirs_log_dir()?;
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let file_appender = tracing_appender::rolling::never(&dir, "nstat.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // `--debug` takes precedence over NSTAT_LOG — easier than asking the user
    // to remember the env var when they just want "show me everything".
    let filter = if debug {
        EnvFilter::new("nstat=debug")
    } else {
        EnvFilter::try_from_env("NSTAT_LOG").unwrap_or_else(|_| EnvFilter::new("nstat=info,warn"))
    };

    let _ = tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init();
    Some(guard)
}

fn dirs_log_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = std::path::PathBuf::from(home);
    p.push("Library");
    p.push("Logs");
    p.push("nstat");
    Some(p)
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = ratatui::try_restore();
        default(info);
    }));
}

struct Cli {
    check: bool,
    help: bool,
    debug: bool,
    targets: Vec<String>,
}

fn parse_args() -> Cli {
    let mut cli = Cli {
        check: false,
        help: false,
        debug: false,
        targets: Vec::new(),
    };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" | "-c" => cli.check = true,
            "--help" | "-h" => cli.help = true,
            "--debug" => cli.debug = true,
            _ => cli.targets.push(arg),
        }
    }
    cli
}

async fn resolve_targets(specs: &[String]) -> anyhow::Result<Vec<Target>> {
    if specs.is_empty() {
        return Ok(default_targets());
    }
    let mut out = Vec::with_capacity(specs.len());
    for spec in specs {
        // Bare IP → use as-is, preserving the user's label exactly. Hostname →
        // resolve to the first A/AAAA, use the hostname for display.
        if let Ok(addr) = spec.parse::<IpAddr>() {
            out.push(Target::new(spec, addr));
            continue;
        }
        let resolved = lookup_host((spec.as_str(), 0))
            .await
            .map_err(|e| anyhow::anyhow!("failed to resolve {spec}: {e}"))?
            .map(|sa| sa.ip())
            .next()
            .ok_or_else(|| anyhow::anyhow!("no addresses for {spec}"))?;
        out.push(Target::new(spec, resolved));
    }
    Ok(out)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = parse_args();
    let _log_guard = init_logging(cli.debug);

    if cli.help {
        print_help();
        return Ok(());
    }

    let targets = resolve_targets(&cli.targets).await?;

    if cli.check {
        return run_check(targets).await;
    }

    install_panic_hook();

    let state = Arc::new(RwLock::new(AppState::new(targets)));
    let network_change = Arc::new(Notify::new());

    probe::icmp::spawn_all(state.clone()).await?;
    probe::http::spawn(state.clone()).await?;
    probe::pubnet::spawn(state.clone(), network_change.clone()).await?;
    probe::netmon::spawn(network_change.clone()).await?;
    wifi::spawn(state.clone(), network_change.clone()).await?;

    let terminal = ratatui::init();
    let input = app::spawn_input();
    let result = app::run(terminal, state, input).await;
    ratatui::restore();
    result
}

fn print_help() {
    println!("nstat — network health TUI");
    println!();
    println!("USAGE:");
    println!("    nstat [TARGETS]…           run the TUI");
    println!("    nstat --check [TARGETS]…   probe for ~8s, print summary, exit");
    println!("    nstat --debug              verbose logging to ~/Library/Logs/nstat/nstat.log");
    println!("    nstat --help               show this message");
    println!();
    println!("TARGETS: IP addresses or hostnames to ping. Defaults to 1.1.1.1 and 8.8.8.8");
    println!("         when no targets are given. Hostnames are resolved once at startup.");
    println!();
    println!("KEYS:");
    println!("    w    cycle view (1m / 10m / 1h / recent list)");
    println!("    r    reset history (clear samples, restart uptime)");
    println!("    q    quit");
    println!();
    println!("LOGS: ~/Library/Logs/nstat/nstat.log");
}

async fn run_check(targets: Vec<Target>) -> anyhow::Result<()> {
    println!("nstat --check: probing for ~8s…");
    for t in &targets {
        println!("  target: {} ({})", t.label, t.addr);
    }
    let state = Arc::new(RwLock::new(AppState::new(targets)));
    let network_change = Arc::new(Notify::new());
    probe::icmp::spawn_all(state.clone()).await?;
    wifi::spawn(state.clone(), network_change).await?;
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
    let s = state.read().await;
    let total = s.samples.len();
    let timeouts = s.samples.iter().filter(|x| x.rtt.is_none()).count();
    let oks = total - timeouts;
    println!("samples: {} (ok={} timeout={})", total, oks, timeouts);
    if let Some(last) = s.samples.iter().rev().find(|x| x.rtt.is_some()) {
        let label = s
            .targets
            .get(last.target_idx)
            .map(|t| t.label.as_str())
            .unwrap_or("?");
        println!(
            "last ok: {} → {:.1}ms",
            label,
            last.rtt.unwrap().as_secs_f64() * 1000.0
        );
    }
    println!(
        "wifi: iface={:?} ssid={:?} rssi={:?} ch={:?} phy={:?}",
        s.wifi.interface, s.wifi.ssid, s.wifi.rssi_dbm, s.wifi.channel, s.wifi.phy_mode
    );
    if s.wifi.interface.is_none() {
        match wifi::probe_once().await {
            Ok(info) => println!("[direct] {:?}", info),
            Err(e) => println!("[direct] error: {:#}", e),
        }
    }
    if oks == 0 {
        anyhow::bail!("no successful pings — check connectivity (or ICMP may be blocked)");
    }
    Ok(())
}
