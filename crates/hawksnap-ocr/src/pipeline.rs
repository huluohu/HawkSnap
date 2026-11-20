//! 推理流水线（M2 Task M4）：det(DBNet) → cls(0/180°) → rec(CRNN+CTC) 三模型串行。
//!
//! 口径（paddleocr 对齐）：det 限长 960 + 32 补边 + ImageNet 归一化；cls 48×192、
//! 0.9 阈值翻转；rec 高 48、宽按比例 clamp[16,320]（8 倍数）、0.5/0.5 归一化。
//! det 后处理的 M2 简化（轴对齐框）见 det.rs 模块注。

use crate::det::{self, TextBox};
use crate::models::ModelPaths;
use crate::preprocess::{self, Norm};
use crate::rec;

/// det 限长（rapidocr 默认：limit_type=min，短边不足时放大到该值）。
pub const DET_LIMIT: u32 = 736;
/// cls 翻转阈值（paddleocr cls_thresh 默认 0.9）。
pub const CLS_THRESH: f32 = 0.9;
/// cls 启用门槛（M2 裁定，实测校准）：宽高比 ≥ 3 的横条（截图正文常态）跳过
/// 0/180 校验——极宽裁剪被压扁到 48×192 后分类器失真，会误翻转（实测 24 行中
/// 4 行被误转 → 空解码）；截图场景不存在整幅倒置文本，纵向/近方形裁剪（比例
/// < 3）保留校验。裁定记录于 task-4-report.md。
pub const CLS_MAX_ASPECT: f32 = 3.0;

/// 识别结果行（box 为原图坐标 TL→TR→BR→BL；serde camelCase 对齐前端契约）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct OcrLine {
    #[serde(rename = "box")]
    pub points: [OcrPoint; 4],
    pub text: String,
    pub score: f32,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct OcrPoint {
    pub x: f32,
    pub y: f32,
}

impl OcrLine {
    fn from_box(b: &TextBox, text: String, score: f32) -> Self {
        OcrLine {
            points: b.pts.map(|(x, y)| OcrPoint { x, y }),
            text,
            score,
        }
    }
}

/// 就绪的 OCR 引擎（三会话 + 标签表）。会话运行需 `&mut`（ort 约束）。
pub struct OcrEngine {
    det: ort::session::Session,
    cls: ort::session::Session,
    rec: ort::session::Session,
    labels: Vec<String>,
}

/// OCR 推理链错误 → 哨兵码（`<hserr:ocr_{stage}>|detail=…`，词典 err.ocr_{stage}
/// 双语渲染；2026-11-20 错误码扫尾——此前内嵌中文，en 用户在编辑器 OCR 横幅看到
/// 中文）。stage 字面量保留在调用点，便于 grep 对照。
fn ocr_err(stage: &str, e: impl std::fmt::Display) -> String {
    hawksnap_core::errcode::coded_kv(&format!("ocr_{stage}"), &[("detail", &e.to_string())])
}

impl OcrEngine {
    /// 装载：init_from ORT dylib（load-dynamic）→ 三会话 + 字典。
    pub fn load(paths: &ModelPaths) -> Result<Self, String> {
        let env = ort::init_from(&paths.ort_dylib)
            .map_err(|e| ocr_err("dylib_load", e))?
            .with_name("hawksnap-ocr")
            .build()
            .map_err(|e| ocr_err("env_build", e))?;
        Self::with_env(&env, paths)
    }

    /// 在给定 ORT 环境下装载（测试可复用环境）。
    pub fn with_env(env: &ort::environment::Environment, paths: &ModelPaths) -> Result<Self, String> {
        let load = |p: &std::path::Path| -> Result<ort::session::Session, String> {
            let mut b = ort::session::Session::builder(env)
                .map_err(|e| ocr_err("session_build", e))?;
            b.commit_from_file(p)
                .map_err(|e| ocr_err("model_load", format!("{}: {e}", p.display())))
        };
        let det = load(&paths.det)?;
        let cls = load(&paths.cls)?;
        let rec_s = load(&paths.rec)?;
        let dict = std::fs::read_to_string(&paths.dict)
            .map_err(|e| ocr_err("dict_read", e))?;
        let labels = rec::build_labels(
            dict.lines().map(|s| s.to_string()).collect(),
            6625, // v4 rec mobile 输出类别（pipeline 内省兜底见 recognize）
        );
        Ok(OcrEngine { det, cls, rec: rec_s, labels })
    }

