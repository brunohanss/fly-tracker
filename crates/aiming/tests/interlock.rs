use aiming::{AimRequest, AimingDevice, VirtualAimer};
use fly_core::{
    Confidence, FrameId, FrameTimestamp, NormalizedAim, PixelPosition, TargetId, config::Config,
};
use safety::{Evidence, EvidenceScope, SafetyAuthority, Verdict};
#[test]
fn revoked_permission_suppresses_command_and_overlay() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();
    let mut gate = SafetyAuthority::new(&config)?;
    let mut aimer = VirtualAimer::new(EvidenceScope::ReplayFixture);
    let request = AimRequest {
        frame: FrameId(1),
        target: TargetId(1),
        predicted_pixel: PixelPosition::new(10.0, 10.0)?,
        aim: NormalizedAim::new(0.0, 0.0)?,
    };
    gate.record_camera(request.frame, FrameTimestamp(0));
    gate.observe(Evidence {
        frame: request.frame,
        timestamp: FrameTimestamp(0),
        scope: EvidenceScope::ReplayFixture,
        verdict: Verdict::Clear(Confidence::new(1.0)?),
    });
    assert!(aimer.aim(&mut gate, FrameTimestamp(0), request)?.issued);
    gate.observe(Evidence {
        frame: request.frame,
        timestamp: FrameTimestamp(1),
        scope: EvidenceScope::ReplayFixture,
        verdict: Verdict::Error,
    });
    assert!(!aimer.aim(&mut gate, FrameTimestamp(1), request)?.issued);
    assert_eq!(aimer.issued_count(), 1);
    let mut pixels = vec![0; config.frame_size.pixels()];
    aimer.render_overlay(&mut pixels, config.frame_size)?;
    assert!(pixels.iter().all(|p| *p == 0));
    Ok(())
}
