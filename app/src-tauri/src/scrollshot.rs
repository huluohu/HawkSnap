//! 长截图（滚动捕获 + 拼接）会话：M3 Task N5。
//!
//! ## 链路
//!
//! 托盘「长截图」→ 覆盖层**选区模式**（intent=scrollshot，与截图/录制选区同一套
//! 选区 UI）→ 确认 → [`scrollshot_start`]（建会话 + 拉起提示条 + 注册 ⌘S 完成热键
//! + 隐藏覆盖层）→ **采集线程**每 500ms 抓一次该区域（平台侧
//! `hawksnap_platform_mac::windowcap`：CGWindowListCreateImage 窗口抓取，失败降级
//! 屏幕抓取）→ 帧进有界队列 → 用户滚动完按 ⌘S（或点提示条的「完成」）→
//! [`scrollshot_stop`]：拼接（`hawksnap_core::stitch`）→ 存 PNG 到 save_dir →
//! 缩略图（首帧）+ 历史入库（kind="scrollshot"）。
//!
//! ## 设计要点
//!
//! - **采集源选型**（plan 裁定）：`SCScrollWindowImage` 不可用 → 用
//!   `CGWindowListCreateImage` 定时 500ms。窗口抓取拿的是**窗口自身内容**，
//!   因此本应用的提示条**不会**被拍进内容（屏幕兜底路径则可能——已知局限）。
//! - **帧队列有界**：帧数（240）与字节（384MB）双限；超限**丢最旧帧**——剩余帧
//!   仍然首尾相接（拼接链不断），代价是长图顶部被截。内存上界优先于完整性
//!   （G2 纪律：绝不无界增长）。
//! - **拼接拒绝优于错拼**：纯色画面 / 重复内容（条纹、重复表格行）→ 匹配判为
//!   歧义 → 拒绝出图（宁可不产出，绝不产出重影图）。见 `hawksnap_core::stitch`。
//! - **⌘S 完成**：全局热键（会话期间注册、结束即注销）。它会**吞掉**会话期间
//!   浏览器自己的 ⌘S（保存网页）——这是「按 ⌘S 结束」的既定语义，用户在会话
//!   期间的意图就是结束长截图。

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use hawksnap_core::geometry::Rect;
use hawksnap_core::stitch::Frame;

/// 采集间隔（plan 裁定：500ms）——**基准间隔**。
const CAPTURE_INTERVAL: Duration = Duration::from_millis(500);
/// 快速滚动时的加密采集间隔（见 [`next_interval`] 的滞回规则）。
const CAPTURE_INTERVAL_FAST: Duration = Duration::from_millis(200);
/// 判定「滚得快」的位移阈值（相对选区高度）。
const FAST_SHIFT_RATIO: f64 = 0.4;
/// 判定「滚得慢」的位移阈值（相对选区高度）。
const SLOW_SHIFT_RATIO: f64 = 0.2;
/// 首帧延迟：给「隐藏覆盖层」留出时间（覆盖层是带遮罩的全屏窗——被拍进去就毁了）。
const FIRST_FRAME_DELAY: Duration = Duration::from_millis(250);
/// 帧队列上限（帧数 + 字节双限）。
const MAX_FRAMES: usize = 240;
const MAX_BYTES: usize = 384 * 1024 * 1024;
/// 完成热键（会话期间全局注册）。
const FINISH_COMBO: &str = "Cmd+S";
/// 提示条窗口标签（**ACL 按标签授权**——capabilities/scrollshot-bar.json 必须存在，
/// 否则 listen 被 ACL 拒绝、提示条永远收不到帧数）。
const BAR_LABEL: &str = "scrollshot-bar";
const BAR_W: f64 = 360.0;
const BAR_H: f64 = 64.0;

// ---------------------------------------------------------------------------
// 会话状态
// ---------------------------------------------------------------------------

