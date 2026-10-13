use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

// 热键注册失败的 pull 通道：setup 阶段的 hawk://hotkey-error emit 早于前端 listen 注册，
// 必丢；错误同时落入此静态，前端挂载时经 hotkey_error 命令拉取补显（读取即清除）。
static HOTKEY_ERROR: Mutex<Option<String>> = Mutex::new(None);

// 仅测试用：镜像 commands::DisplayPayload 的序列化契约（camelCase），供下方契约测试断言
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPayloadForTest {
    pub display_id: u64, pub x: i32, pub y: i32,
    pub width: u32, pub height: u32, pub scale: f64, pub png_url: String,
}

pub fn register_hotkeys(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut};
    let gs = app.global_shortcut();
    let f1 = Shortcut::new(None, Code::F1);
    let app2 = app.clone();
    // on_shortcut 注册快捷键并挂处理器（勿再单独 register：同一组合二次注册会被 macOS
    // 以 eventHotKeyExistsErr 拒绝）。热键回调在主线程（Carbon 事件）触发，而
    // start_capture_flow 里的 close() 在主线程会直接 panic（tauri-runtime-wry 禁止
    // 在主线程处理 Close 消息），故整个流程放到独立线程执行，热键处理立即返回。
    gs.on_shortcut(f1, move |_app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            let app = app2.clone();
            std::thread::spawn(move || trigger_capture_flow(&app));
        }
    })
    .map_err(|e| {
        let msg = format!("F1 注册失败（可能被占用）: {e}");
        // 除返回值外落入 pull 通道（HOTKEY_ERROR）：setup 期的 push emit 早于前端 listen 必丢
        *HOTKEY_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg.clone());
        msg
    })?;
    Ok(())
}

/// F1（Carbon）与媒体键（NSEvent 亮度下）两条触发通道共用的入口：300ms 去重窗内只启动
/// 一次截图流程（个别键盘/系统组合下两条事件可能同时到达）。任意线程可调：函数体只做
/// 原子置位 + spawn，start_capture_flow 永远跑在 spawn 出的非主线程上（主线程建窗/关窗
/// 会 panic）。
pub(crate) fn trigger_capture_flow(app: &AppHandle) {
    if !media::arm_once(&media::TRIGGER_INFLIGHT) {
        return; // 去重窗口内已有触发在跑
    }
    let app2 = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = start_capture_flow(&app2) {
            let _ = tauri::Emitter::emit(&app2, "hawk://capture-error", e);
        }
    });
    // 去重窗按时长解除（而非按流程结束）：恰好挡住同一次按键的双通道竞态
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        media::disarm(&media::TRIGGER_INFLIGHT);
    });
}

// —— Task 13：媒体键（F1 亮度键）兼容 ——

/// 媒体键路径：macOS 默认 `com.apple.keyboard.fnState` 未开启，物理 F1 不发 keycode 122，
/// 而是发 systemDefined 的亮度下事件（NX_KEYTYPE_BRIGHTNESS_DOWN=22）。此处经 NSEvent
/// 全局监听该事件，走与 F1 完全相同的截图流程。监听不可用（无辅助功能权限等）→
/// eprintln + `Ok(())`，绝不阻塞启动；标准 F1（Carbon）路径不受影响。
/// 必须在主线程调用（全局事件监听挂在 AppKit 事件循环上）。
#[cfg(target_os = "macos")]
pub fn register_media_key_f1(app: &AppHandle) -> Result<(), String> {
    let fnstate = media::read_fnstate();
    let monitor_ok = media::install_media_key_monitor(app);
    // macOS 的全局监听对按键类事件仅在被授予「辅助功能」时投递（NSEvent.h：见
    // AXIsProcessTrusted）——监听建立成功但未授权同样是"亮度键路径不通"。
    let usable = monitor_ok && media::ax_is_process_trusted();
    if !usable {
        eprintln!(
            "[hawksnap] 媒体键监听不可用（monitor={monitor_ok}，辅助功能授权未确认），F1 亮度键路径降级"
        );
        // F1 真失联（fnState 是媒体键模式 且 监听不可用）才提示；走 hotkey-error 横幅通道，
        // 带「提示：」前缀与注册失败文案区分；已有真实注册错误时保留错误、仅 eprintln。
        if let Some(hint) = media::fnstate_hint(fnstate) {
            eprintln!("[hawksnap] 提示: {hint}");
            let mut slot = HOTKEY_ERROR.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                let msg = format!("提示：{hint}");
                *slot = Some(msg.clone());
                drop(slot);
                use tauri::Emitter;
                let _ = app.emit("hawk://hotkey-error", msg);
            }
        }
    }
    Ok(())
}

