// edition 2021 下 crate 自身名字不在作用域内，别名以支持测试中 `hawksnap_platform_mac::` 路径。
extern crate self as hawksnap_platform_mac;

use hawksnap_core::capture::{CaptureError, ScreenFrame, ScreenGrabber};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};

pub struct XcapGrabber;

impl XcapGrabber {
    pub fn new() -> Self { XcapGrabber }
}

impl Default for XcapGrabber { fn default() -> Self { Self::new() } }

// xcap 0.4.x 的 Monitor 访问器返回 XCapResult<T>（而非 T），此处统一映射为 Backend。
fn backend(e: xcap::XCapError) -> CaptureError {
    CaptureError::Backend(e.to_string())
}

impl ScreenGrabber for XcapGrabber {
    fn grab_all(&mut self) -> Result<Vec<ScreenFrame>, CaptureError> {
        let monitors = xcap::Monitor::all().map_err(|e| CaptureError::Backend(e.to_string()))?;
        if monitors.is_empty() { return Err(CaptureError::NoDisplays); }
        let mut out = Vec::with_capacity(monitors.len());
        for m in monitors {
            let img = m.capture_image().map_err(|e| {
                if e.to_string().to_lowercase().contains("permission") {
                    CaptureError::Permission
                } else { CaptureError::Backend(e.to_string()) }
            })?;
            let mut png = Vec::new();
            // 快速压缩（Fast + NoFilter）：整帧 Retina PNG 编码从数百 ms 降到 ~10ms 级，
            // 满足热键→覆盖层 ≤150ms 预算；产物仍是合法 PNG。
            image::DynamicImage::ImageRgba8(img)
                .write_with_encoder(PngEncoder::new_with_quality(
                    &mut std::io::Cursor::new(&mut png),
                    CompressionType::Fast,
                    FilterType::NoFilter,
                ))
                .map_err(|e| CaptureError::Backend(e.to_string()))?;
            out.push(ScreenFrame {
                display_id: m.id().map_err(backend)? as u64,
                x: m.x().map_err(backend)?,
                y: m.y().map_err(backend)?,
                width: m.width().map_err(backend)?,
                height: m.height().map_err(backend)?,
                scale: m.scale_factor().map_err(backend)? as f64,
                png,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use hawksnap_core::capture::ScreenGrabber;

    #[test]
    #[ignore = "需要 macOS 屏幕录制权限；授权后 cargo test -- --ignored 运行"]
    fn real_grab_produces_decodable_png() {
        let mut g = hawksnap_platform_mac::XcapGrabber::new();
        let frames = g.grab_all().unwrap();
        assert!(!frames.is_empty());
        let f = &frames[0];
        let img = image::load_from_memory(&f.png).unwrap();
        assert_eq!(img.width(), (f.width as f64 * f.scale).round() as u32);
    }
}
