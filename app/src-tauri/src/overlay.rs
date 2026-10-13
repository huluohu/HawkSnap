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
            std::thread::spawn(move || {
                if let Err(e) = start_capture_flow(&app) {
                    let _ = tauri::Emitter::emit(&app, "hawk://capture-error", e);
                }
            });
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
    // Task 11 埋点：热键/托盘共用此入口，首行即触发时刻（stage: hotkey）
    crate::perf::mark("hotkey");
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
