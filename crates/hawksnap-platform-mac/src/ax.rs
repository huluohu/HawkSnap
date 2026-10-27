//! Plan B Task 5：智能吸附——AX 悬停探测（数据与查询本体）。
//!
//! 绑定选型（任务裁定 6 的选型点）：**不引入 `accessibility` crate**（eiz/accessibility
//! 0.1.6，2023-11 后无更新，且会携带 core-foundation 0.9 / objc2 0.5 旧主版本与树内
//! 0.10 / 0.6 并存），改用对 HIServices（经 ApplicationServices 汇出）C API 的裸 extern
//! 声明——与 overlay.rs 媒体键模块的 `AXIsProcessTrusted` 裸声明同一模式，共 6 个函数。
//! 属性名（"AXRole"/"AXPosition"/"AXSize"）按 CFEqual 匹配，新建同内容 CFString 即命中
//! 系统常量（与 Task 4 的 kCGWindow* 键同一手法），无需引用 extern static。

/// AX 元素框。全局逻辑坐标，**top-left 原点**（与 `CGEventGetLocation`、AX 的
/// kAXPositionAttribute 同一坐标系，无需任何翻转）。
#[derive(Debug, Clone, PartialEq)]
pub struct AxFrame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub role: String,
}

/// 把元素框裁剪到显示器范围 (0,0)-(disp_w, disp_h)；裁后任一边 <2px 或原框非法
/// （空框/完全在显示器外）→ None。纯逻辑，测试见下方 ax_tests。
pub fn clip_frame(f: AxFrame, disp_w: f64, disp_h: f64) -> Option<AxFrame> {
    let x1 = f.x.max(0.0);
    let y1 = f.y.max(0.0);
    let x2 = (f.x + f.w).min(disp_w);
    let y2 = (f.y + f.h).min(disp_h);
    if x2 - x1 < 2.0 || y2 - y1 < 2.0 {
        return None;
    }
    Some(AxFrame { x: x1, y: y1, w: x2 - x1, h: y2 - y1, role: f.role })
}

// —— HIServices（经 ApplicationServices 汇出）的 AX C API 裸声明 ——

#[cfg(target_os = "macos")]
mod ffi {
    use core_foundation::base::CFTypeRef;
    use core_foundation::dictionary::CFDictionaryRef;
    use core_foundation::string::CFStringRef;

    pub type AXUIElementRef = *mut std::ffi::c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        /// options 传 {kAXTrustedCheckOptionPrompt: false} 时仅查询、不弹授权窗。
        /// 返回 MacTypes Boolean（unsigned char），非 0 = 已信任。
        pub fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
        /// 返回 +1 引用的 system-wide AX 元素（调用方负责释放）。
        pub fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        /// 注意实签名是两个 **f32 形参**（AXUIElement.h:353：`float x, float y`），
        /// 不是 CGPoint 结构。命中时 element_out 为 +1 引用；返回 AXError（i32），0 = 成功。
        pub fn AXUIElementCopyElementAtPosition(
            element: AXUIElementRef,
            x: f32,
            y: f32,
            element_out: *mut AXUIElementRef,
        ) -> i32;
        /// 元素所属进程 pid（pid_t = i32）；返回 AXError（i32），0 = 成功。
        pub fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> i32;
        /// 返回 +1 引用的属性值（CFString / AXValue / ...）；返回 AXError，0 = 成功。
        pub fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        /// 从 AXValue 包裹中取原生结构；theType: 1=kAXValueCGPointType，2=kAXValueCGSizeType
        /// （AXValue.h）。返回 MacTypes Boolean，非 0 = 成功。
        pub fn AXValueGetValue(
            value: CFTypeRef,
            the_type: u32,
            value_ptr: *mut std::ffi::c_void,
        ) -> u8;
    }
}

/// 本进程是否已授予「辅助功能」（仅查询，不弹授权窗——弹窗交由用户手动去系统设置，
/// 与「未授权 → 悬停链路缺席、不打扰」的裁定一致）。
#[cfg(target_os = "macos")]
pub fn ax_trusted() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;

    let dict = CFDictionary::from_CFType_pairs(&[(
        CFString::new("AXTrustedCheckOptionPrompt"),
        CFBoolean::false_value(),
    )]);
    // SAFETY: dict 为本地持有且仍存活的合法 CFDictionaryRef；该函数只读取入参、无副作用。
    unsafe { ffi::AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef()) != 0 }
}

#[cfg(not(target_os = "macos"))]
pub fn ax_trusted() -> bool {
    false
}

