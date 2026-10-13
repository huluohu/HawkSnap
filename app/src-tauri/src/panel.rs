//! 主面板（C6）：懒创建的 webview 窗口，label "main"，空路由 `/` → App.tsx。
//!
//! G2 整改口径：启动零窗口（tauri.conf `app.windows: []`，托盘 + 热键常驻，
//! spec G2「常驻内存 ≤80MB」按零 webview 口径达标）——主面板按需创建/复用，
//! 隐藏满 10 分钟由 idle_reaper 销毁（面板 webview 也在 G2 账单上）。
//!
//! 线程纪律（与 start_capture_flow 同款，**注意与贴图 pin 相反**）：wry webview
//! 的建窗/关窗禁止在主线程做（tauri-runtime-wry 在主线程处理 Close 消息直接
//! panic，见 hotkey.rs 注），故 ensure_main_panel 必须在非主线程调用；原生
//! NSWindow（贴图）才走 run_on_main_thread。托盘/热键失败路径等调用方一律
//! spawn 独立线程直呼本函数（托盘 shot 同款模式）。

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// 主面板窗口 label（与 capabilities/default.json 的 `windows: ["main"]` 对应）。
pub const PANEL_LABEL: &str = "main";

/// 确保主面板存在且可见：已存在 → show + set_focus 后返回既有窗口；不存在 →
/// 按原 scaffold 几何（800×560，min 600×420）创建并返回。close（红绿灯叉）走
/// tauri 默认 destroy → 下次调用即重建，无需额外状态。
///
/// 必须在非主线程调用（webview 建窗禁主线程，见模块注）。
pub fn ensure_main_panel(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(PANEL_LABEL) {
        // 复用：与 overlay 复用分支同口径——show/focus 都是投递给主线程事件循环
        // 的消息，非主线程调用安全。
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(w);
    }
    WebviewWindowBuilder::new(app, PANEL_LABEL, WebviewUrl::App("/".into()))
        .title("HawkSnap")
        .inner_size(800.0, 560.0)
        .min_inner_size(600.0, 420.0)
        .visible(true)
        .build()
        .map_err(|e| e.to_string())
}

/// 面板入口按钮（C6 最小真实版）：「截图」「贴图」两枚入口与 F1/F2 热键、托盘
/// 同链路同反馈（失败均 emit hawk://capture-error → 面板横幅）。fire-and-forget：
/// spawn 后立即返回 Ok，错误经事件通道回流（spec §9 不静默）。
#[tauri::command(async)]
pub fn panel_trigger(app: AppHandle, action: String) -> Result<(), String> {
    // async 命令已在运行时线程（非主线程），但 start_capture_flow 内部 block_on
    // 抓屏耗时数秒——仍派发独立线程，命令立即返回（与托盘 shot 纪律一致）。
    std::thread::spawn(move || match action.as_str() {
        "capture" => {
            if let Err(e) = crate::overlay::start_capture_flow(&app) {
                let _ = tauri::Emitter::emit(&app, "hawk://capture-error", e);
            }
        }
        "pin" => {
            if let Err(e) = crate::pins::pin_latest(app.clone()) {
                let _ = tauri::Emitter::emit(&app, "hawk://capture-error", e);
            }
        }
        other => {
            eprintln!("[hawksnap] panel_trigger: 未知操作 {other:?}");
        }
    });
    Ok(())
}
