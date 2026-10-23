//! OCR 命令（M2 Task M4）：`ocr_recognize(png_path)` 全链——模型/ORT dylib 就绪
//! （首次触发官方 release 下载，进度经 `hawk://ocr-model-progress` 推给前端）→
//! 读 PNG → hawksnap-ocr 三模型推理 → 命中历史行则回填 ocr_text（FTS 同步可检索）
//! → 返回行列表。`ocr_export_txt`：识别结果落 `<png_stem>.txt`（与 editor_save
//! 同款「写回源文件旁」口径）。
//!
//! 并发契约：模型就绪阶段经进程级互斥（双击/双窗口不会并发下载两份）；引擎会话
//! 惰性装载并常驻（Mutex 串行推理——编辑器单窗口语义下单路使用，无吞吐诉求）。
//! 下载/推理均为 `#[tauri::command(async)]`（命令线程执行，不卡事件循环）。

use hawksnap_ocr::models::ProgressPhase;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// 模型目录：`app_data_dir()/models`（与 history.db 同根，用户可整目录清理）。
fn models_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法定位应用数据目录: {e}"))?
        .join("models"))
}

/// `hawk://ocr-model-progress` 载荷（camelCase 对齐前端 `e.payload.downloaded/total`）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OcrModelProgress {
    /// "download" | "verify"（对齐前端 i18n 文案分支）。
    phase: &'static str,
    downloaded: u64,
    total: u64,
}

/// 模型就绪互斥：并发首次调用串行化（后者等待前者下载完成，ensure_models 幂等短路）。
static MODELS_READY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// 常驻引擎（惰性装载）。Mutex<Option<…>>：装载失败不留半成品，下次重试。
static ENGINE: std::sync::OnceLock<std::sync::Mutex<Option<hawksnap_ocr::OcrEngine>>> =
    std::sync::OnceLock::new();

fn engine_slot() -> &'static std::sync::Mutex<Option<hawksnap_ocr::OcrEngine>> {
    ENGINE.get_or_init(|| std::sync::Mutex::new(None))
}

/// 确保模型就绪（带进度事件 + 并发互斥）。
fn ensure_models_with_progress(app: &AppHandle) -> Result<hawksnap_ocr::ModelPaths, String> {
    let _guard = MODELS_READY_LOCK.lock().map_err(|e| e.to_string())?;
    let dir = models_dir(app)?;
    let emitter = app.clone();
    let mut on_progress = |p: hawksnap_ocr::models::Progress| {
        let phase = match p.phase {
            ProgressPhase::Download => "download",
            ProgressPhase::Verify => "verify",
        };
        let _ = emitter.emit(
            "hawk://ocr-model-progress",
            OcrModelProgress { phase, downloaded: p.downloaded, total: p.total },
        );
    };
    hawksnap_ocr::ensure_models(&dir, &mut on_progress)
}

/// 取就绪引擎（缓存命中直接用；未装载则装载后常驻）。
fn with_engine<T>(
    app: &AppHandle,
    f: impl FnOnce(&mut hawksnap_ocr::OcrEngine) -> Result<T, String>,
) -> Result<T, String> {
    let paths = ensure_models_with_progress(app)?;
    let mut slot = engine_slot().lock().map_err(|e| e.to_string())?;
    if slot.is_none() {
        *slot = Some(hawksnap_ocr::OcrEngine::load(&paths)?);
    }
    f(slot.as_mut().expect("引擎已装载"))
}

/// OCR 识别行（hawksnap-ocr::OcrLine 的转发别名——serde 形状在 pipeline.rs 定义：
/// `box: [{x,y}×4]`（TL→TR→BR→BL 原图坐标）+ text + score，camelCase）。
pub use hawksnap_ocr::OcrLine as OcrLineDto;

