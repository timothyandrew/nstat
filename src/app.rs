use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use tokio::sync::RwLock;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::time::{Instant, interval};

use crate::state::AppState;
use crate::stats::classify;
use crate::ui;

const REDRAW_INTERVAL: Duration = Duration::from_millis(250);

pub enum AppEvent {
    Key(crossterm::event::KeyEvent),
    Quit,
}

pub fn spawn_input() -> UnboundedReceiver<AppEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        loop {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                        if tx.send(AppEvent::Key(k)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
        let _ = tx.send(AppEvent::Quit);
    });
    rx
}

pub async fn run(
    mut terminal: DefaultTerminal,
    state: Arc<RwLock<AppState>>,
    mut input: UnboundedReceiver<AppEvent>,
) -> anyhow::Result<()> {
    let mut redraw = interval(REDRAW_INTERVAL);
    redraw.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_classify = Instant::now();

    loop {
        tokio::select! {
            _ = redraw.tick() => {
                if last_classify.elapsed() >= Duration::from_millis(500) {
                    let new_health = {
                        let s = state.read().await;
                        classify(&s)
                    };
                    let mut s = state.write().await;
                    s.health = new_health;
                    last_classify = Instant::now();
                }
                let snapshot = state.read().await;
                terminal.draw(|f| ui::draw(f, &snapshot))?;
            }
            event = input.recv() => {
                match event {
                    Some(AppEvent::Key(k)) => {
                        if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                            return Ok(());
                        }
                        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                            return Ok(());
                        }
                        if matches!(k.code, KeyCode::Char('w')) {
                            let mut s = state.write().await;
                            s.cycle_window();
                        }
                    }
                    Some(AppEvent::Quit) | None => return Ok(()),
                }
            }
        }
    }
}
