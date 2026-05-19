mod app;
mod probe;
mod state;
mod stats;
mod ui;
mod wifi;

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = dirs_log_dir()?;
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let file_appender = tracing_appender::rolling::never(&dir, "nstat.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_env("NSTAT_LOG")
        .unwrap_or_else(|_| EnvFilter::new("nstat=info,warn"));

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _log_guard = init_logging();

    let mode = std::env::args().nth(1);
    if matches!(mode.as_deref(), Some("--check") | Some("-c")) {
        return run_check().await;
    }
    if matches!(mode.as_deref(), Some("--help") | Some("-h")) {
        print_help();
        return Ok(());
    }

    install_panic_hook();

    let state = Arc::new(RwLock::new(AppState::new()));

    probe::icmp::spawn_all(state.clone()).await?;
    probe::http::spawn(state.clone()).await?;
    wifi::spawn(state.clone()).await?;

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
    println!("    nstat              run the TUI (default)");
    println!("    nstat --check      run probes for 3s, print summary, exit");
    println!("    nstat --help       show this message");
    println!();
    println!("KEYS:");
    println!("    w    cycle time window (1m / 10m / 1h)");
    println!("    q    quit");
    println!();
    println!("LOGS: ~/Library/Logs/nstat/nstat.log");
}

async fn run_check() -> anyhow::Result<()> {
    println!("nstat --check: probing for ~8s…");
    let state = Arc::new(RwLock::new(AppState::new()));
    probe::icmp::spawn_all(state.clone()).await?;
    wifi::spawn(state.clone()).await?;
    // system_profiler is ~7s on macOS 15+, so wait long enough for it to land.
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
    let s = state.read().await;
    let total = s.samples.len();
    let timeouts = s.samples.iter().filter(|x| x.rtt.is_none()).count();
    let oks = total - timeouts;
    println!("samples: {} (ok={} timeout={})", total, oks, timeouts);
    if let Some(last) = s.samples.iter().rev().find(|x| x.rtt.is_some()) {
        println!(
            "last ok: {} → {:.1}ms",
            last.target.label(),
            last.rtt.unwrap().as_secs_f64() * 1000.0
        );
    }
    println!(
        "wifi: iface={:?} ssid={:?} rssi={:?} ch={:?} phy={:?}",
        s.wifi.interface, s.wifi.ssid, s.wifi.rssi_dbm, s.wifi.channel, s.wifi.phy_mode
    );
    if s.wifi.interface.is_none() {
        // Run an inline probe to surface why parsing failed.
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