struct Session {
    /// 会话号（单调）：前端据此清上一会话的行内错误残留。
    session: u64,
    /// 全局逻辑矩形（Quartz points）——采集区域。
    global: hawksnap_platform_mac::windowcap::GlobalRect,
    /// 选区全局矩形（提示条定位用：避开选区，防被拍进内容）。
    frames: Mutex<VecDeque<Frame>>,
    bytes: AtomicUsize,
    stop: AtomicBool,
    captured: AtomicU64,
}

impl Session {
    fn push_frame(&self, frame: Frame) {
        let size = frame.rgba.len();
        let mut q = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(frame);
        self.bytes.fetch_add(size, Ordering::Relaxed);
        // 有界：超帧数或字节数 → 丢最旧帧（剩余帧仍连续，链不断）。
        while q.len() > MAX_FRAMES || self.bytes.load(Ordering::Relaxed) > MAX_BYTES {
            let Some(old) = q.pop_front() else { break };
            self.bytes.fetch_sub(old.rgba.len(), Ordering::Relaxed);
        }
        self.captured.fetch_add(1, Ordering::Relaxed);
    }

    fn take_frames(&self) -> Vec<Frame> {
        let mut q = self.frames.lock().unwrap_or_else(|e| e.into_inner());
        self.bytes.store(0, Ordering::Relaxed);
        q.drain(..).collect()
    }

    fn frame_count(&self) -> usize {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// 进程级会话槽（None = 无进行中的长截图）。与 REC/HOTKEY_ERROR 同款单槽模式。
static SESSION: Mutex<Option<Arc<Session>>> = Mutex::new(None);

/// 会话号单调计数（每次成功启动取号 +1）。
static SESSION_SEQ: AtomicU64 = AtomicU64::new(0);

fn lock_session() -> std::sync::MutexGuard<'static, Option<Arc<Session>>> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner())
}

/// 是否有进行中的长截图（截图/录屏流程据此互斥拒绝）。
pub fn scrollshot_active() -> bool {
    lock_session().is_some()
}

/// 下一次采集的间隔（**滞回式自适应**）。
///
/// 动因：选区高度 h、采集间隔 T ⇒ 用户滚动 v px/s 时，每帧位移 ≈ v·T。两帧的
/// 重叠 = h − v·T；位移逼近 h 时重叠趋零 → **模板匹配无重叠可配**（eval 实测：
/// 900px 视口下每帧滚 800px+ 的场景整段失配）。
///
/// 规则（带滞回，避免在两档之间抖动）：
/// - 上次位移 > 0.4h → 加密到 200ms（滚得快 → 多拍几帧）；
/// - 上次位移 < 0.2h → 回到 500ms（滚得慢 → 省资源）；
/// - 介于两者之间 → 保持当前间隔。
///
/// 效果：快速滚动时每帧位移自动收敛到 0.2h~0.4h（重叠 ≥ 0.6h）→ 稳定可配。
fn next_interval(current: Duration, shift: Option<i32>, region_h: u32) -> Duration {
    let Some(s) = shift else { return current };
    if region_h == 0 {
        return current;
    }
    let ratio = s as f64 / region_h as f64;
    if ratio > FAST_SHIFT_RATIO {
        CAPTURE_INTERVAL_FAST
    } else if ratio < SLOW_SHIFT_RATIO {
        CAPTURE_INTERVAL
    } else {
        current
    }
}

// ---------------------------------------------------------------------------
// 载荷
// ---------------------------------------------------------------------------

/// tick 载荷（提示条渲染输入）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScrollshotTick {
    pub session: u64,
    pub frames: usize,
}

/// pull 通道载荷（提示条 mount 时补显——push 早于 listen 必丢，同 countdown）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScrollshotStatus {
    pub session: u64,
    pub frames: usize,
    pub active: bool,
}

/// 启动请求（覆盖层选区确认载荷）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollshotReq {
    pub display_id: u64,
    /// display 本地逻辑坐标选区（与 finish_capture 的 FinishReq.rect 同系同源）。
    pub rect: Rect,
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

