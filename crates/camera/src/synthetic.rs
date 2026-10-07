use crate::{CameraError, Frame, FrameSource, GroundTruth, Hazard, TruthTarget};
use fly_core::{FrameId, FrameSize, FrameTimestamp, ScenePosition, TargetId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    EmptyWall,
    SlowFly,
    FastFly,
    AbruptAcceleration,
    Takeoff,
    Landing,
    LeavingFrame,
    Reentry,
    MultipleFlies,
    Occlusion,
    DifficultBackground,
    DustDebris,
    MovingObject,
    FullHuman,
    PartialHuman,
    EdgeHand,
    EdgeArm,
    FullDog,
    PartialDog,
    FullCat,
    PartialCat,
    FlyHuman,
    FlyDog,
    FlyCat,
}
impl Scenario {
    pub const ALL: [Self; 24] = [
        Self::EmptyWall,
        Self::SlowFly,
        Self::FastFly,
        Self::AbruptAcceleration,
        Self::Takeoff,
        Self::Landing,
        Self::LeavingFrame,
        Self::Reentry,
        Self::MultipleFlies,
        Self::Occlusion,
        Self::DifficultBackground,
        Self::DustDebris,
        Self::MovingObject,
        Self::FullHuman,
        Self::PartialHuman,
        Self::EdgeHand,
        Self::EdgeArm,
        Self::FullDog,
        Self::PartialDog,
        Self::FullCat,
        Self::PartialCat,
        Self::FlyHuman,
        Self::FlyDog,
        Self::FlyCat,
    ];
    pub fn hazard(self) -> Option<Hazard> {
        match self {
            Self::FullHuman
            | Self::PartialHuman
            | Self::EdgeHand
            | Self::EdgeArm
            | Self::FlyHuman => Some(Hazard::Human),
            Self::FullDog | Self::PartialDog | Self::FlyDog => Some(Hazard::Dog),
            Self::FullCat | Self::PartialCat | Self::FlyCat => Some(Hazard::Cat),
            _ => None,
        }
    }
}
pub struct SyntheticSource {
    size: FrameSize,
    scenario: Scenario,
    seed: u64,
    count: u64,
    index: u64,
    period_us: u64,
}
impl SyntheticSource {
    pub fn new(
        size: FrameSize,
        scenario: Scenario,
        seed: u64,
        count: u64,
        period_us: u64,
    ) -> Result<Self, CameraError> {
        if size.width() < 32
            || size.height() < 32
            || count == 0
            || period_us == 0
            || count.checked_mul(period_us).is_none()
        {
            return Err(CameraError::Format(
                "synthetic dimensions >=32, positive count/period without overflow required",
            ));
        }
        Ok(Self {
            size,
            scenario,
            seed,
            count,
            index: 0,
            period_us,
        })
    }
    fn rectangle(&self, frame: &mut Frame, x: i32, y: i32, width: i32, height: i32, value: u8) {
        for row in y.max(0)..(y + height).min(self.size.height() as i32) {
            for column in x.max(0)..(x + width).min(self.size.width() as i32) {
                frame.pixels[row as usize * self.size.width() as usize + column as usize] = value;
            }
        }
    }
}
impl FrameSource for SyntheticSource {
    fn next_timestamp(&self) -> Option<FrameTimestamp> {
        if self.index < self.count {
            self.index.checked_mul(self.period_us).map(FrameTimestamp)
        } else {
            None
        }
    }
    fn total_frames(&self) -> Option<u64> {
        Some(self.count)
    }
    fn size(&self) -> FrameSize {
        self.size
    }
    fn reset(&mut self) -> Result<(), CameraError> {
        self.index = 0;
        Ok(())
    }
    fn next_into(&mut self, frame: &mut Frame) -> Result<bool, CameraError> {
        if self.index == self.count {
            return Ok(false);
        }
        if frame.size != self.size {
            return Err(CameraError::Format("frame size mismatch"));
        }
        frame.pixels.resize(self.size.pixels(), 0);
        // Fixed, seed-dependent wall texture. No platform random generator.
        for (i, value) in frame.pixels.iter_mut().enumerate() {
            let hash = (i as u64)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(self.seed);
            *value = if self.scenario == Scenario::DifficultBackground {
                140 + ((hash >> 32) % 70) as u8
            } else {
                190 + ((hash >> 32) % 8) as u8
            };
        }
        frame.id = FrameId(self.index);
        frame.timestamp = FrameTimestamp(self.index * self.period_us);
        let truth = frame.truth.get_or_insert_with(|| GroundTruth {
            targets: Vec::with_capacity(2),
            hazard: None,
        });
        truth.targets.clear();
        truth.hazard = self.scenario.hazard();
        if self.index > 0 {
            let i = self.index as f64;
            let width = f64::from(self.size.width());
            let mut x = 8.0
                + (self.seed % 7) as f64
                + i * match self.scenario {
                    Scenario::FastFly => 3.0,
                    Scenario::AbruptAcceleration if self.index > self.count / 2 => 4.0,
                    _ => 0.7,
                };
            let y = f64::from(self.size.height()) * 0.35;
            if self.scenario == Scenario::AbruptAcceleration && self.index > self.count / 2 {
                x = 8.0
                    + (self.seed % 7) as f64
                    + (self.count / 2) as f64 * 0.7
                    + (self.index - self.count / 2) as f64 * 4.0;
            }
            let has_fly = !matches!(
                self.scenario,
                Scenario::EmptyWall | Scenario::DustDebris | Scenario::MovingObject
            ) && (self.scenario.hazard().is_none()
                || matches!(
                    self.scenario,
                    Scenario::FlyHuman | Scenario::FlyDog | Scenario::FlyCat
                ));
            let mut visible = has_fly;
            if self.scenario == Scenario::Takeoff {
                x = 12.0 + self.index.saturating_sub(self.count / 3) as f64 * 0.7;
            }
            if self.scenario == Scenario::Landing {
                x = 12.0 + i.min(self.count as f64 / 2.0) * 0.7;
            }
            if self.scenario == Scenario::LeavingFrame {
                x = 8.0 + i * width / (self.count as f64 * 0.65);
            }
            if self.scenario == Scenario::Reentry {
                let leave = (self.count / 3).max(1);
                let return_at = self.count / 3 * 2 + self.count % 3 * 2 / 3;
                x = if self.index < leave {
                    8.0 + i * (width - 4.0) / leave as f64
                } else if self.index < return_at {
                    width + 4.0
                } else {
                    width
                        - 8.0
                        - (self.index - return_at) as f64 * (width - 20.0)
                            / (self.count - return_at).max(1) as f64
                };
                if (leave..return_at).contains(&self.index) {
                    visible = false;
                }
            }
            if self.scenario == Scenario::Occlusion
                && (self.count / 3..self.count / 2).contains(&self.index)
            {
                visible = false;
            }
            visible = visible && x >= 0.0 && x < width - 3.0;
            if visible {
                self.rectangle(frame, x as i32, y as i32, 3, 3, 20);
            }
            if has_fly {
                let position = ScenePosition::new(x.floor() + 1.0, y.floor() + 1.0)?;
                if let Some(truth) = &mut frame.truth {
                    truth.targets.push(TruthTarget {
                        id: TargetId(1),
                        position,
                        visible,
                    });
                }
            }
            if self.scenario == Scenario::MultipleFlies {
                let second_x = width - 12.0 - i * 0.5;
                let visible = second_x >= 0.0 && second_x < width - 3.0;
                if visible {
                    self.rectangle(frame, second_x as i32, (y * 2.0) as i32, 3, 3, 20);
                }
                if let Some(truth) = &mut frame.truth {
                    truth.targets.push(TruthTarget {
                        id: TargetId(2),
                        position: ScenePosition::new(
                            second_x.floor() + 1.0,
                            (y * 2.0).floor() + 1.0,
                        )?,
                        visible,
                    });
                }
            }
            if self.scenario == Scenario::DustDebris {
                self.rectangle(
                    frame,
                    (self.index.wrapping_mul(2) % u64::from(self.size.width())) as i32,
                    self.size.height() as i32 / 2,
                    1,
                    1,
                    20,
                );
            }
            if self.scenario == Scenario::MovingObject {
                self.rectangle(
                    frame,
                    (self.index % u64::from(self.size.width())) as i32,
                    self.size.height() as i32 / 2,
                    20,
                    15,
                    20,
                );
            }
        }
        if self.scenario.hazard().is_some() {
            // Abstract silhouettes test interlocks, not real semantic recognition.
            let partial = matches!(
                self.scenario,
                Scenario::PartialHuman
                    | Scenario::PartialDog
                    | Scenario::PartialCat
                    | Scenario::EdgeHand
                    | Scenario::EdgeArm
            );
            self.rectangle(
                frame,
                if partial {
                    -8
                } else {
                    self.size.width() as i32 / 2
                },
                self.size.height() as i32 / 2,
                16,
                24,
                60,
            );
        }
        frame.validate()?;
        self.index += 1;
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_scenarios_repeat_with_ground_truth() -> Result<(), CameraError> {
        for seed in [0, 1, u64::MAX] {
            for scenario in Scenario::ALL {
                let size = FrameSize::new(96, 64)?;
                let mut source = SyntheticSource::new(size, scenario, seed, 12, 10_000)?;
                let mut frame = Frame::new(size);
                let mut first = Vec::new();
                while source.next_into(&mut frame)? {
                    first.push(frame.clone());
                }
                source.reset()?;
                for expected in first {
                    assert!(source.next_into(&mut frame)?);
                    assert_eq!(frame, expected, "seed={seed}, scenario={scenario:?}");
                }
                assert!(!source.next_into(&mut frame)?);
            }
        }
        Ok(())
    }
    #[test]
    fn invisible_coordinates_and_real_reentry_are_present() -> Result<(), CameraError> {
        let size = FrameSize::new(96, 64)?;
        for scenario in [Scenario::Occlusion, Scenario::Reentry] {
            let mut source = SyntheticSource::new(size, scenario, 42, 60, 10_000)?;
            let mut frame = Frame::new(size);
            let mut invisible = false;
            let mut returned = false;
            while source.next_into(&mut frame)? {
                if let Some(target) = frame.truth.as_ref().and_then(|truth| truth.targets.first()) {
                    if !target.visible {
                        invisible = true;
                        assert!(target.position.x().is_finite());
                    } else if invisible {
                        returned = true;
                    }
                }
            }
            assert!(invisible && returned, "scenario={scenario:?}");
        }
        let mut large = SyntheticSource::new(size, Scenario::Reentry, 42, u64::MAX, 1)?;
        let mut frame = Frame::new(size);
        assert!(large.next_into(&mut frame)?);
        assert!(large.next_into(&mut frame)?);
        Ok(())
    }
}
