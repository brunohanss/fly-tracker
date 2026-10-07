//! Bounded image view. Pixel preparation is performed on snapshot receipt.
use fly_core::{FrameSize, PixelPosition};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use telemetry::Preview;

#[derive(Debug, Default)]
pub struct CameraImage {
    width: u16,
    height: u16,
    pixels: Vec<u8>,
}

impl CameraImage {
    pub fn prepare(&mut self, preview: &Preview) {
        self.pixels.clear();
        self.width = 0;
        self.height = 0;
        if preview.width == 0
            || preview.height == 0
            || preview.width > 160
            || preview.height > 120
            || preview.pixels.len() != usize::from(preview.width) * usize::from(preview.height)
        {
            return;
        }
        self.width = preview.width;
        self.height = preview.height;
        self.pixels.extend_from_slice(&preview.pixels);
    }

    /// Returns the actual image rectangle, shared by all coordinate overlays.
    /// Assumes a terminal cell is approximately twice as tall as it is wide.
    pub fn render(&self, area: Rect, size: FrameSize, ascii: bool, buffer: &mut Buffer) -> Rect {
        if self.pixels.is_empty() || area.is_empty() {
            return Rect::new(area.x, area.y, 0, 0);
        }
        let image = fit(area, size);
        for y in 0..image.height {
            for x in 0..image.width {
                let column = usize::from(x) * usize::from(self.width) / usize::from(image.width);
                let sample = |half: u16| {
                    let row = (usize::from(y) * 2 + usize::from(half)) * usize::from(self.height)
                        / (usize::from(image.height) * 2);
                    self.pixels[row * usize::from(self.width) + column]
                };
                let top = sample(0);
                let bottom = sample(1);
                let cell = &mut buffer[(image.x + x, image.y + y)];
                if ascii {
                    let value = (u16::from(top) + u16::from(bottom)) / 2;
                    let ramp = b" .:-=+*#%@";
                    cell.set_char(char::from(
                        ramp[usize::from(value) * (ramp.len() - 1) / 255],
                    ))
                    .set_fg(Color::White)
                    .set_bg(Color::Black);
                } else {
                    cell.set_char('▀')
                        .set_fg(Color::Rgb(top, top, top))
                        .set_bg(Color::Rgb(bottom, bottom, bottom));
                }
            }
        }
        image
    }
}

pub fn fit(area: Rect, size: FrameSize) -> Rect {
    if area.is_empty() {
        return area;
    }
    let ratio = f64::from(size.width()) / f64::from(size.height());
    let width = area
        .width
        .min((f64::from(area.height) * 2.0 * ratio).floor().max(1.0) as u16);
    let height = area
        .height
        .min((f64::from(width) / (2.0 * ratio)).ceil().max(1.0) as u16);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

pub fn project(point: PixelPosition, size: FrameSize, image: Rect) -> Option<(u16, u16)> {
    if image.is_empty() || !size.contains(point) {
        return None;
    }
    Some((
        image.x + (point.x() / f64::from(size.width()) * f64::from(image.width)) as u16,
        image.y + (point.y() / f64::from(size.height()) * f64::from(image.height)) as u16,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_blocks_keep_both_luminances() -> Result<(), fly_core::DomainError> {
        let mut image = CameraImage::default();
        image.prepare(&Preview {
            width: 2,
            height: 2,
            pixels: vec![0, 64, 255, 128],
        });
        let area = Rect::new(0, 0, 2, 1);
        let mut buffer = Buffer::empty(area);
        image.render(area, FrameSize::new(2, 2)?, false, &mut buffer);
        assert_eq!(buffer[(0, 0)].symbol(), "▀");
        assert_eq!(buffer[(0, 0)].fg, Color::Rgb(0, 0, 0));
        assert_eq!(buffer[(0, 0)].bg, Color::Rgb(255, 255, 255));
        assert_eq!(buffer[(1, 0)].fg, Color::Rgb(64, 64, 64));
        Ok(())
    }

    #[test]
    fn aspect_and_overlay_share_letterboxed_coordinates() -> Result<(), fly_core::DomainError> {
        let size = FrameSize::new(640, 480)?;
        let area = fit(Rect::new(10, 5, 80, 10), size);
        assert_eq!(area.width, 26);
        assert_eq!(area.height, 10);
        assert_eq!(
            project(PixelPosition::new(320.0, 240.0)?, size, area),
            Some((area.x + 13, area.y + 5))
        );
        assert_eq!(project(PixelPosition::new(640.0, 480.0)?, size, area), None);
        Ok(())
    }

    #[test]
    fn invalid_image_clears_old_pixels_and_ascii_works() -> Result<(), fly_core::DomainError> {
        let mut image = CameraImage::default();
        let size = FrameSize::new(2, 2)?;
        image.prepare(&Preview {
            width: 2,
            height: 2,
            pixels: vec![255; 4],
        });
        let area = Rect::new(0, 0, 2, 1);
        let mut buffer = Buffer::empty(area);
        image.render(area, size, true, &mut buffer);
        assert_eq!(buffer[(0, 0)].symbol(), "@");
        image.prepare(&Preview {
            width: 160,
            height: 120,
            pixels: vec![],
        });
        assert!(image.render(area, size, false, &mut buffer).is_empty());
        assert!(
            image
                .render(Rect::default(), size, false, &mut buffer)
                .is_empty()
        );
        Ok(())
    }
}
