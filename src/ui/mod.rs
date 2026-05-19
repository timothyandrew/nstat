pub mod chart;
pub mod footer;
pub mod header;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::state::AppState;

pub fn draw(frame: &mut Frame, state: &AppState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(8),    // chart
            Constraint::Length(4), // footer
        ])
        .split(area);

    header::draw(frame, chunks[0], state);
    chart::draw(frame, chunks[1], state);
    footer::draw(frame, chunks[2], state);
}
