//! Host-only adapter cost against a register mock. No I2C or GPIO timing claim.
use aiming::{AimRequest, AimingDevice};
use fly_core::{
    Confidence, FrameId, FrameTimestamp, NormalizedAim, PixelPosition, TargetId, config::Config,
    servo::PanTiltConfig,
};
use hardware::pca9685::{MockPca9685Aimer, Pca9685Config};
use safety::{Evidence, EvidenceScope, SafetyAuthority, Verdict};
use std::{hint::black_box, time::Instant};
use telemetry::Series;

fn main() -> anyhow::Result<()> {
    let config: Pca9685Config =
        serde_json::from_str(include_str!("../../../config/pca9685-mock.json"))?;
    let mut device = MockPca9685Aimer::new(
        config,
        PanTiltConfig::simulation_example(),
        EvidenceScope::ReplayFixture,
    )?;
    device.initialize()?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    let mut samples = Series::new(2000)?;
    let verdict = Verdict::Clear(Confidence::new(1.0)?);
    let point = PixelPosition::new(10.0, 10.0)?;
    // Measure 100 commands per sample to reduce timer quantization.
    for batch in 0..2100u64 {
        let start = Instant::now();
        for item in 0..100 {
            let index = batch * 100 + item;
            let frame = FrameId(index);
            let now = FrameTimestamp(index * 10_000);
            gate.record_camera(frame, now);
            gate.observe(Evidence {
                frame,
                timestamp: now,
                scope: EvidenceScope::ReplayFixture,
                verdict,
            });
            let x = if index % 2 == 0 { 1.0 } else { -1.0 };
            let request = AimRequest {
                frame,
                target: TargetId(1),
                predicted_pixel: point,
                aim: NormalizedAim::new(x, -x)?,
            };
            anyhow::ensure!(
                black_box(device.aim(&mut gate, now, request)?).issued,
                "mock command denied"
            );
        }
        if batch >= 100 {
            samples.push(start.elapsed().as_secs_f64() * 1_000_000.0 / 100.0)?;
        }
    }
    let status = device.snapshot();
    device.stop();
    eprintln!(
        "pca9685 mock: command_batch_average_us={:?} total_commands={} actual_frequency_hz={:.6} prescale={} trace_events={} inhibited_after_stop={}",
        samples.distribution(),
        status.mock_commands,
        status.timing.actual_frequency_hz(),
        status.timing.prescale(),
        device.mock().trace().len(),
        device.mock().inhibited()
    );
    Ok(())
}
