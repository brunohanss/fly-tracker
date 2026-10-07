//! Inspect image-only detections and verify approximate annotated stationary targets.
use anyhow::{Context, Result};
use camera::{Frame, FrameSource, replay::ImageSequence};
use fly_core::config::Config;
use std::{fs::OpenOptions, path::Path};
use vision::{Detector, DetectorConfig};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 3, "MANIFEST CONFIG NEW_OUTPUT_JSON");
    let config = Config::load(Path::new(&args[1]))?;
    let mut source = ImageSequence::open(Path::new(&args[0]))?;
    anyhow::ensure!(source.size() == config.frame_size, "Size mismatch");
    let p = &config.processing;
    let mut detector = Detector::new(
        source.size(),
        DetectorConfig {
            stationary_detection: p.stationary_detection,
            region: p.detection_region,
            threshold: p.detection_threshold,
            min_neighbors: p.morphology_min_neighbors,
            min_area: p.min_area,
            max_area: p.max_area,
            capacity: config.max_detections,
            ..DetectorConfig::default()
        },
    )?;
    let mut frame = Frame::new(source.size());
    let mut records = Vec::new();
    let mut first_pixels = None;
    while source.next_into(&mut frame)? {
        if let Some(pixels) = &first_pixels {
            anyhow::ensure!(*pixels == frame.pixels, "Frames differ");
        } else {
            first_pixels = Some(frame.pixels.clone());
        }
        let detections = detector.detect(&frame)?;
        let points: Vec<_> = detections
            .iter()
            .map(|d| serde_json::json!({"x":d.position.x(),"y":d.position.y(),"area":d.area}))
            .collect();
        let expected = frame
            .truth
            .as_ref()
            .context("Target annotations required")?;
        let mut used = vec![false; detections.len()];
        let mut missed = 0;
        for target in &expected.targets {
            let position = target
                .position
                .pixel_in(frame.size)
                .context("Annotation outside frame")?;
            let matched = detections.iter().enumerate().find(|(i, d)| {
                !used[*i]
                    && (d.position.x() - position.x()).hypot(d.position.y() - position.y()) <= 6.0
            });
            if let Some((i, _)) = matched {
                used[i] = true;
            } else {
                missed += 1;
            }
        }
        records.push(serde_json::json!({"frame":frame.id.0,"detections":points,"missed_targets":missed,"unmatched_candidates":used.iter().filter(|v| !**v).count()}));
    }
    anyhow::ensure!(records.len() == 5, "Expected exactly five frames");
    serde_json::to_writer_pretty(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[2])?,
        &records,
    )?;
    Ok(())
}