/// 启动长截图会话（覆盖层选区确认路径）。
///
/// 成功：会话建立 + 提示条上屏 + ⌘S 完成热键注册 + **覆盖层全部隐藏**（覆盖层是
/// 带遮罩的全屏窗——不藏就会被拍进内容）。失败：Err 回流（覆盖层保留可重试）。
#[tauri::command(async)]
pub fn scrollshot_start(app: AppHandle, req: ScrollshotReq) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, req);
        return Err("仅 macOS 支持长截图".into());
    }
    #[cfg(target_os = "macos")]
    {
    if scrollshot_active() {
        return Err("已有长截图进行中".into());
    }
    if crate::recording::recording_active() {
        return Err("录屏进行中，请先停止录屏".into());
    }
    if req.rect.w < 16 || req.rect.h < 16 {
        return Err(format!("选区尺寸过小：{}×{}", req.rect.w, req.rect.h));
    }
    // 屏幕录制权限预检（与截图/录屏同门槛；未授权时 macOS 静默返回空图）。
    if !hawksnap_platform_mac::permissions::screen_recording_prechecked() {
        let _ = hawksnap_platform_mac::permissions::open_screen_recording_settings();
        return Err("屏幕录制权限未授予：已为你打开系统设置，授权后重试".into());
    }
    // 显示器全局几何（覆盖层那一轮抓屏留下的几何快照；释放整帧后仍保留）。
    let Some(geo) = crate::commands::captured_display_geometry(req.display_id) else {
        return Err("找不到显示器的位置信息，请重新框选".into());
    };
    let global = hawksnap_platform_mac::windowcap::GlobalRect::new(
        geo.x + req.rect.x as f64,
        geo.y + req.rect.y as f64,
        req.rect.w as f64,
        req.rect.h as f64,
    );

    let session = Arc::new(Session {
        session: SESSION_SEQ.fetch_add(1, Ordering::Relaxed),
        global,
        frames: Mutex::new(VecDeque::new()),
        bytes: AtomicUsize::new(0),
        stop: AtomicBool::new(false),
        captured: AtomicU64::new(0),
    });
    *lock_session() = Some(session.clone());

    // 覆盖层先藏（否则遮罩被拍进内容）；再上提示条；再注册 ⌘S。
    if let Err(e) = crate::overlay::hide_all_overlays(&app) {
        eprintln!("[hawksnap] 长截图启动后隐藏覆盖层失败: {e}");
    }
    show_scrollshot_bar(&app, &global)?;
    register_finish_hotkey(&app);

    spawn_capture_thread(app.clone(), session.clone());
    emit_tick(&app, &session);
    Ok(())
    }
}

/// 结束长截图：拼接 → 存 PNG → 缩略图 + 历史入库（kind="scrollshot"）。
///
/// 返回产物路径。失败（无有效匹配 / 写盘失败）→ Err（提示条行内显示）。
#[tauri::command(async)]
pub fn scrollshot_stop(app: AppHandle) -> Result<String, String> {
    let session = {
        let mut guard = lock_session();
        guard.take()
    }
    .ok_or_else(|| "没有进行中的长截图".to_string())?;
    session.stop.store(true, Ordering::SeqCst);
    unregister_finish_hotkey(&app);
    close_scrollshot_bar(&app);

    let frames = session.take_frames();
    if frames.len() < 2 {
        return Err("没有捕获到足够的帧（请先滚动页面再结束）".into());
    }
    let stitched = hawksnap_core::stitch::stitch(&frames).ok_or_else(|| {
        "未能识别滚动位移：画面内容没有明显变化，或内容重复导致匹配歧义".to_string()
    })?;

    let settings = crate::settings::Settings::load(&app);
    let dir = crate::commands::resolve_save_dir(&settings).ok_or("no picture dir")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = crate::commands::save_path_for(&dir, "HawkSnap", "png");
    let png = hawksnap_core::crop::rgba_to_png(&stitched.rgba, stitched.width, stitched.height)
        .map_err(|e| e.to_string())?;
    std::fs::write(&path, &png).map_err(|e| format!("保存失败: {e}"))?;

    // 缩略图 = **首帧**（plan 契约：thumb 取首帧——它是长图顶部，网格里一眼能认出）。
    let thumb_path = write_thumbnail(&dir, &path, &frames[0])?;
    // 历史入库（旁路语义：失败仅记录，不影响产物返回）。
    if let Err(e) = crate::history::record(
        &app,
        &path.to_string_lossy(),
        &thumb_path.to_string_lossy(),
        stitched.width as i64,
        stitched.height as i64,
        "scrollshot",
        "scrollshot",
        None,
    ) {
        eprintln!("[hawksnap] 长截图历史入库失败: {e}");
    }
    Ok(path.to_string_lossy().into_owned())
}

