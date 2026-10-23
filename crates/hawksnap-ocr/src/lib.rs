//! hawksnap-ocr：RapidOCR（PP-OCRv4 ONNX）本地推理 + 模型管理（M2 Task M4）。
//!
//! 布局：models.rs（下载器/SHA256 门/ORT dylib）· preprocess.rs（PP-OCR 预处理）·
//! det.rs（DBNet 后处理，M2 轴对齐简化）· rec.rs（CTC 解码/标签表）·
//! pipeline.rs（ORT 三会话串联）。app crate 仅经 pipeline::OcrEngine 与
//! models::ensure_models 两个入口使用本 crate。

pub mod det;
pub mod models;
pub mod pipeline;
pub mod preprocess;
pub mod rec;

pub use models::{ensure_models, ModelPaths};
pub use pipeline::{OcrEngine, OcrLine, OcrPoint};
