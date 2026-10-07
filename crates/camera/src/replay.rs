use crate::{CameraError, Frame, FrameSource, GroundTruth};
use fly_core::{FrameId, FrameSize, FrameTimestamp};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

const MAX_METADATA: u64 = 16 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceEntry {
    pub file: PathBuf,
    pub id: FrameId,
    pub timestamp: FrameTimestamp,
    pub truth: Option<GroundTruth>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceManifest {
    pub version: u32,
    pub size: FrameSize,
    pub frames: Vec<SequenceEntry>,
}
pub struct ImageSequence {
    manifest: SequenceManifest,
    base: PathBuf,
    index: usize,
}
impl ImageSequence {
    pub fn open(path: &Path) -> Result<Self, CameraError> {
        let file = File::open(path)?;
        if file.metadata()?.len() > MAX_METADATA {
            return Err(CameraError::Format("manifest too large"));
        }
        let manifest: SequenceManifest = serde_json::from_reader(file)?;
        if manifest.version != 1 || manifest.frames.is_empty() || manifest.frames.len() > 100_000 {
            return Err(CameraError::Format(
                "invalid manifest version or frame count",
            ));
        }
        for pair in manifest.frames.windows(2) {
            if pair[1].timestamp <= pair[0].timestamp || pair[1].id <= pair[0].id {
                return Err(CameraError::Format(
                    "frame IDs and timestamps must increase",
                ));
            }
        }
        let base = path.parent().unwrap_or(Path::new(".")).canonicalize()?;
        for entry in &manifest.frames {
            if !base.join(&entry.file).canonicalize()?.starts_with(&base) {
                return Err(CameraError::Format("image path outside sequence directory"));
            }
        }
        Ok(Self {
            manifest,
            base,
            index: 0,
        })
    }
}
// Read one small header line without allowing unbounded allocation.
fn line(reader: &mut impl BufRead) -> Result<Option<String>, CameraError> {
    let mut bytes = Vec::new();
    let count = reader.take(4097).read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    if count > 4096 || bytes.last() != Some(&b'\n') {
        return Err(CameraError::Format("truncated or oversized header"));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| CameraError::Format("non-UTF8 header"))
}
impl FrameSource for ImageSequence {
    fn next_timestamp(&self) -> Option<FrameTimestamp> {
        self.manifest
            .frames
            .get(self.index)
            .map(|entry| entry.timestamp)
    }
    fn total_frames(&self) -> Option<u64> {
        Some(self.manifest.frames.len() as u64)
    }
    fn size(&self) -> FrameSize {
        self.manifest.size
    }
    fn reset(&mut self) -> Result<(), CameraError> {
        self.index = 0;
        Ok(())
    }
    fn next_into(&mut self, frame: &mut Frame) -> Result<bool, CameraError> {
        let Some(entry) = self.manifest.frames.get(self.index) else {
            return Ok(false);
        };
        if frame.size != self.size() {
            return Err(CameraError::Format("frame size mismatch"));
        }
        let mut file = BufReader::new(File::open(self.base.join(&entry.file))?);
        if line(&mut file)?.as_deref() != Some("P5\n") {
            return Err(CameraError::Format("require binary PGM P5"));
        }
        let dimensions = line(&mut file)?.ok_or(CameraError::Format("missing PGM dimensions"))?;
        let expected = format!("{} {}", frame.size.width(), frame.size.height());
        if dimensions.trim() != expected || line(&mut file)?.as_deref() != Some("255\n") {
            return Err(CameraError::Format("PGM size/depth mismatch"));
        }
        frame.pixels.resize(frame.size.pixels(), 0);
        file.read_exact(&mut frame.pixels)?;
        let mut extra = [0];
        if file.read(&mut extra)? != 0 {
            return Err(CameraError::Format("extra PGM data"));
        }
        frame.id = entry.id;
        frame.timestamp = entry.timestamp;
        frame.truth.clone_from(&entry.truth);
        frame.validate()?;
        self.index += 1;
        Ok(true)
    }
}

/// Streaming YUV4MPEG2 Cmono/Cmono8 video. Memory does not grow with video length.
pub struct MonoVideo {
    path: PathBuf,
    reader: BufReader<File>,
    size: FrameSize,
    numerator: u64,
    denominator: u64,
    index: u64,
}
impl MonoVideo {
    pub fn open(path: &Path) -> Result<Self, CameraError> {
        let mut reader = BufReader::new(File::open(path)?);
        let header = line(&mut reader)?.ok_or(CameraError::Format("missing video header"))?;
        let mut fields = header.split_whitespace();
        if fields.next() != Some("YUV4MPEG2") {
            return Err(CameraError::Format("require YUV4MPEG2"));
        }
        let (mut width, mut height, mut rate, mut mono) = (None, None, None, false);
        for field in fields {
            if let Some(value) = field.strip_prefix('W') {
                width = value.parse::<u32>().ok();
            }
            if let Some(value) = field.strip_prefix('H') {
                height = value.parse::<u32>().ok();
            }
            if let Some(value) = field.strip_prefix('F') {
                rate = value
                    .split_once(':')
                    .and_then(|(n, d)| Some((n.parse::<u64>().ok()?, d.parse::<u64>().ok()?)));
            }
            if field == "Cmono" || field == "Cmono8" {
                mono = true;
            }
            if field.starts_with('I') && field != "Ip" {
                return Err(CameraError::Format("interlaced video unsupported"));
            }
        }
        let size = FrameSize::new(
            width.ok_or(CameraError::Format("missing width"))?,
            height.ok_or(CameraError::Format("missing height"))?,
        )?;
        let (numerator, denominator) = rate.ok_or(CameraError::Format("missing frame rate"))?;
        if !mono
            || numerator == 0
            || denominator == 0
            || numerator > 1_000_000
            || denominator > 1_000_000
        {
            return Err(CameraError::Format("require mono8 and valid frame rate"));
        }
        Ok(Self {
            path: path.to_owned(),
            reader,
            size,
            numerator,
            denominator,
            index: 0,
        })
    }
}
impl FrameSource for MonoVideo {
    fn next_timestamp(&self) -> Option<FrameTimestamp> {
        u64::try_from(
            u128::from(self.index) * 1_000_000 * u128::from(self.denominator)
                / u128::from(self.numerator),
        )
        .ok()
        .map(FrameTimestamp)
    }
    fn size(&self) -> FrameSize {
        self.size
    }
    fn reset(&mut self) -> Result<(), CameraError> {
        *self = Self::open(&self.path)?;
        Ok(())
    }
    fn next_into(&mut self, frame: &mut Frame) -> Result<bool, CameraError> {
        let Some(header) = line(&mut self.reader)? else {
            return Ok(false);
        };
        if header.trim() != "FRAME" || frame.size != self.size {
            return Err(CameraError::Format(
                "unsupported frame header or dimensions",
            ));
        }
        frame.pixels.resize(self.size.pixels(), 0);
        self.reader.read_exact(&mut frame.pixels)?;
        frame.id = FrameId(self.index);
        frame.timestamp = FrameTimestamp(
            u64::try_from(
                u128::from(self.index) * 1_000_000 * u128::from(self.denominator)
                    / u128::from(self.numerator),
            )
            .map_err(|_| CameraError::Format("video timestamp overflow"))?,
        );
        frame.truth = None;
        self.index = self
            .index
            .checked_add(1)
            .ok_or(CameraError::Format("frame ID overflow"))?;
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn headers_are_bounded_and_truncation_is_an_error() {
        assert!(line(&mut std::io::Cursor::new(vec![b'x'; 5000])).is_err());
        assert!(line(&mut std::io::Cursor::new(b"FRAME")).is_err());
        assert!(matches!(line(&mut std::io::Cursor::new(b"")), Ok(None)));
    }
}
