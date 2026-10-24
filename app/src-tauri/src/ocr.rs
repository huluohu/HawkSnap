//! OCR 命令（M2 Task M4）：`ocr_recognize(png_path)` 全链——模型/ORT dylib 就绪
//! （首次触发官方 release 下载，进度经 `hawk://ocr-model-progress` 推给前端）→
//! 读 PNG → hawksnap-ocr 三模型推理 → 命中历史行则回填 ocr_text（FTS 同步可检索）
//! → 返回行列表。`ocr_export_txt`：识别结果落 `<png_stem>.txt`（与 editor_save
//! 同款「写回源文件旁」口径）。
//!
//! 并发契约：模型就绪阶段经进程级互斥（双击/双窗口不会并发下载两份）；引擎会话
//! 惰性装载，**空闲 10 分钟自动卸载**（M7 idle-unload，见 [`ENGINE_IDLE_TTL`]；
//! Mutex 串行推理——编辑器单窗口语义下单路使用，无吞吐诉求）。
//! 下载/推理均为 `#[tauri::command(async)]`（命令线程执行，不卡事件循环）。

use hawksnap_ocr::models::ProgressPhase;
use serde::Serialize;
use std::time::{Duration, Instant};
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

/// 引擎空闲卸载阈值（M7）：10 分钟未用即 drop（与 idle_reaper 主面板同档——
/// OCR 是低频入口）。实测依据（2026-10-24，`scripts/sample-ocr-mem.sh`，归档
/// task-7-report.md）：装载+首推理常驻 **+349MB**（release 365.9MB）、稳态
/// ~400-580MB——超 M4 遗留裁定线 150MB，若用后常驻会把 G2 空载预算（spec §5
/// ≤80MB）吃穿一个数量级。卸载后下次 `ocr_recognize` 重装（模型哈希 + 三会话
/// 装载，秒级，用户无感）。ORT arena 调优（进一步压单会话峰值）列 M2.1 候选。
const ENGINE_IDLE_TTL: Duration = Duration::from_secs(10 * 60);

/// 卸载看护轮询周期：60s（idle_reaper 同款节奏；回收延迟上界 = TTL + 一个周期）。
const ENGINE_REAP_TICK: Duration = Duration::from_secs(60);

/// 引擎最近使用时刻：None = 从未使用（引擎必未装载）。在 ENGINE 锁**内**登记，
/// 卸载看护据此做「锁内复核」区分在途/新调用（见 [`reap_engine_if_idle`]）。
static ENGINE_LAST_USE: std::sync::OnceLock<std::sync::Mutex<Option<Instant>>> =
    std::sync::OnceLock::new();

fn last_use_slot() -> &'static std::sync::Mutex<Option<Instant>> {
    ENGINE_LAST_USE.get_or_init(|| std::sync::Mutex::new(None))
}

/// idle 卸载判定（纯逻辑，可测）：引擎已装载且距上次使用满 TTL。从未使用（None）
/// 不卸——那意味着引擎也未装载，卸载是空操作，显式 false 保持判定单一职责。
fn engine_idle(loaded: bool, idle_for: Option<Duration>) -> bool {
    loaded && idle_for.is_some_and(|d| d >= ENGINE_IDLE_TTL)
}

/// 引擎已装载前提下距上次使用的时长（锁中毒容忍同 engine_slot）。
fn last_use_elapsed() -> Option<Duration> {
    last_use_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .map(|t| t.elapsed())
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

/// 取就绪引擎（缓存命中直接用；未装载则装载；每次使用刷新 idle 时钟）。
fn with_engine<T>(
    app: &AppHandle,
    f: impl FnOnce(&mut hawksnap_ocr::OcrEngine) -> Result<T, String>,
) -> Result<T, String> {
    let paths = ensure_models_with_progress(app)?;
    let mut slot = engine_slot().lock().map_err(|e| e.to_string())?;
    if slot.is_none() {
        *slot = Some(hawksnap_ocr::OcrEngine::load(&paths)?);
    }
    // 使用时刻在 ENGINE 锁内登记（锁序恒为 ENGINE → LAST_USE，与卸载看护一致，
    // 无死锁）：卸载看护的锁内复核据此不会误卸刚启动的调用。
    *last_use_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    f(slot.as_mut().expect("引擎已装载"))
}

// ---------------------------------------------------------------------------
// idle 卸载看护（M7，idle_reaper 同款模式）：每 60s 检查，空闲满 10 分钟 drop。
// ---------------------------------------------------------------------------

/// 启动卸载看护线程（lib.rs setup 末尾，进程生命周期常驻）。先睡一轮再查（启动期
/// 引擎必未装载，首轮空转无意义）；轮内失败无路径（try_lock 失败 = 有在途调用，
/// 下轮再看），绝不影响 OCR 主链路。
pub fn spawn_engine_idle_unloader() {
    std::thread::spawn(|| loop {
        std::thread::sleep(ENGINE_REAP_TICK);
        reap_engine_if_idle();
    });
}

/// 单轮回收（纯驱动逻辑，判定在 [`engine_idle`]）：快照过线 → try_lock 引擎
/// （在途推理持有锁即跳过，绝不阻塞）→ 锁内复核使用时刻（快照与取锁之间可能有
/// 新调用刚刷新）→ 仍过线才 drop。
fn reap_engine_if_idle() {
    let loaded = {
        // 只判「是否装载」不持锁久留：poisoned 视为已装载（保守，下轮 try_lock 自证）。
        engine_slot().try_lock().is_ok_and(|s| s.is_some())
    };
    if !engine_idle(loaded, last_use_elapsed()) {
        return;
    }
    let Ok(mut slot) = engine_slot().try_lock() else { return };
    if engine_idle(slot.is_some(), last_use_elapsed()) {
        *slot = None; // drop 引擎：实测 ~350-580MB RSS 归还（见 ENGINE_IDLE_TTL 注）
    }
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

    /// idle 卸载判定（M7 纯逻辑）：装载 + 满线才卸；未装载/未满/从未使用均不卸。
    #[test]
    fn engine_idle_requires_loaded_and_past_ttl() {
        assert!(!engine_idle(false, Some(ENGINE_IDLE_TTL)), "未装载不卸");
        assert!(!engine_idle(true, None), "从未使用（必未装载）不卸");
        assert!(!engine_idle(true, Some(ENGINE_IDLE_TTL - Duration::from_secs(1))), "差一秒未满不卸");
        assert!(engine_idle(true, Some(ENGINE_IDLE_TTL)), "恰满线即卸（含等边界）");
        assert!(engine_idle(true, Some(ENGINE_IDLE_TTL * 10)), "远超线卸");
    }

    /// 阈值契约：10 分钟（与 idle_reaper 主面板同档）+ 60s 轮询（回收延迟上界
    /// = TTL + 一周期）。
    #[test]
    fn engine_idle_ttl_constants() {
        assert_eq!(ENGINE_IDLE_TTL, Duration::from_secs(600));
        assert_eq!(ENGINE_REAP_TICK, Duration::from_secs(60));
    }
}
