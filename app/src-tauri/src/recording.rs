//! M6 基础录屏 app 层：状态机 + 命令 + 悬浮条窗口 + tick 广播 + 历史入库。
//!
//! ## 范围裁定（任务简报记录在案）
//!
//! **M6 首版 = F6 直接录制整个主显示器**（无选区）。区域录制需要 overlay 选区模式
//! 的状态机改造（覆盖层正忙于截图流），值得独立任务——scope cut 归 M3 计划，README
//! 与任务报告均有显著记录。悬浮条 280×56 常驻复用（与 countdown 同款 show/hide）。
//!
//! ## 状态机（纯逻辑可测）
//!
//! `Idle →(start) Recording ⇄(pause/resume) Paused →(stop) Idle`；非法转换返回 Err
//! （前端按钮与 F6 toggle 都先查 `record_status`，双保险）。计时 = 挂钟 − 暂停累计
//! （暂停中含当段），由 Rust 每秒广播 `hawk://record-tick {seconds, paused}`，悬浮条
//! 纯渲染——时间语义单源在 Rust（前端不做本地计时，规避 webview 节流）。
//!
//! ## 线程纪律
//!
//! 命令全部 `#[tauri::command(async)]`（record_stop 内部阻塞收尾 ≤25s：stopCapture
//! 10s + finishWriting 15s——async 命令在运行时线程，不卡事件循环/主线程）。悬浮条
//! 建窗禁主线程（与 countdown 同款），全部入口 spawn/async 保证非主线程。
//!
//! ## 音频裁定（ladder 首版口径）
//!
//! 系统声（SCStream `capturesAudio`）进 AAC 音轨；**麦克风 M6 不做**（AVAudioEngine
//! tap + 双路混音过深，计划预授权 ladder 的该项裁定，列 M2.1/M3）。

use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use hawksnap_platform_mac::record::recording_output_path;

// ---------------------------------------------------------------------------
// 纯逻辑（可测）：状态机 + 计时
// ---------------------------------------------------------------------------

/// 录屏阶段。与平台 record.rs 的 phase 数字（0/1/2）解耦——这里是用户可见语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Recording,
    Paused,
}

/// 用户动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecAction {
    Start,
    Pause,
    Resume,
    Stop,
}

/// 状态机转移：非法转换 → Err（消息面向 F6/悬浮条按钮的错误横幅）。
pub fn transition(phase: Phase, action: RecAction) -> Result<Phase, String> {
    match (phase, action) {
        (Phase::Idle, RecAction::Start) => Ok(Phase::Recording),
        (Phase::Recording, RecAction::Pause) => Ok(Phase::Paused),
        (Phase::Paused, RecAction::Resume) => Ok(Phase::Recording),
        (Phase::Recording | Phase::Paused, RecAction::Stop) => Ok(Phase::Idle),
        (Phase::Idle, RecAction::Pause | RecAction::Resume) => {
            Err("没有进行中的录屏".into())
        }
        (Phase::Idle, RecAction::Stop) => Err("没有进行中的录屏".into()),
        (Phase::Recording, RecAction::Start) => Err("已有录屏进行中".into()),
        (Phase::Paused, RecAction::Start) => Err("录屏已暂停（先恢复或停止）".into()),
        (Phase::Recording, RecAction::Resume) | (Phase::Paused, RecAction::Pause) => {
            // 幂等方向：F6 双通道/悬浮条双击的良性竞态——语义上无害，直接照办。
            Ok(phase)
        }
    }
}

/// 计时秒数：挂钟活跃时长 − 暂停累计（暂停中含当段）。纯 Duration 运算可测。
pub fn elapsed_secs(active: Duration, paused_total: Duration, current_pause: Option<Duration>) -> u64 {
    let paused = paused_total + current_pause.unwrap_or_default();
    active.saturating_sub(paused).as_secs()
}

// ---------------------------------------------------------------------------
// 运行态（static Mutex<Option<...>>：None = Idle；与 HOTKEY_ERROR/DELAY_ACTIVE 同款）
// ---------------------------------------------------------------------------

struct RecRuntime {
    session: u64,
    phase: Phase,
    started_at: Instant,
    paused_total: Duration,
    pause_started: Option<Instant>,
    #[cfg(target_os = "macos")]
    recorder: hawksnap_platform_mac::record::ScreenRecorder,
    output_path: std::path::PathBuf,
}

static REC: Mutex<Option<RecRuntime>> = Mutex::new(None);

