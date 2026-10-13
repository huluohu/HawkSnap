use serde::Serialize;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;

use super::candidates::RectLog;
use crate::commands::DisplayFrame;

/// hawk://hover payload：Some = 高亮该元素框（全局逻辑坐标，top-left 原点，含义同
/// hawksnap_platform_mac::ax::AxFrame）；None = 清除高亮（cursor 下无元素 / 查询失败）。
/// 字段均为单词、无需 rename，derive 保持与既有 payload（DisplayPayloadForTest 等）一致。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AxFrameJson {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub role: String,
}

impl From<hawksnap_platform_mac::ax::AxFrame> for AxFrameJson {
    fn from(f: hawksnap_platform_mac::ax::AxFrame) -> Self {
        AxFrameJson { x: f.x, y: f.y, w: f.w, h: f.h, role: f.role }
    }
}

/// 16ms 节流 ≈ 60fps，与覆盖层重绘节奏契合；同时限定内联 AX 同步查询的成本上限
/// （裁定 2：查询 inline 在 handler 主线程做，绝不 per-event spawn 线程）。
const THROTTLE: Duration = Duration::from_millis(16);

/// 上次发射时刻（poison 容忍与 HOTKEY_ERROR 同款）。
static LAST_HOVER: Mutex<Option<Instant>> = Mutex::new(None);

/// NSEventMaskMouseMoved = 1 << 5。
const NSEVENT_MASK_MOUSE_MOVED: u64 = 1 << 5;

/// 安装全局 + 本地两个 mouseMoved 监听，token 泄漏保活（同 media 监听，永不 remove）。
///
/// 为何本地监听不可省（对 brief「全局监听」的必要补充，已在任务报告登记为适配）：
/// NSEvent 全局监听按定义不投递「发往本应用的事件」；而悬停探测恰在覆盖层可见时
/// 工作，此时全屏命中本应用窗口（拖拽框选就靠这些事件），只有本地监听能看到。
/// 两个监听的 handler 都在主线程执行，共用同一 hover_tick 主体。
pub(super) fn install_hover_monitors(app: &AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::NSEvent;

    // 全局监听：光标在其他应用窗口上时的移动（覆盖层尚未完成命中/菜单栏等场景）。
    let app_g = app.clone();
    let global = RcBlock::new(move |_event: NonNull<NSEvent>| {
        hover_tick(&app_g);
    });
    // objc2 0.6 生成的绑定将此方法标为 safe（监听仅观察、不修改事件）——与 media 同。
    let global_token = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        objc2_app_kit::NSEventMask(NSEVENT_MASK_MOUSE_MOVED),
        &global,
    );
    if let Some(t) = global_token {
        // 泄漏监听 token：与进程同寿命（监听常驻，永不 remove）
        std::mem::forget(t);
    }

    // 本地监听：光标在本应用窗口（覆盖层）上的移动。返回值契约：必须原样返回事件，
    // 返回 null 会吞掉事件、拖拽框选直接失效。
    let app_l = app.clone();
    let local = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        hover_tick(&app_l);
        event.as_ptr() // 原样放行（NonNull → 裸指针，不取 null 分支）
    });
    // SAFETY: block 返回原事件的有效指针（非 null），满足绑定的 Safety 契约
    // （"block's return must be a valid pointer or null"——我们绝不返回 null）。
    let local_token = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            objc2_app_kit::NSEventMask(NSEVENT_MASK_MOUSE_MOVED),
            &local,
        )
    };
    if let Some(t) = local_token {
        std::mem::forget(t); // 泄漏保活，同上
    }
}