/// Copy* 族属性读取：AXUIElementCopyAttributeValue 返回 +1 引用，包成 CFType 托管
/// （Drop 即 CFRelease），任何错误/空值 → None。
#[cfg(target_os = "macos")]
unsafe fn copy_attr(element: ffi::AXUIElementRef, name: &str) -> Option<core_foundation::base::CFType> {
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::string::CFString;

    let name_cf = CFString::new(name);
    let mut out: CFTypeRef = std::ptr::null();
    // SAFETY: element 是调用方仍持有的合法 AXUIElement；name_cf 与 out 都指向本地变量，
    // 调用期间合法。out 按约定收到 +1 引用（或出错保持 null）。
    let err = unsafe {
        ffi::AXUIElementCopyAttributeValue(element, name_cf.as_CFTypeRef() as core_foundation::string::CFStringRef, &mut out)
    };
    if err != 0 || out.is_null() {
        return None;
    }
    // SAFETY: out 是上一行的 +1 引用，wrap_under_create_rule 接管所有权，Drop 时释放。
    Some(unsafe { CFType::wrap_under_create_rule(out) })
}

/// 查询屏幕坐标 (x, y)（top-left 原点全局逻辑坐标）处的 AX 元素（不带 pid）。
/// 任何一步失败（未授权 / 无元素 / 属性缺失 / 类型不符）→ None，绝不 panic。
pub fn element_at_point(x: f64, y: f64) -> Option<AxFrame> {
    element_at_point_with_pid(x, y).map(|(f, _)| f)
}

/// 同 [`element_at_point`]，另带元素所属进程 pid：overlay 自命中识别用（Task 6 修复——
/// 悬停时覆盖层恰是全系统最顶层的鼠标接受窗，system-wide 探测会命中本应用并返回
/// 整屏框，调用方需按 pid 排除自身后回退窗口级命中）。
#[cfg(target_os = "macos")]
pub fn element_at_point_with_pid(x: f64, y: f64) -> Option<(AxFrame, i32)> {
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::string::CFString;
    use core_graphics::geometry::{CGPoint, CGSize};

    // AXValue.h：kAXValueCGPointType=1、kAXValueCGSizeType=2（UInt32 常量，CF_ENUM）。
    const KAX_VALUE_POINT_TYPE: u32 = 1;
    const KAX_VALUE_SIZE_TYPE: u32 = 2;

    // SAFETY: AXUIElementCreateSystemWide 无入参；成功返回 +1 引用。先查空再托管
    // （wrap_under_create_rule 对 null 会 assert panic），null 视为系统级失败 → None。
    let syswide_ptr = unsafe { ffi::AXUIElementCreateSystemWide() };
    if syswide_ptr.is_null() {
        return None;
    }
    // SAFETY: 接管上一行的 +1 引用；此后任何早退路径由 Drop 兜底释放，不泄漏。
    let syswide = unsafe { CFType::wrap_under_create_rule(syswide_ptr as CFTypeRef) };
    let syswide_ref = syswide.as_CFTypeRef() as ffi::AXUIElementRef;

    // SAFETY: syswide_ref 为仍持有的合法元素；坐标按头文件实签名传两个 f32
    // （AXUIElement.h:353）；out 指向本地变量。任何非 0（未授权 -25204 / 无元素等）→ None；
    // 命中时 hit 为 +1 引用。
    let mut hit: ffi::AXUIElementRef = std::ptr::null_mut();
    let err = unsafe { ffi::AXUIElementCopyElementAtPosition(syswide_ref, x as f32, y as f32, &mut hit) };
    if err != 0 || hit.is_null() {
        return None;
    }
    // SAFETY: 接管命中的 +1 引用，Drop 兜底释放；element 活到本函数结束。
    let element = unsafe { CFType::wrap_under_create_rule(hit as CFTypeRef) };
    let element_ref = element.as_CFTypeRef() as ffi::AXUIElementRef;

    // kAXPosition / kAXSize 即 top-left 原点全局逻辑坐标的 CGPoint/CGSize 包装，无需翻转。
    // SAFETY: element_ref 为仍持有的合法 AXUIElement（见上方创建处）。
    let position = unsafe { copy_attr(element_ref, "AXPosition") }?;
    let mut pos = CGPoint { x: 0.0, y: 0.0 };
    // SAFETY: position 是 AXValue 包装；AXValueGetValue 校验 theType 匹配后写 value_ptr，
    // 返回 0（类型不符）→ None。position 在该语句存活，读完后由 Drop 释放。
    if unsafe {
        ffi::AXValueGetValue(position.as_CFTypeRef(), KAX_VALUE_POINT_TYPE, &mut pos as *mut CGPoint as *mut std::ffi::c_void)
    } == 0
    {
        return None;
    }
    // SAFETY: 同上，element_ref 仍合法。
    let size = unsafe { copy_attr(element_ref, "AXSize") }?;
    let mut sz = CGSize { width: 0.0, height: 0.0 };
    // SAFETY: 同上，AXValue(CGSize)（theType=2）。
    if unsafe {
        ffi::AXValueGetValue(size.as_CFTypeRef(), KAX_VALUE_SIZE_TYPE, &mut sz as *mut CGSize as *mut std::ffi::c_void)
    } == 0
    {
        return None;
    }
    // SAFETY: 同上，element_ref 仍合法。
    let role = match unsafe { copy_attr(element_ref, "AXRole") }?.downcast_into::<CFString>() {
        // downcast_into 转移所有权（mem::forget 原包装），无双释放；非 UTF-8 字节 lossy 替换。
        Some(s) => s.to_string(),
        None => return None, // 角色不是 CFString（异常系统状态）→ None
    };
    // 所属 pid：pid_t = i32，出参本地变量。非 0（理论罕见：元素存活但进程信息不可得）
    // → None，让调用方走窗口级回退。
    // SAFETY: element_ref 为仍持有的合法 AXUIElement；pid 指向本地 i32。
    let mut pid: i32 = 0;
    if unsafe { ffi::AXUIElementGetPid(element_ref, &mut pid) } != 0 {
        return None;
    }
    Some((AxFrame { x: pos.x, y: pos.y, w: sz.width, h: sz.height, role }, pid))
}

