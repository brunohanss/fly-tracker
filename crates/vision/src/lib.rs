#![forbid(unsafe_code)]
use camera::Frame;
use fly_core::{Confidence, Detection, FrameSize, PixelPosition, config::PixelRegion};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct DetectorConfig {
    pub stationary_detection: bool,
    pub region: Option<PixelRegion>,
    pub min_neighbors: u8,
    pub threshold: u8,
    pub min_area: u32,
    pub max_area: u32,
    pub max_aspect: f64,
    pub capacity: usize,
}
impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            stationary_detection: true,
            region: None,
            min_neighbors: 1,
            threshold: 35,
            min_area: 3,
            max_area: 64,
            max_aspect: 4.0,
            capacity: 128,
        }
    }
}
#[derive(Debug, Error)]
pub enum VisionError {
    #[error("Invalid detector configuration or frame size")]
    Invalid,
    #[error("Frame failed validation: {0}")]
    Frame(#[from] camera::CameraError),
    #[error("Invalid geometry: {0}")]
    Domain(#[from] fly_core::DomainError),
}
pub struct Detector {
    size: FrameSize,
    config: DetectorConfig,
    initialized: bool,
    background: Vec<f64>,
    integral: Vec<u64>,
    mask: Vec<u8>,
    raw_mask: Vec<u8>,
    visited: Vec<bool>,
    stack: Vec<usize>,
    detections: Vec<Detection>,
    truncated: bool,
}
impl Detector {
    pub fn new(size: FrameSize, config: DetectorConfig) -> Result<Self, VisionError> {
        if config.threshold == 0
            || config.region.is_some_and(|region| !region.valid(size))
            || config.min_neighbors > 8
            || config.min_area == 0
            || config.max_area < config.min_area
            || !config.max_aspect.is_finite()
            || config.max_aspect < 1.0
            || !(1..=65536).contains(&config.capacity)
        {
            return Err(VisionError::Invalid);
        }
        Ok(Self {
            size,
            config,
            initialized: false,
            background: vec![0.0; size.pixels()],
            integral: if config.stationary_detection {
                vec![0; (size.width() as usize + 1) * (size.height() as usize + 1)]
            } else {
                Vec::new()
            },
            mask: vec![0; size.pixels()],
            raw_mask: vec![0; size.pixels()],
            visited: vec![false; size.pixels()],
            stack: Vec::with_capacity(size.pixels()),
            detections: Vec::with_capacity(config.capacity),
            truncated: false,
        })
    }
    pub fn reset(&mut self) {
        self.initialized = false;
        self.detections.clear();
        self.mask.fill(0);
    }
    pub fn mask(&self) -> &[u8] {
        &self.mask
    }
    pub fn truncated(&self) -> bool {
        self.truncated
    }
    pub fn detected(&self) -> &[Detection] {
        &self.detections
    }
    pub fn detect(&mut self, frame: &Frame) -> Result<&[Detection], VisionError> {
        frame.validate()?;
        if frame.size != self.size {
            return Err(VisionError::Invalid);
        }
        self.detections.clear();
        self.truncated = false;
        if !self.initialized {
            for (background, pixel) in self.background.iter_mut().zip(&frame.pixels) {
                *background = f64::from(*pixel);
            }
            self.initialized = true;
            self.mask.fill(0);
            if !self.config.stationary_detection {
                return Ok(&self.detections);
            }
        }
        for ((background, pixel), mask) in self
            .background
            .iter_mut()
            .zip(&frame.pixels)
            .zip(&mut self.raw_mask)
        {
            let difference = (f64::from(*pixel) - *background).abs();
            *mask = if difference >= f64::from(self.config.threshold) {
                255
            } else {
                0
            };
            // Foreground must not be absorbed immediately after landing.
            if *mask == 0 {
                *background += (f64::from(*pixel) - *background) / 32.0;
            }
        }
        self.visited.fill(false);
        let width = self.size.width() as usize;
        let height = self.size.height() as usize;
        if self.config.stationary_detection {
            // Local spatial contrast detects dark targets even in the first frame.
            // Summed-area storage is allocated once; lighting gradients stay in the local mean.
            let stride = width + 1;
            self.integral.fill(0);
            for y in 0..height {
                let mut row_sum = 0_u64;
                for x in 0..width {
                    row_sum += u64::from(frame.pixels[y * width + x]);
                    self.integral[(y + 1) * stride + x + 1] =
                        self.integral[y * stride + x + 1] + row_sum;
                }
            }
            const RADIUS: usize = 8;
            for y in 0..height {
                for x in 0..width {
                    let left = x.saturating_sub(RADIUS);
                    let top = y.saturating_sub(RADIUS);
                    let right = (x + RADIUS + 1).min(width);
                    let bottom = (y + RADIUS + 1).min(height);
                    let sum = (self.integral[bottom * stride + right]
                        - self.integral[top * stride + right])
                        - (self.integral[bottom * stride + left]
                            - self.integral[top * stride + left]);
                    let mean = sum as f64 / ((right - left) * (bottom - top)) as f64;
                    let index = y * width + x;
                    if mean - f64::from(frame.pixels[index]) >= f64::from(self.config.threshold) {
                        self.raw_mask[index] = 255;
                    }
                }
            }
        }
        if let Some(region) = self.config.region {
            for y in 0..height {
                for x in 0..width {
                    if !region.contains(x, y) {
                        self.raw_mask[y * width + x] = 0;
                    }
                }
            }
        }
        // Minimal morphology removes isolated pixels and preserves tiny connected flies.
        for index in 0..self.raw_mask.len() {
            if self.raw_mask[index] == 0 {
                self.mask[index] = 0;
                continue;
            }
            let x = index % width;
            let y = index / width;
            let mut neighbors = 0_u8;
            for row in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for column in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let other = row * width + column;
                    if other != index && self.raw_mask[other] != 0 {
                        neighbors += 1;
                    }
                }
            }
            self.mask[index] = if neighbors >= self.config.min_neighbors {
                255
            } else {
                0
            };
        }
        for start in 0..self.mask.len() {
            if self.mask[start] == 0 || self.visited[start] {
                continue;
            }
            self.stack.clear();
            self.stack.push(start);
            self.visited[start] = true;
            let (mut area, mut sum_x, mut sum_y) = (0_u32, 0_u64, 0_u64);
            let (mut min_x, mut max_x, mut min_y, mut max_y) = (width, 0, height, 0);
            while let Some(index) = self.stack.pop() {
                let x = index % width;
                let y = index / width;
                area += 1;
                sum_x += x as u64;
                sum_y += y as u64;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                for row in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                    for column in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                        let neighbor = row * width + column;
                        if self.mask[neighbor] != 0 && !self.visited[neighbor] {
                            self.visited[neighbor] = true;
                            self.stack.push(neighbor);
                        }
                    }
                }
            }
            let w = (max_x - min_x + 1) as f64;
            let h = (max_y - min_y + 1) as f64;
            if area < self.config.min_area
                || area > self.config.max_area
                || w.max(h) / w.min(h) > self.config.max_aspect
            {
                continue;
            }
            if self.detections.len() == self.config.capacity {
                self.truncated = true;
                continue;
            }
            self.detections.push(Detection {
                position: PixelPosition::new(
                    sum_x as f64 / f64::from(area),
                    sum_y as f64 / f64::from(area),
                )?,
                area,
                confidence: Confidence::new(f64::from(area) / (w * h))?,
            });
        }
        Ok(&self.detections)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use camera::{
        FrameSource,
        synthetic::{Scenario, SyntheticSource},
    };
    #[test]
    fn stationary_targets_in_initial_background_persist_and_roi_is_validated()
    -> Result<(), Box<dyn std::error::Error>> {
        let size = FrameSize::new(96, 64)?;
        let mut frame = Frame::new(size);
        // Slow lighting gradient; five small dark targets and isolated dark noise.
        for y in 0..64_usize {
            for x in 0..96_usize {
                frame.pixels[y * 96 + x] = 150 + (x / 4) as u8;
            }
        }
        let centers = [(12, 12), (30, 12), (50, 20), (70, 40), (85, 52)];
        for (x, y) in centers {
            for row in y - 1..=y + 1 {
                for col in x - 1..=x + 1 {
                    frame.pixels[row * 96 + col] = 20;
                }
            }
        }
        frame.pixels[60 * 96 + 4] = 0;
        let mut detector = Detector::new(size, DetectorConfig::default())?;
        for _ in 0..5 {
            let found = detector.detect(&frame)?;
            assert_eq!(found.len(), 5);
            for (x, y) in centers {
                assert!(found.iter().any(|d| d.position
                    == PixelPosition::new(x as f64, y as f64).unwrap_or_else(|_| unreachable!())));
            }
        }
        detector.reset();
        assert_eq!(detector.detect(&frame)?.len(), 5);
        let mut motion_only = Detector::new(
            size,
            DetectorConfig {
                stationary_detection: false,
                ..DetectorConfig::default()
            },
        )?;
        for _ in 0..5 {
            assert!(motion_only.detect(&frame)?.is_empty());
        }
        let region = PixelRegion {
            x: 0,
            y: 0,
            width: 60,
            height: 64,
        };
        let mut limited = Detector::new(
            size,
            DetectorConfig {
                region: Some(region),
                ..DetectorConfig::default()
            },
        )?;
        assert_eq!(limited.detect(&frame)?.len(), 3);
        assert!(
            Detector::new(
                size,
                DetectorConfig {
                    region: Some(PixelRegion {
                        x: u32::MAX,
                        y: 0,
                        width: 10,
                        height: 10
                    }),
                    ..DetectorConfig::default()
                }
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn fly_centroids_are_measured_without_ground_truth_input()
    -> Result<(), Box<dyn std::error::Error>> {
        let size = FrameSize::new(96, 64)?;
        let mut source = SyntheticSource::new(size, Scenario::SlowFly, 123, 20, 10_000)?;
        let mut frame = Frame::new(size);
        let mut detector = Detector::new(size, DetectorConfig::default())?;
        while source.next_into(&mut frame)? {
            let expected = frame.truth.take();
            let detections = detector.detect(&frame)?;
            if frame.id.0 > 0 {
                let truth = expected.ok_or("missing truth")?;
                assert_eq!(detections.len(), 1);
                assert_eq!(
                    Some(detections[0].position),
                    truth.targets[0].position.pixel_in(size)
                );
            } else {
                assert!(detections.is_empty());
            }
        }
        Ok(())
    }
    #[test]
    fn empty_wall_dust_and_large_objects_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let size = FrameSize::new(96, 64)?;
        for scenario in [
            Scenario::EmptyWall,
            Scenario::DustDebris,
            Scenario::MovingObject,
        ] {
            let mut source = SyntheticSource::new(size, scenario, 42, 20, 10_000)?;
            let mut frame = Frame::new(size);
            let mut detector = Detector::new(size, DetectorConfig::default())?;
            while source.next_into(&mut frame)? {
                assert!(detector.detect(&frame)?.is_empty(), "scenario={scenario:?}");
            }
        }
        Ok(())
    }
}