/// 提示条 pull 通道（帧数补显）。
#[tauri::command]
pub fn scrollshot_status() -> Option<ScrollshotStatus> {
    let guard = lock_session();
    let s = guard.as_ref()?;
    Some(ScrollshotStatus {
        session: s.session,
        frames: s.frame_count(),
        active: true,
    })
}

// ---------------------------------------------------------------------------
// 采集线程
// ---------------------------------------------------------------------------

fn spawn_capture_thread(app: AppHandle, session: Arc<Session>) {
    std::thread::spawn(move || {
        // 首帧延迟：覆盖层 hide 是投递给主线程的异步消息，留出时间避免拍到遮罩。
        std::thread::sleep(FIRST_FRAME_DELAY);
        let mut interval = CAPTURE_INTERVAL;
        // 上一帧的行亮度剖面（用于估算帧间位移 → 自适应采集间隔）。
        let mut last_profile: Option<Vec<f32>> = None;
        while !session.stop.load(Ordering::Relaxed) {
            let t0 = Instant::now();
            match hawksnap_platform_mac::windowcap::capture_global_rect(session.global) {
                Ok(img) => {
                    if let Some(frame) = Frame::new(img.width, img.height, img.rgba) {
                        // 帧间位移（1D 行谱，纯函数、毫秒级）→ 决定下一拍间隔。
                        let profile = frame.row_profile();
                        let shift = last_profile
                            .as_ref()
                            .and_then(|prev| hawksnap_core::stitch::estimate_shift(prev, &profile));
                        interval = next_interval(interval, shift, frame.height);
                        last_profile = Some(profile);
                        session.push_frame(frame);
                    }
                }
                Err(e) => eprintln!("[hawksnap] 长截图抓帧失败: {e}"),
            }
            emit_tick(&app, &session);
            let elapsed = t0.elapsed();
            if elapsed < interval {
                std::thread::sleep(interval - elapsed);
            }
        }
    });
}

fn emit_tick(app: &AppHandle, session: &Session) {
    let _ = app.emit(
        "hawk://scrollshot-tick",
        ScrollshotTick {
            session: session.session,
            frames: session.frame_count(),
        },
    );
}

// ---------------------------------------------------------------------------
// 完成热键（⌘S）
// ---------------------------------------------------------------------------

fn register_finish_hotkey(app: &AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let handle = app.clone();
    if let Err(e) = app.global_shortcut().on_shortcut(FINISH_COMBO, move |_app, _sc, event| {
        use tauri_plugin_global_shortcut::ShortcutState;
        if event.state() != ShortcutState::Pressed {
            return;
        }
        let app = handle.clone();
        // 拼接是重活（秒级）→ 绝不占主线程（热键回调在主线程）。
        std::thread::spawn(move || {
            if let Err(e) = scrollshot_stop(app.clone()) {
                crate::panel::show_capture_error(&app, e);
            }
        });
    }) {
        eprintln!("[hawksnap] 注册 ⌘S 完成热键失败: {e}");
    }
}

fn unregister_finish_hotkey(app: &AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    if let Err(e) = app.global_shortcut().unregister(FINISH_COMBO) {
        eprintln!("[hawksnap] 注销 ⌘S 完成热键失败: {e}");
    }
}

// ---------------------------------------------------------------------------
// 提示条窗口
// ---------------------------------------------------------------------------