/// handler 主体（全局/本地共用）。顺序固定：总闸 → 节流 → 光标 → AX 查询 → emit。
/// 在主线程执行；16ms 节流保证 AX 同步查询至多 ~60 次/秒。
fn hover_tick(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    use tauri::Emitter;

    // 门 1：无覆盖层可见 → 立即返回（空转成本 = 一次原子读 + 一次函数返回）。
    if !super::ANY_OVERLAY_VISIBLE.load(Ordering::SeqCst) {
        return;
    }
    // 门 2：16ms 节流——同一把锁内读旧值 + 写新值，窗口内重复事件直接丢弃。
    {
        let mut last = LAST_HOVER.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < THROTTLE) {
            return;
        }
        *last = Some(Instant::now());
    }
    // 门 3：光标位置。CGEventCreate(nil) + CGEventGetLocation：top-left 原点全局逻辑
    // 坐标，与 AxFrame/AXPosition 同一坐标系。勿用 NSEvent 的 mouseLocation /
    // locationInWindow（bottom-left 原点——brief 明言的坐标陷阱）。
    let Some((cx, cy)) = cursor_position() else { return };
    // 悬停回退链（Task 7 收敛为三级）：元素命中先验 pid——外来进程直通；本进程
    // 自命中（覆盖层是最顶层鼠标接受窗）或探测失败 → 窗口级点命中 → 仍无 → 显示器
    // 全帧（全屏模式：悬停空白处高亮整屏，单击即全选本屏）→ 全部落空才 None。None
    // 同样 emit（前端以 null 清除高亮）。回退是增强而非关键路径：CGWindowList 拉取
    // 失败按空候选处理（与 snap_candidates 同款降级），绝不 panic 打断 hover 链。
    // pid 在 resolve_hover 内消费，emit 前剥除（AxFrameJson 契约不变）。显示器候选
    // 取自 CAPTURES 的几何快照（captured_display_frames：锁内克隆、立即释放）。
    let element = hawksnap_platform_mac::ax::element_at_point_with_pid(cx, cy);
    let windows = super::candidates::visible_window_rects().unwrap_or_default();
    let displays = crate::commands::captured_display_frames();
    let frame = resolve_hover(element, &windows, &displays, cx, cy, own_pid());
    let _ = app.emit("hawk://hover", frame.map(AxFrameJson::from));
}

/// 当前光标位置（全局逻辑坐标，top-left 原点）。
fn cursor_position() -> Option<(f64, f64)> {
    use core_foundation::base::{CFType, TCFType};

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        /// source 传 nil：合成一个携带当前光标位置的事件（Swift `CGEvent(source: nil)`）。
        fn CGEventCreate(source: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        /// 返回事件位置 = 全局显示器坐标（逻辑点，top-left 原点）。
        fn CGEventGetLocation(event: *const std::ffi::c_void) -> core_graphics::geometry::CGPoint;
    }

    // SAFETY: CGEventCreate 返回 +1 引用的 CF 对象；先查空（wrap 对 null 会 panic），
    // 成功则包成 CFType 托管（Drop 即 CFRelease），任何早退路径不泄漏。
    let ev = unsafe { CGEventCreate(std::ptr::null()) };
    if ev.is_null() {
        return None;
    }
    let owned = unsafe { CFType::wrap_under_create_rule(ev) };
    // SAFETY: CGEvent 本就是 CF 类型，仅借用其指针读取位置字段，不改变所有权。
    let loc = unsafe { CGEventGetLocation(owned.as_CFTypeRef()) };
    Some((loc.x, loc.y))
}

