use crate::{
    CameraError, Frame, FrameSource,
    replay::{SequenceEntry, SequenceManifest},
};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// Write a new sequence directory. Never replace an existing recording.
pub fn export_sequence(
    source: &mut dyn FrameSource,
    directory: &Path,
) -> Result<PathBuf, CameraError> {
    std::fs::create_dir(directory)?;
    source.reset()?;
    let size = source.size();
    let mut frame = Frame::new(size);
    let mut entries = Vec::new();
    while source.next_into(&mut frame)? {
        if entries.len() == 100_000 {
            return Err(CameraError::Format("recording exceeds 100000 frame limit"));
        }
        let file = PathBuf::from(format!("{:08}.pgm", entries.len()));
        let mut image = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join(&file))?;
        writeln!(image, "P5\n{} {}\n255", size.width(), size.height())?;
        image.write_all(&frame.pixels)?;
        entries.push(SequenceEntry {
            file,
            id: frame.id,
            timestamp: frame.timestamp,
            truth: frame.truth.clone(),
        });
    }
    if entries.is_empty() {
        return Err(CameraError::Format("cannot export empty source"));
    }
    let manifest = SequenceManifest {
        version: 1,
        size,
        frames: entries,
    };
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(CameraError::Format(
            "recording metadata exceeds 16MiB limit",
        ));
    }
    let path = directory.join("sequence.json");
    let mut file = File::create(&path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    source.reset()?;
    Ok(path)
}
/// Include all pixels, timing and labels in replay identity. Uses one reusable frame.
pub fn source_fingerprint(source: &mut dyn FrameSource) -> Result<String, CameraError> {
    source.reset()?;
    let mut frame = Frame::new(source.size());
    let mut hash = 14695981039346656037_u64;
    let mut update = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(1099511628211);
        }
    };
    update(&source.size().width().to_le_bytes());
    update(&source.size().height().to_le_bytes());
    while source.next_into(&mut frame)? {
        update(&frame.id.0.to_le_bytes());
        update(&frame.timestamp.0.to_le_bytes());
        update(&frame.pixels);
        update(&serde_json::to_vec(&frame.truth)?);
    }
    source.reset()?;
    Ok(format!("{hash:016x}"))
}