/// 提示条上屏（懒建 + 常驻复用，与 recording-bar 同款）。
///
/// **位置刻意避开选区**：屏幕兜底抓取路径会把「屏幕上真实可见的像素」拍进去，
/// 提示条若压在选区上就会在长图里留下自己的残影。选区在上下都避开时，退到
/// 底边（与 keycast 同款，让开 Dock）。
fn show_scrollshot_bar(app: &AppHandle, global: &hawksnap_platform_mac::windowcap::GlobalRect) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(BAR_LABEL) {
        position_bar(&w, global);
        let _ = w.show();
        return Ok(());
    }
    let win = WebviewWindowBuilder::new(
        app,
        BAR_LABEL,
        WebviewUrl::App("scrollshot-bar?window=scrollshot-bar".into()),
    )
    .inner_size(BAR_W, BAR_H)
    .decorations(false)
    .always_on_top(true)
    .resizable(false)
    .skip_taskbar(true)
    // 点击穿透：滚动页面的点击必须落到被拍的应用上（本窗只是提示）。
    .focusable(false)
    .focused(false)
    .transparent(false)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;
    let _ = win.set_ignore_cursor_events(true);
    position_bar(&win, global);
    let _ = win.show();
    Ok(())
}

/// 提示条定位：优先避开选区的上下边，都避开不了才压边。
fn position_bar(w: &tauri::WebviewWindow, global: &hawksnap_platform_mac::windowcap::GlobalRect) {
    let Ok(Some(m)) = w.primary_monitor() else {
        let _ = w.center();
        return;
    };
    let size = w
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(BAR_W as u32, BAR_H as u32));
    let mx = m.position().x as f64;
    let my = m.position().y as f64;
    let mw = m.size().width as f64;
    let mh = m.size().height as f64;
    let bar_x = mx + ((mw - size.width as f64) / 2.0).max(0.0);
    // 选区与屏幕的交集（用于判断是否压住选区）。
    let sel_top = global.y.max(my);
    let sel_bottom = (global.y + global.h).min(my + mh);
    let top_y = my + 24.0; // 顶部（刘海安全区下移）
    let bottom_y = my + mh - size.height as f64 - 24.0;
    // 顶部不压选区 → 用顶部；否则底部不压选区 → 用底部；否则用底部（并由窗口抓取
    // 路径兜底：本应用的窗口不会进入窗口抓取内容）。
    let y = if !(sel_top < top_y + size.height as f64 && sel_bottom > top_y) {
        top_y
    } else if !(sel_top < bottom_y + size.height as f64 && sel_bottom > bottom_y) {
        bottom_y
    } else {
        bottom_y
    };
    let _ = w.set_position(tauri::PhysicalPosition::new(
        bar_x.round() as i32,
        y.round().max(0.0) as i32,
    ));
}

fn close_scrollshot_bar(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(BAR_LABEL) {
        let _ = w.close();
    }
}

// ---------------------------------------------------------------------------
// 缩略图
// ---------------------------------------------------------------------------

/// 缩略图落盘（首帧 → 320px 缩略图 → save_dir/thumbs/<stem>.png）。
fn write_thumbnail(
    dir: &std::path::Path,
    out_path: &std::path::Path,
    first: &Frame,
) -> Result<std::path::PathBuf, String> {
    let thumbs_dir = dir.join("thumbs");
    std::fs::create_dir_all(&thumbs_dir).map_err(|e| e.to_string())?;
    let stem = out_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "shot".into());
    let thumb_path = thumbs_dir.join(format!("{stem}.png"));
    let (thumb_rgba, tw, th) = hawksnap_core::crop::rgba_thumbnail(
        &first.rgba,
        first.width,
        first.height,
        320,
    )
    .map_err(|e| e.to_string())?;
    let thumb_png = hawksnap_core::crop::rgba_to_png(&thumb_rgba, tw, th).map_err(|e| e.to_string())?;
    std::fs::write(&thumb_path, &thumb_png).map_err(|e| e.to_string())?;
    Ok(thumb_path)
}