    /// 全链识别：RGBA → 行列表（检测序，行序排序）。box = **原图**坐标（前端画布
    /// 高亮契约）。失败 Err（码化文案，见 ocr_err）。
    pub fn recognize(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<Vec<OcrLine>, String> {
        let img = preprocess::rgba_to_image(rgba, w, h)?;
        // 大图分块检测（R-F2，2026-11-20 性能审计 + 质量复核）：
        // det 输入张量按**整图**线性膨胀——编辑器合法上限（8192²）导出 → f32 张量
        // + nchw 各 ~800MB（瞬时 ~1.9GB，有 OOM 面）。
        //
        // **为什么是分块而不是整图降采样**（2026-11-20 自检复核拍板）：整图缩到
        // 2400 会把 8192² 图里的 20px 小字压成 6px → det 大概率漏检——那是用
        // 内存换质量的**功能回归**。分块（paddleocr 大图模式的通行做法）两块兼得：
        // 每块 ≤2400² → 张量 ≈ 69MB（有界），且每块按**原尺寸**推理（与中等图
        // 同口径）→ 文字尺度不缩、质量不降，框按块偏移即回原图坐标。
        let boxes = self.detect(&img)?;
        #[cfg(feature = "ocr-debug")]
        for b in &boxes {
            eprintln!("[ocr-debug] box ({:.0},{:.0})-({:.0},{:.0}) score {:.3}", b.pts[0].0, b.pts[0].1, b.pts[2].0, b.pts[2].1, b.score);
        }
        let mut out = Vec::with_capacity(boxes.len());
        for b in &boxes {
            // 框已是**原图坐标** → 直接在原图上裁剪（cls/rec 输入与画布高亮同源）。
            let mut crop = crop_box(&img, b);
            {
                let (cw, ch) = crop.dimensions();
                if ch as f32 * CLS_MAX_ASPECT > cw as f32 {
                    if let Some(rotated) = self.maybe_rotate_180(&crop)? {
                        crop = rotated;
                    }
                }
            }
            let (text, score) = self.recognize_line(&crop)?;
            if text.trim().is_empty() {
                continue;
            }
            out.push(OcrLine::from_box(b, text, score));
        }
        Ok(out)
    }

    /// det：大图分块 → 逐块检测 → 框偏移回原图 → 重叠区去重。
    ///
    /// ≤ 一块时走单块路径（**与旧实现逐位等价**，常规截图零行为变化）。
    fn detect(&mut self, img: &image::RgbaImage) -> Result<Vec<TextBox>, String> {
        let (w, h) = img.dimensions();
        if detect_tiles(w, h).len() <= 1 {
            return self.detect_one(img);
        }
        let mut err: Option<String> = None;
        // 分块骨架（可注入检测器 → 单测覆盖坐标链路）。
        let boxes = detect_with(img, |tile, _tx, _ty| match self.detect_one(tile) {
            Ok(v) => v,
            Err(e) => {
                err = Some(e);
                Vec::new()
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
        Ok(boxes)
    }

    /// 单块 det：预处理 → 推理 → 概率图后处理（回**块内**原尺寸坐标）。
    fn detect_one(&mut self, img: &image::RgbaImage) -> Result<Vec<TextBox>, String> {
        let (w, h) = img.dimensions();
        let (iw, ih, ratio) = preprocess::det_input_size(w, h, DET_LIMIT);
        let rw = ((w as f32 * ratio).round() as u32).max(1).min(iw);
        let rh = ((h as f32 * ratio).round() as u32).max(1).min(ih);
        let scaled = preprocess::resize(img, rw, rh);
        let mut input = ndarray::Array4::<f32>::zeros((1, 3, ih as usize, iw as usize));
        let nchw = preprocess::nchw_from_rgba(&scaled, Norm::Det);
        let plane = (rw * rh) as usize;
        for c in 0..3usize {
            for y in 0..rh as usize {
                let src = &nchw[c * plane + y * rw as usize..c * plane + (y + 1) * rw as usize];
                for (x, &v) in src.iter().enumerate() {
                    input[[0, c, y, x]] = v;
                }
            }
        }
        let tensor = ort::value::Tensor::from_array(input)
            .map_err(|e| ocr_err("det_input", e))?;
        let outputs = self
            .det
            .run(ort::inputs![tensor])
            .map_err(|e| ocr_err("det_infer", e))?;
        let (_, prob_view) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ocr_err("det_parse", e))?;
        let prob: Vec<f32> = prob_view.to_vec();
        Ok(det::boxes_from_probmap(
            &prob,
            iw,
            ih,
            1.0 / ratio,
            1.0 / ratio,
            det::DB_THRESH,
            det::BOX_THRESH,
            det::UNCLIP_RATIO,
        ))
    }

    /// cls：48×192 分类，`180°` 概率 > 0.9 → 返回旋转 180° 后的裁剪图。
    fn maybe_rotate_180(
        &mut self,
        crop: &image::RgbaImage,
    ) -> Result<Option<image::RgbaImage>, String> {
        let scaled = preprocess::resize(crop, 192, 48);
        let input = ndarray::Array4::from_shape_vec(
            (1, 3, 48, 192),
            preprocess::nchw_from_rgba(&scaled, Norm::ClsRec),
        )
        .map_err(|e| ocr_err("cls_shape", e))?;
        let tensor = ort::value::Tensor::from_array(input)
            .map_err(|e| ocr_err("cls_input", e))?;
        let outputs = self
            .cls
            .run(ort::inputs![tensor])
            .map_err(|e| ocr_err("cls_infer", e))?;
        let (_, cls_view) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ocr_err("cls_parse", e))?;
        let rot = cls_view.get(1).copied().unwrap_or(0.0);
        if rot > CLS_THRESH {
            Ok(Some(image::imageops::rotate180(crop)))
        }
        else {
            Ok(None)
        }
    }

    /// rec：高 48 等比 → CRNN → CTC 解码（文本 + 置信度）。
    fn recognize_line(&mut self, crop: &image::RgbaImage) -> Result<(String, f32), String> {
        let (bw, bh) = crop.dimensions();
        let tw = preprocess::rec_input_width(bw, bh);
        let scaled = preprocess::resize(crop, tw, 48);
        let input = ndarray::Array4::from_shape_vec(
            (1, 3, 48, tw as usize),
            preprocess::nchw_from_rgba(&scaled, Norm::ClsRec),
        )
        .map_err(|e| ocr_err("rec_shape", e))?;
        let tensor = ort::value::Tensor::from_array(input)
            .map_err(|e| ocr_err("rec_input", e))?;
        let outputs = self
            .rec
            .run(ort::inputs![tensor])
            .map_err(|e| ocr_err("rec_infer", e))?;
        let (_, rec_view) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ocr_err("rec_parse", e))?;
        let flat: Vec<f32> = rec_view.to_vec();
        // [1, T, C]：T 从 logits 形状取（动态宽），C 取标签表长度（6625）。
        let classes = self.labels.len();
        let frames = flat.len() / classes;
        Ok(rec::ctc_greedy_decode(&flat, frames, classes, &self.labels))
    }
}

/// 轴对齐裁剪（clamp 到图内；M2 det 简化配套）。
fn crop_box(img: &image::RgbaImage, b: &TextBox) -> image::RgbaImage {
    let (iw, ih) = img.dimensions();
    let (tl, br) = (b.pts[0], b.pts[2]);
    let x0 = (tl.0.round() as i32).clamp(0, iw as i32 - 1) as u32;
    let y0 = (tl.1.round() as i32).clamp(0, ih as i32 - 1) as u32;
    let x1 = (br.0.round() as i32).clamp(0, iw as i32) as u32;
    let y1 = (br.1.round() as i32).clamp(0, ih as i32) as u32;
    image::imageops::crop_imm(img, x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)).to_image()
}