/// Task 6 窗口级回退的点命中（纯逻辑，独立于 CGWindowList 可单测）：返回包含 (x, y)
/// 且面积最小的窗口 rect——面积最小者胜出 controller 裁定，防止全屏级大窗（桌面/
/// 壁纸残留）抢走普通应用窗口的命中；并列时取先出现者（稳定）。包含判定用半开区间
/// [x, x+w)：右/下边缘上的点不属于该窗（标准 hit-test，相邻窗不双重命中）。
/// Task 7：谓词收编 core 的 contains_point_f64（与显示器级命中、i32 Rect 共用同一
/// 经测试的边缘判定）。
fn window_hit(rects: &[RectLog], x: f64, y: f64) -> Option<RectLog> {
    rects.iter().copied()
        .filter(|r| hawksnap_core::geometry::contains_point_f64(x, y, r.x, r.y, r.w, r.h))
        .min_by(|a, b| {
            (a.w * a.h).partial_cmp(&(b.w * b.h)).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Task 7 显示器级回退的点命中（纯逻辑）：包含光标点的候选中取面积最小者——与
/// window_hit 同一裁定（显示器几乎不嵌套，规则保持统一）；半开区间，右/下边缘上的
/// 点不属于该屏（相邻屏不双重命中）。空候选 / 全部落空 → None。
fn display_frame_at(displays: &[DisplayFrame], x: f64, y: f64) -> Option<DisplayFrame> {
    displays.iter().copied()
        .filter(|d| hawksnap_core::geometry::contains_point_f64(x, y, d.x, d.y, d.w, d.h))
        .min_by(|a, b| {
            (a.w * a.h).partial_cmp(&(b.w * b.h)).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// 窗口级 → 显示器级的点回退（Task 7，brief 命名）：命中窗口取最小包含者（role
/// "AXWindow"）；无窗口命中 → 包含光标的最小显示器全帧（role "display"，全屏模式
/// 语义）。次序不可颠倒：显示器全帧若放前会吞掉一切窗口高亮，窗口级回退沦为死代码。
/// role 仅信息性（前端只画框，不按 role 分支）。二者皆空 → None（前端清除高亮）。
fn window_or_display_frame(
    windows: &[RectLog], displays: &[DisplayFrame], x: f64, y: f64,
) -> Option<hawksnap_platform_mac::ax::AxFrame> {
    window_hit(windows, x, y)
        .map(|w| hawksnap_platform_mac::ax::AxFrame {
            x: w.x, y: w.y, w: w.w, h: w.h, role: "AXWindow".into(),
        })
        .or_else(|| {
            display_frame_at(displays, x, y).map(|d| hawksnap_platform_mac::ax::AxFrame {
                x: d.x, y: d.y, w: d.w, h: d.h, role: "display".into(),
            })
        })
}

/// 悬停回退链的纯决策（Task 6 引入、Task 7 收敛为三级，controller 裁定）：外来进程的
/// 元素命中直通；元素为本进程（覆盖层悬停自命中——覆盖层恰是此刻最顶层鼠标接受窗，
/// system-wide 探测会返回自身整屏框，若直通将常驻全屏高亮、单击全选整屏）或探测失败
/// → 窗口级点命中 → 仍无 → 显示器全帧（role "display"）；全部落空 → None。
fn resolve_hover(
    element: Option<(hawksnap_platform_mac::ax::AxFrame, i32)>,
    windows: &[RectLog], displays: &[DisplayFrame], x: f64, y: f64, own_pid: i32,
) -> Option<hawksnap_platform_mac::ax::AxFrame> {
    match element {
        Some((f, pid)) if pid != own_pid => Some(f),
        _ => window_or_display_frame(windows, displays, x, y),
    }
}

/// 本进程 pid 按 AX 的 pid_t（i32）域取值（实际 pid 值远小于 i32::MAX，u32→i32 直接转）。
fn own_pid() -> i32 {
    std::process::id() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_payload_serializes_frame_or_null() {
        // Task 5 契约：hawk://hover payload = Option<AxFrameJson>，扁平 x/y/w/h/role
        // 键（camelCase 对单词字段是无操作，断言防误改）；None 必须序列化为 null——
        // 前端以 null 清除高亮。
        let some = serde_json::to_string(&Some(AxFrameJson {
            x: 1.0, y: 2.0, w: 3.0, h: 4.0, role: "AXButton".into(),
        }))
        .unwrap();
        assert_eq!(some, r#"{"x":1.0,"y":2.0,"w":3.0,"h":4.0,"role":"AXButton"}"#);
        let none = serde_json::to_string(&None::<AxFrameJson>).unwrap();
        assert_eq!(none, "null");
    }

    #[test]
    fn resolve_hover_prefers_foreign_pid_element() {
        // 修复（controller 裁定）：外来进程的元素命中直通——窗口/显示器候选不参与、不覆盖
        let windows = vec![RectLog { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 }];
        let own = i32::from(u16::MAX); // 任一稳定 pid 值即可（纯逻辑不查真实 pid）
        let el = hawksnap_platform_mac::ax::AxFrame {
            x: 10.0, y: 10.0, w: 40.0, h: 20.0, role: "AXButton".into(),
        };
        let got = resolve_hover(Some((el.clone(), own + 1)), &windows, &[], 20.0, 20.0, own).unwrap();
        assert_eq!(got, el);
    }

    #[test]
    fn resolve_hover_own_pid_element_falls_through_to_window_hit() {
        // 自命中（覆盖层自身是此刻最顶层鼠标接受窗，system-wide 探测会返回本进程的
        // 整屏框）：丢弃该元素框，回退窗口级命中（取面积最小者）
        let windows = vec![
            RectLog { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 },
            RectLog { x: 100.0, y: 100.0, w: 400.0, h: 300.0 },
        ];
        let own = i32::from(u16::MAX);
        let own_fullscreen = hawksnap_platform_mac::ax::AxFrame {
            x: 0.0, y: 0.0, w: 1512.0, h: 982.0, role: "AXGroup".into(),
        };
        let hit = resolve_hover(Some((own_fullscreen, own)), &windows, &[], 200.0, 200.0, own).unwrap();
        assert_eq!((hit.w, hit.h, hit.role), (400.0, 300.0, "AXWindow".to_string()));
    }

    #[test]
    fn resolve_hover_none_element_uses_window_hit() {
        // 无元素命中（探测失败/未授权等）→ 纯窗口回退
        let windows = vec![RectLog { x: 5.0, y: 5.0, w: 50.0, h: 40.0 }];
        let hit = resolve_hover(None, &windows, &[], 20.0, 20.0, 1).unwrap();
        assert_eq!((hit.x, hit.y, hit.w, hit.h), (5.0, 5.0, 50.0, 40.0));
        assert_eq!(hit.role, "AXWindow");
    }

    #[test]
    fn resolve_hover_both_miss_returns_none() {
        // 元素与窗口皆未命中、且无显示器候选 → None（前端清除高亮）
        assert!(resolve_hover(None, &[], &[], 5.0, 5.0, 1).is_none());
    }

    #[test]
    fn display_frame_at_prefers_smallest_containing_area() {
        // Task 7 显示器级回退语义：包含光标点的候选中取面积最小者（与 window_hit 同一
        // 裁定，显示器几乎不嵌套但规则保持统一）；半开区间，右/下边缘不属于该屏。
        let displays = vec![
            DisplayFrame { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 },     // 主屏
            DisplayFrame { x: 1512.0, y: 0.0, w: 1920.0, h: 1080.0 }, // 副屏（右侧）
        ];
        assert_eq!(display_frame_at(&displays, 100.0, 100.0).unwrap(), displays[0]);
        assert_eq!(display_frame_at(&displays, 2000.0, 500.0).unwrap(), displays[1]);
        // 相邻屏边界：主屏右边缘 1512 上的点不属于主屏（半开），命中含左边缘的副屏
        assert_eq!(display_frame_at(&displays, 1512.0, 0.0).unwrap(), displays[1]);
        assert!(display_frame_at(&displays, 1511.9, 0.0).is_some());
        // 两屏之外 → None
        assert!(display_frame_at(&displays, -0.5, 500.0).is_none());
        assert!(display_frame_at(&displays, 5000.0, 5000.0).is_none());
        // 空候选 → None（首轮流程前 CAPTURES 为空的形态）
        assert!(display_frame_at(&[], 1.0, 1.0).is_none());
        // 嵌套候选（防御性）：面积最小者胜出
        let nested = vec![
            DisplayFrame { x: 0.0, y: 0.0, w: 3024.0, h: 1964.0 },
            DisplayFrame { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 },
        ];
        assert_eq!(display_frame_at(&nested, 100.0, 100.0).unwrap(), nested[1]);
    }

    #[test]
    fn resolve_hover_display_fallback_when_element_and_window_miss() {
        // 三级回退（Task 7）：元素与窗口皆未命中 → 包含光标的显示器全帧（全屏模式：
        // 悬停空白处高亮整屏，单击即全选本屏），role "display"
        let displays = vec![DisplayFrame { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 }];
        let hit = resolve_hover(None, &[], &displays, 800.0, 900.0, 1).unwrap();
        assert_eq!((hit.x, hit.y, hit.w, hit.h), (0.0, 0.0, 1512.0, 982.0));
        assert_eq!(hit.role, "display");
    }

    #[test]
    fn resolve_hover_window_hit_shadows_display_frame() {
        // 次序守护：窗口命中优先于显示器帧——回退链若把显示器放前，全屏帧会吞掉一切
        // 窗口高亮，窗口级回退沦为死代码
        let windows = vec![RectLog { x: 100.0, y: 100.0, w: 400.0, h: 300.0 }];
        let displays = vec![DisplayFrame { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 }];
        let hit = resolve_hover(None, &windows, &displays, 200.0, 200.0, 1).unwrap();
        assert_eq!((hit.w, hit.h, hit.role), (400.0, 300.0, "AXWindow".to_string()));
        // 光标在窗口外但在屏内 → 才落显示器帧
        let hit = resolve_hover(None, &windows, &displays, 20.0, 20.0, 1).unwrap();
        assert_eq!((hit.w, hit.h, hit.role), (1512.0, 982.0, "display".to_string()));
    }

    #[test]
    fn resolve_hover_own_pid_element_still_falls_through_to_display() {
        // 自命中 + 窗口未命中 → 一路落显示器帧（回退链贯穿三级）
        let displays = vec![DisplayFrame { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 }];
        let own = i32::from(u16::MAX);
        let own_fullscreen = hawksnap_platform_mac::ax::AxFrame {
            x: 0.0, y: 0.0, w: 1512.0, h: 982.0, role: "AXGroup".into(),
        };
        let hit = resolve_hover(Some((own_fullscreen, own)), &[], &displays, 1200.0, 900.0, own).unwrap();
        assert_eq!((hit.w, hit.h, hit.role), (1512.0, 982.0, "display".to_string()));
    }

    #[test]
    fn window_hit_prefers_smallest_containing_area() {
        // Task 6 窗口级回退语义（controller 裁定 1）：包含光标点的候选中取面积最小者——
        // 全屏级大窗（桌面/壁纸层残留）不得抢走普通应用窗口的命中。
        let rects = vec![
            RectLog { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 },    // 全屏窗
            RectLog { x: 100.0, y: 100.0, w: 400.0, h: 300.0 }, // 普通窗（面积更小）
        ];
        // 点在两窗交集内 → 命中面积最小者（普通窗），而非首个/最大者
        let hit = window_hit(&rects, 200.0, 200.0).unwrap();
        assert_eq!((hit.x, hit.y, hit.w, hit.h), (100.0, 100.0, 400.0, 300.0));
        // 点在所有窗之外 → None（handler 据此 emit null 清除高亮）
        assert!(window_hit(&rects, 2000.0, 2000.0).is_none());
        // 小窗右边缘上的点被小窗排除（半开区间 [x, x+w)，标准 hit-test），
        // 但仍在全屏窗内 → 回退命中全屏窗
        assert_eq!(window_hit(&rects, 500.0, 200.0).map(|r| (r.w, r.h)), Some((1512.0, 982.0)));
        // 仅小窗时同一点 → None
        let only_small = &[RectLog { x: 100.0, y: 100.0, w: 400.0, h: 300.0 }];
        assert!(window_hit(only_small, 500.0, 200.0).is_none());
        assert!(window_hit(only_small, 200.0, 400.0).is_none());
        // 左/上边缘（含）属于该窗
        assert!(window_hit(only_small, 100.0, 100.0).is_some());
        // 空候选 → None
        assert!(window_hit(&[], 1.0, 1.0).is_none());
    }
}
