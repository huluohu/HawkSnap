//! 设置窗口（M3 重做）：独立于主面板的第二个 webview，label "settings"。
//!
//! 为什么要独立窗口（参考 Snapzy 的偏好设置页）：主面板是**历史/快速访问**的
//! 日常操作面，设置是**低频、层级深**的配置面。混在一个滚动页面里两头都难用——
//! 主面板被设置挤成两屏长尾，设置本身没有导航骨架（几十项平铺，无分区）。
//! 拆成两个窗口后：主面板只留历史（首屏即内容），设置按侧栏分区组织。
//!
//! 线程纪律：与 panel.rs 同款——wry webview 建窗/关窗禁止在主线程做（tauri-runtime-wry
//! 在主线程处理 Close 消息直接 panic），故 ensure_settings_window 必须在非主线程调用；
//! 托盘/面板调用方一律 spawn 独立线程直呼。

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// 设置窗口 label（与 capabilities/default.json 的 `windows: ["settings"]` 对应）。
pub const SETTINGS_LABEL: &str = "settings";

/// 确保设置窗口存在且可见：已存在 → show + set_focus；不存在 → 新建。
///
/// 几何：900×620（侧栏 300 + 内容区 600 起），min 760×520——比主面板宽，因为
/// 设置卡片行是「左标题 / 右控件」的双列布局，窄窗口会把控件挤到换行。
/// 必须在非主线程调用（见模块注）。
pub fn ensure_settings_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(w);
    }
    // URL 形式与既有窗口一致（overlay?window=overlay / editor?window=editor …）：
    // 路径段 + query 里的 window 标记，前端按 query 分发渲染。dev 模式下 vite
    // dev server 对未知路径回退 index.html；生产端 custom-protocol 同理。
    // 窗口标题随设置语言（v1.2 i18n 修复：Dock/调度器可见的用户可见文案）。
    let settings = crate::settings::Settings::load(app);
    let title = if crate::rust_lang_zh(&settings) { "HawkSnap 设置" } else { "HawkSnap Settings" };
    WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        WebviewUrl::App("settings?window=settings".into()),
    )
        .title(title)
        .inner_size(900.0, 620.0)
        .min_inner_size(760.0, 520.0)
        .visible(true)
        .build()
        .map_err(|e| e.to_string())
}

/// 打开设置窗口（托盘「设置…」/ 主面板齿轮按钮的落地）。
///
/// async 命令已在运行时线程（非主线程），建窗合法。
#[tauri::command(async)]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    std::thread::spawn(move || {
        if let Err(e) = ensure_settings_window(&app) {
            eprintln!("[hawksnap] 设置窗口打开失败: {e}");
        }
    });
    Ok(())
}

/// 打开系统设置的某个权限面板（设置页「权限」分区的按钮落地）。
///
/// `which` 取值："screenRecording" | "accessibility"。macOS 深链：
/// - 屏幕录制：?Privacy_ScreenCapture
/// - 辅助功能：?Privacy_Accessibility
/// 其余平台 → Err（明确拒绝，不静默假装成功）。
#[tauri::command(async)]
pub fn open_permission_settings(which: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let anchor = match which.as_str() {
            "screenRecording" => "Privacy_ScreenCapture",
            "accessibility" => "Privacy_Accessibility",
            other => {
                return Err(crate::errcode::coded_kv("perm_panel_unknown", &[("which", other)]));
            }
        };
        std::process::Command::new("open")
            .arg(format!("x-apple.systempreferences:com.apple.preference.security?{anchor}"))
            .spawn()
            .map(|_| ())
            .map_err(|e| crate::errcode::coded_kv("perm_settings_open_failed", &[("detail", &e.to_string())]))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = which;
        Err(crate::errcode::coded("perm_deep_link_macos_only"))
    }
}

/// 权限状态查询（设置页「权限」分区的状态点）。
///
/// macOS 返回真实授权态；其余平台 `platform_applicable=false`——权限门不存在，
/// 前端据此前置显示「无需授权」而不是假绿「已授权」+ 一点就报错的深链按钮
/// （v1.2 跨平台审计修复：此前 Windows/Linux 用户看到两项绿点，点按钮得 Err）。
/// 屏幕录制项复用 `hawksnap_platform_mac::permissions::screen_recording_prechecked()`
/// ——含 preflight 假阴性兜底（窗口标题实证探测），不与截图路径给出矛盾结论。
#[tauri::command(async)]
pub fn permission_status() -> Result<PermissionStatus, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(PermissionStatus {
            screen_recording: hawksnap_platform_mac::permissions::screen_recording_prechecked(),
            accessibility: hawksnap_platform_mac::ax::ax_trusted(),
            platform_applicable: true,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(PermissionStatus {
            screen_recording: false,
            accessibility: false,
            platform_applicable: false,
        })
    }
}

/// 权限状态载荷（serde camelCase → `{screenRecording, accessibility, platformApplicable}`）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    pub screen_recording: bool,
    pub accessibility: bool,
    pub platform_applicable: bool,
}

/// 打开外部链接（设置页「关于」分区的仓库/官网/第三方声明）。
///
/// 只允许 http/https（防 file://、javascript: 等协议注入面）；URL 直传 opener
/// 插件的 open_url（不经 shell，不 spawn 外部命令）。
#[tauri::command(async)]
pub fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(crate::errcode::coded("open_url_invalid"));
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| crate::errcode::coded_kv("open_url_failed", &[("detail", &e.to_string())]))
}