/// det 分块边长（像素）：大图按此切块检测。
///
/// **内存有界**：块 2400² 的 f32 张量 ≈ 69MB（+nchw 同量级），而非整图 8192² 的
/// ≈ 806MB。**质量不降**：每块按**原尺寸**推理（与中等图同口径）——文字尺度不缩。
pub const DET_TILE: u32 = 2400;
/// 相邻块重叠边长：避免文字正好被块边界切断（边界字会半检/漏检）。
pub const DET_TILE_OVERLAP: u32 = 160;

/// 分块矩形（纯函数）：覆盖 w×h，步长 = TILE - OVERLAP；边缘块按剩余尺寸裁短。
/// 返回 (x, y, w, h) 列表。
pub fn detect_tiles(w: u32, h: u32) -> Vec<(u32, u32, u32, u32)> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for &y in &tile_starts(h) {
        for &x in &tile_starts(w) {
            out.push((x, y, DET_TILE.min(w - x), DET_TILE.min(h - y)));
        }
    }
    out
}

/// 分块检测骨架（纯逻辑，**可注入检测器** → 单测覆盖坐标链路，无需模型）：
/// 逐块切图 → 交给检测器（返回**块内**坐标的框）→ 偏移到原图 → 重叠区去重。
fn detect_with<F>(img: &image::RgbaImage, mut per_tile: F) -> Vec<TextBox>
where
    F: FnMut(&image::RgbaImage, u32, u32) -> Vec<TextBox>,
{
    let (w, h) = img.dimensions();
    let mut all: Vec<TextBox> = Vec::new();
    for (tx, ty, tw, th) in detect_tiles(w, h) {
        let tile = image::imageops::crop_imm(img, tx, ty, tw, th).to_image();
        let mut boxes = per_tile(&tile, tx, ty);
        // 块内坐标 → 原图坐标（偏移即得，无需缩放因子）。
        for b in &mut boxes {
            for p in &mut b.pts {
                p.0 += tx as f32;
                p.1 += ty as f32;
            }
        }
        all.extend(boxes);
    }
    dedup_boxes(all)
}

