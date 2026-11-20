//! 真机自检（2026-11-20 真机验收战役）。
//!
//! **它解决什么**：HawkSnap 是零窗口架构（启动只起托盘），于是「应用能不能真的
//! 把前端跑起来、真实素材能不能真的加载」在无人值守环境里无从观察；而 v1.2.1
//! 新增的 CSP 与媒体白名单恰恰是「代码全绿但运行时空白/被拒」的高风险面。
//!
//! 本模块提供自检能力（CI 三平台真机 runner + 用户本机均可跑）：
//! 1. **前端握手**（`frontend_ready`）：前端挂载后 invoke 一次 → 后端落一行
//!    `[hawksnap] frontend ready: <window>`。**这一行同时证明**：CSP 放行了
//!    script-src（bundle 执行了）且 connect-src 放行了 IPC（invoke 往返成功）。
//!    页面因 CSP 空白时 Rust 日志毫发无损——只有它能区分「前端没跑」与「前端跑了
//!    但功能坏了」。
//! 2. **自检模式**（`HAWKSNAP_SMOKE=1`）：自动造真实素材（PNG + 缩略图 + 历史行）
//!    → 跑白名单断言（放行 / 越界）→ 打开编辑器装载真实素材 → 打开主面板 →
//!    等前端回报「缩略图真的加载出来了」（asset 协议的 img-src 运行时实证）→
//!    打印汇总 → 以退出码表态（0 = 全过，1 = 有断言失败）。
//!
//! 沿用仓库既有模式约定（`HAWKSNAP_BENCH` / `HAWKSNAP_G4` 同款环境变量门），
//! 生产运行**零影响**（变量未设置即 no-op）。

use serde::Serialize;
use tauri::AppHandle;

/// 握手回执：前端据此决定**是否**做自检专属的资产探测（生产路径不做任何额外事）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendReadyAck {
    /// 当前是否处于自检模式（true = 前端应做资产探测并回报）。
    pub smoke: bool,
}

/// 自检结果汇总（落在日志里的单行，便于 CI grep）。
#[derive(Debug, Default)]
struct SmokeReport {
    /// 前端握手（script-src + connect-src）。
    frontend: bool,
    /// 资产协议：前端缩略图真的 onload（img-src 的 asset: 放行）。
    asset: Option<bool>,
    /// 编辑器窗口握手（第二个窗口的 CSP + 白名单装载）。
    editor: bool,
    /// 白名单断言：应用落盘根内的路径被放行。
    allow_ok: bool,
    /// 白名单断言：应用落盘根外的路径被拒。
    allow_blocked: bool,
}

/// 自检模式下的跨线程共享状态（OnceLock + Mutex；仅自检模式使用）。
static SMOKE_REPORT: std::sync::OnceLock<std::sync::Mutex<SmokeReport>> = std::sync::OnceLock::new();

fn report() -> &'static std::sync::Mutex<SmokeReport> {
    SMOKE_REPORT.get_or_init(|| std::sync::Mutex::new(SmokeReport::default()))
}

fn smoke_enabled() -> bool {
    std::env::var("HAWKSNAP_SMOKE").as_deref() == Ok("1")
}

/// 前端就绪握手。`window` = 窗口路由名（main / overlay / editor / settings…）。
/// `asset` = 自检模式下前端对「资产协议加载」的回报（ok / fail）。
#[tauri::command]
pub fn frontend_ready(
    app: AppHandle,
    window: Option<String>,
    asset: Option<String>,
) -> FrontendReadyAck {
    let label = window.unwrap_or_else(|| "main".to_string());
    eprintln!("[hawksnap] frontend ready: {label}");
    if let Some(a) = asset.as_deref() {
        eprintln!("[hawksnap] asset probe: {a}");
        // 前端回报形如 `ok:320x200`（带自然尺寸，便于确认加载的是缩略图而非
        // 别的图）——判据取前缀，**不是**严格相等（2026-11-20 CI 实测教训：
        // 严格相等会把「ok:320x200」误判为失败）。
        let ok = a.eq_ignore_ascii_case("ok") || a.to_ascii_lowercase().starts_with("ok:");
        if let Ok(mut r) = report().lock() {
            r.asset = Some(ok);
        }
        if label == "editor" {
            if let Ok(mut r) = report().lock() {
                r.editor = true;
            }
        }
    } else if label == "editor" {
        if let Ok(mut r) = report().lock() {
            r.editor = true;
        }
    } else if label == "main" {
        if let Ok(mut r) = report().lock() {
            r.frontend = true;
        }
    }
    let _ = &app;
    FrontendReadyAck { smoke: smoke_enabled() }
}

