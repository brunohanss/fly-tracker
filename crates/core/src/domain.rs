use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("Value must be finite and within {0}")]
    Range(&'static str),
    #[error("Timestamp is invalid or out of order")]
    Timestamp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FrameId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TargetId(pub u64);
/// Microseconds since the source epoch. Never compare different epochs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FrameTimestamp(pub u64);
impl FrameTimestamp {
    pub fn elapsed_since(self, earlier: Self) -> Result<Duration, DomainError> {
        self.0
            .checked_sub(earlier.0)
            .map(Duration::from_micros)
            .ok_or(DomainError::Timestamp)
    }
    pub fn advance(self, duration: Duration) -> Result<Self, DomainError> {
        let micros = u64::try_from(duration.as_micros()).map_err(|_| DomainError::Timestamp)?;
        self.0
            .checked_add(micros)
            .map(Self)
            .ok_or(DomainError::Timestamp)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Confidence(f64);
impl Confidence {
    pub fn new(value: f64) -> Result<Self, DomainError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(DomainError::Range("[0, 1]"))
        }
    }
    pub fn get(self) -> f64 {
        self.0
    }
}
impl TryFrom<f64> for Confidence {
    type Error = DomainError;
    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<Confidence> for f64 {
    fn from(value: Confidence) -> Self {
        value.0
    }
}
macro_rules! coordinate {
    ($name:ident, $range:expr, $valid:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
        #[serde(try_from = "[f64; 2]", into = "[f64; 2]")]
        pub struct $name {
            x: f64,
            y: f64,
        }
        impl $name {
            pub fn new(x: f64, y: f64) -> Result<Self, DomainError> {
                if x.is_finite() && y.is_finite() && ($valid)(x) && ($valid)(y) {
                    Ok(Self { x, y })
                } else {
                    Err(DomainError::Range($range))
                }
            }
            pub fn x(self) -> f64 {
                self.x
            }
            pub fn y(self) -> f64 {
                self.y
            }
        }
        impl TryFrom<[f64; 2]> for $name {
            type Error = DomainError;
            fn try_from(value: [f64; 2]) -> Result<Self, Self::Error> {
                Self::new(value[0], value[1])
            }
        }
        impl From<$name> for [f64; 2] {
            fn from(value: $name) -> Self {
                [value.x, value.y]
            }
        }
    };
}
coordinate!(PixelPosition, "nonnegative pixels", |v: f64| v >= 0.0);
coordinate!(PixelVelocity, "finite pixels/second", |_v: f64| true);
coordinate!(
    ScenePosition,
    "finite source-plane pixels, including outside the image",
    |_v: f64| true
);
impl ScenePosition {
    pub fn pixel_in(self, size: FrameSize) -> Option<PixelPosition> {
        PixelPosition::new(self.x(), self.y())
            .ok()
            .filter(|point| size.contains(*point))
    }
}
coordinate!(NormalizedAim, "[-1, 1]", |v: f64| (-1.0..=1.0).contains(&v));
coordinate!(
    GalvoPosition,
    "[-1, 1] normalized analog range",
    |v: f64| (-1.0..=1.0).contains(&v)
);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "[u32; 2]", into = "[u32; 2]")]
pub struct FrameSize {
    width: u32,
    height: u32,
}
impl FrameSize {
    pub fn new(width: u32, height: u32) -> Result<Self, DomainError> {
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 16_777_216 {
            return Err(DomainError::Range("1..=16777216 pixels"));
        }
        Ok(Self { width, height })
    }
    pub fn width(self) -> u32 {
        self.width
    }
    pub fn height(self) -> u32 {
        self.height
    }
    pub fn pixels(self) -> usize {
        self.width as usize * self.height as usize
    }
    pub fn contains(self, point: PixelPosition) -> bool {
        point.x() < f64::from(self.width) && point.y() < f64::from(self.height)
    }
}
impl TryFrom<[u32; 2]> for FrameSize {
    type Error = DomainError;
    fn try_from(value: [u32; 2]) -> Result<Self, Self::Error> {
        Self::new(value[0], value[1])
    }
}
impl From<FrameSize> for [u32; 2] {
    fn from(value: FrameSize) -> Self {
        [value.width, value.height]
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub position: PixelPosition,
    pub area: u32,
    pub confidence: Confidence,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackLifecycle {
    Candidate,
    Confirmed,
    TemporarilyLost,
    Removed,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TargetId,
    pub position: PixelPosition,
    pub velocity: PixelVelocity,
    pub confidence: Confidence,
    pub lifecycle: TrackLifecycle,
    pub first_seen: FrameTimestamp,
    pub updated_at: FrameTimestamp,
    pub hits: u32,
    pub lost_count: u32,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numerical_boundaries_and_deserialization() -> Result<(), Box<dyn std::error::Error>> {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(PixelPosition::new(value, 0.0).is_err());
            assert!(PixelVelocity::new(0.0, value).is_err());
            assert!(Confidence::new(value).is_err());
        }
        for value in [-1.0, 0.0, 1.0] {
            assert!(NormalizedAim::new(value, value).is_ok());
        }
        assert!(NormalizedAim::new(1.01, 0.0).is_err());
        assert!(serde_json::from_str::<NormalizedAim>("[2,0]").is_err());
        assert!(serde_json::from_str::<Confidence>("-0.1").is_err());
        assert!(FrameSize::new(0, 2).is_err());
        assert!(FrameSize::new(u32::MAX, u32::MAX).is_err());
        let size = FrameSize::new(10, 20)?;
        assert!(size.contains(PixelPosition::new(9.99, 19.99)?));
        assert!(!size.contains(PixelPosition::new(10.0, 0.0)?));
        Ok(())
    }
    #[test]
    fn timestamps_do_not_wrap() {
        assert!(FrameTimestamp(1).elapsed_since(FrameTimestamp(2)).is_err());
        assert!(
            FrameTimestamp(u64::MAX)
                .advance(Duration::from_micros(1))
                .is_err()
        );
        assert_eq!(
            FrameTimestamp(10).elapsed_since(FrameTimestamp(5)).ok(),
            Some(Duration::from_micros(5))
        );
    }
}
