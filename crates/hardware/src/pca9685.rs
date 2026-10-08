//! PCA9685 register adapter. Only an in-memory transport is provided.
//! No Linux I2C, GPIO, physical enable flag, or emitter interface exists here.
pub mod mock;
use aiming::{AimRecord, AimRequest, AimingDevice, AimingError};
use fly_core::{
    FrameId, FrameTimestamp,
    servo::{PanTiltConfig, PanTiltPulses},
};
use mock::{MockTransport, Transport};
use safety::{EvidenceScope, SafetyAuthority};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MODE1: u8 = 0x00;
const MODE2: u8 = 0x01;
const LED0: u8 = 0x06;
const ALL_OFF_H: u8 = 0xfd;
const PRE_SCALE: u8 = 0xfe;
const AI: u8 = 0x20;
const SLEEP: u8 = 0x10;
const FULL_OFF: u8 = 0x10;
const OUTDRV: u8 = 0x04;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pca9685Config {
    pub version: u32,
    pub address: u8,
    /// Configured internal oscillator estimate. It must be measured before live use.
    pub oscillator_hz: f64,
    pub frequency_hz: f64,
}
impl Pca9685Config {
    pub fn mock_example() -> Self {
        Self {
            version: 1,
            address: 0x40,
            oscillator_hz: 25_000_000.0,
            frequency_hz: 50.0,
        }
    }
    pub fn validate(self) -> Result<PwmTiming, PcaError> {
        // 0x70 is the reset ALLCALL address; 0x78..0x7f are reserved I2C addresses.
        if self.version != 1
            || !(0x40..=0x77).contains(&self.address)
            || self.address == 0x70
            || !(40.0..=60.0).contains(&self.frequency_hz)
            || !self.oscillator_hz.is_finite()
            || !(1_000_000.0..=50_000_000.0).contains(&self.oscillator_hz)
        {
            return Err(PcaError::Invalid(
                "invalid PCA9685 version, address, clock, or servo frequency",
            ));
        }
        PwmTiming::new(self.oscillator_hz, self.frequency_hz)
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PwmTiming {
    prescale: u8,
    oscillator_hz: f64,
}
impl PwmTiming {
    pub fn new(oscillator_hz: f64, frequency_hz: f64) -> Result<Self, PcaError> {
        if !oscillator_hz.is_finite()
            || !frequency_hz.is_finite()
            || !(1_000_000.0..=50_000_000.0).contains(&oscillator_hz)
            || frequency_hz <= 0.0
        {
            return Err(PcaError::Invalid(
                "clock and PWM frequency must be finite and positive",
            ));
        }
        let prescale = (oscillator_hz / (4096.0 * frequency_hz)).round() - 1.0;
        if !(3.0..=255.0).contains(&prescale) {
            return Err(PcaError::Invalid("PWM frequency outside prescaler range"));
        }
        Ok(Self {
            prescale: prescale as u8,
            oscillator_hz,
        })
    }
    pub fn prescale(self) -> u8 {
        self.prescale
    }
    pub fn actual_frequency_hz(self) -> f64 {
        self.oscillator_hz / (4096.0 * (f64::from(self.prescale) + 1.0))
    }
    pub fn tick_us(self) -> f64 {
        (f64::from(self.prescale) + 1.0) * 1_000_000.0 / self.oscillator_hz
    }
    pub fn pulse_ticks(self, pulse_us: f64) -> Result<PwmTicks, PcaError> {
        let count = (pulse_us / self.tick_us()).round();
        if !pulse_us.is_finite() || pulse_us <= 0.0 || !(1.0..=4095.0).contains(&count) {
            return Err(PcaError::Invalid(
                "pulse must fit a nonzero 12-bit PWM count",
            ));
        }
        Ok(PwmTicks(count as u16))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PwmTicks(u16);
impl PwmTicks {
    pub fn count(self) -> u16 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverState {
    Uninitialized,
    ReadyInhibited,
    ActiveMock,
    Faulted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Inhibit,
    Write,
    Read,
    OscillatorWait,
    Readback,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransportError {
    #[error("mock transport disconnected")]
    Disconnected,
    #[error("injected mock transport failure")]
    Injected,
    #[error("partial mock write accepted {0} data bytes")]
    PartialWrite(usize),
    #[error("invalid mock transaction")]
    InvalidTransaction,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverFault {
    pub operation: Operation,
    pub cause: TransportError,
}
#[derive(Debug, Error)]
pub enum PcaError {
    #[error("Invalid command timestamp: {0}")]
    Time(#[from] fly_core::DomainError),
    #[error("Invalid PCA9685 configuration: {0}")]
    Invalid(&'static str),
    #[error("Invalid servo profile: {0}")]
    Servo(#[from] fly_core::config::ConfigError),
    #[error("PCA9685 transport failed: {0:?}")]
    Transport(DriverFault),
    #[error("PCA9685 is not initialized or requires explicit fault recovery")]
    NotReady,
}
impl PcaError {
    fn summary(&self) -> &'static str {
        match self {
            Self::Invalid(_) | Self::Servo(_) => "invalid PCA9685 configuration or servo request",
            Self::Transport(_) => {
                "PCA9685 mock transport or readback failed; output inhibited or unconfirmed"
            }
            Self::NotReady => "PCA9685 mock adapter not ready; initialize explicitly",
            Self::Time(_) => "PCA9685 mock command timestamp overflow or regression",
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct DriverSnapshot {
    pub state: DriverState,
    pub fault: Option<DriverFault>,
    /// None means that asserting inhibition failed. Never infer safe output from it.
    pub inhibit_confirmed: Option<bool>,
    pub last_pulses: Option<PanTiltPulses>,
    pub mock_commands: u64,
    pub timing: PwmTiming,
}

/// Owns a dedicated mock board. All unused channels are kept full-off.
/// Live scope is rejected. No caller can substitute a physical transport.
pub struct MockPca9685Aimer {
    config: Pca9685Config,
    profile: PanTiltConfig,
    scope: EvidenceScope,
    transport: MockTransport,
    snapshot: DriverSnapshot,
}
impl MockPca9685Aimer {
    pub fn new(
        config: Pca9685Config,
        profile: PanTiltConfig,
        scope: EvidenceScope,
    ) -> Result<Self, PcaError> {
        let timing = config.validate()?;
        profile.validate()?;
        if scope == EvidenceScope::Live {
            return Err(PcaError::Invalid("mock adapter requires replay evidence"));
        }
        // Check all profile pulse limits against the selected PWM period before writes.
        for axis in [profile.pan, profile.tilt] {
            for pulse in [axis.min_pulse_us, axis.centre_pulse_us, axis.max_pulse_us] {
                timing.pulse_ticks(f64::from(pulse))?;
            }
        }
        Ok(Self {
            config,
            profile,
            scope,
            transport: MockTransport::new(config.address),
            snapshot: DriverSnapshot {
                state: DriverState::Uninitialized,
                fault: None,
                inhibit_confirmed: Some(true),
                last_pulses: None,
                mock_commands: 0,
                timing,
            },
        })
    }
    pub fn snapshot(&self) -> DriverSnapshot {
        self.snapshot
    }
    pub fn mock(&self) -> &MockTransport {
        &self.transport
    }
    pub fn mock_mut(&mut self) -> &mut MockTransport {
        &mut self.transport
    }
    fn io<T>(
        &mut self,
        operation: Operation,
        action: impl FnOnce(&mut MockTransport) -> Result<T, TransportError>,
    ) -> Result<T, PcaError> {
        action(&mut self.transport)
            .map_err(|cause| PcaError::Transport(DriverFault { operation, cause }))
    }
    fn inhibit(&mut self, value: bool) -> Result<(), PcaError> {
        self.snapshot.inhibit_confirmed = None;
        self.io(Operation::Inhibit, |bus| bus.set_inhibited(value))?;
        self.snapshot.inhibit_confirmed = Some(value);
        Ok(())
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), PcaError> {
        let address = self.config.address;
        self.io(Operation::Write, |bus| bus.write(address, bytes))
    }
    fn verify(&mut self, register: u8, expected: &[u8]) -> Result<(), PcaError> {
        let mut values = [0u8; 64];
        let address = self.config.address;
        self.io(Operation::Read, |bus| {
            bus.read(address, register, &mut values[..expected.len()])
        })?;
        if &values[..expected.len()] != expected {
            return Err(PcaError::Transport(DriverFault {
                operation: Operation::Readback,
                cause: TransportError::InvalidTransaction,
            }));
        }
        Ok(())
    }
    fn fault(&mut self, error: &PcaError) {
        self.snapshot.state = DriverState::Faulted;
        self.snapshot.last_pulses = None;
        if let PcaError::Transport(fault) = error {
            self.snapshot.fault = Some(*fault);
        }
        // Inhibition is independent of I2C. A failed write may already have changed registers.
        let _ = self.inhibit(true);
        let _ = self.write(&[ALL_OFF_H, FULL_OFF]);
    }
    pub fn initialize(&mut self) -> Result<(), PcaError> {
        self.snapshot.last_pulses = None;
        let result = self.initialize_inner();
        if let Err(error) = &result {
            self.fault(error);
        }
        result
    }
    fn initialize_inner(&mut self) -> Result<(), PcaError> {
        self.inhibit(true)?;
        let mut mode = [0u8];
        let address = self.config.address;
        self.io(Operation::Read, |bus| bus.read(address, MODE1, &mut mode))?;
        // EXTCLK is sticky. Do not issue a general-call reset to other bus devices.
        if mode[0] & 0x40 != 0 {
            return Err(PcaError::Invalid(
                "external clock configured; power-cycle dedicated board",
            ));
        }
        // Recovery can start with an oscillator whose startup wait was interrupted.
        if mode[0] & SLEEP == 0 {
            self.io(Operation::OscillatorWait, |bus| bus.wait_us(500))?;
        }
        self.write(&[ALL_OFF_H, FULL_OFF])?;
        self.write(&[MODE1, AI | SLEEP])?;
        self.write(&[MODE2, OUTDRV])?;
        self.write(&[PRE_SCALE, self.snapshot.timing.prescale()])?;
        self.write(&[MODE1, AI])?;
        self.io(Operation::OscillatorWait, |bus| bus.wait_us(500))?;
        self.write(&off_image())?;
        self.verify(MODE1, &[AI, OUTDRV])?;
        self.verify(PRE_SCALE, &[self.snapshot.timing.prescale()])?;
        self.verify(LED0, &off_image()[1..])?;
        self.snapshot.state = DriverState::ReadyInhibited;
        self.snapshot.fault = None;
        Ok(())
    }
    /// Independent watchdog entry point. Call even if no new command is requested.
    pub fn watchdog(
        &mut self,
        authority: &mut SafetyAuthority,
        now: FrameTimestamp,
        frame: FrameId,
    ) -> Result<(), PcaError> {
        if authority.execute(now, frame, self.scope, || ()).is_err() {
            self.try_stop()?;
        }
        Ok(())
    }
    pub fn command(
        &mut self,
        authority: &mut SafetyAuthority,
        now: FrameTimestamp,
        request: AimRequest,
    ) -> Result<AimRecord, PcaError> {
        if !matches!(
            self.snapshot.state,
            DriverState::ReadyInhibited | DriverState::ActiveMock
        ) {
            return Err(PcaError::NotReady);
        }
        let prepared = self
            .profile
            .map(request.aim)
            .and_then(|position| self.profile.pulses(position));
        let pulses = match prepared {
            Ok(pulses) => pulses,
            Err(error) => {
                self.try_stop()?;
                return Err(error.into());
            }
        };
        let result = self.command_inner(authority, now, request, pulses);
        if let Err(error) = &result {
            self.fault(error);
        }
        result
    }
    fn command_inner(
        &mut self,
        authority: &mut SafetyAuthority,
        now: FrameTimestamp,
        request: AimRequest,
        pulses: PanTiltPulses,
    ) -> Result<AimRecord, PcaError> {
        let started_us = self.transport.elapsed_us();
        if let Err(denied) = authority.execute(now, request.frame, self.scope, || ()) {
            self.try_stop()?;
            return Ok(AimRecord {
                request,
                issued: false,
                suppressed_by: Some(denied.0),
            });
        }
        self.inhibit(true)?;
        self.snapshot.last_pulses = None;
        self.verify(MODE1, &[AI, OUTDRV])?;
        self.verify(PRE_SCALE, &[self.snapshot.timing.prescale()])?;
        let mut image = off_image();
        for (channel, pulse) in [
            (self.profile.pan.channel, pulses.pan),
            (self.profile.tilt.channel, pulses.tilt),
        ] {
            let ticks = self
                .snapshot
                .timing
                .pulse_ticks(f64::from(pulse.microseconds()))?
                .count();
            let offset = 1 + usize::from(channel) * 4;
            image[offset..offset + 4].copy_from_slice(&[0, 0, ticks as u8, (ticks >> 8) as u8]);
        }
        // One transaction and STOP for the complete image, including nonadjacent axes.
        self.write(&image)?;
        self.verify(LED0, &image[1..])?;
        let release_at = now.advance(std::time::Duration::from_micros(
            self.transport.elapsed_us().saturating_sub(started_us),
        ))?;
        let release = authority.execute(release_at, request.frame, self.scope, || {
            self.inhibit(false)
        });
        match release {
            Ok(result) => result?,
            Err(denied) => {
                self.try_stop()?;
                return Ok(AimRecord {
                    request,
                    issued: false,
                    suppressed_by: Some(denied.0),
                });
            }
        }
        self.snapshot.state = DriverState::ActiveMock;
        self.snapshot.last_pulses = Some(pulses);
        self.snapshot.mock_commands = self.snapshot.mock_commands.saturating_add(1);
        Ok(AimRecord {
            request,
            issued: true,
            suppressed_by: None,
        })
    }
    pub fn try_stop(&mut self) -> Result<(), PcaError> {
        let was_faulted = self.snapshot.state == DriverState::Faulted;
        let was_uninitialized = self.snapshot.state == DriverState::Uninitialized;
        self.snapshot.last_pulses = None;
        // Attempt both paths even when inhibition fails.
        let inhibited = self.inhibit(true);
        let off = self.write(&[ALL_OFF_H, FULL_OFF]);
        let result = inhibited.and(off);
        if let Err(error) = &result {
            self.fault(error);
        } else if !was_faulted && !was_uninitialized {
            self.snapshot.state = DriverState::ReadyInhibited;
        }
        result
    }
}
impl AimingDevice for MockPca9685Aimer {
    fn aim(
        &mut self,
        authority: &mut SafetyAuthority,
        now: FrameTimestamp,
        request: AimRequest,
    ) -> Result<AimRecord, AimingError> {
        self.command(authority, now, request)
            .map_err(|error| AimingError::DeviceFault(error.summary()))
    }
    fn stop(&mut self) {
        let _ = self.try_stop();
    }
}
impl Drop for MockPca9685Aimer {
    fn drop(&mut self) {
        let _ = self.try_stop();
    }
}
fn off_image() -> [u8; 65] {
    let mut image = [0u8; 65];
    image[0] = LED0;
    for channel in 0..16 {
        image[4 + channel * 4] = FULL_OFF;
    }
    image
}