/// 识别指定 PNG（M4 计划接口）。路径校验（存在 + 可解码）→ 模型/引擎就绪 → 推理
/// → 历史回填（旁路降级：回填失败不影响识别结果返回）。
#[tauri::command(async)]
pub fn ocr_recognize(app: AppHandle, path: String) -> Result<Vec<OcrLineDto>, String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    let png = std::fs::read(&path).map_err(|e| format!("读取截图文件失败: {e}"))?;
    let (rgba, w, h) =
        hawksnap_core::crop::png_to_rgba(&png).map_err(|e| format!("截图文件解码失败: {e}"))?;
    let lines = with_engine(&app, |engine| engine.recognize(&rgba, w, h))?;
    // 历史回填：识别文本拼接（行序）。失败仅记录——识别结果仍返回前端。
    if !lines.is_empty() {
        let text = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        if let Err(e) = crate::history::set_ocr_text(&app, &path, &text) {
            eprintln!("[hawksnap] OCR 文本回填历史失败（忽略）: {e}");
        }
    }
    Ok(lines)
}

/// 识别结果导出 txt：写 `<png_stem>.txt` 到源图旁（M4：复用 editor_save 的
/// 「写回源文件所在目录」口径）。源必须是已存在的 .png（防任意路径写入）。
#[tauri::command(async)]
pub fn ocr_export_txt(path: String, text: String) -> Result<String, String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    let p = std::path::Path::new(&path);
    let is_png = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png {
        return Err("OCR 导出仅支持 PNG 源图".into());
    }
    if !p.is_file() {
        return Err(format!("源图不存在: {path}"));
    }
    let txt_path = p.with_extension("txt");
    std::fs::write(&txt_path, text.as_bytes()).map_err(|e| format!("写入文本文件失败: {e}"))?;
    Ok(txt_path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod ocr_tests {
    use super::*;

    /// ocr_export_txt：路径校验（仅 .png / 必须存在）+ 落点 = 同目录同名 .txt。
    #[test]
    fn export_txt_writes_beside_source() {
        let dir = std::env::temp_dir().join(format!("hawksnap-ocr-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = hawksnap_core::crop::rgba_to_png(&[1, 2, 3, 255], 1, 1).unwrap();
        let src = dir.join("shot.png");
        std::fs::write(&src, &png).unwrap();
        let out = ocr_export_txt(src.to_string_lossy().into_owned(), "第一行\n第二行".into()).unwrap();
        assert_eq!(out, dir.join("shot.txt").to_string_lossy().into_owned());
        assert_eq!(std::fs::read(&out).unwrap(), "第一行\n第二行".as_bytes());
        // 非 png 扩展拒绝。
        let txt = dir.join("shot.txt");
        std::fs::write(&txt, b"x").unwrap();
        assert!(ocr_export_txt(txt.to_string_lossy().into_owned(), "y".into()).is_err());
        // 不存在的源拒绝。
        assert!(ocr_export_txt(dir.join("gone.png").to_string_lossy().into_owned(), "y".into()).is_err());
        // 空路径拒绝。
        assert_eq!(ocr_export_txt("  ".into(), "y".into()).unwrap_err(), "路径为空");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 进度载荷 serde 形状契约（前端按 e.payload.phase/downloaded/total 消费）。
    #[test]
    fn progress_payload_serializes_camel_case() {
        let p = OcrModelProgress { phase: "download", downloaded: 1024, total: 4096 };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, r#"{"phase":"download","downloaded":1024,"total":4096}"#);
    }

    /// OcrLine（DTO）serde 形状：box 字段名（Rust 关键字经 rename）+ camelCase。
    #[test]
    fn ocr_line_serializes_box_field() {
        let line = hawksnap_ocr::OcrLine {
            points: [
                hawksnap_ocr::OcrPoint { x: 1.0, y: 2.0 },
                hawksnap_ocr::OcrPoint { x: 3.0, y: 2.0 },
                hawksnap_ocr::OcrPoint { x: 3.0, y: 4.0 },
                hawksnap_ocr::OcrPoint { x: 1.0, y: 4.0 },
            ],
            text: "你好".into(),
            score: 0.98,
        };
        let s = serde_json::to_string(&line).unwrap();
        assert!(s.contains(r#""box":[{"x":1.0,"y":2.0}"#), "box 4 点 + 对象形状: {s}");
        assert!(s.contains(r#""text":"你好""#));
        assert!(s.contains(r#""score":0.98"#));
    }
}
