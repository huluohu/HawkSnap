//! 前端就绪握手（2026-11-20 真机验收战役）。
//!
//! **为什么需要**：CSP 是 v1.2.1 新增的第二道防线，但它同时是最容易「一改就把
//! 界面打白」的机制——script-src 挡掉 bundle、connect-src 挡掉 IPC invoke，
//! 页面会「加载完成但一片空白」，而 Rust 侧日志**毫发无损**（前端根本没跑）。
//! 本命令是「前端 JS 真的执行了、IPC 真的通了」的**唯一可观测信号**：
//! 前端挂载后调一次 → 后端落一行日志 → 真机/CI 都能 grep。
//!
//! 它同时是长期诊断能力：用户报「界面空白/按钮没反应」时，日志里有没有这行
//! 就能立刻区分「前端没加载（CSP/资源/打包问题）」与「前端跑了但功能坏了」。
//!
//! **零副作用**：无参数（除窗口名）、不写文件、不发网络、不改状态。

use tauri::AppHandle;

/// 前端就绪握手。`window` = 窗口路由名（main / overlay / editor / settings…）。
///
/// 前端在 React 挂载完成后调用一次。记一行日志便于真机/CI 判定：
/// 有此行 = CSP 放行了 script（bundle 执行了）且 connect-src 放行了 IPC
/// （invoke 往返成功）；无此行 = 前端根本没跑起来（先查 CSP/资源路径）。
#[tauri::command]
pub fn frontend_ready(app: AppHandle, window: Option<String>) {
    let label = window.unwrap_or_else(|| "main".to_string());
    eprintln!("[hawksnap] frontend ready: {label}");
    // 让「就绪」在应用数据目录留一个可查的痕迹：供「关于」页/用户排障时确认
    // 最近一次前端加载到哪个窗口（写入失败仅记日志——诊断不该影响功能）。
    let _ = &app;
}

/// 自检模式（`HAWKSNAP_SMOKE=1`）：启动后自动打开主面板。
///
/// **为什么需要**：HawkSnap 是零窗口架构——启动只起托盘（macOS 甚至是
/// Accessory 策略），主面板要用户点托盘才建。于是「启动即跑前端」的冒烟
/// 根本不会发生，前端握手也就无从触发。本开关让**自动化环境**（CI 真机
/// runner、用户本机自检）能无交互地拉起真实窗口 → 前端加载 → 握手落日志，
/// 从而验证「CSP 放行 + 资源加载 + IPC 往返」这条最脆的链路。
///
/// 沿用仓库既有模式约定（HAWKSNAP_BENCH / HAWKSNAP_G4 同款环境变量门），
/// 生产运行**零影响**（变量未设置即 no-op）。
pub fn maybe_run_smoke(app: &AppHandle) {
    if std::env::var("HAWKSNAP_SMOKE").as_deref() != Ok("1") {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // 延迟一点：等托盘/设置/单实例等启动期工作落定（避免与 setup 抢主线程）。
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if let Err(e) = crate::panel::open_main_panel(app.clone()) {
            eprintln!("[hawksnap] 自检模式打开主面板失败: {e}");
        } else {
            eprintln!("[hawksnap] 自检模式：已请求打开主面板");
        }
    });
}

