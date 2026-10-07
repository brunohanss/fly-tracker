use std::{
    io::{self, IsTerminal},
    time::{Duration, Instant},
};

use crossterm::{
    cursor::Show,
    event, execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{
    app::Dashboard,
    event::{UiEvent, translate},
};

// Created before the first mutation so partial startup also restores the terminal.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}

pub fn run() -> io::Result<()> {
    run_inner(None, None, Vec::new())
}
pub fn run_connected(
    receiver: std::sync::mpsc::Receiver<telemetry::DashboardSnapshot>,
    commands: telemetry::Publisher<telemetry::ReplayCommand>,
    runs: Vec<telemetry::RunMetrics>,
) -> io::Result<()> {
    run_inner(Some(receiver), Some(commands), runs)
}
fn run_inner(
    receiver: Option<std::sync::mpsc::Receiver<telemetry::DashboardSnapshot>>,
    mut commands: Option<telemetry::Publisher<telemetry::ReplayCommand>>,
    mut runs: Vec<telemetry::RunMetrics>,
) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "Dashboard requires an interactive terminal; use --headless",
        ));
    }
    let _guard = TerminalGuard;
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.hide_cursor()?;
    let mut app = Dashboard::default();
    let interval = Duration::from_millis(67);
    let mut next_draw = Instant::now();
    let mut last_received = Instant::now();
    while !app.quit {
        if let Some(receiver) = &receiver {
            if let Some(snapshot) = telemetry::drain_latest(receiver) {
                last_received = snapshot.publication_time;
                if let Some(view) = &mut app.connected {
                    view.receive(snapshot);
                } else {
                    let mut view = crate::views::ConnectedView::new(snapshot);
                    view.runs = std::mem::take(&mut runs);
                    app.connected = Some(view);
                }
            }
            if last_received.elapsed() > Duration::from_millis(500)
                && let Some(view) = &mut app.connected
            {
                view.stale = true;
            }
        }
        if let Some(view) = &mut app.connected
            && let Some(command) = view.command.take()
            && let Some(commands) = &mut commands
            && !commands.publish(command)
        {
            view.notice = "Control channel full or disconnected".into();
        }
        let now = Instant::now();
        if now >= next_draw {
            app.update(UiEvent::Tick);
            terminal.draw(|frame| app.render(frame))?;
            next_draw = Instant::now() + interval;
        }
        if event::poll(next_draw.saturating_duration_since(Instant::now()))?
            && let Some(event) = translate(event::read()?)
        {
            app.update(event);
        }
    }
    Ok(())
}