/// 纯逻辑 + 媒体键运行时（macOS 专属部分见下方 cfg 门控）。
pub(crate) mod media {
    /// NSEvent systemDefined 的 data1：高 16 位 = NX 键类型；低 16 位 = flags
    /// （0x0A00=按下 / 0x0B00=释放，详见 parse_media_key 注）。
    pub const NX_KEYTYPE_BRIGHTNESS_DOWN: i64 = 22;

    #[derive(Debug, PartialEq)]
    pub enum MediaKey {
        BrightnessDown,
    }

    /// 解析 systemDefined 事件的 data1；仅识别亮度下键的按下沿。
    ///
    /// data1 高 16 位 = NX 键类型；低 16 位 = flags。keyDown 有两种等价编码：
    /// `0x0A00`（真实硬件/系统事件，SPMediaKeyTap/Chromium/MonitorControl 一致采用：
    /// `((flags & 0xFF00) >> 8) == 0xA`）与 `0x000A`（本模块测试契约）。两者都接受；
    /// keyUp（`0x0B00` / `0x000B`）与其余键类型一律 None。
    pub fn parse_media_key(data1: i64) -> Option<MediaKey> {
        let key = (data1 & 0xFFFF_0000) >> 16;
        let flags = data1 & 0x0000_FFFF;
        let pressed = matches!(flags, 0x0A00 | 0x000A);
        if pressed && key == NX_KEYTYPE_BRIGHTNESS_DOWN {
            Some(MediaKey::BrightnessDown)
        } else {
            None
        }
    }

    /// fnState 提示：true=标准功能键（无需提示）；None=未设置（默认媒体键模式）；
    /// false=媒体键模式。
    pub fn fnstate_hint(fnstate: Option<bool>) -> Option<String> {
        match fnstate {
            Some(true) => None,
            _ => Some("F1 当前是亮度键：可在 系统设置→键盘 开启「将 F1、F2 等键用作标准功能键」，\
                     或在 系统设置→隐私与安全性→辅助功能 中添加 HawkSnap（推荐，开箱即用）。".into()),
        }
    }

    /// 300ms 去重闸门（进程级）：F1 与媒体键双通道竞态时只启动一次流程。
    /// `arm_once` 是纯状态转换（可测）；窗口内重复触发返回 false。
    pub(crate) fn arm_once(flag: &std::sync::atomic::AtomicBool) -> bool {
        use std::sync::atomic::Ordering;
        !flag.swap(true, Ordering::SeqCst)
    }

    pub(crate) fn disarm(flag: &std::sync::atomic::AtomicBool) {
        use std::sync::atomic::Ordering;
        flag.store(false, Ordering::SeqCst);
    }

    pub(crate) static TRIGGER_INFLIGHT: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    /// fnState 读取：`defaults read -g com.apple.keyboard.fnState`。
    /// 退出码 0 且输出 "1" → Some(true)（标准功能键）；退出码非 0（键不存在）→ None
    /// （默认媒体键模式）；其余输出 → Some(false)。
    #[cfg(target_os = "macos")]
    pub(crate) fn read_fnstate() -> Option<bool> {
        let out = std::process::Command::new("defaults")
            .args(["read", "-g", "com.apple.keyboard.fnState"])
            .output()
            .ok()?; // 命令执行失败 → 按未设置（默认媒体键模式）处理
        match out.status.code() {
            Some(0) => Some(String::from_utf8_lossy(&out.stdout).trim() == "1"),
            _ => None,
        }
    }

    /// 本进程是否已被授予「辅助功能」权限（公开 API，仅查询，不弹权限窗）。
    #[cfg(target_os = "macos")]
    pub(crate) fn ax_is_process_trusted() -> bool {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }

