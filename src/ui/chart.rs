use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::Span;
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType};

use crate::state::{AppState, Sample, Target, TimeWindow};
use crate::ui::header::health_color;

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    let now = Instant::now();
    let window = state.window.duration();
    let cutoff = now.checked_sub(window).unwrap_or(now);

    let (cf_points, gg_points, timeout_points, max_ms) =
        collect_points(&state.samples, cutoff, now);

    let y_max = (max_ms * 1.15).max(50.0).ceil();
    let y_mid = y_max / 2.0;

    let cf_ds = Dataset::default()
        .name("1.1.1.1")
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Cyan))
        .data(&cf_points);

    let gg_ds = Dataset::default()
        .name("8.8.8.8")
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::LightYellow))
        .data(&gg_points);

    let to_ds = Dataset::default()
        .name("timeout")
        .marker(Marker::Dot)
        .graph_type(GraphType::Scatter)
        .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .data(&timeout_points);

    let h_color = health_color(state.health);
    let window_secs = window.as_secs_f64();

    let chart = Chart::new(vec![cf_ds, gg_ds, to_ds])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(h_color))
                .title(Span::styled(
                    format!(" ping (window: {}) ", state.window.label()),
                    Style::default().fg(Color::White),
                )),
        )
        .x_axis(
            Axis::default()
                .style(Style::default().fg(Color::DarkGray))
                .bounds([-window_secs, 0.0])
                .labels(window_labels(state.window)),
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
        );

    frame.render_widget(chart, area);
}

fn collect_points(
    samples: &std::collections::VecDeque<Sample>,
    cutoff: Instant,
    now: Instant,
) -> (Vec<(f64, f64)>, Vec<(f64, f64)>, Vec<(f64, f64)>, f64) {
    let mut cf = Vec::new();
    let mut gg = Vec::new();
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
                match s.target {
                    Target::Cloudflare => cf.push((x, ms)),
                    Target::Google => gg.push((x, ms)),
                }
            }
            None => {
                // Pin timeouts to current max to draw red dots at the top of the chart.
                to.push((x, 0.0));
            }
        }
    }

    // Adjust the timeout dots to sit at max (after we know it).
    for p in to.iter_mut() {
        p.1 = max_ms;
    }

    (cf, gg, to, max_ms)
}

fn window_labels(w: TimeWindow) -> Vec<Span<'static>> {
    let left = match w {
        TimeWindow::OneMinute => "-60s",
        TimeWindow::TenMinutes => "-10m",
        TimeWindow::OneHour => "-1h",
    };
    let mid = match w {
        TimeWindow::OneMinute => "-30s",
        TimeWindow::TenMinutes => "-5m",
        TimeWindow::OneHour => "-30m",
    };
    vec![
        Span::raw(left),
        Span::raw(mid),
        Span::raw("now"),
    ]
}

#[allow(dead_code)]
pub fn _format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{}s", s)
    } else if s < 3600 {
        format!("{}m{}s", s / 60, s % 60)
    } else {
        format!("{}h{}m", s / 3600, (s % 3600) / 60)
    }
}