// ---------------------------------------------------------------------------
// 测试（纯逻辑：载荷契约 + 队列边界）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_payload_serializes_camel_case() {
        let s = serde_json::to_string(&ScrollshotTick { session: 3, frames: 12 }).unwrap();
        assert_eq!(s, r#"{"session":3,"frames":12}"#);
    }

    #[test]
    fn status_payload_serializes_camel_case() {
        let s = serde_json::to_string(&ScrollshotStatus {
            session: 1,
            frames: 2,
            active: true,
        })
        .unwrap();
        assert_eq!(s, r#"{"session":1,"frames":2,"active":true}"#);
    }

    #[test]
    fn req_deserializes_camel_case() {
        let req: ScrollshotReq =
            serde_json::from_str(r#"{"displayId":2,"rect":{"x":10,"y":20,"w":800,"h":600}}"#)
                .unwrap();
        assert_eq!(req.display_id, 2);
        assert_eq!((req.rect.x, req.rect.y, req.rect.w, req.rect.h), (10, 20, 800, 600));
    }

    /// 帧队列有界：超过字节上限时丢**最旧**帧，剩余帧仍首尾相接（拼接链不断）。
    #[test]
    fn frame_queue_drops_oldest_when_over_budget() {
        let s = Session {
            session: 0,
            global: hawksnap_platform_mac::windowcap::GlobalRect::new(0.0, 0.0, 10.0, 10.0),
            frames: Mutex::new(VecDeque::new()),
            bytes: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            captured: AtomicU64::new(0),
        };
        // 造若干 1×1 帧（4 字节）——用 MAX_FRAMES 上限验证帧数边界。
        for _ in 0..(MAX_FRAMES + 5) {
            s.push_frame(Frame { width: 1, height: 1, rgba: vec![1, 2, 3, 4] });
        }
        let q = s.frames.lock().unwrap();
        assert_eq!(q.len(), MAX_FRAMES, "帧数上限必须生效");
        assert_eq!(s.bytes.load(Ordering::Relaxed), MAX_FRAMES * 4, "字节计数须与实际一致");
    }

    /// 自适应采集间隔（滞回规则，纯函数）：
    /// - 位移 > 0.4h → 加密到 200ms（滚得快）；
    /// - 位移 < 0.2h → 回到 500ms（滚得慢）；
    /// - 中间区间 → 保持当前间隔（不抖动）。
    #[test]
    fn next_interval_adapts_with_hysteresis() {
        let h = 900u32;
        // 800/900 = 0.89 > 0.4 → 加密
        assert_eq!(next_interval(CAPTURE_INTERVAL, Some(800), h), CAPTURE_INTERVAL_FAST);
        // 400/900 = 0.44 > 0.4 → 仍算快 → 加密
        assert_eq!(next_interval(CAPTURE_INTERVAL, Some(400), h), CAPTURE_INTERVAL_FAST);
        // 300/900 = 0.33 落在滞回带内 → 保持当前间隔
        assert_eq!(next_interval(CAPTURE_INTERVAL, Some(300), h), CAPTURE_INTERVAL);
        // 100/900 = 0.11 < 0.2 → 滚得慢 → 回到基准间隔
        assert_eq!(next_interval(CAPTURE_INTERVAL_FAST, Some(100), h), CAPTURE_INTERVAL);
        // 滞回带内保持加密（不来回抖动）
        assert_eq!(next_interval(CAPTURE_INTERVAL_FAST, Some(250), h), CAPTURE_INTERVAL_FAST);
        // 位移测不出（None）→ 保持当前间隔。
        assert_eq!(next_interval(CAPTURE_INTERVAL_FAST, None, h), CAPTURE_INTERVAL_FAST);
    }

    /// 会话槽空态：idle → scrollshot_active() == false。
    #[test]
    fn no_session_when_idle() {
        let idle = lock_session().is_none();
        if idle {
            assert!(!scrollshot_active());
        }
    }
}
