use std::collections::VecDeque;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph};

use crate::state::{AppState, Sample, TimeWindow};
use crate::ui::header::health_color;

// Avoid green/yellow/red (and their Light* variants): those are reserved
// for health status so a chart line never reads as a status signal.
const TARGET_PALETTE: &[Color] = &[
    Color::Cyan,
    Color::LightMagenta,
    Color::LightBlue,
    Color::LightCyan,
    Color::White,
    Color::Magenta,
];

pub fn target_color(idx: usize) -> Color {
    TARGET_PALETTE[idx % TARGET_PALETTE.len()]
}

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    let now = Instant::now();
    let window = state.window.duration();
    let cutoff = now.checked_sub(window).unwrap_or(now);
    let h_color = health_color(state.health);
    let window_secs = window.as_secs_f64();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(h_color))
        .title(Span::styled(
            format!(" ping (window: {}) ", state.window.label()),
            Style::default().fg(Color::White),
        ));

    match state.window {
        TimeWindow::OneMinute => {
            let (per_target, timeouts, max_ms) =
                collect_lines(&state.samples, state.targets.len(), cutoff, now);
            let y_max = chart_y_max(max_ms);
            let mut datasets: Vec<Dataset> = state
                .targets
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    Dataset::default()
                        .name(t.label.clone())
                        .marker(Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(target_color(i)))
                        .data(&per_target[i])
                })
                .collect();
            datasets.push(
                Dataset::default()
                    .name("timeout")
                    .marker(Marker::Dot)
                    .graph_type(GraphType::Scatter)
                    .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                    .data(&timeouts),
            );
            frame.render_widget(
                build_chart(datasets, block, window_secs, y_max, state.window),
                area,
            );
        }
        TimeWindow::TenMinutes | TimeWindow::OneHour => {
            let bucket_count = bucket_count(state.window);
            let (bars, timeouts, max_ms) =
                collect_bars(&state.samples, cutoff, now, window_secs, bucket_count);
            let y_max = chart_y_max(max_ms);
            let datasets = vec![
                Dataset::default()
                    .name("max rtt")
                    .marker(Marker::Bar)
                    .graph_type(GraphType::Bar)
                    .style(Style::default().fg(Color::Cyan))
                    .data(&bars),
                Dataset::default()
                    .name("timeout")
                    .marker(Marker::Block)
                    .graph_type(GraphType::Scatter)
                    .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                    .data(&timeouts),
            ];
            frame.render_widget(
                build_chart(datasets, block, window_secs, y_max, state.window),
                area,
            );
        }
        TimeWindow::Recent => {
            draw_recent(frame, area, state, now, block, h_color);
        }
    }
}

const MIN_PANE_WIDTH: u16 = 28;

fn draw_recent(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    now: Instant,
    outer_block: Block<'_>,
    h_color: Color,
) {
    let n = state.targets.len() as u16;
    let split = n > 1 && area.width >= n * MIN_PANE_WIDTH;

    if !split {
        let inner = outer_block.inner(area);
        frame.render_widget(outer_block, area);
        let lines = combined_lines(state, now, inner.height as usize);
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // Per-target panes: drop the outer block so each pane carries its own
    // bordered, target-titled frame.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, n as u32); n as usize])
        .split(area);

    for (i, target) in state.targets.iter().enumerate() {
        let color = target_color(i);
        let pane = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(h_color))
            .title(Span::styled(
                format!(" {} ", target.label),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
        let inner = pane.inner(cols[i]);
        frame.render_widget(pane, cols[i]);
        let lines = target_lines(state, now, i, inner.height as usize);
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// Newest at the bottom: take the most recent `max_rows` samples, then reverse
/// so iteration runs oldest-to-newest before rendering.
fn combined_lines(state: &AppState, now: Instant, max_rows: usize) -> Vec<Line<'static>> {
    let label_pad = state.targets.iter().map(|t| t.label.len()).max().unwrap_or(0);
    let mut recent: Vec<&Sample> = state.samples.iter().rev().take(max_rows).collect();
    recent.reverse();
    recent
        .into_iter()
        .map(|s| {
            let label = state
                .targets
                .get(s.target_idx)
                .map(|t| t.label.clone())
                .unwrap_or_else(|| format!("#{}", s.target_idx));
            let color = target_color(s.target_idx);
            sample_line(
                Some((label, color, label_pad)),
                s.rtt,
                now.saturating_duration_since(s.t).as_secs(),
            )
        })
        .collect()
}

fn target_lines(
    state: &AppState,
    now: Instant,
    target_idx: usize,
    max_rows: usize,
) -> Vec<Line<'static>> {
    let mut recent: Vec<&Sample> = state
        .samples
        .iter()
        .rev()
        .filter(|s| s.target_idx == target_idx)
        .take(max_rows)
        .collect();
    recent.reverse();
    recent
        .into_iter()
        .map(|s| sample_line(None, s.rtt, now.saturating_duration_since(s.t).as_secs()))
        .collect()
}

fn sample_line(
    label: Option<(String, Color, usize)>,
    rtt: Option<std::time::Duration>,
    age_secs: u64,
) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    let timeout_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let body = match rtt {
        Some(d) => Span::styled(
            format!("time={:>6.1} ms", d.as_secs_f64() * 1000.0),
            Style::default().fg(Color::White),
        ),
        None => Span::styled("timeout", timeout_style),
    };
    let mut spans = Vec::with_capacity(4);
    if let Some((text, color, pad)) = label {
        spans.push(Span::styled(
            format!("{:>width$}", text, width = pad),
            Style::default().fg(color),
        ));
        spans.push(Span::styled(": ", dim));
    }
    spans.push(body);
    spans.push(Span::styled(format!("   ({}s ago)", age_secs), dim));
    Line::from(spans)
}

fn build_chart<'a>(
    datasets: Vec<Dataset<'a>>,
    block: Block<'a>,
    window_secs: f64,
    y_max: f64,
    window: TimeWindow,
) -> Chart<'a> {
    let y_mid = y_max / 2.0;
    Chart::new(datasets)
        .block(block)
        .x_axis(
            Axis::default()
                .style(Style::default().fg(Color::DarkGray))
                .bounds([-window_secs, 0.0])
                .labels(window_labels(window)),
        )
        .y_axis(
            Axis::default()
                .style(Style::default().fg(Color::DarkGray))
                .bounds([0.0, y_max])
                .labels(vec![
                    Span::raw("0ms"),
                    Span::raw(format!("{:.0}ms", y_mid)),
                    Span::raw(format!("{:.0}ms", y_max)),
                ]),
        )
}

