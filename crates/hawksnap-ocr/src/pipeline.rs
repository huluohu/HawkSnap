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

impl OcrEngine {
    /// 装载：init_from ORT dylib（load-dynamic）→ 三会话 + 字典。
    pub fn load(paths: &ModelPaths) -> Result<Self, String> {
        let env = ort::init_from(&paths.ort_dylib)
            .map_err(|e| format!("ORT dylib 装载失败: {e}"))?
            .with_name("hawksnap-ocr")
            .build()
            .map_err(|e| format!("ORT 环境创建失败: {e}"))?;
        Self::with_env(&env, paths)
    }

    /// 在给定 ORT 环境下装载（测试可复用环境）。
    pub fn with_env(env: &ort::environment::Environment, paths: &ModelPaths) -> Result<Self, String> {
        let load = |p: &std::path::Path| -> Result<ort::session::Session, String> {
            let mut b = ort::session::Session::builder(env)
                .map_err(|e| format!("会话构造失败: {e}"))?;
            b.commit_from_file(p).map_err(|e| format!("装载模型 {} 失败: {e}", p.display()))
        };
        let det = load(&paths.det)?;
        let cls = load(&paths.cls)?;
        let rec_s = load(&paths.rec)?;
        let dict = std::fs::read_to_string(&paths.dict)
            .map_err(|e| format!("读取字典失败: {e}"))?;
        let labels = rec::build_labels(
            dict.lines().map(|s| s.to_string()).collect(),
            6625, // v4 rec mobile 输出类别（pipeline 内省兜底见 recognize）
        );
        Ok(OcrEngine { det, cls, rec: rec_s, labels })
    }

    /// 全链识别：RGBA → 行列表（检测序，行序排序）。失败 Err（中文错误文案）。
    pub fn recognize(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<Vec<OcrLine>, String> {
        let img = preprocess::rgba_to_image(rgba, w, h)?;
        let boxes = self.detect(&img)?;
        #[cfg(feature = "ocr-debug")]
        for b in &boxes {
            eprintln!("[ocr-debug] box ({:.0},{:.0})-({:.0},{:.0}) score {:.3}", b.pts[0].0, b.pts[0].1, b.pts[2].0, b.pts[2].1, b.score);
        }
        let mut out = Vec::with_capacity(boxes.len());
        for b in &boxes {
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

    /// det：预处理 → 推理 → 概率图后处理（回原图坐标）。
    fn detect(&mut self, img: &image::RgbaImage) -> Result<Vec<TextBox>, String> {
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
            .map_err(|e| format!("det 输入构造失败: {e}"))?;
        let outputs = self
            .det
            .run(ort::inputs![tensor])
            .map_err(|e| format!("det 推理失败: {e}"))?;
        let (_, prob_view) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("det 输出解析失败: {e}"))?;
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
        .map_err(|e| format!("cls 输入形状失败: {e}"))?;
        let tensor = ort::value::Tensor::from_array(input)
            .map_err(|e| format!("cls 输入构造失败: {e}"))?;
        let outputs = self
            .cls
            .run(ort::inputs![tensor])
            .map_err(|e| format!("cls 推理失败: {e}"))?;
        let (_, cls_view) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("cls 输出解析失败: {e}"))?;
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
        .map_err(|e| format!("rec 输入形状失败: {e}"))?;
        let tensor = ort::value::Tensor::from_array(input)
            .map_err(|e| format!("rec 输入构造失败: {e}"))?;
        let outputs = self
            .rec
            .run(ort::inputs![tensor])
            .map_err(|e| format!("rec 推理失败: {e}"))?;
        let (_, rec_view) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("rec 输出解析失败: {e}"))?;
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
