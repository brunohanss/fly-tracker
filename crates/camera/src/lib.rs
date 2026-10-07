#![forbid(unsafe_code)]
pub mod export;
pub mod replay;
pub mod synthetic;
use fly_core::{DomainError, FrameId, FrameSize, FrameTimestamp, ScenePosition, TargetId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CameraError {
    #[error("Input I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Input metadata failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid frame: {0}")]
    Domain(#[from] DomainError),
    #[error("Invalid source: {0}")]
    Format(&'static str),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hazard {
    Human,
    Dog,
    Cat,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TruthTarget {
    pub id: TargetId,
    pub position: ScenePosition,
    #[serde(default = "default_visibility")]
    pub visible: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundTruth {
    pub targets: Vec<TruthTarget>,
    pub hazard: Option<Hazard>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub id: FrameId,
    pub timestamp: FrameTimestamp,
    pub size: FrameSize,
    pub pixels: Vec<u8>,
    pub truth: Option<GroundTruth>,
}
impl Frame {
    pub fn new(size: FrameSize) -> Self {
        Self {
            id: FrameId(0),
            timestamp: FrameTimestamp(0),
            size,
            pixels: vec![0; size.pixels()],
            truth: None,
        }
    }
    pub fn validate(&self) -> Result<(), CameraError> {
        if self.pixels.len() != self.size.pixels() {
            return Err(CameraError::Format("pixel count does not match dimensions"));
        }
        if let Some(truth) = &self.truth
            && (truth.targets.len() > 65536
                || truth
                    .targets
                    .iter()
                    .any(|target| target.visible && target.position.pixel_in(self.size).is_none()))
        {
            return Err(CameraError::Format("invalid ground truth"));
        }
        Ok(())
    }
}
pub trait FrameSource: Send {
    fn next_timestamp(&self) -> Option<FrameTimestamp> {
        None
    }
    fn total_frames(&self) -> Option<u64> {
        None
    }
    fn size(&self) -> FrameSize;
    /// Reuse the supplied frame. False means clean EOF. Errors never mean EOF.
    fn next_into(&mut self, frame: &mut Frame) -> Result<bool, CameraError>;
    fn reset(&mut self) -> Result<(), CameraError>;
}
fn default_visibility() -> bool {
    true
}
