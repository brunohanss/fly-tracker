use aiming::{
    AimRequest, AimingDevice,
    pan_tilt::{MotionState, VirtualPanTilt},
};
use fly_core::{
    Confidence, FrameId, FrameTimestamp, NormalizedAim, PixelPosition, TargetId, config::Config,
    servo::PanTiltConfig,
};
use safety::{Evidence, EvidenceScope, SafetyAuthority, Verdict};

fn observe(gate: &mut SafetyAuthority, time: u64, verdict: Verdict) {
    gate.record_camera(FrameId(time), FrameTimestamp(time));
    gate.observe(Evidence {
        frame: FrameId(time),
        timestamp: FrameTimestamp(time),
        scope: EvidenceScope::ReplayFixture,
        verdict,
    });
}
fn clear(gate: &mut SafetyAuthority, time: u64) -> Result<(), fly_core::DomainError> {
    observe(gate, time, Verdict::Clear(Confidence::new(1.0)?));
    Ok(())
}
fn request(time: u64, x: f64, y: f64) -> Result<AimRequest, fly_core::DomainError> {
    Ok(AimRequest {
        frame: FrameId(time),
        target: TargetId(1),
        predicted_pixel: PixelPosition::new(10.0, 10.0)?,
        aim: NormalizedAim::new(x, y)?,
    })
}
#[test]
fn measured_intervals_move_both_axes_then_settle_without_overshoot()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = PanTiltConfig::simulation_example();
    config.tilt.max_speed_degrees_per_s = 45.0;
    let mut servo = VirtualPanTilt::new(config, EvidenceScope::ReplayFixture)?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    assert!(
        servo
            .aim(&mut gate, FrameTimestamp(0), request(0, 1.0, -1.0)?)?
            .issued
    );
    for (time, pan, tilt, state) in [
        (125_000, 11.25, -5.625, MotionState::Moving),
        (500_000, 45.0, -22.5, MotionState::Moving),
        (1_000_000, 45.0, -45.0, MotionState::Settling),
        (1_099_999, 45.0, -45.0, MotionState::Settling),
        (1_100_000, 45.0, -45.0, MotionState::Settled),
        (2_000_000, 45.0, -45.0, MotionState::Settled),
    ] {
        clear(&mut gate, time)?;
        servo.advance(&mut gate, FrameTimestamp(time), FrameId(time))?;
        let s = servo.snapshot();
        assert_eq!(s.simulated.pan.degrees(), pan);
        assert_eq!(s.simulated.tilt.degrees(), tilt);
        assert_eq!(s.state, state, "time={time}");
    }
    // An identical request must not restart settling.
    servo.aim(
        &mut gate,
        FrameTimestamp(2_000_000),
        request(2_000_000, 1.0, -1.0)?,
    )?;
    assert_eq!(servo.snapshot().state, MotionState::Settled);
    Ok(())
}

