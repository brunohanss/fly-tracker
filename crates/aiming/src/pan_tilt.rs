//! Deterministic simulation. This module opens no device and emits no PWM.
use crate::{AimRecord, AimRequest, AimingDevice, AimingError};
use fly_core::{
    FrameId, FrameTimestamp,
    servo::{PanTiltConfig, PanTiltPosition, PanTiltPulses, ServoAngle},
};
use safety::{EvidenceScope, SafetyAuthority};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionState {
    Idle,
    Moving,
    Settling,
    Settled,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanTiltSnapshot {
    pub requested_target: Option<fly_core::TargetId>,
    pub simulated: PanTiltPosition,
    pub requested: Option<PanTiltPosition>,
    /// Hypothetical destination pulses. These are never sent to hardware.
    pub requested_pulses: Option<PanTiltPulses>,
    pub state: MotionState,
    pub settling_remaining_us: u64,
    pub pan_channel: u8,
    pub tilt_channel: u8,
}
pub struct VirtualPanTilt {
    config: PanTiltConfig,
    scope: EvidenceScope,
    snapshot: PanTiltSnapshot,
    last_time: Option<FrameTimestamp>,
    settling_remaining_us: f64,
}
impl VirtualPanTilt {
    pub fn new(config: PanTiltConfig, scope: EvidenceScope) -> Result<Self, AimingError> {
        config.validate()?;
        if scope == EvidenceScope::Live {
            return Err(AimingError::DeviceUnavailable(
                "pan/tilt simulation requires replay evidence",
            ));
        }
        let centre = config.map(fly_core::NormalizedAim::new(0.0, 0.0)?)?;
        Ok(Self {
            config,
            scope,
            snapshot: PanTiltSnapshot {
                requested_target: None,
                simulated: centre,
                requested: None,
                requested_pulses: None,
                state: MotionState::Idle,
                settling_remaining_us: 0,
                pan_channel: config.pan.channel,
                tilt_channel: config.tilt.channel,
            },
            last_time: None,
            settling_remaining_us: 0.0,
        })
    }
    pub fn snapshot(&self) -> PanTiltSnapshot {
        self.snapshot
    }
    /// Replay reset restores the assumed centre. A stop never returns to centre.
    pub fn reset(&mut self) -> Result<(), AimingError> {
        *self = Self::new(self.config, self.scope)?;
        Ok(())
    }
    /// Check current-frame authority before each simulation update. A denied interval
    /// freezes position and removes its destination; recovery cannot resume old work.
    pub fn advance(
        &mut self,
        authority: &mut SafetyAuthority,
        now: FrameTimestamp,
        frame: FrameId,
    ) -> Result<(), AimingError> {
        let elapsed = self
            .last_time
            .map_or(Ok(std::time::Duration::ZERO), |last| {
                now.elapsed_since(last)
            });
        let elapsed = match elapsed {
            Ok(value) => value,
            Err(error) => {
                self.stop();
                return Err(error.into());
            }
        };
        self.last_time = Some(now);
        match authority.execute(now, frame, self.scope, || {
            self.integrate(elapsed.as_secs_f64())
        }) {
            Ok(result) => result,
            Err(_) => {
                self.stop();
                Ok(())
            }
        }
    }
    fn integrate(&mut self, seconds: f64) -> Result<(), AimingError> {
        let Some(destination) = self.snapshot.requested else {
            return Ok(());
        };
        if self.snapshot.state == MotionState::Settled {
            return Ok(());
        }
        let from = self.snapshot.simulated;
        let pan_time = (destination.pan.degrees() - from.pan.degrees()).abs()
            / self.config.pan.max_speed_degrees_per_s;
        let tilt_time = (destination.tilt.degrees() - from.tilt.degrees()).abs()
            / self.config.tilt.max_speed_degrees_per_s;
        let travel_time = pan_time.max(tilt_time);
        let move_axis = |from: ServoAngle, to: ServoAngle, speed: f64| {
            let delta = to.degrees() - from.degrees();
            if delta.abs() <= speed * seconds {
                Ok(to)
            } else {
                ServoAngle::new(from.degrees() + delta.signum() * speed * seconds)
            }
        };
        self.snapshot.simulated = PanTiltPosition {
            pan: move_axis(
                from.pan,
                destination.pan,
                self.config.pan.max_speed_degrees_per_s,
            )?,
            tilt: move_axis(
                from.tilt,
                destination.tilt,
                self.config.tilt.max_speed_degrees_per_s,
            )?,
        };
        if seconds >= travel_time {
            self.settling_remaining_us =
                (self.settling_remaining_us - (seconds - travel_time) * 1_000_000.0).max(0.0);
            // Round-off below one nanosecond must not add a replay frame of settling.
            if self.settling_remaining_us < 0.001 {
                self.settling_remaining_us = 0.0;
            }
            self.snapshot.state = if self.settling_remaining_us == 0.0 {
                MotionState::Settled
            } else {
                MotionState::Settling
            };
        } else {
            self.snapshot.state = MotionState::Moving;
        }
        self.snapshot.settling_remaining_us = self.settling_remaining_us.ceil() as u64;
        Ok(())
    }
}
impl AimingDevice for VirtualPanTilt {
    fn aim(
        &mut self,
        authority: &mut SafetyAuthority,
        now: FrameTimestamp,
        request: AimRequest,
    ) -> Result<AimRecord, AimingError> {
        // A rejected request must cancel an existing destination as well.
        let destination = match self.config.map(request.aim) {
            Ok(value) => value,
            Err(error) => {
                self.stop();
                return Err(error.into());
            }
        };
        let pulses = self.config.pulses(destination)?;
        self.advance(authority, now, request.frame)?;
        let denied = authority
            .execute(now, request.frame, self.scope, || {
                self.snapshot.requested_target = Some(request.target);
                if self.snapshot.requested != Some(destination) {
                    self.snapshot.requested = Some(destination);
                    self.snapshot.requested_pulses = Some(pulses);
                    self.settling_remaining_us = self.config.settling_us as f64;
                    self.snapshot.settling_remaining_us = self.config.settling_us;
                    self.snapshot.state = if self.snapshot.simulated != destination {
                        MotionState::Moving
                    } else if self.config.settling_us > 0 {
                        MotionState::Settling
                    } else {
                        MotionState::Settled
                    };
                }
            })
            .err()
            .map(|denied| denied.0);
        if denied.is_some() {
            self.stop();
        }
        Ok(AimRecord {
            request,
            issued: denied.is_none(),
            suppressed_by: denied,
        })
    }
    fn stop(&mut self) {
        self.snapshot.requested_target = None;
        self.snapshot.requested = None;
        self.snapshot.requested_pulses = None;
        self.snapshot.state = MotionState::Cancelled;
        self.snapshot.settling_remaining_us = 0;
        self.settling_remaining_us = 0.0;
    }
}
