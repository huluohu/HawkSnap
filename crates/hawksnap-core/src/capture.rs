use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ScreenFrame {
    pub display_id: u64,
    pub x: i32, pub y: i32,          // 逻辑坐标（多屏虚拟桌面）
    pub width: u32, pub height: u32, // 逻辑尺寸
    pub scale: f64,                  // DPI 缩放（Retina=2.0）
    pub png: Vec<u8>,                // 物理像素 PNG
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("no display found")]
    NoDisplays,
    #[error("capture failed: {0}")]
    Backend(String),
    #[error("permission denied for screen capture")]
    Permission,
}

pub trait ScreenGrabber {
    fn grab_all(&mut self) -> Result<Vec<ScreenFrame>, CaptureError>;
}

/// 测试桩：预置帧直接返回。
pub struct InMemGrabber { frames: Vec<ScreenFrame> }

impl InMemGrabber {
    pub fn new(frames: Vec<ScreenFrame>) -> Self { InMemGrabber { frames } }
}

impl ScreenGrabber for InMemGrabber {
    fn grab_all(&mut self) -> Result<Vec<ScreenFrame>, CaptureError> {
        if self.frames.is_empty() { Err(CaptureError::NoDisplays) } else { Ok(self.frames.clone()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::new(w, h);
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[test]
    fn in_mem_grabber_returns_frames_in_order() {
        let mut g = InMemGrabber::new(vec![
            ScreenFrame { display_id: 1, x: 0, y: 0, width: 100, height: 50, scale: 2.0, png: frame_png(200, 100) },
        ]);
        let frames = g.grab_all().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!((frames[0].width, frames[0].height), (100, 50)); // 逻辑尺寸
        assert_eq!(frames[0].png.len() > 8, true); // 是 PNG 字节
    }

    #[test]
    fn grab_error_is_typed() {
        let mut g = InMemGrabber::new(vec![]);
        assert!(matches!(g.grab_all(), Err(CaptureError::NoDisplays)));
    }
}
