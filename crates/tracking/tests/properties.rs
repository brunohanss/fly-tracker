use fly_core::{Confidence, Detection, FrameSize, FrameTimestamp, PixelPosition};
use tracking::{Tracker, TrackerConfig, predict};
#[test]
fn seeded_association_preserves_unique_ids_bounds_and_finite_predictions()
-> Result<(), Box<dyn std::error::Error>> {
    let size = FrameSize::new(320, 240)?;
    for seed in 0..32_u64 {
        let mut random = seed;
        let mut next = || {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            random >> 32
        };
        let mut tracker = Tracker::new(size, TrackerConfig::default())?;
        let mut timestamp = 0;
        let mut detections = Vec::with_capacity(10);
        for _ in 0..100 {
            timestamp += 5000 + next() % 10_000;
            detections.clear();
            for _ in 0..next() % 10 {
                detections.push(Detection {
                    position: PixelPosition::new((next() % 320) as f64, (next() % 240) as f64)?,
                    area: 9,
                    confidence: Confidence::new(1.0)?,
                });
            }
            tracker.update(&detections, FrameTimestamp(timestamp))?;
            assert!(tracker.tracks().len() <= 32, "seed={seed}");
            assert!(tracker.recent().len() <= 32, "seed={seed}");
            for (index, track) in tracker.tracks().iter().enumerate() {
                assert!(
                    tracker.tracks()[index + 1..]
                        .iter()
                        .all(|other| track.id != other.id),
                    "seed={seed}"
                );
                for horizon in [0, 5000, 10000, 20000] {
                    if let Some(point) =
                        predict(track, FrameTimestamp(timestamp + horizon), size, 1_000_000)?
                    {
                        assert!(size.contains(point), "seed={seed}");
                        assert!(
                            point.x().is_finite() && point.y().is_finite(),
                            "seed={seed}"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
