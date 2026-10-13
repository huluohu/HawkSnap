use serde::Serialize;
use tauri::AppHandle;

// —— Task 4：方向键磁吸的窗口候选 ——

/// 磁吸候选窗口 rect（逻辑坐标，全 f64）。与 hawksnap_core::geometry::Rect（i32/u32）
/// 同形但独立定义：候选来自 CGWindowList（f64 逻辑坐标），且前端磁吸走自己的
/// 「最近单边」语义（geometry.ts），不经 Rust snap_to——两套语义并存、互不调用。
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct RectLog { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }

/// 全部在屏 layer==0 窗口的逻辑坐标 rect（排除自身 pid 的覆盖层窗口）。display 过滤
/// 在前端做（只有它知道自身 display 边界），display_id 参数预留。空结果 = Ok(vec![])，
/// 绝不以 Err 打断截图流程（磁吸是增强，不是关键路径）。
#[tauri::command]
pub fn snap_candidates(app: AppHandle, _display_id: u64) -> Result<Vec<RectLog>, String> {
    let _ = app; // 预留 per-display 过滤；前端再按自身 display 过滤
    visible_window_rects()
}

/// 绑定选型（Task 4 裁定 3）：用依赖树既有的 core-graphics 0.25（其 window 模块暴露
/// copy_window_info + kCGWindow* 键常量）配 core-foundation 0.10 做 CFArray 迭代与
/// CFNumber 取值——与 xcap 0.4.1 macos impl_window.rs 的 CGWindowList 模式同源，但走
/// core-graphics/core-foundation 这对 crate，不新增依赖族、无需手写 objc2 extern。
#[cfg(target_os = "macos")]
pub(super) fn visible_window_rects() -> Result<Vec<RectLog>, String> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly,
    };

    // CFDictionary 取值按 CFEqual 匹配键，新建同内容 CFString 即可命中系统常量键
    // （kCGWindowLayer 等），无需 unsafe 引用 extern static。
    fn number_of(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<f64> {
        let value = dict.find(CFString::new(key))?;
        value.downcast::<CFNumber>().and_then(|n| n.to_f64())
    }

    // CGWindowListCopyWindowInfo 返回 null（系统级失败）也按空候选处理：磁吸可降级。
    let array = match copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    ) {
        Some(a) => a,
        None => return Ok(vec![]),
    };

    let own_pid = f64::from(std::process::id()); // u32 → f64 精确
    let mut out = Vec::new();
    for i in 0..array.len() {
        // ItemRef<*const c_void> → 借用为窗口信息字典（get-rule 借用，不取得所有权）
        let Some(item) = array.get(i) else { continue };
        let dict = unsafe {
            CFDictionary::<CFString, CFType>::wrap_under_get_rule(*item as *const _)
        };
        // layer==0 = 普通应用窗口（菜单栏/Dock/状态项等更高 layer 一概排除）
        if number_of(&dict, KEY_WINDOW_LAYER) != Some(0.0) {
            continue;
        }
        if number_of(&dict, KEY_WINDOW_OWNER_PID) == Some(own_pid) {
            continue; // 排除自身 pid：覆盖层窗口不作为磁吸候选
        }
        // kCGWindowBounds 是 {X,Y,Width,Height} 子字典（CGRectMakeWithDictionaryRepresentation
        // 消费的同一形态；core-graphics 0.25 未暴露该函数，故手工读四个数）。
        // 类型化 CFDictionary<CFString, CFType> 未实现 ConcreteCFType，downcast 不可用，
        // 经 as_CFTypeRef 取原始指针再按 get-rule 借用为子字典。
        let Some(bounds_ref) = dict.find(CFString::new(KEY_WINDOW_BOUNDS)) else { continue };
        let bounds = unsafe {
            CFDictionary::<CFString, CFType>::wrap_under_get_rule(
                bounds_ref.as_CFTypeRef() as *const _
            )
        };
        let (Some(x), Some(y), Some(w), Some(h)) = (
            number_of(&bounds, "X"), number_of(&bounds, "Y"),
            number_of(&bounds, "Width"), number_of(&bounds, "Height"),
        ) else { continue };
        if w <= 0.0 || h <= 0.0 {
            continue; // 非空 rect 才是可用候选
        }
        out.push(RectLog { x, y, w, h });
    }
    Ok(out)
}

// 系统键名集中于此（内容即 CGWindowList 信息字典的 kCGWindow* 键字符串值）。
#[cfg(target_os = "macos")]
const KEY_WINDOW_LAYER: &str = "kCGWindowLayer";
#[cfg(target_os = "macos")]
const KEY_WINDOW_OWNER_PID: &str = "kCGWindowOwnerPID";
#[cfg(target_os = "macos")]
const KEY_WINDOW_BOUNDS: &str = "kCGWindowBounds";

#[cfg(not(target_os = "macos"))]
pub(super) fn visible_window_rects() -> Result<Vec<RectLog>, String> { Ok(vec![]) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_log_serializes_flat_xywh_keys() {
        // Task 4 契约：snap_candidates 的 rect 为扁平 x/y/w/h 数字键（camelCase rename
        // 对单字母字段是无操作，但显式断言防止未来误改字段名或嵌套成 {x:{...}}）；
        // 前端 Rect 类型按同名结构消费。
        let r = RectLog { x: 1.5, y: 2.0, w: 300.0, h: 4.0 };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, r#"{"x":1.5,"y":2.0,"w":300.0,"h":4.0}"#);
    }
}
