//! Servo coordinates and an unmeasured, simulation-only assembly profile.
use crate::{DomainError, NormalizedAim, config::ConfigError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct ServoAngle(f64);
impl ServoAngle {
    pub fn new(degrees: f64) -> Result<Self, DomainError> {
        if degrees.is_finite() && (-360.0..=360.0).contains(&degrees) {
            Ok(Self(degrees))
        } else {
            Err(DomainError::Range("servo degrees [-360, 360]"))
        }
    }
    pub fn degrees(self) -> f64 {
        self.0
    }
}
impl TryFrom<f64> for ServoAngle {
    type Error = DomainError;
    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<ServoAngle> for f64 {
    fn from(value: ServoAngle) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServoPulse(u16);
impl ServoPulse {
    pub fn microseconds(self) -> u16 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanTiltPosition {
    pub pan: ServoAngle,
    pub tilt: ServoAngle,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanTiltPulses {
    pub pan: ServoPulse,
    pub tilt: ServoPulse,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServoConfig {
    pub channel: u8,
    pub reversed: bool,
    pub min_degrees: f64,
    pub centre_degrees: f64,
    pub max_degrees: f64,
    pub min_pulse_us: u16,
    pub centre_pulse_us: u16,
    pub max_pulse_us: u16,
    pub max_speed_degrees_per_s: f64,
}
impl ServoConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.channel >= 16
            || [self.min_degrees, self.centre_degrees, self.max_degrees]
                .iter()
                .any(|v| !v.is_finite() || !(-360.0..=360.0).contains(v))
            || self.min_degrees >= self.centre_degrees
            || self.centre_degrees >= self.max_degrees
            || self.min_pulse_us < 100
            || self.max_pulse_us > 3000
            || self.min_pulse_us >= self.centre_pulse_us
            || self.centre_pulse_us >= self.max_pulse_us
            || !self.max_speed_degrees_per_s.is_finite()
            || !(0.001..=1000.0).contains(&self.max_speed_degrees_per_s)
        {
            return Err(ConfigError::Invalid(
                "invalid servo channel, limits, centre, or speed",
            ));
        }
        Ok(())
    }
    /// Position is relative to the assembly; reversal changes only the pulse direction.
    pub fn position(&self, normalized: f64) -> Result<ServoAngle, ConfigError> {
        self.validate()?;
        if !normalized.is_finite() || !(-1.0..=1.0).contains(&normalized) {
            return Err(ConfigError::Invalid(
                "servo request outside normalized travel",
            ));
        }
        let span = if normalized < 0.0 {
            self.centre_degrees - self.min_degrees
        } else {
            self.max_degrees - self.centre_degrees
        };
        ServoAngle::new(self.centre_degrees + normalized * span)
            .map_err(|_| ConfigError::Invalid("invalid servo position"))
    }
    pub fn pulse(&self, position: ServoAngle) -> Result<ServoPulse, ConfigError> {
        self.validate()?;
        let angle = position.degrees();
        if !(self.min_degrees..=self.max_degrees).contains(&angle) {
            return Err(ConfigError::Invalid(
                "servo position outside configured travel",
            ));
        }
        let normalized = if angle < self.centre_degrees {
            (angle - self.centre_degrees) / (self.centre_degrees - self.min_degrees)
        } else {
            (angle - self.centre_degrees) / (self.max_degrees - self.centre_degrees)
        };
        let directed = if self.reversed {
            -normalized
        } else {
            normalized
        };
        let span = if directed < 0.0 {
            self.centre_pulse_us - self.min_pulse_us
        } else {
            self.max_pulse_us - self.centre_pulse_us
        };
        Ok(ServoPulse(
            (f64::from(self.centre_pulse_us) + directed * f64::from(span)).round() as u16,
        ))
    }
    pub fn normalized(&self, position: ServoAngle) -> Result<f64, ConfigError> {
        self.validate()?;
        let angle = position.degrees();
        if !(self.min_degrees..=self.max_degrees).contains(&angle) {
            return Err(ConfigError::Invalid(
                "servo position outside configured travel",
            ));
        }
        Ok((angle - self.centre_degrees)
            / if angle < self.centre_degrees {
                self.centre_degrees - self.min_degrees
            } else {
                self.max_degrees - self.centre_degrees
            })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanTiltConfig {
    pub version: u32,
    pub pan: ServoConfig,
    pub tilt: ServoConfig,
    pub settling_us: u64,
}
impl PanTiltConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.pan.validate()?;
        self.tilt.validate()?;
        if self.version != 1
            || self.pan.channel == self.tilt.channel
            || self.settling_us > 10_000_000
        {
            return Err(ConfigError::Invalid(
                "invalid pan/tilt version, duplicate channels, or settling time",
            ));
        }
        Ok(())
    }
    pub fn map(&self, aim: NormalizedAim) -> Result<PanTiltPosition, ConfigError> {
        self.validate()?;
        Ok(PanTiltPosition {
            pan: self.pan.position(aim.x())?,
            tilt: self.tilt.position(aim.y())?,
        })
    }
    pub fn pulses(&self, position: PanTiltPosition) -> Result<PanTiltPulses, ConfigError> {
        self.validate()?;
        Ok(PanTiltPulses {
            pan: self.pan.pulse(position.pan)?,
            tilt: self.tilt.pulse(position.tilt)?,
        })
    }
    pub fn normalized(&self, position: PanTiltPosition) -> Result<NormalizedAim, ConfigError> {
        self.validate()?;
        NormalizedAim::new(
            self.pan.normalized(position.pan)?,
            self.tilt.normalized(position.tilt)?,
        )
        .map_err(|_| ConfigError::Invalid("invalid normalized servo position"))
    }
    /// Illustrative values only. Never use this profile as physical calibration.
    pub fn simulation_example() -> Self {
        let pan = ServoConfig {
            channel: 0,
            reversed: false,
            min_degrees: -45.0,
            centre_degrees: 0.0,
            max_degrees: 45.0,
            min_pulse_us: 1000,
            centre_pulse_us: 1500,
            max_pulse_us: 2000,
            max_speed_degrees_per_s: 90.0,
        };
        Self {
            version: 1,
            pan,
            tilt: ServoConfig { channel: 1, ..pan },
            settling_us: 100_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asymmetric_centres_reversal_and_travel_boundaries() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut axis = PanTiltConfig::simulation_example().pan;
        axis.min_degrees = -20.0;
        axis.centre_degrees = 10.0;
        axis.max_degrees = 60.0;
        axis.centre_pulse_us = 1400;
        for (normalized, degrees, normal_pulse, reversed_pulse) in [
            (-1.0, -20.0, 1000, 2000),
            (-0.5, -5.0, 1200, 1700),
            (0.0, 10.0, 1400, 1400),
            (0.5, 35.0, 1700, 1200),
            (1.0, 60.0, 2000, 1000),
        ] {
            let position = axis.position(normalized)?;
            assert_eq!(position.degrees(), degrees);
            axis.reversed = false;
            assert_eq!(axis.pulse(position)?.microseconds(), normal_pulse);
            axis.reversed = true;
            assert_eq!(axis.pulse(position)?.microseconds(), reversed_pulse);
        }
        for invalid in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.00001,
            1.00001,
        ] {
            assert!(axis.position(invalid).is_err());
        }
        assert!(axis.pulse(ServoAngle::new(-20.001)?).is_err());
        assert!(axis.pulse(ServoAngle::new(60.001)?).is_err());
        assert!(ServoAngle::new(f64::NAN).is_err());
        assert!(serde_json::from_str::<ServoAngle>("361").is_err());
        Ok(())
    }
    #[test]
    fn profiles_reject_bad_channels_limits_speed_and_schema()
    -> Result<(), Box<dyn std::error::Error>> {
        let profile = PanTiltConfig::simulation_example();
        profile.validate()?;
        let mut cases = Vec::new();
        let mut p = profile;
        p.version = 2;
        cases.push(p);
        let mut p = profile;
        p.tilt.channel = 0;
        cases.push(p);
        let mut p = profile;
        p.pan.channel = 16;
        cases.push(p);
        let mut p = profile;
        p.settling_us = 10_000_001;
        cases.push(p);
        let mut p = profile;
        p.pan.min_pulse_us = 99;
        cases.push(p);
        let mut p = profile;
        p.pan.max_pulse_us = 3001;
        cases.push(p);
        let mut p = profile;
        p.pan.centre_pulse_us = 1000;
        cases.push(p);
        let mut p = profile;
        p.pan.centre_degrees = 45.0;
        cases.push(p);
        let mut p = profile;
        p.pan.min_degrees = f64::NAN;
        cases.push(p);
        let mut p = profile;
        p.pan.max_speed_degrees_per_s = f64::INFINITY;
        cases.push(p);
        let mut p = profile;
        p.pan.max_speed_degrees_per_s = 0.0;
        cases.push(p);
        for invalid in cases {
            assert!(invalid.validate().is_err(), "{invalid:?}");
        }
        let config = crate::config::Config {
            pan_tilt_simulation: Some(profile),
            ..Default::default()
        };
        assert_eq!(
            crate::config::Config::parse(&serde_json::to_string(&config)?)?.pan_tilt_simulation,
            Some(profile)
        );
        crate::config::Config::parse(include_str!("../../../config/pan-tilt-simulation.json"))?;
        let older = include_str!("../../../config/example.json");
        assert!(
            crate::config::Config::parse(older)?
                .pan_tilt_simulation
                .is_none()
        );
        let mut value = serde_json::to_value(profile)?;
        value["physical_output"] = serde_json::json!(true);
        assert!(serde_json::from_value::<PanTiltConfig>(value).is_err());
        Ok(())
    }
}
