# nstat

A terminal network-health monitor you can leave open during meetings. Shows current latency, packet loss, WiFi info, and a sliding-window line chart so you can spot trouble *before* a meeting falls apart.

## What it does

- Pings `1.1.1.1` and `8.8.8.8` once per second over **unprivileged ICMP** (no `sudo` required)
- Falls back to an HTTP probe against `captive.apple.com` when ICMP starts failing — distinguishes "this network blocks ICMP" from "actually offline"
- Renders a line chart of latency with selectable windows (1m / 10m / 1h)
- Shows aggregate stats: p50 / p95 / p99, packet loss %, uptime
- Surfaces the active WiFi interface, SSID, RSSI, channel, PHY mode
- Classifies overall health (Healthy / Degraded / Bad / ICMP-blocked / Offline) and reflects it in colors across the header, chart border, and badge

History is in-memory and session-only.

## Run

```sh
cargo run --release
# or, after building:
./target/release/nstat
```

### Keys

| Key | Action |
|-----|--------|
| `w` | cycle time window (1m → 10m → 1h) |
| `q` / `Esc` / `Ctrl-C` | quit |

### Other modes

```sh
nstat --check    # spawn probes for ~8s, print a summary, exit
nstat --help     # usage
```

`--check` is a quick way to verify connectivity and the WiFi probe before relying on the TUI in a meeting.

## Logs

`~/Library/Logs/nstat/nstat.log`. Bump verbosity with `NSTAT_LOG=nstat=debug`.

## macOS privacy notes

- **SSID/BSSID may show as `<redacted>`** unless your terminal has Location Services access. This is a macOS 15+ restriction on unprivileged processes. To unredact: System Settings → Privacy & Security → Location Services → enable for your terminal app.
- `system_profiler SPAirPortDataType` takes ~7 seconds to return on recent macOS, so WiFi metadata refreshes every 30s and may show stale signal strength briefly after launch.

## Health thresholds (trailing 30s)

| State | Color | Condition |
|-------|-------|-----------|
| Healthy | green | loss == 0 AND p95 < 100 ms |
| Degraded | yellow | anything between Healthy and Bad |
| Bad | red | loss ≥ 10% OR p95 ≥ 300 ms |
| ICMP-blocked | magenta | ≥ 5 consecutive ICMP timeouts but HTTP fallback succeeds |
| Offline | bright red | ICMP and HTTP both failing |
