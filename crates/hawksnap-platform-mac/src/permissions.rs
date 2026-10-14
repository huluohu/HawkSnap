//! Plan D Task D4：屏幕录制权限预检与系统设置引导（修 Plan B 裁定的静默壁纸缺陷）。
//!
//! 缺陷：macOS 14+ 上未授权应用调 `CGWindowListCreateImage` **不报错**，而是静默返回
//! 纯壁纸帧——用户按 F1 后看到一张"冻结的画面"，实为壁纸，且零解释。修复：抓屏前经
//! `CGPreflightScreenCaptureAccess`（macOS 10.14+，仅查询、不弹授权窗）预检，未授权 →
//! 打开系统设置对应面板 + 拒绝本轮截图（Err 经既有 hawk://capture-error 横幅链回流）。
//!
//! 绑定选型：与 ax.rs 同款裸 extern 声明（不引入新 crate）。`CGPreflightScreenCaptureAccess`
//! 声明于 CoreGraphics 框架（CGPreflightScreenCaptureAccess.h），返回 C99 `bool`
//!（非 MacTypes Boolean），Rust `bool` 与其 ABI 兼容，直接 `-> bool` 即为正确签名。

/// 本进程是否已授予「屏幕录制」权限（仅查询，**不弹授权窗**——弹窗交由下方
/// [`open_screen_recording_settings`] 的深链引导，不打断用户当前操作）。
/// 非 macOS 平台恒 true（权限模型不同，各平台适配归后续）。
#[cfg(target_os = "macos")]
pub fn screen_recording_prechecked() -> bool {
    // SAFETY: 无入参、无副作用（仅查询 TCC 授权态），任何平台状态均安全。
    unsafe { ffi::CGPreflightScreenCaptureAccess() }
}

#[cfg(not(target_os = "macos"))]
pub fn screen_recording_prechecked() -> bool {
    true
}

/// 打开「系统设置 → 隐私与安全性 → 屏幕录制」面板（x-apple.systempreferences 深链）。
/// **spawn 不等待**：`open` 把 URL 手递 LaunchServices 即返回，本函数只报告 spawn 成败，
/// 不等子进程退出、不查退出码（调用方按尽力而为处理——深链失败时拒绝横幅照发，
/// 用户仍可手动前往设置）。
#[cfg(target_os = "macos")]
pub fn open_screen_recording_settings() -> Result<(), String> {
    // ?Privacy_ScreenCapture 是系统设置面板锚点（Apple 官方深链参数，macOS 13+ System
    // Settings 均响应）。spawn 句柄即弃：子进程与父进程无进一步交互。
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("open 系统设置失败: {e}"))
}

#[cfg(not(target_os = "macos"))]
pub fn open_screen_recording_settings() -> Result<(), String> {
    Err("仅 macOS 支持系统设置深链".into())
}

// —— CoreGraphics 的屏幕录制预检 C API 裸声明 ——

#[cfg(target_os = "macos")]
mod ffi {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        /// CGPreflightScreenCaptureAccess.h（macOS 10.14+）：仅查询屏幕录制授权态，
        /// 不触发授权弹窗（区别于 CGRequestScreenCaptureAccess）。返回 C99 bool，
        /// true = 已授予。
        pub fn CGPreflightScreenCaptureAccess() -> bool;
    }
}

#[cfg(test)]
mod permissions_tests {
    use super::*;

    #[test]
    fn preflight_returns_bool_without_panic() {
        // 真值依赖本机授权态（开发机已授予 → true；CI/裸机 → false），两值皆合法。
        // 仅断言：可安全调用（FFI 边界无 panic/UB 崩溃）且返回自洽布尔（恒真式，clippy
        // 对 `v == true` 会告警，故写 `v || !v`——意义全在"调用成功"本身）。
        let v = screen_recording_prechecked();
        assert!(v || !v);
    }

    /// 未授权语义无法脚本化（撤销授权需人工在系统设置操作 + TCC 对运行中进程的授权态
    /// 只在重启后刷新）：人工验证步骤——系统设置撤销 HawkSnap 屏幕录制授权 → 重启应用
    /// → 按 F1 → 预期系统设置面板被打开 + 主面板横幅提示权限未授予，且**不出现**覆盖层
    /// /壁纸帧。授权后 cargo test -p hawksnap-platform-mac -- --ignored 运行本文件上方
    /// 的 preflight 用例应得 true。
    #[test]
    #[ignore = "需要人工撤销屏幕录制授权；步骤见上方文档注释"]
    fn unauthorized_semantics_manual_verification() {
        assert!(!screen_recording_prechecked());
    }
}
