use crate::pipeline::ReplayPipeline;
use anyhow::Result;
use std::{
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};
use telemetry::{DashboardSnapshot, Publisher, ReplayCommand, RunMetrics};

pub fn run_headless(pipeline: &mut ReplayPipeline) -> Result<RunMetrics> {
    while pipeline.step()? {}
    pipeline.report()
}
pub fn run_observed(
    mut pipeline: ReplayPipeline,
    mut publication: Publisher<DashboardSnapshot>,
    commands: Receiver<ReplayCommand>,
) -> Result<Option<RunMetrics>> {
    let mut paused = false;
    let mut speed = 1.0;
    let mut next_frame = Instant::now();
    let mut next_publication = Instant::now();
    loop {
        loop {
            match commands.try_recv() {
                Ok(ReplayCommand::Stop) => {
                    let report = if pipeline.complete() {
                        Some(pipeline.report()?)
                    } else {
                        None
                    };
                    pipeline.shutdown();
                    return Ok(report);
                }
                Ok(ReplayCommand::TogglePause) => {
                    paused = !paused;
                    pipeline.set_paused(paused);
                    next_frame = Instant::now();
                }
                Ok(ReplayCommand::Speed(value))
                    if value.is_finite() && (0.125..=16.0).contains(&value) =>
                {
                    speed = value;
                }
                Ok(ReplayCommand::Step(delta)) if paused || pipeline.complete() => {
                    let current = pipeline.counters().processed_frames.saturating_sub(1);
                    let target = if delta < 0 {
                        current.saturating_sub(delta.unsigned_abs())
                    } else {
                        current.saturating_add(delta as u64)
                    };
                    if let Err(error) = pipeline.seek(target) {
                        tracing::warn!(%error,"Replay step rejected");
                    }
                    paused = true;
                    pipeline.set_paused(true);
                    next_publication = Instant::now();
                }
                Ok(ReplayCommand::Seek(index)) => {
                    if let Err(error) = pipeline.seek(index) {
                        tracing::warn!(%error,"Replay seek rejected");
                    }
                    paused = true;
                    pipeline.set_paused(true);
                    next_publication = Instant::now();
                }
                Ok(_) => tracing::warn!("Unsupported or unsafe replay command rejected"),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let report = if pipeline.complete() {
                        Some(pipeline.report()?)
                    } else {
                        None
                    };
                    pipeline.shutdown();
                    return Ok(report);
                }
            }
        }
        if !paused && !pipeline.complete() && Instant::now() >= next_frame {
            let previous = pipeline.frame().timestamp;
            if let Err(error) = pipeline.step() {
                publication.publish(pipeline.snapshot());
                return Err(error);
            }
            let timestamp = pipeline.frame().timestamp;
            let period = pipeline
                .next_timestamp()
                .map_or_else(
                    || timestamp.0.saturating_sub(previous.0),
                    |next| next.0.saturating_sub(timestamp.0),
                )
                .max(1);
            // Schedule from source time. Processing time does not add to every replay interval.
            next_frame += Duration::from_secs_f64(period as f64 / 1_000_000.0 / speed);
        }
        if Instant::now() >= next_publication {
            let mut snapshot = pipeline.snapshot();
            snapshot.paused = paused;
            snapshot.replay_speed = speed;
            publication.publish(snapshot);
            next_publication = Instant::now() + Duration::from_millis(67);
        }
        // Dedicated observer worker only; this sleep never delays a live pipeline.
        std::thread::sleep(Duration::from_millis(1));
    }
}
