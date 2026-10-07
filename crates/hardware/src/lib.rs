#![forbid(unsafe_code)]
use aiming::{AimRecord, AimRequest, AimingDevice, AimingError};
use fly_core::FrameTimestamp;
use safety::SafetyAuthority;
/// No physical device is opened. Replace only after exact hardware and inhibition validation.
pub struct DisabledGalvo;
impl AimingDevice for DisabledGalvo {
    fn aim(
        &mut self,
        _authority: &mut SafetyAuthority,
        _now: FrameTimestamp,
        _request: AimRequest,
    ) -> Result<AimRecord, AimingError> {
        Err(AimingError::DeviceUnavailable(
            "DAC, analog interface and physical inhibition are not validated",
        ))
    }
    fn stop(&mut self) {}
}
