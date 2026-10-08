use aiming::{
    AimingError,
    commissioning::{Axis, CommissioningSession, Provenance, ServoSample, ServoSamples},
};
use fly_core::{
    Confidence, FrameId, FrameSize, FrameTimestamp, NormalizedAim, PixelPosition, config::Config,
    servo::PanTiltConfig,
};
use safety::{Evidence, EvidenceScope, SafetyAuthority, Verdict};

fn observe(gate: &mut SafetyAuthority, t: u64, verdict: Verdict) {
    gate.record_camera(FrameId(t), FrameTimestamp(t));
    gate.observe(Evidence {
        frame: FrameId(t),
        timestamp: FrameTimestamp(t),
        scope: EvidenceScope::ReplayFixture,
        verdict,
    });
}
fn samples() -> Result<ServoSamples, AimingError> {
    let profile = PanTiltConfig::simulation_example();
    let make = |id, x, y| -> Result<ServoSample, AimingError> {
        Ok(ServoSample {
            frame: FrameId(id),
            timestamp: FrameTimestamp(id),
            pixel: PixelPosition::new(x, y)?,
            position: profile.map(NormalizedAim::new((x - 50.0) / 50.0, (y - 40.0) / 40.0)?)?,
        })
    };
    Ok(ServoSamples {
        version: 1,
        provenance: Provenance::SimulationOnly,
        assembly_id: "bench-simulation".into(),
        camera_size: FrameSize::new(101, 81)?,
        profile,
        fit_points: vec![
            make(1, 10.0, 10.0)?,
            make(2, 90.0, 10.0)?,
            make(3, 90.0, 70.0)?,
            make(4, 10.0, 70.0)?,
        ],
        validation_points: vec![make(5, 30.0, 20.0)?, make(6, 65.0, 55.0)?],
    })
}
#[test]
fn fit_round_trip_bindings_and_sampled_region() -> Result<(), Box<dyn std::error::Error>> {
    let s = samples()?;
    let report = s.fit("simulation".into(), 0.01)?;
    assert!(report.validation_pixels.maximum < 1e-8);
    for x in (10..=90).step_by(5) {
        for y in (10..=70).step_by(5) {
            let pixel = PixelPosition::new(f64::from(x), f64::from(y))?;
            let position = report.map(pixel, s.profile, s.camera_size, &s.assembly_id)?;
            assert!((position.pan.degrees() - (f64::from(x) - 50.0) * 0.9).abs() < 1e-8);
            assert!((position.tilt.degrees() - (f64::from(y) - 40.0) * 1.125).abs() < 1e-8);
            let normalized = s.profile.normalized(position)?;
            assert_eq!(s.profile.map(normalized)?, position);
        }
    }
    assert!(
        report
            .map(
                PixelPosition::new(0.0, 0.0)?,
                s.profile,
                s.camera_size,
                &s.assembly_id
            )
            .is_err()
    );
    let mut wrong = s.profile;
    wrong.pan.reversed = true;
    assert!(
        report
            .map(
                PixelPosition::new(50.0, 40.0)?,
                wrong,
                s.camera_size,
                &s.assembly_id
            )
            .is_err()
    );
    assert!(
        report
            .map(
                PixelPosition::new(50.0, 40.0)?,
                s.profile,
                s.camera_size,
                "another-mount"
            )
            .is_err()
    );
    Ok(())
}
#[test]
fn reject_bad_holdouts_duplicate_observations_and_degenerate_fit() -> Result<(), AimingError> {
    let mut s = samples()?;
    s.validation_points[0].pixel = PixelPosition::new(35.0, 20.0)?;
    assert!(s.fit("simulation".into(), 1.0).is_err());
    assert!(s.fit("simulation".into(), f64::NAN).is_err());
    let mut s = samples()?;
    s.validation_points[0] = s.fit_points[0];
    assert!(s.fit("simulation".into(), 1.0).is_err());
    let mut s = samples()?;
    s.version = 2;
    assert!(s.fit("simulation".into(), 1.0).is_err());
    let mut s = samples()?;
    for (i, p) in s.fit_points.iter_mut().enumerate() {
        p.pixel = PixelPosition::new(10.0 + i as f64, 10.0)?;
    }
    assert!(s.fit("simulation".into(), 1.0).is_err());
    Ok(())
}
#[test]
fn jog_has_no_queue_and_samples_require_settled_fresh_clearance()
-> Result<(), Box<dyn std::error::Error>> {
    let mut session = CommissioningSession::new(PanTiltConfig::simulation_example())?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    let size = FrameSize::new(101, 81)?;
    let pixel = PixelPosition::new(50.0, 40.0)?;
    observe(&mut gate, 0, Verdict::Clear(Confidence::new(1.0)?));
    assert!(session.jog(&mut gate, FrameTimestamp(0), FrameId(0), Axis::Pan, 5.0)?);
    assert!(
        session
            .jog(&mut gate, FrameTimestamp(0), FrameId(0), Axis::Tilt, 5.0)
            .is_err()
    );
    assert!(
        session
            .sample(&mut gate, FrameTimestamp(0), FrameId(0), pixel, size)
            .is_err()
    );
    observe(&mut gate, 200_000, Verdict::Clear(Confidence::new(1.0)?));
    let sample = session.sample(
        &mut gate,
        FrameTimestamp(200_000),
        FrameId(200_000),
        pixel,
        size,
    )?;
    assert_eq!(sample.position.pan.degrees(), 5.0);
    assert_eq!(sample.position.tilt.degrees(), 0.0);
    assert!(
        session
            .jog(
                &mut gate,
                FrameTimestamp(200_000),
                FrameId(200_000),
                Axis::Pan,
                5.001
            )
            .is_err()
    );
    assert!(
        session
            .sample(
                &mut gate,
                FrameTimestamp(250_001),
                FrameId(200_000),
                pixel,
                size
            )
            .is_err()
    );
    for (index, verdict) in [
        Verdict::Unavailable,
        Verdict::Error,
        Verdict::Uncertain,
        Verdict::Hazard(camera::Hazard::Human),
        Verdict::Hazard(camera::Hazard::Dog),
        Verdict::Hazard(camera::Hazard::Cat),
    ]
    .into_iter()
    .enumerate()
    {
        let time = 300_000 + index as u64;
        observe(&mut gate, time, verdict);
        assert!(!session.jog(
            &mut gate,
            FrameTimestamp(time),
            FrameId(time),
            Axis::Tilt,
            -5.0
        )?);
        assert!(
            session
                .sample(&mut gate, FrameTimestamp(time), FrameId(time), pixel, size)
                .is_err()
        );
    }
    session.stop();
    Ok(())
}
#[test]
fn report_save_is_non_overwriting_and_load_recomputes() -> Result<(), Box<dyn std::error::Error>> {
    let report = samples()?.fit("simulation".into(), 0.01)?;
    let path = std::env::temp_dir().join(format!("servo-report-{}.json", std::process::id()));
    report.save_new(&path)?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        assert!(report.save_new(&path).is_err());
        let mut value = serde_json::to_value(&report)?;
        value["fit"]["calibration"]["matrix"][0][0] = serde_json::json!(123.0);
        std::fs::write(&path, serde_json::to_vec(&value)?)?;
        let loaded = aiming::commissioning::ServoFitReport::load(&path)?;
        assert!(loaded.validation_pixels.maximum < 1e-8);
        assert_ne!(loaded.fit.calibration.matrix[0][0], 123.0);
        Ok(())
    })();
    std::fs::remove_file(path)?;
    result
}