/// 会话单调计数（M7）：每次**成功启动**（REC 安装时）取号 +1，随 tick 广播。前端以
/// session 变化识别「新会话」并清除上一会话的行内错误（M6 遗留 minor：失败文案跨
/// 会话残留）。Relaxed 足够：仅需唯一 + 单调，取号在 REC 锁内（启动天然串行），
/// 载荷同步由事件通道自身保证。
static SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn lock_rec() -> std::sync::MutexGuard<'static, Option<RecRuntime>> {
    REC.lock().unwrap_or_else(|e| e.into_inner())
}

/// hawk://record-tick 载荷（悬浮条渲染输入；单词字段无需 rename）。
#[derive(Serialize, Clone)]
struct RecordTick {
    /// 会话号（record_start 单调取号）：前端据此清上一会话的错误残留。
    session: u64,
    seconds: u64,
    paused: bool,
}

/// pull 通道载荷（悬浮条 mount 拉取补显——push 早于 listen 必丢，同 countdown）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordStatus {
    pub seconds: u64,
    pub paused: bool,
    pub active: bool,
}

#[tauri::command]
pub fn record_status() -> Option<RecordStatus> {
    let guard = lock_rec();
    let rt = guard.as_ref()?;
    let current_pause = rt
        .pause_started
        .map(|t| t.elapsed());
    Some(RecordStatus {
        seconds: elapsed_secs(rt.started_at.elapsed(), rt.paused_total, current_pause),
        paused: rt.phase == Phase::Paused,
        active: true,
    })
}

// ---------------------------------------------------------------------------
// 输出目录：~/Movies/HawkSnap（spec §3.4；dirs_picture_dir 同款跨平台 HOME 口径）
// ---------------------------------------------------------------------------

fn movies_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| std::path::PathBuf::from(h).join("Movies/HawkSnap"))
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

/// F6 / 悬浮条入口：启动全屏（主显示器）录制 + 拉起悬浮条 + 启动 tick 广播。
#[tauri::command(async)]
pub fn record_start(app: AppHandle) -> Result<(), String> {
    // 平台门槛（范围裁定）：录屏管线仅 macOS；其余平台显式拒绝（不静默）。
    #[cfg(not(target_os = "macos"))]
    return Err("仅 macOS 支持录屏（跨平台适配归后续计划）".into());

    #[cfg(target_os = "macos")]
    {
    // 独占检查：构造（ScreenRecorder::start 阻塞百 ms 级）在锁外完成后装入；
    // 并发第二请求在锁上排队后走 is_some 拒绝 + 半成品清理。
    {
        let guard = lock_rec();
        if guard.is_some() {
            return Err("已有录屏进行中".into());
        }
    }

    // 屏幕录制权限预检（Plan D4 同款：拒绝 → 深链系统设置 + Err 文案）。
    #[cfg(target_os = "macos")]
    if !hawksnap_platform_mac::permissions::screen_recording_prechecked() {
        if let Err(e) = hawksnap_platform_mac::permissions::open_screen_recording_settings() {
            eprintln!("[hawksnap] 打开屏幕录制权限设置失败: {e}");
        }
        return Err("屏幕录制权限未授予：已为你打开系统设置，授权后重新按 F6 即可".into());
    }

    let base = movies_dir().ok_or_else(|| "无法定位用户目录（HOME/USERPROFILE 均缺失）".to_string())?;
    let path = recording_output_path(&base, chrono::Local::now())?;

    // 构造录屏器（阻塞至 startCapture 完成，~百 ms 级；async 命令线程，不卡主线程）。
    #[cfg(target_os = "macos")]
    let recorder = hawksnap_platform_mac::record::ScreenRecorder::start(path.clone())?;

    {
        let mut guard = lock_rec();
        if guard.is_some() {
            // 并发窗口内第二请求胜出检查（理论不可达：锁外无 await 点）。
            #[cfg(target_os = "macos")]
            let _ = recorder.stop();
            let _ = std::fs::remove_file(&path);
            return Err("已有录屏进行中".into());
        }
        // 会话取号在安装锁内：只有成功启动才消耗序号（失败路径不跳号不影响单调）。
        let session = SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        guard.replace(RecRuntime {
            session,
            phase: Phase::Recording,
            started_at: Instant::now(),
            paused_total: Duration::ZERO,
            pause_started: None,
            #[cfg(target_os = "macos")]
            recorder,
            output_path: path,
        });
    }

    show_recording_bar(&app)?;
    spawn_tick_thread(app);
    Ok(())
    }
}