    /// 安装 NSEvent 全局监听（仅 systemDefined 事件）。为何不用 CGEventTap：项目依赖树里
    /// 的 core-graphics 0.25 的 CGEventType 枚举缺 SystemDefined(14) 变体，安全 API 无法
    /// 构造掩码，且实测现代 macOS 对合成 systemDefined 事件丢弃 kCGMouseEvent* 槽位写入
    /// （data1 无法从 CGEvent 字段可靠还原）；NSEvent 全局监听则把 data1 作为一等属性
    /// 暴露，语义由系统保证。监听 token 进程生命周期持有（泄漏 = 常驻，永不 remove）。
    /// 失败（返回 nil）→ false，由调用方降级处理。
    #[cfg(target_os = "macos")]
    pub(crate) fn install_media_key_monitor(app: &super::AppHandle) -> bool {
        use block2::RcBlock;
        use objc2_app_kit::NSEvent;
        use std::ptr::NonNull;

        // NSEventMaskSystemDefined = 1 << 14；监听对"其他应用收到的事件"生效——截图场景
        // 正是别的应用在前台，无需 local monitor。
        const NSEVENT_MASK_SYSTEM_DEFINED: u64 = 1 << 14;

        let app2 = app.clone();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
            // 回调由 AppKit 投递（线程不保证），保持最小：解析 + spawn，绝不建窗/关窗。
            // 亮度下键 → 与 F1 同一去重入口（trigger_capture_flow 内部再 spawn 流程线程）。
            // subtype==8 = NX_SUBTYPE_AUX_CONTROL_BUTTONS（媒体键家族，亮度键在此流），
            // 与 SPMediaKeyTap / Chromium / MonitorControl-MediaKeyTap 的判据一致。
            let ev = unsafe { event.as_ref() };
            if ev.subtype().0 == 8 {
                let data1 = ev.data1() as i64;
                if parse_media_key(data1).is_some() {
                    super::trigger_capture_flow(&app2);
                }
            }
        });
        // objc2 0.6 生成的绑定将此方法标为 safe（监听仅观察、不修改事件），直接调用
        let token = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
            objc2_app_kit::NSEventMask(NSEVENT_MASK_SYSTEM_DEFINED),
            &handler,
        );
        match token {
            Some(t) => {
                // 泄漏监听 token：与进程同寿命（tap/handler 引用需进程生命周期存活）
                std::mem::forget(t);
                true
            }
            None => false,
        }
    }
}

/// 拉取并清除最近一次热键注册错误（None = 无错误）；补上 setup 期早于前端 listen 的丢失。
#[tauri::command]
pub fn hotkey_error() -> Option<String> {
    HOTKEY_ERROR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

// 必须在非主线程调用（register_hotkeys 已保证；Task 10 的托盘入口同样需要放到线程里）。
pub fn start_capture_flow(app: &AppHandle) -> Result<(), String> {
    // Task 11 埋点：热键/托盘共用此入口，首行即触发时刻（stage: hotkey）；
    // mark_hotkey 同时武装 overlay_shown 单发门闩（多显示器只记首个 shown，与 hotkey 1:1）
    crate::perf::mark_hotkey();
    let displays = tauri::async_runtime::block_on(commands::capture_screens(app.clone()))?;
    crate::perf::mark("grab_done"); // 抓屏成功返回
    for d in &displays {
        let label = format!("overlay-{}", d.display_id);
        if let Some(w) = app.get_webview_window(&label) { let _ = w.close(); }
        // close() 是投递给主线程事件循环的异步消息：旧窗口经 Destroyed 事件注销前，
        // 重建同 label 窗口会报 WindowLabelAlreadyExists。轮询等待注销完成（上限 500ms，
        // 超时则让 build 报错并走 capture-error，不会卡死）。
        let mut waited = 0u32;
        while app.get_webview_window(&label).is_some() && waited < 500 {
            std::thread::sleep(Duration::from_millis(5));
            waited += 5;
        }
        // query 让前端 main.tsx 按 ?window=overlay 路由到覆盖层页面（dev 走 SPA fallback，
        // 生产 tauri 对 dist 亦以 index.html 兜底）
        let win = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("overlay?window=overlay".into()))
            .position(d.x as f64, d.y as f64)
            .inner_size(d.width as f64, d.height as f64)
            .decorations(false).transparent(true).always_on_top(true)
            .skip_taskbar(true).resizable(false)
            .build().map_err(|e| e.to_string())?;
        // 注入窗口 label：页面加载前 wry 会把 eval 暂存到 pending_scripts，导航完成后执行，
        // 故此处注入不惧与首帧加载竞态；display_id 为 u64，无需转义。
        let _ = win.eval(&format!("window.__LABEL__='overlay-{}';", d.display_id));
        let _ = win.set_focus();
    }
    crate::perf::mark("overlay_created"); // 全部覆盖层窗口 build 完成（每轮截图一次，便于脚本按索引配对）
    // overlay-ready 在窗口 overlay 路由的前端挂载前可能先到（首次 emit 会丢）。
    // 简化实现：共发 3 次、间隔 150ms，前端以收到的最后一次为准（Task 9 契约）。
    // 重复发送放在独立线程，避免热键处理被拖慢 450ms。
    let app2 = app.clone();
    let repeats = displays.clone();
    std::thread::spawn(move || {
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(150));
            let _ = app2.emit("hawk://overlay-ready", repeats.clone());
        }
    });
    app.emit("hawk://overlay-ready", displays).map_err(|e| e.to_string())
}

