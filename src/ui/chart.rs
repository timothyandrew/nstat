use std::collections::VecDeque;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::Span;
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType};

use crate::state::{AppState, Sample, TimeWindow};
use crate::ui::header::health_color;

const TARGET_PALETTE: &[Color] = &[
    Color::Cyan,
    Color::LightYellow,
    Color::LightGreen,
    Color::LightMagenta,
    Color::LightBlue,
    Color::White,
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
    }
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
    };
    vec![Span::raw(left), Span::raw(mid), Span::raw("now")]
}