/// 暂停：SCStream 回调丢帧 + 计时冻结；恢复时时间戳平移（平台侧实现）。
#[tauri::command(async)]
pub fn record_pause() -> Result<(), String> {
    let mut guard = lock_rec();
    let rt = guard.as_mut().ok_or_else(|| "没有进行中的录屏".to_string())?;
    rt.phase = transition(rt.phase, RecAction::Pause)?;
    if rt.pause_started.is_none() {
        rt.pause_started = Some(Instant::now());
    }
    #[cfg(target_os = "macos")]
    rt.recorder.pause();
    Ok(())
}

/// 恢复：时间戳平移在下一帧兑现（平台侧 PauseClock）。
#[tauri::command(async)]
pub fn record_resume() -> Result<(), String> {
    let mut guard = lock_rec();
    let rt = guard.as_mut().ok_or_else(|| "没有进行中的录屏".to_string())?;
    rt.phase = transition(rt.phase, RecAction::Resume)?;
    if let Some(t) = rt.pause_started.take() {
        rt.paused_total += t.elapsed();
    }
    #[cfg(target_os = "macos")]
    rt.recorder.resume();
    Ok(())
}

/// 停止：writer 收尾（阻塞 ≤25s）→ 悬浮条隐藏 → 历史入库（kind=recording，
/// thumb 留空——M0 网格对空 thumbPath 已有占位渲染）。返回 MP4 路径。
#[tauri::command(async)]
pub fn record_stop(app: AppHandle) -> Result<String, String> {
    let rt = {
        let mut guard = lock_rec();
        let rt = guard.take().ok_or_else(|| "没有进行中的录屏".to_string())?;
        // 转移合法性（Idle 态 take 不到 → 已覆盖；此处防未来扩展）。
        transition(rt.phase, RecAction::Stop)?;
        rt
    };

    // 悬浮条在**成功收尾后**才隐藏（评审修复）：stop 失败（磁盘满/收尾超时）时
    // 悬浮条保持可见并展示行内错误——若先隐藏，主停止入口的失败零反馈（不静默）。
    // 收尾阻塞期（≤25s）条冻结在最后秒数（REC 已 take，tick 线程自然退出）。

    let (path, info) = {
        #[cfg(target_os = "macos")]
        {
            let info = rt.recorder.stop()?;
            (rt.output_path, info)
        }
        #[cfg(not(target_os = "macos"))]
        {
            (rt.output_path, ())
        }
    };

    hide_recording_bar(&app);

    // 历史入库（旁路语义：失败仅记录，不影响录制产物返回——与历史库总裁定一致）。
    #[cfg(target_os = "macos")]
    if let Err(e) = crate::history::record(&app, &path.to_string_lossy(), "", info.width as i64, info.height as i64, "recording") {
        eprintln!("[hawksnap] 录屏历史入库失败: {e}");
    }

    Ok(path.to_string_lossy().into_owned())
}

/// F6 toggle（hotkey.rs 调用）：录制/暂停中 → 停止；空闲 → 启动。错误由调用方
/// 走 record-error 横幅（panel::show_record_error）。
pub fn toggle(app: &AppHandle) -> Result<(), String> {
    let phase = lock_rec().as_ref().map(|rt| rt.phase).unwrap_or(Phase::Idle);
    match phase {
        Phase::Idle => record_start(app.clone()),
        Phase::Recording | Phase::Paused => {
            record_stop(app.clone()).map(|_| ())
        }
    }
}

// ---------------------------------------------------------------------------
// tick 广播线程：每秒 hawk://record-tick；Idle（None）即退（每次启动新线程）。
// ---------------------------------------------------------------------------

fn spawn_tick_thread(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let tick = {
            let guard = lock_rec();
            let Some(rt) = guard.as_ref() else { break };
            let current_pause = rt.pause_started.map(|t| t.elapsed());
            RecordTick {
                session: rt.session,
                seconds: elapsed_secs(rt.started_at.elapsed(), rt.paused_total, current_pause),
                paused: rt.phase == Phase::Paused,
            }
        };
        let _ = app.emit("hawk://record-tick", tick);
    });
}

// ---------------------------------------------------------------------------
// 悬浮条窗口（常驻复用，与 countdown 同款 show/hide；280×56 深色实心）
// ---------------------------------------------------------------------------

