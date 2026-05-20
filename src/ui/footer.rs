use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::stats::{Stats, window_stats};
use crate::state::{AppState, HttpStatus};

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    let now = Instant::now();
    let cutoff = now.checked_sub(state.window.duration()).unwrap_or(now);
    let in_window: Vec<&_> = state
        .samples
        .iter()
        .filter(|s| s.t >= cutoff)
        .collect();
    let stats = window_stats(&in_window);

    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    frame.render_widget(Paragraph::new(stats_line(&stats, state)), rows[0]);
    frame.render_widget(Paragraph::new(hints_line(state)), rows[1]);
}

fn stats_line(stats: &Stats, state: &AppState) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    let val = Style::default().fg(Color::White);

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled("p50 ", dim),
        Span::styled(fmt_ms(stats.p50_ms), val),
        Span::styled("  p95 ", dim),
        Span::styled(fmt_ms(stats.p95_ms), val),
        Span::styled("  p99 ", dim),
        Span::styled(fmt_ms(stats.p99_ms), val),
        Span::styled("  loss ", dim),
        Span::styled(format!("{:.1}%", stats.loss_pct), val),
        Span::styled("  last ", dim),
        Span::styled(fmt_ms(stats.last_ms), val),
        Span::styled("  uptime ", dim),
        Span::styled(fmt_dur(state.uptime()), val),
    ];

    if state.http_fallback_active {
        let label = match state.http_last_status {
            Some(HttpStatus::Reachable) => ("  HTTP ok ", Color::Cyan),
            Some(HttpStatus::CaptivePortal) => ("  HTTP captive ", Color::Yellow),
            Some(HttpStatus::Failed) => ("  HTTP fail ", Color::Red),
            None => ("  HTTP — ", Color::DarkGray),
        };
        spans.push(Span::styled(label.0.to_string(), Style::default().fg(label.1).add_modifier(Modifier::BOLD)));
    }

    Line::from(spans)
}

fn hints_line(state: &AppState) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    let key = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    Line::from(vec![
        Span::styled("[w]", key),
        Span::styled(" cycle window  ", dim),
        Span::styled("[r]", key),
        Span::styled(" reset  ", dim),
        Span::styled("[q]", key),
        Span::styled(" quit  ", dim),
        Span::styled("window: ", dim),
        Span::styled(state.window.label().to_string(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ])
}

fn fmt_ms(v: Option<f64>) -> String {
    match v {
        Some(ms) => format!("{:>5.1}ms", ms),
        None => "   —  ".into(),
    }
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{}s", s)
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}
