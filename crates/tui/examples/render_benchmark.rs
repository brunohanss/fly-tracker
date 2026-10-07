//! Host-only foundation rendering measurement; no pipeline performance claim.
use ratatui::{Terminal, backend::TestBackend};
use std::{convert::Infallible, time::Instant};
use tui::app::{Dashboard, Screen};

fn main() -> Result<(), Infallible> {
    let mut terminal = Terminal::new(TestBackend::new(120, 35))?;
    let mut app = Dashboard::default();
    let mut durations = Vec::with_capacity(1_000);
    for index in 0..1_100 {
        app.screen = Screen::ALL[index % Screen::ALL.len()];
        let start = Instant::now();
        terminal.draw(|frame| app.render(frame))?;
        if index >= 100 {
            durations.push(start.elapsed().as_nanos());
        }
    }
    durations.sort_unstable();
    // This example is a measurement tool, not the production logging system.
    eprintln!(
        "TestBackend 120x35, 100 warmups, 1000 draws: P50={}us P95={}us P99={}us max={}us",
        durations[499] / 1000,
        durations[949] / 1000,
        durations[989] / 1000,
        durations[999] / 1000
    );
    Ok(())
}