#[test]
fn zero_settling_and_large_intervals_finish_at_the_destination()
-> Result<(), Box<dyn std::error::Error>> {
    let mut profile = PanTiltConfig::simulation_example();
    profile.settling_us = 0;
    let mut servo = VirtualPanTilt::new(profile, EvidenceScope::ReplayFixture)?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    servo.aim(&mut gate, FrameTimestamp(0), request(0, 0.0, 0.0)?)?;
    assert_eq!(servo.snapshot().state, MotionState::Settled);
    servo.aim(&mut gate, FrameTimestamp(0), request(0, 1.0, -1.0)?)?;
    clear(&mut gate, u64::MAX)?;
    servo.advance(&mut gate, FrameTimestamp(u64::MAX), FrameId(u64::MAX))?;
    assert_eq!(servo.snapshot().state, MotionState::Settled);
    assert_eq!(servo.snapshot().simulated.pan.degrees(), 45.0);
    assert_eq!(servo.snapshot().simulated.tilt.degrees(), -45.0);
    Ok(())
}
#[test]
fn every_denial_freezes_motion_cancels_destination_and_cannot_resume()
-> Result<(), Box<dyn std::error::Error>> {
    for verdict in [
        Verdict::Hazard(camera::Hazard::Human),
        Verdict::Hazard(camera::Hazard::Dog),
        Verdict::Hazard(camera::Hazard::Cat),
        Verdict::Unavailable,
        Verdict::Error,
        Verdict::Timeout,
        Verdict::Stale,
        Verdict::Invalid,
        Verdict::Uncertain,
        Verdict::Clear(Confidence::new(0.5)?),
    ] {
        let mut servo = VirtualPanTilt::new(
            PanTiltConfig::simulation_example(),
            EvidenceScope::ReplayFixture,
        )?;
        let mut gate = SafetyAuthority::new(&Config::default())?;
        clear(&mut gate, 0)?;
        servo.aim(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)?;
        clear(&mut gate, 100_000)?;
        servo.advance(&mut gate, FrameTimestamp(100_000), FrameId(100_000))?;
        let before = servo.snapshot().simulated;
        observe(&mut gate, 200_000, verdict);
        assert!(
            !servo
                .aim(
                    &mut gate,
                    FrameTimestamp(200_000),
                    request(200_000, -1.0, -1.0)?
                )?
                .issued
        );
        assert_eq!(servo.snapshot().state, MotionState::Cancelled);
        assert!(servo.snapshot().requested.is_none());
        assert!(servo.snapshot().requested_pulses.is_none());
        assert_eq!(servo.snapshot().simulated, before);
        clear(&mut gate, 300_000)?;
        servo.advance(&mut gate, FrameTimestamp(300_000), FrameId(300_000))?;
        assert_eq!(servo.snapshot().simulated, before);
        assert_eq!(servo.snapshot().state, MotionState::Cancelled);
    }
    Ok(())
}
#[test]
fn stale_wrong_scope_and_regressed_time_cancel_and_reset_restores_centre()
-> Result<(), Box<dyn std::error::Error>> {
    assert!(VirtualPanTilt::new(PanTiltConfig::simulation_example(), EvidenceScope::Live).is_err());
    let mut servo = VirtualPanTilt::new(
        PanTiltConfig::simulation_example(),
        EvidenceScope::ReplayFixture,
    )?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    servo.aim(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)?;
    // No fresh frame: do not integrate even one stale interval.
    servo.advance(&mut gate, FrameTimestamp(50_001), FrameId(0))?;
    assert_eq!(servo.snapshot().simulated.pan.degrees(), 0.0);
    assert_eq!(servo.snapshot().state, MotionState::Cancelled);
    clear(&mut gate, 100_000)?;
    gate.observe(Evidence {
        frame: FrameId(100_000),
        timestamp: FrameTimestamp(100_000),
        scope: EvidenceScope::Live,
        verdict: Verdict::Clear(Confidence::new(1.0)?),
    });
    assert!(
        !servo
            .aim(
                &mut gate,
                FrameTimestamp(100_000),
                request(100_000, 1.0, 1.0)?
            )?
            .issued
    );
    assert!(
        servo
            .advance(&mut gate, FrameTimestamp(1), FrameId(1))
            .is_err()
    );
    servo.reset()?;
    assert_eq!(servo.snapshot().state, MotionState::Idle);
    assert_eq!(servo.snapshot().simulated.pan.degrees(), 0.0);
    Ok(())
}
#[test]
fn retargeting_uses_current_position_and_stop_does_not_recentre()
-> Result<(), Box<dyn std::error::Error>> {
    let mut servo = VirtualPanTilt::new(
        PanTiltConfig::simulation_example(),
        EvidenceScope::ReplayFixture,
    )?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    servo.aim(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)?;
    clear(&mut gate, 100_000)?;
    servo.aim(
        &mut gate,
        FrameTimestamp(100_000),
        request(100_000, -1.0, -1.0)?,
    )?;
    assert_eq!(servo.snapshot().simulated.pan.degrees(), 9.0);
    clear(&mut gate, 150_000)?;
    servo.advance(&mut gate, FrameTimestamp(150_000), FrameId(150_000))?;
    assert_eq!(servo.snapshot().simulated.pan.degrees(), 4.5);
    servo.stop();
    clear(&mut gate, 1_000_000)?;
    servo.advance(&mut gate, FrameTimestamp(1_000_000), FrameId(1_000_000))?;
    assert_eq!(servo.snapshot().simulated.pan.degrees(), 4.5);
    Ok(())
}
