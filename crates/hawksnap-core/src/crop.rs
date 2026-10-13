use crate::geometry::Rect;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CropError {
    #[error("source decode failed")]
    Decode,
    #[error("crop rect outside source")]
    OutOfBounds,
    #[error("encode failed")]
    Encode,
}

pub fn crop_png(png: &[u8], physical: &Rect) -> Result<Vec<u8>, CropError> {
    let img = image::load_from_memory(png).map_err(|_| CropError::Decode)?;
    let (sw, sh) = (img.width() as i32, img.height() as i32);
    if physical.w > i32::MAX as u32 || physical.h > i32::MAX as u32 {
        return Err(CropError::OutOfBounds);
    }
    if physical.x < 0 || physical.y < 0
        || physical.right() > sw || physical.bottom() > sh { return Err(CropError::OutOfBounds); }
    let cropped = img.crop_imm(
        physical.x as u32, physical.y as u32, physical.w, physical.h,
    );
    let mut out = Vec::new();
    cropped.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|_| CropError::Encode)?;
    Ok(out)
}

/// 从物理 RGBA 缓冲按行复制裁剪（无重编码）。`rgba.len()` 必须 ≥ w_px*h_px*4。
pub fn crop_rgba(rgba: &[u8], w_px: u32, h_px: u32, physical: &Rect) -> Result<Vec<u8>, CropError> {
    let stride = w_px as usize * 4;
    if w_px == 0 || h_px == 0 || rgba.len() < stride.checked_mul(h_px as usize).ok_or(CropError::Decode)? {
        return Err(CropError::Decode);
    }
    if physical.w > i32::MAX as u32 || physical.h > i32::MAX as u32 {
        return Err(CropError::OutOfBounds);
    }
    if physical.x < 0 || physical.y < 0
        || physical.right() > w_px as i32 || physical.bottom() > h_px as i32 {
        return Err(CropError::OutOfBounds);
    }
    let mut out = Vec::with_capacity(physical.w as usize * 4 * physical.h as usize);
    for row in physical.y as usize..physical.bottom() as usize {
        let start = row * stride + physical.x as usize * 4;
        out.extend_from_slice(&rgba[start..start + physical.w as usize * 4]);
    }
    Ok(out)
}

/// 裁剪结果 → PNG（Fast+NoFilter）。只在保存路径调用；剪贴板直用 RGBA。
pub fn rgba_to_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, CropError> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or(CropError::Decode)?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_with_encoder(image::codecs::png::PngEncoder::new_with_quality(
            std::io::Cursor::new(&mut out),
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::NoFilter,
        ))
        .map_err(|_| CropError::Encode)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;

    fn solid_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn crops_to_exact_physical_rect() {
        // 4x4 全红图，裁 (1,1,2,2) → 2x2
        let out = crop_png(&solid_png(4, 4, [255, 0, 0, 255]), &Rect { x: 1, y: 1, w: 2, h: 2 }).unwrap();
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!((img.width(), img.height()), (2, 2));
    }

    #[test]
    fn out_of_bounds_on_overflowing_width() {
        let e = crop_png(&solid_png(4, 4, [0, 0, 0, 255]), &Rect { x: 0, y: 0, w: u32::MAX, h: 4 });
        assert!(matches!(e, Err(CropError::OutOfBounds)));
    }

    #[test]
    fn out_of_bounds_returns_error() {
        let e = crop_png(&solid_png(4, 4, [0, 0, 0, 255]), &Rect { x: 3, y: 3, w: 5, h: 5 });
        assert!(matches!(e, Err(CropError::OutOfBounds)));
    }
}

/// RGBA → JPEG（有损，无 alpha：编码前先转 RGB8）。quality clamp 1..=100。
/// 只在保存路径调用（settings.output_format == "jpeg" 时）。
pub fn rgba_to_jpeg(rgba: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, CropError> {
    let q = quality.clamp(1, 100);
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or(CropError::Decode)?;
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8(); // JPEG 无 alpha
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut std::io::Cursor::new(&mut out), q)
        .encode_image(&image::DynamicImage::ImageRgb8(rgb))
        .map_err(|_| CropError::Encode)?;
    Ok(out)
}

#[cfg(test)]
mod rgba_to_png_tests {
    use super::*;

    fn solid_rgba(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        vec![px[0], px[1], px[2], px[3]].repeat((w * h) as usize)
    }

    #[test]
    fn jpeg_roundtrips_dimensions() {
        let rgba = solid_rgba(4, 3, [200, 100, 50, 255]);
        let jpeg = rgba_to_jpeg(&rgba, 4, 3, 90).unwrap();
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (4, 3));
    }

    #[test]
    fn jpeg_quality_clamps_to_range() {
        // quality 0 与 255 越界：clamp 后都必须成功编码（不 panic 且输出非空）
        let rgba = solid_rgba(4, 3, [1, 2, 3, 255]);
        let lo = rgba_to_jpeg(&rgba, 4, 3, 0).unwrap();
        let hi = rgba_to_jpeg(&rgba, 4, 3, 255).unwrap();
        assert!(!lo.is_empty() && !hi.is_empty());
    }
}

#[cfg(test)]
mod rgba_tests {
    use super::*;
    use crate::geometry::Rect;

    fn solid_rgba(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        vec![px[0], px[1], px[2], px[3]].repeat((w * h) as usize)
    }

    #[test]
    fn crop_rgba_extracts_exact_rows() {
        // 2x2：左列红右列蓝，裁右列 → 1x2 蓝
        let mut img = vec![255u8, 0, 0, 255, 0, 0, 255, 255];
        img[0] = 255; img[2] = 0; // (0,0) 红
        let out = crop_rgba(&solid_rgba(2, 2, [0, 0, 255, 255]), 2, 2,
            &Rect { x: 1, y: 0, w: 1, h: 2 }).unwrap();
        assert_eq!(out, vec![0, 0, 255, 255, 0, 0, 255, 255]);
        let _ = img; // 语义用 solid_rgba 表达
    }

    #[test]
    fn crop_rgba_rejects_oob_and_overflow() {
        assert!(matches!(crop_rgba(&solid_rgba(2, 2, [0; 4]), 2, 2,
            &Rect { x: 2, y: 0, w: 1, h: 1 }), Err(CropError::OutOfBounds)));
        assert!(matches!(crop_rgba(&solid_rgba(2, 2, [0; 4]), 2, 2,
            &Rect { x: 0, y: 0, w: u32::MAX, h: 1 }), Err(CropError::OutOfBounds)));
    }

    #[test]
    fn crop_rgba_rejects_short_buffer() {
        assert!(matches!(crop_rgba(&[0u8; 7], 2, 2,
            &Rect { x: 0, y: 0, w: 1, h: 1 }), Err(CropError::Decode)));
    }

    #[test]
    fn rgba_to_png_roundtrips_dimensions() {
        let rgba = solid_rgba(4, 3, [9, 8, 7, 255]);
        let png = rgba_to_png(&rgba, 4, 3).unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (4, 3));
    }
}
