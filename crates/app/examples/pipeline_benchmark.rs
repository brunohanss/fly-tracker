//! Repeatable host comparison. TestBackend rendering, not terminal or Pi cost.
use aiming::calibration::Calibration;
use app::pipeline::ReplayPipeline;
use camera::synthetic::{Scenario, SyntheticSource};
use fly_core::config::Config;
use ratatui::{Terminal, backend::TestBackend};
use safety::FixtureDetector;
use std::time::Instant;
use telemetry::{Series, Stage, bounded, drain_latest};
use tui::{app::Dashboard, views::ConnectedView};

fn main() -> anyhow::Result<()> {
    for mode in ["headless", "normal", "stalled", "disconnected"] {
        let config = Config::default();
        let count = 2000;
        let source = Box::new(SyntheticSource::new(
            config.frame_size,
            Scenario::Reentry,
            42,
            count,
            10_000,
        )?);
        let calibration = Some(Calibration::virtual_plane(config.frame_size)?);
        let mut pipeline = ReplayPipeline::new(
            source,
            Box::new(FixtureDetector),
            config,
            "benchmark:reentry:42:2000:10000".into(),
            calibration,
        )?;
        let (mut publication, receiver) = bounded(2)?;
        let receiver = if mode == "disconnected" {
            drop(receiver);
            None
        } else {
            Some(receiver)
        };
        let mut dashboard = Dashboard::default();
        let mut terminal = Terminal::new(TestBackend::new(120, 35))?;
        let mut aggregation = Series::new(1024)?;
        let mut publish = Series::new(1024)?;
        let mut draw = Series::new(1024)?;
        let start = Instant::now();
        while pipeline.step()? {
            // 100Hz source, one publication per 7 frames (~14.3Hz source time).
            if mode != "headless" && pipeline.counters().processed_frames % 7 == 0 {
                let tick = Instant::now();
                let snapshot = pipeline.snapshot();
                aggregation.push(tick.elapsed().as_secs_f64() * 1_000_000.0)?;
                let tick = Instant::now();
                publication.publish(snapshot);
                publish.push(tick.elapsed().as_secs_f64() * 1_000_000.0)?;
                if mode == "normal"
                    && let Some(receiver) = &receiver
                    && let Some(snapshot) = drain_latest(receiver)
                {
                    if let Some(view) = &mut dashboard.connected {
                        view.receive(snapshot);
                    } else {
                        dashboard.connected = Some(ConnectedView::new(snapshot));
                    }
                    let tick = Instant::now();
                    terminal.draw(|frame| dashboard.render(frame))?;
                    draw.push(tick.elapsed().as_secs_f64() * 1_000_000.0)?;
                }
            }
        }
        let wall_seconds = start.elapsed().as_secs_f64();
        let report = pipeline.report()?;
        eprintln!(
            "{mode}: frames={} wall={wall_seconds:.3}s dropped_frames={} dropped_snapshots={} pipeline_us={:?} aggregation_us={:?} publication_us={:?} draw_us={:?}",
            report.counters.processed_frames,
            report.counters.dropped_frames,
            publication.dropped,
            report.latency[Stage::FrameProcessing.index()],
            aggregation.distribution(),
            publish.distribution(),
            draw.distribution()
        );
    }
    Ok(())
}
