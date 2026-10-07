use camera::{
    Frame, FrameSource,
    replay::{ImageSequence, MonoVideo, SequenceEntry, SequenceManifest},
};
use fly_core::{FrameId, FrameSize, FrameTimestamp};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Files(PathBuf);
impl Files {
    fn new() -> std::io::Result<Self> {
        let root = std::env::temp_dir().join(format!(
            "fly-tracker-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root)?;
        Ok(Self(root))
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        if self.0.starts_with(std::env::temp_dir())
            && self
                .0
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("fly-tracker-test-"))
        {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
#[test]
fn image_and_video_sources_share_frames_and_reset() -> Result<(), Box<dyn std::error::Error>> {
    let files = Files::new()?;
    let size = FrameSize::new(2, 2)?;
    let mut video = b"YUV4MPEG2 W2 H2 F100:1 Ip Cmono\n".to_vec();
    let mut entries = Vec::new();
    for index in 0..3_u8 {
        let pixels = [index; 4];
        let name = format!("{index}.pgm");
        let mut pgm = b"P5\n2 2\n255\n".to_vec();
        pgm.extend(pixels);
        std::fs::write(files.0.join(&name), pgm)?;
        video.extend(b"FRAME\n");
        video.extend(pixels);
        entries.push(SequenceEntry {
            file: name.into(),
            id: FrameId(u64::from(index)),
            timestamp: FrameTimestamp(u64::from(index) * 10_000),
            truth: None,
        });
    }
    let manifest = SequenceManifest {
        version: 1,
        size,
        frames: entries,
    };
    let manifest_path = files.0.join("sequence.json");
    let video_path = files.0.join("video.y4m");
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest)?)?;
    std::fs::write(&video_path, &video)?;
    let mut images = ImageSequence::open(&manifest_path)?;
    let mut movie = MonoVideo::open(&video_path)?;
    for _ in 0..2 {
        let mut a = Frame::new(size);
        let mut b = Frame::new(size);
        while images.next_into(&mut a)? {
            assert!(movie.next_into(&mut b)?);
            assert_eq!(a, b);
        }
        assert!(!movie.next_into(&mut b)?);
        images.reset()?;
        movie.reset()?;
    }
    video.pop();
    std::fs::write(&video_path, video)?;
    let mut truncated = MonoVideo::open(&video_path)?;
    let mut frame = Frame::new(size);
    assert!(truncated.next_into(&mut frame)?);
    assert!(truncated.next_into(&mut frame)?);
    assert!(truncated.next_into(&mut frame).is_err());
    Ok(())
}
#[test]
fn exported_seeded_sequence_matches_original_and_cannot_replace_recording()
-> Result<(), Box<dyn std::error::Error>> {
    use camera::{
        export::{export_sequence, source_fingerprint},
        synthetic::{Scenario, SyntheticSource},
    };
    let files = Files::new()?;
    let directory = files.0.join("recording");
    let size = FrameSize::new(96, 64)?;
    let mut original = SyntheticSource::new(size, Scenario::MultipleFlies, 42, 20, 10_000)?;
    let fingerprint = source_fingerprint(&mut original)?;
    let manifest = export_sequence(&mut original, &directory)?;
    assert!(export_sequence(&mut original, &directory).is_err());
    let mut replay = ImageSequence::open(&manifest)?;
    assert_eq!(source_fingerprint(&mut replay)?, fingerprint);
    let mut frame = Frame::new(size);
    assert!(replay.next_into(&mut frame)?);
    assert_eq!(frame.id, FrameId(0));
    Ok(())
}