/// 自检模式入口（`HAWKSNAP_SMOKE=1`）。见模块注：造素材 → 白名单断言 → 编辑器
/// 装载 → 主面板 → 等前端回报 → 汇总 → 退出（退出码即门禁判据）。
pub fn maybe_run_smoke(app: &AppHandle) {
    if !smoke_enabled() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        run_smoke(app);
    });
}

/// 自检主体（独立线程）：任一断言失败 → 汇总行标 ✗ 并以退出码 1 结束。
fn run_smoke(app: AppHandle) {
    eprintln!("[hawksnap] 自检模式启动");

    // —— ① 造真实素材（save_dir 下子目录）：PNG 主图 + thumbs 缩略图 + 历史行 ——
    let (media_path, thumb_path, row_id) = match seed_media(&app) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[hawksnap] smoke: 造素材失败: {e}");
            finish(&app, false);
            return;
        }
    };
    eprintln!("[hawksnap] smoke: 素材 {} (历史行 {row_id})", media_path.display());

    // —— ② 白名单断言：根内放行 / 根外拒绝 ——
    {
        let mut r = report().lock().unwrap_or_else(|e| e.into_inner());
        r.allow_ok = crate::commands::validate_media_path(&app, &media_path.to_string_lossy()).is_ok();
        // 越界样本：用户主目录（几乎不可能是应用的落盘根）。
        let outside = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
            .join(".hawksnap-not-in-scope.png");
        r.allow_blocked = crate::commands::validate_media_path(&app, &outside.to_string_lossy())
            .is_err();
        eprintln!(
            "[hawksnap] smoke: 白名单断言 allow_ok={} allow_blocked={}",
            r.allow_ok, r.allow_blocked
        );
    }

    // —— ③ 编辑器装载真实素材（第二窗口的 CSP + 白名单路径）——
    match crate::editor::editor_open(
        app.clone(),
        media_path.to_string_lossy().into_owned(),
        None,
        false,
    ) {
        Ok(()) => eprintln!("[hawksnap] smoke: 编辑器装载已发起"),
        Err(e) => eprintln!("[hawksnap] smoke: 编辑器装载失败: {e}"),
    }

    // —— ④ 打开主面板：历史里有刚造的缩略图 → 前端经 asset 协议加载它 ——
    if let Err(e) = crate::panel::open_main_panel(app.clone()) {
        eprintln!("[hawksnap] smoke: 打开主面板失败: {e}");
    }

    // —— ⑤ 等前端回报（资产探测 + 编辑器握手）——
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
    loop {
        let (asset, editor, frontend) = {
            let r = report().lock().unwrap_or_else(|e| e.into_inner());
            (r.asset, r.editor, r.frontend)
        };
        if asset.is_some() && editor && frontend {
            break;
        }
        if std::time::Instant::now() > deadline {
            eprintln!("[hawksnap] smoke: 等待前端回报超时（asset={asset:?} editor={editor} frontend={frontend}）");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    // —— ⑥ 汇总 + 退出码表态 ——
    let (frontend, asset, editor, allow_ok, allow_blocked) = {
        let r = report().lock().unwrap_or_else(|e| e.into_inner());
        (r.frontend, r.asset, r.editor, r.allow_ok, r.allow_blocked)
    };
    let asset_ok = asset == Some(true);
    let all_ok = frontend && asset_ok && editor && allow_ok && allow_blocked;
    eprintln!(
        "[hawksnap] smoke summary: frontend={frontend} asset={asset:?} editor={editor} allow_ok={allow_ok} allow_blocked={allow_blocked} => {}",
        if all_ok { "PASS" } else { "FAIL" }
    );
    // 清理素材与历史行（不留用户目录垃圾、不留指向已删文件的坏历史行；
    // 失败仅记日志——清理不该影响自检结论）。
    let db = crate::history::db_path(&app);
    if let Err(e) = crate::history::delete_at(&db, row_id) {
        eprintln!("[hawksnap] smoke: 清理历史行失败: {e}");
    }
    let _ = std::fs::remove_file(&media_path);
    let _ = std::fs::remove_file(&thumb_path);
    finish(&app, all_ok);
}

/// 造自检素材：save_dir/hawksnap-smoke/ 下写 PNG（640×400 渐变）+ thumbs 缩略图，
/// 并插一行历史（前端主面板因此会渲染这张缩略图 → 走 asset 协议 → img-src 实证）。
/// 返回 (主图路径, 缩略图路径, 历史行 id)。
fn seed_media(
    app: &AppHandle,
) -> Result<(std::path::PathBuf, std::path::PathBuf, i64), String> {
    let settings = crate::settings::Settings::load(app);
    let dir = crate::commands::resolve_save_dir(&settings).ok_or("no picture dir")?;
    let media_dir = dir.join("hawksnap-smoke");
    hawksnap_core::fsutil::ensure_private_dir(&media_dir).map_err(|e| e.to_string())?;
    let w = 640u32;
    let h = 400u32;
    // 渐变 + 几个色块：像素非平凡（缩略图能看出是图不是纯色块），且体积小。
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let band = (x / 80) % 4;
            let (r, g, b) = match band {
                0 => (244u8, 63u8, 94u8),
                1 => (250u8, 204u8, 21u8),
                2 => (34u8, 197u8, 94u8),
                _ => (59u8, 130u8, 246u8),
            };
            let shade = ((y * 255) / h.max(1)) as u8;
            rgba.push(r.saturating_add(shade / 4));
            rgba.push(g.saturating_add(shade / 4));
            rgba.push(b.saturating_add(shade / 4));
            rgba.push(255);
        }
    }
    let png = hawksnap_core::crop::rgba_to_png_borrowed(&rgba, w, h).map_err(|e| e.to_string())?;
    let media_path = media_dir.join("smoke-shot.png");
    hawksnap_core::fsutil::write_private_bytes(&media_path, &png).map_err(|e| e.to_string())?;

    let thumbs_dir = dir.join("thumbs");
    hawksnap_core::fsutil::ensure_private_dir(&thumbs_dir).map_err(|e| e.to_string())?;
    let (thumb_rgba, tw, th) =
        hawksnap_core::crop::rgba_thumbnail(&rgba, w, h, 320).map_err(|e| e.to_string())?;
    let thumb_png =
        hawksnap_core::crop::rgba_to_png_borrowed(&thumb_rgba, tw, th).map_err(|e| e.to_string())?;
    let thumb_path = thumbs_dir.join("smoke-shot.png");
    hawksnap_core::fsutil::write_private_bytes(&thumb_path, &thumb_png).map_err(|e| e.to_string())?;

    // 历史行（前端主面板据此渲染缩略图）。
    let db = crate::history::db_path(app);
    let row_id = crate::history::record_at(
        &db,
        &media_path.to_string_lossy(),
        &thumb_path.to_string_lossy(),
        w as i64,
        h as i64,
        "HawkSnap",
        "region",
        None,
    )?;
    Ok((media_path, thumb_path, row_id))
}

/// 收尾：汇总行已落日志，按结果给退出码（CI 门禁判据）。
fn finish(app: &AppHandle, ok: bool) {
    // 退出码：Tauri 的 app.exit(code) → 进程退出码（Linux/macOS 可观测）。
    // Windows runner 同样可见（CI 脚本按退出码判定）。
    app.exit(if ok { 0 } else { 1 });
}
