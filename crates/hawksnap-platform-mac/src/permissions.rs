//! Plan D Task D4：屏幕录制权限预检与系统设置引导（修 Plan B 裁定的静默壁纸缺陷）。
//!
//! 缺陷：macOS 14+ 上未授权应用调 `CGWindowListCreateImage` **不报错**，而是静默返回
//! 纯壁纸帧——用户按 F1 后看到一张"冻结的画面"，实为壁纸，且零解释。修复：抓屏前预检，
//! 未授权 → 打开系统设置对应面板 + 拒绝本轮截图（Err 经既有 hawk://capture-error 横幅
//! 链回流）。
//!
//! ## 预检为什么不能只信 `CGPreflightScreenCaptureAccess`（2026-10-31 用户实测缺陷）
//!
//! 用户已授权「屏幕录制与系统录音」，预检仍每次返回 false → 每次截图都弹「权限未授予」
//! 横幅。两类已知成因：
//! 1. **adhoc 签名授权失配**：TCC 把授权锚定在代码签名上；adhoc 签名的 cdhash 每次构建
//!    必变 → 升级后旧授权对不上新二进制（系统设置里还显示已勾选——陈旧条目）。根修 =
//!    稳定自签证书（Ottr 同款，见 .github/workflows/pipeline.yml 的 release job 注释；
//!    2026-10-31 起接入），预检对旧构建仍是假阴性。
//! 2. **preflight 本身的假阴性 bug**（多个 macOS 版本均有报告）：已授权进程查询仍 false。
//!
//! 故预检实现为两级：preflight 为快路径；false 时再做**实证探测**兜底——
//! `CGWindowListCopyWindowInfo` 里**其它进程**窗口的 `kCGWindowName` 标题只有屏幕录制
//! 权限持有者才拿得到（macOS 公开行为；未授权时一律隐去，本进程窗口不受影响）。
//! 标题可读 = 权限实际在握，preflight 是假阴性 → 放行。探测全空 = 真未授权。
//!
//! 绑定选型：与 ax.rs 同款裸 extern 声明（不引入新 crate）；窗口列表走
//! `core-graphics` 的 `window` feature（windowcap.rs 同款依赖，无新增）。

/// 本进程是否已授予「屏幕录制」权限（仅查询，**不弹授权窗**——弹窗交由下方
/// [`open_screen_recording_settings`] 的深链引导，不打断用户当前操作）。
/// 非 macOS 平台恒 true（权限模型不同，各平台适配归后续）。
#[cfg(target_os = "macos")]
pub fn screen_recording_prechecked() -> bool {
    // SAFETY: 无入参、无副作用（仅查询 TCC 授权态），任何平台状态均安全。
    if unsafe { ffi::CGPreflightScreenCaptureAccess() } {
        return true;
    }
    // preflight 假阴性兜底（见模块注释）：实证探测其它进程窗口标题可读性。
    other_process_window_titles_readable()
}

#[cfg(not(target_os = "macos"))]
pub fn screen_recording_prechecked() -> bool {
    true
}

/// 实证探测：能否读到**其它进程**窗口的 `kCGWindowName` 标题。
/// 屏幕录制权限持有者 → 其它应用窗口标题可读 → true；未授权 → 一律被隐去 → false。
/// 窗口列表拿不到（空/异常）→ 按未授权处理（保守：宁可深链引导，不可放行出壁纸帧）。
#[cfg(target_os = "macos")]
fn other_process_window_titles_readable() -> bool {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{copy_window_info, kCGNullWindowID, kCGWindowListOptionAll};

    fn number_of(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<f64> {
        let value = dict.find(CFString::new(key))?;
        value.downcast::<CFNumber>().and_then(|n| n.to_f64())
    }
    fn string_of(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<String> {
        let value = dict.find(CFString::new(key))?;
        value.downcast::<CFString>().map(|s| s.to_string())
    }

    let Some(array) = copy_window_info(kCGWindowListOptionAll, kCGNullWindowID) else {
        return false;
    };
    let own_pid = f64::from(std::process::id());
    for i in 0..array.len() {
        let Some(item) = array.get(i) else { continue };
        let dict =
            unsafe { CFDictionary::<CFString, CFType>::wrap_under_get_rule(*item as *const _) };
        // 只看**其它进程**的窗口（本进程标题不受权限门控，会污染判定）。
        if number_of(&dict, "kCGWindowOwnerPID") == Some(own_pid) {
            continue;
        }
        if string_of(&dict, "kCGWindowName")
            .is_some_and(|name| !name.trim().is_empty())
        {
            return true;
        }
    }
    false
}

/// 打开「系统设置 → 隐私与安全性 → 屏幕录制」面板（x-apple.systempreferences 深链）。
/// **每进程至多打开一次**（2026-10-31 用户裁定：已授权却因 preflight 假阴性反复弹横幅时，
/// 不能每次截图都强开设置窗口——首次失败引导一次，之后横幅只提示路径，不再开窗）。
/// **spawn 不等待**：`open` 把 URL 手递 LaunchServices 即返回，本函数只报告 spawn 成败，
/// 不等子进程退出、不查退出码（调用方按尽力而为处理——深链失败时拒绝横幅照发，
/// 用户仍可手动前往设置）。
#[cfg(target_os = "macos")]
pub fn open_screen_recording_settings() -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SETTINGS_DEEPLINKED: AtomicBool = AtomicBool::new(false);
    if SETTINGS_DEEPLINKED.swap(true, Ordering::SeqCst) {
        return Ok(()); // 本次会话已引导过——不重复开窗（横幅照发，路径见文案）
    }
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
        /// true = 已授予。**已知假阴性**（见模块注释）——调用方必须带实证兜底。
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
    /// → 按 F1 → 预期系统设置面板被打开（**仅此一次**，同会话第二次失败不再开窗）+
    /// 主面板横幅提示权限未授予，且**不出现**覆盖层/壁纸帧。授权后 cargo test -p
    /// hawksnap-platform-mac -- --ignored 运行本文件上方的 preflight 用例应得 true。
    /// **假阴性场景的人工验证**：正常授权状态下按 F1 多轮——横幅绝不出现（preflight
    /// 假阴性时由窗口标题实证兜底放行）。
    #[test]
    #[ignore = "需要人工撤销屏幕录制授权；步骤见上方文档注释"]
    fn unauthorized_semantics_manual_verification() {
        assert!(!screen_recording_prechecked());
    }
}