/// 单轴分块起点（纯函数）：0, step, 2*step… 直到覆盖到末端。
fn tile_starts(extent: u32) -> Vec<u32> {
    let mut starts = vec![0u32];
    if extent <= DET_TILE {
        return starts;
    }
    let step = DET_TILE.saturating_sub(DET_TILE_OVERLAP).max(1);
    let mut x = 0u32;
    while x + DET_TILE < extent {
        x += step;
        starts.push(x);
    }
    starts
}

/// 跨块重复框去重（纯函数）：重叠区里的同一文字会被相邻两块各检一次。
/// 按 score 降序保留，IoU > 0.3 视为同一处。
pub fn dedup_boxes(mut boxes: Vec<TextBox>) -> Vec<TextBox> {
    boxes.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<TextBox> = Vec::with_capacity(boxes.len());
    for b in boxes {
        if kept.iter().any(|k| box_iou(k, &b) > 0.3) {
            continue;
        }
        kept.push(b);
    }
    kept
}

/// 轴对齐外接矩形的 IoU（纯函数）：det 框为轴对齐（det.rs 口径）。
fn box_iou(a: &TextBox, b: &TextBox) -> f32 {
    let rect = |t: &TextBox| {
        let xs = [t.pts[0].0, t.pts[1].0, t.pts[2].0, t.pts[3].0];
        let ys = [t.pts[0].1, t.pts[1].1, t.pts[2].1, t.pts[3].1];
        (
            xs.iter().cloned().fold(f32::INFINITY, f32::min),
            ys.iter().cloned().fold(f32::INFINITY, f32::min),
            xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
            ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
        )
    };
    let (ax0, ay0, ax1, ay1) = rect(a);
    let (bx0, by0, bx1, by1) = rect(b);
    let ix = (ax1.min(bx1) - ax0.max(bx0)).max(0.0);
    let iy = (ay1.min(by1) - ay0.max(by0)).max(0.0);
    let inter = ix * iy;
    if inter <= 0.0 {
        return 0.0;
    }
    let area_a = ((ax1 - ax0).max(0.0)) * ((ay1 - ay0).max(0.0));
    let area_b = ((bx1 - bx0).max(0.0)) * ((by1 - by0).max(0.0));
    let union = area_a + area_b - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_at(x: f32, y: f32, w: f32, h: f32, score: f32) -> TextBox {
        TextBox {
            pts: [
                (x, y),
                (x + w, y),
                (x + w, y + h),
                (x, y + h),
            ],
            score,
        }
    }

    #[test]
    fn small_image_is_a_single_tile() {
        // ≤ 块边长 → 单块（常规截图路径逐位等价于旧实现）。
        let tiles = detect_tiles(800, 600);
        assert_eq!(tiles, vec![(0, 0, 800, 600)]);
    }

    #[test]
    fn large_image_tiles_cover_everything_and_overlap() {
        let (w, h) = (8192u32, 4000u32);
        let tiles = detect_tiles(w, h);
        assert!(tiles.len() > 1, "大图必须切块");
        // 覆盖性：任一像素都落在某块内（用块起点网格粗验：每块起点 ≤ 维度和
        // 块宽能覆盖到末端）。
        for &(x, y, tw, th) in &tiles {
            assert!(x + tw <= w && y + th <= h, "块越界: {x},{y},{tw},{th}");
        }
        // 末块必须贴到末端。
        let max_x = tiles.iter().map(|t| t.0 + t.2).max().unwrap();
        let max_y = tiles.iter().map(|t| t.1 + t.3).max().unwrap();
        assert_eq!(max_x, w, "横向必须覆盖到右缘");
        assert_eq!(max_y, h, "纵向必须覆盖到下缘");
        // 重叠：相邻起点间距 = TILE - OVERLAP < TILE → 有重叠带。
        let mut xs: Vec<u32> = tiles.iter().map(|t| t.0).collect();
        xs.sort_unstable();
        xs.dedup();
        if xs.len() > 1 {
            assert!(xs[1] - xs[0] < DET_TILE, "相邻块必须有重叠");
        }
    }

    #[test]
    fn dedup_removes_overlapping_duplicates_and_keeps_distinct() {
        let a = box_at(100.0, 100.0, 200.0, 40.0, 0.9);
        let dup = box_at(102.0, 101.0, 200.0, 40.0, 0.8); // 高度重叠
        let far = box_at(1000.0, 1000.0, 100.0, 30.0, 0.7); // 远处独立
        let out = dedup_boxes(vec![a, dup, far]);
        assert_eq!(out.len(), 2, "重复框应被合并，独立框保留");
        // 保留的是高分那个。
        assert!(out.iter().any(|b| (b.pts[0].0 - 100.0).abs() < 0.01));
    }

    #[test]
    fn iou_is_zero_for_disjoint_boxes() {
        let a = box_at(0.0, 0.0, 10.0, 10.0, 0.9);
        let b = box_at(100.0, 100.0, 10.0, 10.0, 0.9);
        assert_eq!(box_iou(&a, &b), 0.0);
    }

    #[test]
    fn multi_tile_boxes_land_in_their_own_tile_region() {
        // 大图分块后的**坐标链路**（不依赖模型）：假检测器在每块返回一块内框 →
        // 骨架偏移后，每个框必须落在自己那块的原图区域里。
        let (w, h) = (8192u32, 100u32);
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut seen_tiles: Vec<(u32, u32)> = Vec::new();
        let boxes = detect_with(&img, |_tile, tx, ty| {
            seen_tiles.push((tx, ty));
            // 块内坐标：一个 20×20 的框，落在块内 (10,10)。
            vec![box_at(10.0, 10.0, 20.0, 20.0, 0.9)]
        });
        assert!(seen_tiles.len() > 1, "大图必须切多块");
        assert_eq!(boxes.len(), seen_tiles.len(), "每块一个框");
        // 每个框的原图坐标应 ≈ (tx+10, ty+10)——偏移正确。
        for (i, b) in boxes.iter().enumerate() {
            let (tx, ty) = seen_tiles[i];
            assert!(
                (b.pts[0].0 - (tx as f32 + 10.0)).abs() < 0.01,
                "第 {i} 块 x 偏移错: {:?} vs tx={tx}",
                b.pts[0].0
            );
            assert!(
                (b.pts[0].1 - (ty as f32 + 10.0)).abs() < 0.01,
                "第 {i} 块 y 偏移错: {:?} vs ty={ty}",
                b.pts[0].1
            );
        }
    }

    #[test]
    fn single_tile_path_is_untouched() {
        // 小图 → 一块 → 偏移 0 → 框坐标原样（常规截图零行为变化）。
        let img = image::RgbaImage::from_pixel(640, 480, image::Rgba([1, 2, 3, 255]));
        let boxes = detect_with(&img, |_t, _tx, _ty| {
            vec![box_at(5.0, 7.0, 30.0, 10.0, 0.95)]
        });
        assert_eq!(boxes.len(), 1);
        assert!((boxes[0].pts[0].0 - 5.0).abs() < 0.01);
        assert!((boxes[0].pts[0].1 - 7.0).abs() < 0.01);
    }
}