use crate::commands;

// —— Task 9 薄命令 ——

/// 覆盖层取消：直接关闭对应窗口（capture-error 事件链未覆盖此路径，无需 emit）。
#[tauri::command]
pub async fn cancel_capture(app: AppHandle, display_id: u64) -> Result<(), String> {
    close_overlay(&app, display_id)
}

/// 覆盖层确认完成：广播 capture-done（payload = outcome，契约不变）后按 displayId
/// 直接关闭发起窗口。CaptureOutcome 不携带 displayId，无法走“setup 监听再关窗”链，
/// 故由本命令自带目标（Task 9 裁定 2，跳过 setup 监听）。
#[tauri::command]
pub async fn emit_capture_done(app: AppHandle, display_id: u64, outcome: commands::CaptureOutcome) -> Result<(), String> {
    app.emit("hawk://capture-done", &outcome).map_err(|e| e.to_string())?;
    close_overlay(&app, display_id)
}

fn close_overlay(app: &AppHandle, display_id: u64) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&format!("overlay-{display_id}")) {
        // 命令在异步运行时线程执行（非主线程），close() 安全（见 register_hotkeys 注释）
        w.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_serializes_camel_case() {
        let p = DisplayPayloadForTest {
            display_id: 7, x: 0, y: 0, width: 100, height: 50, scale: 2.0, png_url: "hawk-grab:///tmp/a.png".into(),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"displayId\":7"));
        assert!(s.contains("\"pngUrl\":\"hawk-grab:///tmp/a.png\""));
    }
}

// —— Task 13：媒体键纯逻辑测试 ——
#[cfg(test)]
mod media_tests {
    use super::media::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn parses_brightness_down_from_system_defined() {
        // NSEvent systemDefined: data1 高 16 位为 NX 键类型，低 16 位为按下/释放标志
        let data1 = (22i64 << 16) | 0x000A; // NX_KEYTYPE_BRIGHTNESS_DOWN=22, keyDown flag=0xA
        assert_eq!(parse_media_key(data1), Some(MediaKey::BrightnessDown));
    }

    #[test]
    fn ignores_other_media_keys_and_releases() {
        assert_eq!(parse_media_key((23i64 << 16) | 0x000A), None); // BRIGHTNESS_UP
        assert_eq!(parse_media_key((22i64 << 16) | 0x000B), None); // keyUp flag
    }

    #[test]
    fn hint_only_when_fnstate_not_standard() {
        assert!(fnstate_hint(Some(false)).is_some()); // 媒体键模式 → 提示
        assert!(fnstate_hint(None).is_some());        // 未设置（默认）→ 提示
        assert!(fnstate_hint(Some(true)).is_none());  // 标准功能键 → 无提示
    }

    #[test]
    fn dedupe_arms_once() {
        // 去重闸门纯逻辑：首次置位成功，窗口内（未 disarm 前）再触发被拒
        let flag = AtomicBool::new(false);
        assert!(arm_once(&flag));
        assert!(!arm_once(&flag));
        disarm(&flag);
        assert!(arm_once(&flag));
    }
}
