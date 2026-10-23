//! 输入预处理（PP-OCR 规范）：RGBA → 缩放 → NCHW f32 归一化。
//!
//! 两种归一化口径（与 paddleocr 预处理逐字对齐）：
//! - `DET`（DBNet）：ImageNet 统计 mean=[0.485,0.456,0.406] std=[0.229,0.224,0.225]；
//! - `CLS_REC`（分类/识别）：mean=std=0.5（即 (v/255-0.5)/0.5）。

/// 归一化口径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Norm {
    Det,
    ClsRec,
}

const DET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const DET_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// RGBA8 缓冲 → `image::RgbaImage`（owner 拷贝）。
pub fn rgba_to_image(rgba: &[u8], w: u32, h: u32) -> Result<image::RgbaImage, String> {
    let expect = w as usize * h as usize * 4;
    if rgba.len() != expect {
        return Err(format!("RGBA 尺寸不匹配: 期望 {expect} 字节，实得 {}", rgba.len()));
    }
    image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or_else(|| "图像缓冲无法构成图像".into())
}

/// 双线性缩放（image crate Triangle 滤波 ≈ cv2.INTER_LINEAR）。
pub fn resize(img: &image::RgbaImage, w: u32, h: u32) -> image::RgbaImage {
    image::imageops::resize(img, w.max(1), h.max(1), image::imageops::FilterType::Triangle)
}

/// RGB → NCHW f32（HWC→CHW 逐通道平面 + 归一化）。
pub fn nchw_from_rgba(img: &image::RgbaImage, norm: Norm) -> Vec<f32> {
    let (w, h) = img.dimensions();
    let plane = (w * h) as usize;
    let mut out = vec![0.0f32; plane * 3];
    for (i, px) in img.pixels().enumerate() {
        let [r, g, b, _a] = px.0;
        let (mr, sr, mg, sg, mb, sb) = match norm {
            Norm::Det => (DET_MEAN[0], DET_STD[0], DET_MEAN[1], DET_STD[1], DET_MEAN[2], DET_STD[2]),
            Norm::ClsRec => (0.5, 0.5, 0.5, 0.5, 0.5, 0.5),
        };
        out[i] = (r as f32 / 255.0 - mr) / sr;
        out[plane + i] = (g as f32 / 255.0 - mg) / sg;
        out[plane * 2 + i] = (b as f32 / 255.0 - mb) / sb;
    }
    out
}

/// det 输入尺寸（rapidocr 默认口径 limit_type=min）：短边 < `limit`（736）时等比
/// **放大**到 736（小字号检测召回），否则原样；h/w 各补齐到 32 的倍数（补 0）。
/// 返回 (目标宽, 目标高, 缩放系数)。
pub fn det_input_size(w: u32, h: u32, limit: u32) -> (u32, u32, f32) {
    let min_side = w.min(h);
    let ratio = if min_side < limit { limit as f32 / min_side as f32 } else { 1.0 };
    let rw = ((w as f32 * ratio).round() as u32).max(32);
    let rh = ((h as f32 * ratio).round() as u32).max(32);
    let rw = rw.div_ceil(32) * 32;
    let rh = rh.div_ceil(32) * 32;
    (rw, rh, ratio)
}

/// 识别行裁剪的 rec 输入宽（paddleocr 口径）：高固定 48，宽 = round(48 × 行宽高比)
/// **动态不设 320 上限**（rec 模型宽度动态；320 只是 paddle 批处理初始默认），
/// clamp [16,2048] 并取 8 的倍数。
pub fn rec_input_width(box_w: u32, box_h: u32) -> u32 {
    let ratio = box_w as f32 / box_h.max(1) as f32;
    let w = (48.0 * ratio).round() as i32;
    let w = w.clamp(16, 2048);
    ((w / 8) * 8).max(16) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NCHW 平面顺序：先整幅 R 平面，再 G，再 B（CHW）。
    #[test]
    fn nchw_layout_is_channel_planes() {
        // 2x1 图：左红(255,0,0)，右蓝(0,0,255)。
        let img = image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        let out = nchw_from_rgba(&img, Norm::ClsRec);
        assert_eq!(out.len(), 6);
        let v = |x: f32| (x - 0.5) / 0.5;
        assert_eq!(out[0], v(1.0), "R 平面[0]=红像素");
        assert_eq!(out[1], v(0.0), "R 平面[1]=蓝像素");
        assert_eq!(out[2], v(0.0), "G 平面[0]");
        assert_eq!(out[5], v(1.0), "B 平面[1]=蓝像素");
    }

    /// Det 口径用 ImageNet 统计。
    #[test]
    fn det_norm_uses_imagenet_stats() {
        let img = image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 255]).unwrap();
        let out = nchw_from_rgba(&img, Norm::Det);
        assert!((out[0] - (1.0 - 0.485) / 0.229).abs() < 1e-6);
        assert!((out[1] - (0.0 - 0.456) / 0.224).abs() < 1e-6);
    }

    /// limit_type=min 口径：短边 < limit 时等比放大；≥ limit 原样；补齐 32 倍数。
    #[test]
    fn det_input_size_scales_and_pads() {
        // 1920x1080：短边 1080 ≥ 736 → 原样（仅补齐到 32 倍数）。
        let (w, h, r) = det_input_size(1920, 1080, 736);
        assert_eq!((w, h), (1920, 1088));
        assert!((r - 1.0).abs() < 1e-6);
        // 600x400：短边 400 < 736 → 放大 1.84x。
        let (w, h, r) = det_input_size(600, 400, 736);
        assert_eq!((w, h), (1120, 736), "600*1.84=1104→补齐 1120, 400*1.84=736");
        assert!((r - 736.0 / 400.0).abs() < 1e-6);
        // 小图放大后仍补齐 32 倍数。
        let (w, h, _) = det_input_size(100, 60, 736);
        assert_eq!((w, h), (1248, 736), "100*12.267=1227→1248, 60*12.267=736");
    }

    /// rec 宽度：动态比例（不设 320 上限）+ clamp [16,2048] + 8 倍数。
    #[test]
    fn rec_width_clamps_to_bucket() {
        assert_eq!(rec_input_width(960, 48), 960, "宽行动态宽（不压 320）");
        assert_eq!(rec_input_width(48, 480), 16, "竖条给最小 16");
        assert_eq!(rec_input_width(96, 48), 96, "2:1 行 = 96");
        assert_eq!(rec_input_width(100, 48) % 8, 0);
        assert_eq!(rec_input_width(100000, 48), 2048, "极端宽行钳到 2048");
    }

    /// RGBA 缓冲长度校验。
    #[test]
    fn rgba_length_is_validated() {
        assert!(rgba_to_image(&[0; 3], 1, 1).is_err());
        assert!(rgba_to_image(&[0; 4], 1, 1).is_ok());
    }
}
