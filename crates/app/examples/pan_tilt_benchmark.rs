//! Host-only simulator update and command cost. No I2C, camera, or physical motion.
use aiming::{
    AimRequest, AimingDevice,
    pan_tilt::{MotionState, VirtualPanTilt},
};
use fly_core::{
    Confidence, FrameId, FrameTimestamp, NormalizedAim, PixelPosition, TargetId, config::Config,
    servo::PanTiltConfig,
};
use safety::{Evidence, EvidenceScope, SafetyAuthority, Verdict};
use std::{hint::black_box, time::Instant};
use telemetry::Series;

fn main() -> anyhow::Result<()> {
    let mut servo = VirtualPanTilt::new(
        PanTiltConfig::simulation_example(),
        EvidenceScope::ReplayFixture,
    )?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    let mut updates = Series::new(20_000)?;
    let mut commands = Series::new(1024)?;
    let mut settled_frames = 0;
    for index in 0..20_000 {
        let now = FrameTimestamp(index * 10_000);
        let frame = FrameId(index);
        gate.record_camera(frame, now);
        gate.observe(Evidence {
            frame,
            timestamp: now,
            scope: EvidenceScope::ReplayFixture,
            verdict: Verdict::Clear(Confidence::new(1.0)?),
        });
        let start = Instant::now();
        servo.advance(&mut gate, now, frame)?;
        black_box(servo.snapshot());
        updates.push(start.elapsed().as_secs_f64() * 1_000_000.0)?;
        if servo.snapshot().state == MotionState::Settled {
            settled_frames += 1;
        }
        if index % 200 == 0 {
            let value = if index % 400 == 0 { 1.0 } else { -1.0 };
            let request = AimRequest {
                frame,
                target: TargetId(1),
                predicted_pixel: PixelPosition::new(10.0, 10.0)?,
                aim: NormalizedAim::new(value, -value)?,
            };
            let start = Instant::now();
            anyhow::ensure!(
                servo.aim(&mut gate, now, request)?.issued,
                "simulation command denied"
            );
            commands.push(start.elapsed().as_secs_f64() * 1_000_000.0)?;
        }
    }
    eprintln!(
        "pan_tilt: updates_us={:?} commands_us={:?} settled_frames={settled_frames}",
        updates.distribution(),
        commands.distribution()
    );
    Ok(())
}