fn chart_y_max(max_ms: f64) -> f64 {
    (max_ms * 1.15).max(50.0).ceil()
}

fn bucket_count(w: TimeWindow) -> usize {
    match w {
        TimeWindow::OneMinute => 60,
        TimeWindow::TenMinutes => 120,
        TimeWindow::OneHour => 180,
        TimeWindow::Recent => unreachable!("Recent renders as a list, not buckets"),
    }
}

fn collect_lines(
    samples: &VecDeque<Sample>,
    target_count: usize,
    cutoff: Instant,
    now: Instant,
) -> (Vec<Vec<(f64, f64)>>, Vec<(f64, f64)>, f64) {
    let mut per_target: Vec<Vec<(f64, f64)>> = (0..target_count).map(|_| Vec::new()).collect();
    let mut to = Vec::new();
    let mut max_ms = 50.0f64;

    for s in samples.iter() {
        if s.t < cutoff {
            continue;
        }
        let x = -(now.saturating_duration_since(s.t).as_secs_f64());
        match s.rtt {
            Some(d) => {
                let ms = d.as_secs_f64() * 1000.0;
                if ms > max_ms {
                    max_ms = ms;
                }
                if let Some(bucket) = per_target.get_mut(s.target_idx) {
                    bucket.push((x, ms));
                }
            }
            None => to.push((x, 0.0)),
        }
    }
    for p in to.iter_mut() {
        p.1 = max_ms;
    }
    (per_target, to, max_ms)
}

/// Bucketize samples into `bucket_count` time bins across the window.
/// For each bucket, the bar height is the maximum RTT seen across *all*
/// targets (worst-case wins so you actually notice spikes). Buckets with any
/// timeouts emit a separate scatter point at chart-max.
fn collect_bars(
    samples: &VecDeque<Sample>,
    cutoff: Instant,
    now: Instant,
    window_secs: f64,
    bucket_count: usize,
) -> (Vec<(f64, f64)>, Vec<(f64, f64)>, f64) {
    let mut max_per_bucket = vec![f64::NAN; bucket_count];
    let mut timeouts_per_bucket = vec![false; bucket_count];
    let mut max_ms = 50.0f64;

    let bucket_secs = window_secs / bucket_count as f64;

    for s in samples.iter() {
        if s.t < cutoff {
            continue;
        }
        let age = now.saturating_duration_since(s.t).as_secs_f64();
        let bucket = (((window_secs - age) / bucket_secs) as usize).min(bucket_count - 1);
        match s.rtt {
            Some(d) => {
                let ms = d.as_secs_f64() * 1000.0;
                let cur = max_per_bucket[bucket];
                if cur.is_nan() || ms > cur {
                    max_per_bucket[bucket] = ms;
                }
                if ms > max_ms {
                    max_ms = ms;
                }
            }
            None => {
                timeouts_per_bucket[bucket] = true;
            }
        }
    }

    let mut bars = Vec::with_capacity(bucket_count);
    let mut timeouts = Vec::new();
    for i in 0..bucket_count {
        let x = -(window_secs - (i as f64 + 0.5) * bucket_secs);
        let v = max_per_bucket[i];
        if !v.is_nan() {
            bars.push((x, v));
        }
        if timeouts_per_bucket[i] {
            timeouts.push((x, 0.0));
        }
    }
    for p in timeouts.iter_mut() {
        p.1 = max_ms;
    }
    (bars, timeouts, max_ms)
}

fn window_labels(w: TimeWindow) -> Vec<Span<'static>> {
    let (left, mid) = match w {
        TimeWindow::OneMinute => ("-60s", "-30s"),
        TimeWindow::TenMinutes => ("-10m", "-5m"),
        TimeWindow::OneHour => ("-1h", "-30m"),
        TimeWindow::Recent => unreachable!("Recent renders as a list, not a chart axis"),
    };
    vec![Span::raw(left), Span::raw(mid), Span::raw("now")]
}
