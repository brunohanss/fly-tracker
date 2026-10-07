use aiming::calibration::Calibration;
use app::pipeline::ReplayPipeline;
use camera::synthetic::{Scenario, SyntheticSource};
use fly_core::{FrameSize, config::Config};
use ratatui::{Terminal, backend::TestBackend};
use safety::FixtureDetector;
use tui::{
    app::{Dashboard, Screen},
    event::UiEvent,
    views::ConnectedView,
};

fn view(scenario: Scenario) -> anyhow::Result<ConnectedView> {
    let mut config = Config {
        frame_size: FrameSize::new(96, 64)?,
        ..Config::default()
    };
    config.processing.virtual_aim_delay_us = 0;
    let source = Box::new(SyntheticSource::new(
        config.frame_size,
        scenario,
        42,
        20,
        10_000,
    )?);
    let calibration = Some(Calibration::virtual_plane(config.frame_size)?);
    let mut pipeline = ReplayPipeline::new(
        source,
        Box::new(FixtureDetector),
        config,
        "test".into(),
        calibration,
    )?;
    for _ in 0..10 {
        pipeline.step()?;
    }
    Ok(ConnectedView::new(pipeline.snapshot()))
}
fn render(dashboard: &Dashboard, width: u16, height: u16) -> anyhow::Result<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    terminal.draw(|frame| dashboard.render(frame))?;
    let mut text = String::new();
    for row in 0..height {
        for column in 0..width {
            text.push_str(terminal.backend().buffer()[(column, row)].symbol());
        }
        text.push('\n');
    }
    Ok(text)
}
#[test]
fn connected_layouts_preserve_recorded_safety_and_staleness() -> anyhow::Result<()> {
    for scenario in [
        Scenario::EmptyWall,
        Scenario::SlowFly,
        Scenario::MultipleFlies,
        Scenario::FlyHuman,
    ] {
        let mut dashboard = Dashboard {
            connected: Some(view(scenario)?),
            ..Dashboard::default()
        };
        for screen in Screen::ALL {
            dashboard.screen = screen;
            for (width, height) in [(160, 45), (80, 24), (40, 12)] {
                let text = render(&dashboard, width, height)?;
                assert!(text.contains("OUTPUT DISABLED"));
                assert!(!text.contains("OUTPUT PERMITTED"));
                if scenario.hazard().is_some() {
                    assert!(text.contains("SAFETY LOCKOUT"));
                } else {
                    assert!(text.contains("RECORDED CLEAR"));
                }
            }
        }
        dashboard
            .connected
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("missing view"))?
            .stale = true;
        let text = render(&dashboard, 80, 24)?;
        assert!(text.contains("SAFETY UNAVAILABLE"));
        assert!(text.contains("OUTPUT DISABLED"));
        dashboard.help = true;
        assert!(render(&dashboard, 80, 24)?.contains("SAFETY UNAVAILABLE"));
        for (width, height) in [(0, 0), (1, 1), (20, 2), (40, 4)] {
            render(&dashboard, width, height)?;
        }
    }
    Ok(())
}
#[test]
fn controls_keep_selection_separate_from_replay_commands() -> anyhow::Result<()> {
    let mut view = view(Scenario::MultipleFlies)?;
    let first = view.selected;
    view.update(UiEvent::CycleFocus);
    view.update(UiEvent::Step(1));
    assert_ne!(first, view.selected);
    assert!(view.command.is_none());
    view.update(UiEvent::CycleFocus);
    view.update(UiEvent::Step(1));
    assert!(view.command.is_none());
    view.snapshot.paused = true;
    view.update(UiEvent::Step(-10));
    assert_eq!(view.command, Some(telemetry::ReplayCommand::Step(-10)));
    view.update(UiEvent::TogglePause);
    assert_eq!(view.command, Some(telemetry::ReplayCommand::TogglePause));
    Ok(())
}
#[test]
fn queued_old_snapshots_are_stale_even_when_received_now() -> anyhow::Result<()> {
    let mut view = view(Scenario::SlowFly)?;
    let mut delayed = view.snapshot.clone();
    delayed.publication_time = std::time::Instant::now() - std::time::Duration::from_secs(2);
    view.receive(delayed);
    assert!(view.stale);
    assert!(view.safety_text().contains("SAFETY UNAVAILABLE"));
    Ok(())
}

#[test]
fn image_modes_and_mask_use_current_snapshot() -> anyhow::Result<()> {
    let mut dashboard = Dashboard {
        connected: Some(view(Scenario::SlowFly)?),
        ..Dashboard::default()
    };
    assert!(render(&dashboard, 120, 35)?.contains('▀'));
    dashboard.update(UiEvent::ToggleImageStyle);
    assert!(!render(&dashboard, 120, 35)?.contains('▀'));
    dashboard.update(UiEvent::ToggleForeground);
    assert!(render(&dashboard, 120, 35)?.contains("Foreground mask"));
    dashboard.update(UiEvent::ToggleImageStyle);
    assert!(render(&dashboard, 120, 35)?.contains('▀'));
    Ok(())
}

#[test]
fn preview_and_overlay_are_aligned_and_old_aims_are_hidden() -> anyhow::Result<()> {
    use fly_core::{FrameId, PixelPosition};
    let mut view = view(Scenario::SlowFly)?;
    let mut snapshot = view.snapshot.clone();
    snapshot.preview = telemetry::Preview {
        width: 2,
        height: 2,
        pixels: vec![64; 4],
    };
    snapshot.tracks.clear();
    snapshot.predictions.clear();
    snapshot.aim = None;
    snapshot
        .predictions
        .push((fly_core::TargetId(999), PixelPosition::new(48.0, 32.0)?));
    view.receive(snapshot);
    let mut dashboard = Dashboard {
        connected: Some(view),
        ..Dashboard::default()
    };
    let mut terminal = Terminal::new(TestBackend::new(120, 35))?;
    terminal.draw(|frame| dashboard.render(frame))?;
    let buffer = terminal.backend().buffer();
    let marker = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "P" && cell.fg == ratatui::style::Color::Cyan)
        .ok_or_else(|| anyhow::anyhow!("Prediction overlay missing"))?;
    assert_eq!(marker.bg, ratatui::style::Color::Black);
    let view = dashboard
        .connected
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("Missing view"))?;
    view.snapshot.aim = Some(aiming::AimRecord {
        request: aiming::AimRequest {
            frame: FrameId(99999),
            target: fly_core::TargetId(999),
            predicted_pixel: PixelPosition::new(48.0, 32.0)?,
            aim: fly_core::NormalizedAim::new(0.0, 0.0)?,
        },
        issued: true,
        suppressed_by: None,
    });
    terminal.draw(|frame| dashboard.render(frame))?;
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|cell| cell.symbol() == "P")
    );
    Ok(())
}
