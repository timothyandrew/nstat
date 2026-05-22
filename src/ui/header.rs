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

    let dim = Style::default().fg(Color::DarkGray);
    let cyan_bold = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let nstat_badge = Style::default()
        .fg(badge_fg(state.health))
        .bg(h_color)
        .add_modifier(Modifier::BOLD);

    // The left segment describes the active connection. WiFi RSSI/channel take
    // priority when associated; otherwise fall back to the Ethernet link's
    // speed/duplex. Sections with no data are dropped rather than shown as "—".
    let wifi = &state.wifi;
    let eth = &state.ethernet;
    let on_wifi = wifi.rssi_dbm.is_some() || wifi.channel.is_some();

    let (iface, iface_label, metrics): (Option<&str>, Option<&str>, Vec<Span<'static>>) =
        if on_wifi {
            let mut spans: Vec<Span<'static>> = Vec::new();
            if let Some(rssi) = wifi.rssi_dbm {
                spans.push(Span::styled("RSSI ", dim));
                spans.push(Span::raw(format!("{rssi} dBm")));
            }
            if let Some(ch) = wifi.channel.as_deref() {
                if !spans.is_empty() {
                    spans.push(Span::raw("  "));
                }
                spans.push(Span::styled("ch ", dim));
                spans.push(Span::raw(ch.to_string()));
            }
            (wifi.interface.as_deref(), wifi.interface_label.as_deref(), spans)
        } else if eth.interface.is_some() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            if let Some(speed) = eth.link_speed.as_deref() {
                spans.push(Span::styled("link ", dim));
                spans.push(Span::raw(speed.to_string()));
            }
            if let Some(full) = eth.full_duplex {
                if !spans.is_empty() {
                    spans.push(Span::raw("  "));
                }
                spans.push(Span::styled(
                    if full { "full-duplex" } else { "half-duplex" },
                    dim,
                ));
            }
            (eth.interface.as_deref(), eth.interface_label.as_deref(), spans)
        } else {
            (wifi.interface.as_deref(), wifi.interface_label.as_deref(), Vec::new())
        };

    let iface_display = match (iface_label, iface) {
        (Some(label), Some(dev)) => format!("{label} ({dev})"),
        (Some(label), None) => label.to_string(),
        (None, Some(dev)) => dev.to_string(),
        (None, None) => "—".to_string(),
    };

    let mut left_spans: Vec<Span<'static>> = vec![
        Span::styled(" nstat ", nstat_badge),
        Span::raw("  "),
        Span::styled(iface_display, cyan_bold),
    ];
    if !metrics.is_empty() {
        left_spans.push(Span::raw("  "));
        left_spans.extend(metrics);
    }
    let left = Line::from(left_spans);

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