/// 非 macOS 平台：无 AX 可言，恒 None（悬停链路缺席；拖拽框选不经过本函数）。
#[cfg(not(target_os = "macos"))]
pub fn element_at_point_with_pid(_x: f64, _y: f64) -> Option<(AxFrame, i32)> {
    None
}

/// 当前**系统级焦点应用**的 pid（AXFocusedApplication → AXUIElementGetPid）。
///
/// 用途（M3 N4 keycast 评审修复）：验证「悬浮窗 show 是否抢走用户应用的键盘焦点」
/// ——焦点应用 pid 在建窗前后**不变** = 未抢焦点（录屏中用户的按键仍落在被录应用）。
/// 仅查询，不弹窗；未授予辅助功能 → None（调用方按「无法判定」处理，不当失败）。
#[cfg(target_os = "macos")]
pub fn focused_application_pid() -> Option<i32> {
    use core_foundation::base::{CFType, CFTypeRef, TCFType};

    // SAFETY: AXUIElementCreateSystemWide 无入参；成功返回 +1 引用。null → None。
    let syswide_ptr = unsafe { ffi::AXUIElementCreateSystemWide() };
    if syswide_ptr.is_null() {
        return None;
    }
    // SAFETY: 接管 +1 引用；早退路径由 Drop 兜底释放。
    let syswide = unsafe { CFType::wrap_under_create_rule(syswide_ptr as CFTypeRef) };
    let syswide_ref = syswide.as_CFTypeRef() as ffi::AXUIElementRef;

    // AXFocusedApplication → 焦点应用的 AXUIElement（+1 引用）。
    // SAFETY: syswide_ref 合法；out 为本地变量，出参按约定收到 +1 引用。
    let Some(app) = (unsafe { copy_attr(syswide_ref, "AXFocusedApplication") }) else {
        return None;
    };
    let app_ref = app.as_CFTypeRef() as ffi::AXUIElementRef;
    // SAFETY: app_ref 为仍持有的合法 AXUIElement；pid 指向本地 i32。
    let mut pid: i32 = 0;
    if unsafe { ffi::AXUIElementGetPid(app_ref, &mut pid) } != 0 {
        return None;
    }
    Some(pid)
}

#[cfg(not(target_os = "macos"))]
pub fn focused_application_pid() -> Option<i32> {
    None
}

#[cfg(test)]
mod ax_tests {
    use super::*;

    #[test]
    fn clipped_frame_rejects_empty() {
        assert!(clip_frame(AxFrame { x: 0.0, y: 0.0, w: 0.0, h: 0.0, role: "AXButton".into() }, 100.0, 100.0).is_none());
    }

    #[test]
    fn clipped_frame_clamps_to_display() {
        let f = clip_frame(AxFrame { x: 50.0, y: 50.0, w: 200.0, h: 100.0, role: "AXButton".into() }, 100.0, 100.0).unwrap();
        assert_eq!((f.w, f.h), (50.0, 50.0)); // 超出显示器的部分裁掉
        assert_eq!(f.role, "AXButton");
    }

    #[test]
    #[ignore = "需要辅助功能授权；授权后 cargo test -p hawksnap-platform-mac -- --ignored 运行"]
    fn real_element_at_center_returns_frame() {
        // 光标置于 Finder 图标上时手工验证；脚本环境仅验证不 panic 且返回 None 或 Some
        let _ = element_at_point(100.0, 100.0);
    }
}