/// 位置裁定：主显示器顶部居中（录制条贴着屏幕上缘不遮挡内容区）；
/// 主显示器不可得 → center() 兜底（OS 当前屏居中，零假设）。
fn show_recording_bar(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("recording-bar") {
        position_bar(&w);
        let _ = w.show();
        return Ok(());
    }
    let win = WebviewWindowBuilder::new(
        app,
        "recording-bar",
        WebviewUrl::App("recording-bar?window=recording-bar".into()),
    )
    .inner_size(280.0, 56.0)
    .decorations(false)
    .always_on_top(true)
    .resizable(false)
    .skip_taskbar(true)
    .transparent(false) // 深色实心底（plan：非透明）
    .visible(false)     // 先定位再 show，杜绝默认位置闪现（countdown 同款）
    .build()
    .map_err(|e| e.to_string())?;
    position_bar(&win);
    let _ = win.show();
    Ok(())
}

fn position_bar(w: &tauri::WebviewWindow) {
    if let Some(m) = w.primary_monitor().ok().flatten() {
        let size = w.outer_size().unwrap_or(tauri::PhysicalSize::new(280, 56));
        let x = m.position().x + ((m.size().width as i32 - size.width as i32) / 2).max(0);
        // 顶部安全区下移一点（刘海屏）：24 逻辑 px。
        let y = m.position().y + 24;
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    } else {
        let _ = w.center();
    }
}

fn hide_recording_bar(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("recording-bar") {
        let _ = w.hide();
    }
}

// ---------------------------------------------------------------------------
// 测试：状态机转移 + 计时扣除（纯逻辑）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_happy_path() {
        assert_eq!(transition(Phase::Idle, RecAction::Start), Ok(Phase::Recording));
        assert_eq!(transition(Phase::Recording, RecAction::Pause), Ok(Phase::Paused));
        assert_eq!(transition(Phase::Paused, RecAction::Resume), Ok(Phase::Recording));
        assert_eq!(transition(Phase::Recording, RecAction::Stop), Ok(Phase::Idle));
        assert_eq!(transition(Phase::Paused, RecAction::Stop), Ok(Phase::Idle));
    }

    #[test]
    fn transitions_reject_invalid() {
        assert!(transition(Phase::Idle, RecAction::Stop).is_err());
        assert!(transition(Phase::Idle, RecAction::Pause).is_err());
        assert!(transition(Phase::Idle, RecAction::Resume).is_err());
        assert!(transition(Phase::Recording, RecAction::Start).is_err());
        assert!(transition(Phase::Paused, RecAction::Start).is_err());
    }

    #[test]
    fn transitions_idempotent_directions_are_benign() {
        // F6 双通道/悬浮条双击竞态：同方向重复操作语义无害。
        assert_eq!(transition(Phase::Recording, RecAction::Resume), Ok(Phase::Recording));
        assert_eq!(transition(Phase::Paused, RecAction::Pause), Ok(Phase::Paused));
    }

    #[test]
    fn elapsed_excludes_completed_and_running_pauses() {
        // 录 100s、暂停 20s（已恢复）→ 80s。
        assert_eq!(
            elapsed_secs(Duration::from_secs(100), Duration::from_secs(20), None),
            80
        );
        // 录 100s、历史暂停 20s、暂停中 30s → 50s（当前段也冻结）。
        assert_eq!(
            elapsed_secs(
                Duration::from_secs(100),
                Duration::from_secs(20),
                Some(Duration::from_secs(30))
            ),
            50
        );
        // 未暂停 → 原值。
        assert_eq!(elapsed_secs(Duration::from_secs(59), Duration::ZERO, None), 59);
        // 防御：暂停累计超过活跃时长（时钟异常）不 panic 不回绕。
        assert_eq!(
            elapsed_secs(Duration::from_secs(5), Duration::from_secs(20), None),
            0
        );
    }

    #[test]
    fn movies_dir_uses_home_or_userprofile() {
        // 不假设 CI 环境；仅验证函数可调且（若返回）路径尾段正确。
        if let Some(d) = movies_dir() {
            assert!(d.ends_with("Movies/HawkSnap"), "{d:?}");
        }
    }

    /// tick 载荷契约（M7）：session/seconds/paused 三字段（前端按 session 变化清错）。
    #[test]
    fn record_tick_serializes_with_session() {
        let s = serde_json::to_string(&RecordTick { session: 7, seconds: 65, paused: false }).unwrap();
        assert_eq!(s, r#"{"session":7,"seconds":65,"paused":false}"#);
    }

    /// 会话计数单调：连续取号严格递增（Relaxed 下单线程语义即可保证）。
    #[test]
    fn session_counter_is_monotonic() {
        let a = SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let b = SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert!(b > a, "会话号必须单调递增：{a} → {b}");
    }
}
