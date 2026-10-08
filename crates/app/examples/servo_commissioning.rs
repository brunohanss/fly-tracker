//! Generate observations from an explicit synthetic camera model, never from hardware.
use aiming::commissioning::{Axis, CommissioningSession, Provenance, ServoSamples};
use fly_core::{
    Confidence, FrameId, FrameSize, FrameTimestamp, PixelPosition, config::Config,
    servo::PanTiltConfig,
};
use safety::{Evidence, EvidenceScope, SafetyAuthority, Verdict};
use std::io::Write;

fn clear(gate: &mut SafetyAuthority, time: u64) -> anyhow::Result<()> {
    gate.record_camera(FrameId(time), FrameTimestamp(time));
    gate.observe(Evidence {
        frame: FrameId(time),
        timestamp: FrameTimestamp(time),
        scope: EvidenceScope::ReplayFixture,
        verdict: Verdict::Clear(Confidence::new(1.0)?),
    });
    Ok(())
}
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        anyhow::bail!("usage: servo_commissioning NEW_SAMPLES.json");
    }
    let profile = PanTiltConfig::simulation_example();
    let mut session = CommissioningSession::new(profile)?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    let size = FrameSize::new(101, 81)?;
    let mut time = 0;
    let mut samples = ServoSamples {
        version: 1,
        provenance: Provenance::SimulationOnly,
        assembly_id: "synthetic-bench-v1".into(),
        camera_size: size,
        profile,
        fit_points: Vec::new(),
        validation_points: Vec::new(),
    };
    for (index, (pan, tilt)) in [
        (-5.0, -5.0),
        (5.0, -5.0),
        (5.0, 5.0),
        (-5.0, 5.0),
        (0.0, 0.0),
        (2.0, 2.0),
    ]
    .into_iter()
    .enumerate()
    {
        for (axis, destination) in [(Axis::Pan, pan), (Axis::Tilt, tilt)] {
            loop {
                let current = match axis {
                    Axis::Pan => session.snapshot().simulated.pan.degrees(),
                    Axis::Tilt => session.snapshot().simulated.tilt.degrees(),
                };
                let delta: f64 = destination - current;
                if delta.abs() < 1e-9 {
                    break;
                }
                time += 1;
                clear(&mut gate, time)?;
                if !session.jog(
                    &mut gate,
                    FrameTimestamp(time),
                    FrameId(time),
                    axis,
                    delta.clamp(-5.0, 5.0),
                )? {
                    anyhow::bail!("simulation safety denied jog");
                }
                time += 200_000;
                clear(&mut gate, time)?;
                session.advance(&mut gate, FrameTimestamp(time), FrameId(time))?;
            }
        }
        let position = session.snapshot().simulated;
        // Explicit ground truth: cross-axis coupling in a synthetic planar model.
        let pixel = PixelPosition::new(
            50.0 + 3.0 * position.pan.degrees() + 0.4 * position.tilt.degrees(),
            40.0 + 0.2 * position.pan.degrees() + 3.0 * position.tilt.degrees(),
        )?;
        time += 1;
        clear(&mut gate, time)?;
        let sample = session.sample(&mut gate, FrameTimestamp(time), FrameId(time), pixel, size)?;
        if index < 4 {
            samples.fit_points.push(sample);
        } else {
            samples.validation_points.push(sample);
        }
    }
    session.stop();
    samples.fit("simulation".into(), 0.01)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[0])?;
    file.write_all(&serde_json::to_vec_pretty(&samples)?)?;
    file.sync_all()?;
    Ok(())
}
