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
    fn out_of_bounds_returns_error() {
        let e = crop_png(&solid_png(4, 4, [0, 0, 0, 255]), &Rect { x: 3, y: 3, w: 5, h: 5 });
        assert!(matches!(e, Err(CropError::OutOfBounds)));
    }
}
