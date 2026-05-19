use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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

pub fn draw(frame: &mut Frame, area: Rect, state: &AppState) {
    let h_color = health_color(state.health);

    let title = Line::from(vec![
        Span::styled(" nstat ", Style::default().fg(Color::White).bg(h_color).add_modifier(Modifier::BOLD)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(h_color))
        .title(title);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),
            Constraint::Length(16),
        ])
        .split(inner);

    let wifi = &state.wifi;
    let iface = wifi.interface.as_deref().unwrap_or("—");
    let rssi = wifi
        .rssi_dbm
        .map(|v| format!("{} dBm", v))
        .unwrap_or_else(|| "—".into());
    let channel = wifi.channel.as_deref().unwrap_or("—");

    // macOS 15+ returns "<redacted>" for SSID/BSSID unless the calling terminal
    // has Location Services access. Surface the cause instead of the noise.
    let ssid_is_redacted = wifi
        .ssid
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("<redacted>"));
    let ssid_span = if ssid_is_redacted {
        Span::styled(
            "(grant Location to terminal)",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )
    } else {
        let ssid = wifi.ssid.as_deref().unwrap_or("—");
        Span::raw(format!("\"{}\"", ssid))
    };

    let left = Line::from(vec![
        Span::styled(iface, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled("SSID ", Style::default().fg(Color::DarkGray)),
        ssid_span,
        Span::raw("  "),
        Span::styled("RSSI ", Style::default().fg(Color::DarkGray)),
        Span::raw(rssi),
        Span::raw("  "),
        Span::styled("ch ", Style::default().fg(Color::DarkGray)),
        Span::raw(channel),
    ]);

    let badge_style = Style::default()
        .fg(h_color)
        .add_modifier(Modifier::BOLD);
    let badge = Line::from(vec![
        Span::styled("● ", badge_style),
        Span::styled(state.health.label(), badge_style),
    ]);

    frame.render_widget(Paragraph::new(left), cols[0]);
    frame.render_widget(
        Paragraph::new(badge).alignment(ratatui::layout::Alignment::Right),
        cols[1],
    );
}
