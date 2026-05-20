use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::state::{AppState, Health};

pub fn health_color(h: Health) -> Color {
    match h {
        Health::Healthy => Color::Green,
        Health::Degraded => Color::Yellow,
        Health::Bad => Color::Red,
        Health::IcmpBlocked => Color::Magenta,
        Health::Offline => Color::LightRed,
        Health::Unknown => Color::DarkGray,
    }
}

fn badge_fg(h: Health) -> Color {
    // Light backgrounds (green/yellow/light-red) need a dark fg to stay legible.
    match h {
        Health::Healthy | Health::Degraded | Health::Offline => Color::Black,
        _ => Color::White,
    }
}

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    let h_color = health_color(state.health);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(h_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let wifi = &state.wifi;
    let iface = wifi.interface.as_deref().unwrap_or("—");
    let rssi = wifi
        .rssi_dbm
        .map(|v| format!("{} dBm", v))
        .unwrap_or_else(|| "—".into());
    let channel = wifi.channel.as_deref().unwrap_or("—");

    let iface_display = match wifi.interface_label.as_deref() {
        Some(label) => format!("{} ({})", label, iface),
        None => iface.to_string(),
    };

    let dim = Style::default().fg(Color::DarkGray);
    let nstat_badge = Style::default()
        .fg(badge_fg(state.health))
        .bg(h_color)
        .add_modifier(Modifier::BOLD);

    let left = Line::from(vec![
        Span::styled(" nstat ", nstat_badge),
        Span::raw("  "),
        Span::styled(iface_display, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled("RSSI ", dim),
        Span::raw(rssi),
        Span::raw("  "),
        Span::styled("ch ", dim),
        Span::raw(channel),
    ]);

    let mid_spans: Vec<Span<'static>> = match (state.pubnet.isp.as_deref(), state.pubnet.ip.as_deref()) {
        (Some(isp), Some(ip)) => vec![
            Span::styled(isp.to_string(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("  ", dim),
            Span::styled(ip.to_string(), dim),
        ],
        (Some(isp), None) => vec![Span::styled(
            isp.to_string(),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )],
        (None, Some(ip)) => vec![Span::styled(ip.to_string(), dim)],
        (None, None) => vec![Span::styled("ISP —", dim)],
    };
    let mid = Line::from(mid_spans);

    let badge_style = Style::default()
        .fg(h_color)
        .add_modifier(Modifier::BOLD);
    let badge = Line::from(vec![
        Span::styled("● ", badge_style),
        Span::styled(state.health.label(), badge_style),
    ]);

    // Center `mid` relative to the *whole* header (not relative to whatever
    // space is left between the left content and the right badge), then carve
    // the side rects around it. Falls back gracefully if there isn't room.
    let mid_w = (mid.width() as u16).min(inner.width);
    let mid_x = inner.x + inner.width.saturating_sub(mid_w) / 2;
    let mid_rect = Rect { x: mid_x, y: inner.y, width: mid_w, height: inner.height };
    let left_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: mid_x.saturating_sub(inner.x),
        height: inner.height,
    };
    let right_start = mid_x.saturating_add(mid_w);
    let right_rect = Rect {
        x: right_start,
        y: inner.y,
        width: (inner.x + inner.width).saturating_sub(right_start),
        height: inner.height,
    };

    frame.render_widget(Paragraph::new(left), left_rect);
    frame.render_widget(Paragraph::new(mid), mid_rect);
    frame.render_widget(
        Paragraph::new(badge).alignment(Alignment::Right),
        right_rect,
    );
}
