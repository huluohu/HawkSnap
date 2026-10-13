use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

// 热键注册失败的 pull 通道：setup 阶段的 hawk://hotkey-error emit 早于前端 listen 注册，
// 必丢；错误同时落入此静态，前端挂载时经 hotkey_error 命令拉取补显（读取即清除）。
static HOTKEY_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// 截图流程代数（单调递增）：每轮 start_capture_flow 分配一个新代数并写入该轮全部
/// DisplayPayload.flow。overlay-ready 的 300ms 补发线程在每次发射前重查此值——已有
/// 更新一代启动则旧代补发自弃（否则快速连按 F1 时旧轮迟到补发会以陈旧 payload 覆盖
/// 新轮显示，且陈旧 raw 文件已被下一轮删除，画布冻结）。
static FLOW_SEQ: AtomicU64 = AtomicU64::new(0);

// —— Plan B Task 5：智能吸附 AX 悬停探测 ——

/// 悬停探测总闸：是否有任一覆盖层可见。start_capture_flow 在窗口就绪后置 true；
/// hide_all_overlays 隐藏全部覆盖层后落 false。悬停 handler 首行检查此门——
/// 覆盖层全隐时空转成本趋近于零（一次原子读即返回），不烧 CPU、不做 AX 查询。
pub(crate) static ANY_OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);

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

/// Plan B Task 5：智能吸附 AX 悬停探测注册。
///
/// 辅助功能未授予 → 整条链路缺席（裁定 1）：eprintln + `Ok(())`，不装监听、不弹授权窗，
/// 桌面上不留任何提示；拖拽框选与窗口级磁吸不受影响。
/// 必须在主线程调用（事件监听挂在 AppKit 事件循环上，与 register_media_key_f1 同）。
#[cfg(target_os = "macos")]
pub fn register_hover_probe(app: &AppHandle) -> Result<(), String> {
    if !hawksnap_platform_mac::ax::ax_trusted() {
        eprintln!("[hawksnap] 未授予辅助功能：悬停元素高亮不可用（拖拽框选不受影响）");
        return Ok(());
    }
    hover::install_hover_monitors(app);
    Ok(())
}

/// 悬停探测监听（macOS 专属，与 Task 13 媒体键监听同一 NSEvent 模式）。
#[cfg(target_os = "macos")]
mod hover {
    use super::AppHandle;
    use std::ptr::NonNull;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

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
        let windows = super::visible_window_rects().unwrap_or_default();
        let displays = crate::commands::captured_display_frames();
        let frame = super::resolve_hover(element, &windows, &displays, cx, cy, super::own_pid());
        let _ = app.emit("hawk://hover", frame.map(super::AxFrameJson::from));
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
}

// 仅测试用：镜像 commands::DisplayPayload 的序列化契约（camelCase），供下方契约测试断言
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPayloadForTest {
    pub display_id: u64, pub x: i32, pub y: i32,
    pub width: u32, pub height: u32, pub scale: f64, pub frame_url: String,
    pub flow: u64,
}

pub fn register_hotkeys(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut};
    let gs = app.global_shortcut();
    let f1 = Shortcut::new(None, Code::F1);
    let app2 = app.clone();
    // on_shortcut 注册快捷键并挂处理器（勿再单独 register：同一组合二次注册会被 macOS
    // 以 eventHotKeyExistsErr 拒绝）。热键回调在主线程（Carbon 事件）触发，而窗口
    // 建窗/关窗类操作禁止在主线程做（tauri-runtime-wry 在主线程处理 Close 消息会直接
    // panic），故整个流程放到独立线程执行，热键处理立即返回。
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
    // 修复（终审 wave）：手动 F1 撞上进行中的延时截图倒计时时，先置中止旗标再走本流程
    // ——延时 worker 的轮询循环每 ≤50ms 检查 DELAY_ABORT，见真即隐藏倒计时窗、清
    // DELAY_ACTIVE、直接返回（跳过它自己到点的 start_capture_flow）。否则倒计时归零与
    // 手动 F1 各触发一轮截图，用户连得两轮覆盖层。只置旗标不等待：中止由 worker 单点
    // 负责（与 cancel_delayed_capture 同款分工），本流程立即继续。
    if DELAY_ACTIVE.load(Ordering::SeqCst) {
        DELAY_ABORT.store(true, Ordering::SeqCst);
    }
    // Task 11 埋点：热键/托盘共用此入口，首行即触发时刻（stage: hotkey）；
    // mark_hotkey 同时武装 overlay_shown 单发门闩（多显示器只记首个 shown，与 hotkey 1:1）
    crate::perf::mark_hotkey();
    let mut displays = tauri::async_runtime::block_on(commands::capture_screens(app.clone()))?;
    crate::perf::mark("grab_done"); // 抓屏成功返回
    // Plan B Task 1（G3-1）：覆盖层窗口常驻复用——首轮 build（webview 冷加载 ~267ms），
    // 之后每轮仅 set_position/set_size/show，页面由 overlay-ready 的新 payload 重绘，
    // 砍掉每轮的冷加载段。show/hide/set_position 都是投递给主线程事件循环的消息，
    // 非主线程调用安全（close 特殊，见 hide_all_overlays 注）。
    for d in &displays {
        let label = format!("overlay-{}", d.display_id);
        if let Some(w) = app.get_webview_window(&label) {
            // 复用：窗口与页面常驻，仅更新几何并显示。页面经 overlay-ready 的新 payload 重绘。
            let _ = w.set_position(tauri::LogicalPosition::new(d.x as f64, d.y as f64));
            let _ = w.set_size(tauri::LogicalSize::new(d.width as f64, d.height as f64));
            let _ = w.show();
            let _ = w.set_focus();
            continue;
        }
        // 首建路径（防御：复用模式下同 label 不会重建，除非窗口被外部强关）。
        // 旧实现的 close 旧窗 + 500ms 轮询等注销块已删除——复用模式下不存在“重建活 label”。
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
    crate::perf::mark("overlay_created"); // 覆盖层窗口就绪（build 或复用 show 完成；每轮一次，便于脚本按索引配对）
    // Plan B Task 5：覆盖层已就绪 → 开启悬停探测总闸（handler 首行检查，见
    // ANY_OVERLAY_VISIBLE 注）；hide_all_overlays 隐藏全部覆盖层后落下。
    ANY_OVERLAY_VISIBLE.store(true, Ordering::SeqCst);
    // overlay-ready 在窗口 overlay 路由的前端挂载前可能先到（首次 emit 会丢）。
    // 简化实现：共发 3 次、间隔 150ms，前端以收到的最后一次为准（Task 9 契约）。
    // 重复发送放在独立线程，避免热键处理被拖慢 450ms。
    //
    // 修复（终审 wave，代数守卫）：本轮分配新代数写入全部 payload。补发线程 spawn 时
    // 捕获 my_flow，每次发射前重查 FLOW_SEQ——期间用户已触发新一轮（代数已推进）→
    // 旧代剩余补发自弃。若无此守卫，快速连按 F1 时旧轮迟到补发会以陈旧 payload 覆盖
    // 新轮显示，而陈旧 payload 的 raw 文件已被新一轮 capture_screens 删除（画布冻结），
    // 且 payload 到达触发的 reset() 会杀掉用户新一轮进行中的拖拽。前端按 payload.flow
    // 丢弃旧代（index.tsx 的 maxFlowRef），双端防线互补。
    let flow = FLOW_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    for d in displays.iter_mut() {
        d.flow = flow;
    }
    let app2 = app.clone();
    let repeats = displays.clone();
    let my_flow = flow;
    std::thread::spawn(move || {
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(150));
            if FLOW_SEQ.load(Ordering::SeqCst) != my_flow {
                return; // 已有更新一代流程启动：本线程的剩余补发全部作废
            }
            let _ = app2.emit("hawk://overlay-ready", repeats.clone());
        }
    });
    app.emit("hawk://overlay-ready", displays).map_err(|e| e.to_string())
}

use crate::commands::{self, DisplayFrame};

// —— Task 9 薄命令 ——

/// 覆盖层取消：隐藏**全部**覆盖层（Task 7 多显示器语义收敛——任一屏 Esc，各屏覆盖层
/// 一致隐藏，不再残留他屏覆盖层；单显示器行为不变）。displayId 保留为前端 invoke 契约
/// （Tauri 命令按参数名匹配），隐藏语义不再限于发起窗口。
#[tauri::command]
pub async fn cancel_capture(app: AppHandle, display_id: u64) -> Result<(), String> {
    let _ = display_id; // 契约参数，语义见上
    hide_all_overlays(&app)
}

/// 覆盖层确认完成：广播 capture-done（payload = outcome，契约不变）后隐藏**全部**
/// 覆盖层（Task 7）。CaptureOutcome 不携带 displayId，无法走“setup 监听再关窗”链，
/// 故由本命令自带目标（Task 9 裁定 2，跳过 setup 监听）。
#[tauri::command]
pub async fn emit_capture_done(app: AppHandle, display_id: u64, outcome: commands::CaptureOutcome) -> Result<(), String> {
    app.emit("hawk://capture-done", &outcome).map_err(|e| e.to_string())?;
    let _ = display_id; // 契约参数，隐藏语义收敛为全部覆盖层（Task 7）
    hide_all_overlays(&app)
}

/// 常驻复用：隐藏而非关闭，窗口由 start_capture_flow 复用（Plan B Task 1）。命令在
/// 异步运行时线程执行（非主线程），hide() 是投递给主线程事件循环的消息，安全
/// （close 须避开主线程，见 register_hotkeys 注；此处本就非主线程，hide 更无此顾虑）。
///
/// 迭代安全（self-review 结论，无需先收集 label）：`webview_windows()` 返回自持的
/// owned HashMap，且 hide 只是投递异步消息——不在此调用内同步增删窗口集合，迭代器
/// 不失效；即便窗口被并发外部强关，手里仍是 owned 句柄，对其 hide 优雅返回 Err 而非
/// panic。单窗 hide 失败不中断其余窗口（记最后一个错误返回）。
///
/// 悬停总闸：全部 overlay-* 均已（请求）隐藏 → 直接落 false，无需再实查可见性。
/// 与 start_capture_flow 并发的极端竞态（新流程 store(true) 后旧取消才 store(false)）
/// 可自愈：下一轮流程会重新置 true。
fn hide_all_overlays(app: &AppHandle) -> Result<(), String> {
    let mut last_err: Option<String> = None;
    for (label, w) in app.webview_windows() {
        if label.starts_with("overlay-") {
            if let Err(e) = w.hide() {
                last_err = Some(e.to_string());
            }
        }
    }
    ANY_OVERLAY_VISIBLE.store(false, Ordering::SeqCst);
    // 修复（终审 wave，G2）：流程到此结束（本时刻起没有任何覆盖层能再确认），释放
    // CAPTURES 钉住的整帧 RGBA（~24-59MB/屏）——此前它常驻至下一轮覆盖，是空载残余
    // 内存的主力。frame_path 保留（下一轮 capture_screens 删除上一轮冻结帧仍需要）。
    // 释放后对已释放条目调 finish_capture 会拿到空 rgba 而报「display not captured」
    // 式错误——可接受：本函数恰在没有任何覆盖层能确认的时刻执行，唯一能走到该错误的
    // 是早已越过代际守卫的僵尸链。锁内只做 Arc 替换，无 IO，锁窗口极短。
    commands::release_captures();
    last_err.map_or(Ok(()), Err)
}

// —— Plan B Task 8：延时截图（托盘子菜单 + 倒计时浮窗） ——

/// 延时进行中门闩（静态状态机，与 HOTKEY_ERROR 同为 static）：try_begin_delay 独占置位，
/// worker 到点/中止时清零。重复启动经此拒绝（「已有延时截图进行中」）。
static DELAY_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 中止旗标：cancel_delayed_capture 置 true，worker 循环每轮（≤50ms）检查——真则隐藏
/// 倒计时窗、清门闩、不截图。每轮 start_delayed_capture 在取门闩前复位（先清后取：
/// 清后至取闩间落下的 cancel 只会留 true 或判非活动期拒绝，不会像先取后清那样被本轮
/// 复位静默抹掉）。
static DELAY_ABORT: AtomicBool = AtomicBool::new(false);

/// 最近一次广播的剩余秒数（pull 通道）：倒计时窗口首建时 webview 冷加载（~267ms），
/// 首条 hawk://countdown emit 早于页面 listen 必丢——与 HOTKEY_ERROR 同一问题形态。
/// 页面 mount 时经 countdown_remaining 命令拉取补显（仅 DELAY_ACTIVE 时有效）。
static DELAY_REMAINING: AtomicU64 = AtomicU64::new(0);

/// hawk://countdown payload：剩余整秒数。secs → 1 逐秒广播，{remaining: 0} 在触发
/// 截图前一瞬间发出。单词字段，无需 rename（与 AxFrameJson 同款 derive 保持一致）。
#[derive(Serialize, Clone)]
struct CountdownPayload {
    remaining: u64,
}

/// worker 轮询周期：终止判定误差上界 = 一个周期（实际经「末段向 deadline 对齐」的
/// sleep 裁剪后为亚毫秒过冲，稳拿 spec §3.1 F4 的 ≤100ms 验收）。
const DELAY_POLL: Duration = Duration::from_millis(50);

/// 延时时长白名单（spec §3.1：1/2/3/5 秒可选）：纯逻辑可测，通过则原样返回。
fn validate_delay_secs(secs: u64) -> Result<u64, String> {
    matches!(secs, 1 | 2 | 3 | 5).then_some(secs).ok_or_else(|| "仅支持 1/2/3/5 秒".into())
}

/// 门闩独占（纯逻辑可测）：swap(true) 前值为 false 才算抢到——进行中重复启动返回 false。
fn try_begin_delay(flag: &AtomicBool) -> bool {
    !flag.swap(true, Ordering::SeqCst)
}

/// 倒计时浮窗（常驻复用，与 overlay 窗口同款「一次创建、show/hide」模式）：
/// 200×120 逻辑尺寸，无边框、置顶、不可缩放、skip_taskbar、非透明（实心深色底，
/// 透明只为 overlay 冻结帧服务）。
///
/// 位置裁定（controller）：不指定 position 先建、再 `center()`——CAPTURES 首entry
/// 定位 / 硬编码分辨率居中均被否，center() 由 OS 在窗口当前所在屏居中，稳妥零假设。
/// 首建以 visible(false) 起建，center 完成再 show，避免窗口先闪现在默认位置再跳心。
/// 必须在非主线程调用（建窗/show 均为投递给主线程事件循环的消息，close 才有主线程
/// 禁忌，但本函数从 start_delayed_capture 进入，本就运行在 worker 线程）。
fn show_countdown_window(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("countdown") {
        // 复用：页面与监听常驻，仅重新居中并显示（窗口 hide 期间事件照常接收）。
        let _ = w.center();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let win = WebviewWindowBuilder::new(app, "countdown", WebviewUrl::App("countdown?window=countdown".into()))
        .inner_size(200.0, 120.0)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .transparent(false) // 实心深色底（brief：非透明；前端自绘 #0D1522）
        .visible(false)     // 先建后 center 再 show，杜绝默认位置闪现
        .build().map_err(|e| e.to_string())?;
    let _ = win.center();
    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

fn hide_countdown_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("countdown") {
        let _ = w.hide();
    }
}

/// 启动延时截图（spec §3.1）：校验 → 门闩独占 → 建/复用倒计时窗 → deadline 轮询。
///
/// 必须在非主线程调用：本命令标记 `async`，前端 invoke 由 tauri 派到异步运行时线程
/// 执行（不带 async 的 sync 命令在 IPC 线程内联运行——macOS/wry 即主线程，整段倒计时
/// 会冻结主线程，且建窗/show 落在禁忌线程）；托盘菜单处理函数本身跑在主线程，故先
/// spawn 平台线程再进入。两路径都满足非主线程约束（start_capture_flow 的主线程禁忌
/// 同源）。
///
/// 时序（controller 裁定 1，deadline-based）：`deadline = Instant::now() + secs`，
/// worker 每 ≤50ms 检查 DELAY_ABORT 与剩余整秒；整秒变化才 emit {remaining: n}
/// （ceil 语义：剩余 2.95s 显示 3——「3、2、1」各占满一秒后归零），{remaining: 0}
/// 在触发截图前一瞬发出。到点顺序固定：emit 0 → 隐藏窗口 → 清门闩 → start_capture_flow
/// （已在 worker 线程，安全；失败走 hawk://capture-error，与托盘 shot 同口径）。
/// 中止（cancel 置 ABORT）：隐藏窗口、清门闩、不截图。
#[tauri::command(async)]
pub fn start_delayed_capture(app: AppHandle, secs: u64) -> Result<(), String> {
    let secs = validate_delay_secs(secs)?;
    // 先清 ABORT 再取门闩：若先取闩后清，「取闩到清之间」落下的 cancel 已把 ABORT 置
    // true 却被本轮复位覆盖——cancel 被静默抹掉，倒计时照跑。先清则该窗口内的 cancel
    // （门闩未取，cancel_delayed_capture 判非活动期返回 Err，良性）不会残留到本轮。
    DELAY_ABORT.store(false, Ordering::SeqCst);
    if !try_begin_delay(&DELAY_ACTIVE) {
        return Err("已有延时截图进行中".into());
    }
    // 建窗失败必须归还门闩：worker 尚未存在，无人会在到点/中止时清 DELAY_ACTIVE，
    // 漏还则此后所有延时启动永远撞「已有延时截图进行中」直至重启。
    if let Err(e) = show_countdown_window(&app) {
        DELAY_ACTIVE.store(false, Ordering::SeqCst);
        return Err(e);
    }
    DELAY_REMAINING.store(secs, Ordering::SeqCst);
    // 初始补发（overlay-ready 同款连发）：首建的 webview 冷加载期早于页面 listen，
    // t=0 的 emit 必丢；1 秒档尤其依赖补发——该档整秒变化只在 0 出现，丢了初始值整轮黑屏。
    // 同值重复发射对显示无副作用（隐藏窗口期间监听照常在线）。
    let app2 = app.clone();
    std::thread::spawn(move || {
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(150));
            let _ = app2.emit("hawk://countdown", CountdownPayload { remaining: secs });
        }
    });
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut emitted = secs;
    loop {
        if DELAY_ABORT.load(Ordering::SeqCst) {
            hide_countdown_window(&app);
            DELAY_ACTIVE.store(false, Ordering::SeqCst);
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = (deadline - now).as_secs_f64().ceil() as u64;
        if remaining != emitted {
            emitted = remaining;
            DELAY_REMAINING.store(remaining, Ordering::SeqCst);
            let _ = app.emit("hawk://countdown", CountdownPayload { remaining });
        }
        // 末段向 deadline 对齐（sleep = min(50ms, 距 deadline)）：触发点过冲从至多
        // 50ms 收紧到亚毫秒级；saturating 兜底已过 deadline 的极端分支（下一轮即 break）。
        let to_deadline = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(to_deadline.min(DELAY_POLL));
    }
    // 到点：{remaining: 0}（触发前一瞬间）→ 隐藏 → 清门闩 → 触发截图。清门闩先于
    // start_capture_flow：捕获流程自身不碰 DELAY 状态，先放行才不会因流程耗时卡住
    // 用户下一步延时入口。
    DELAY_REMAINING.store(0, Ordering::SeqCst);
    let _ = app.emit("hawk://countdown", CountdownPayload { remaining: 0 });
    hide_countdown_window(&app);
    DELAY_ACTIVE.store(false, Ordering::SeqCst);
    // 与托盘 shot 路径同口径：失败走 hawk://capture-error，不静默吞掉
    if let Err(e) = start_capture_flow(&app) {
        let _ = app.emit("hawk://capture-error", e);
    }
    Ok(())
}

/// 取消进行中的延时截图：仅原子置 ABORT，worker ≤50ms 内隐藏窗口并清门闩（隐藏/清门闩
/// 均由 worker 单点负责，本命令与其无共享可变状态）。非活动期调用返回 Err——托盘
/// 「取消倒计时」恒可用（不做动态 enable/disable），良性竞态由调用方 eprintln 吞掉。
#[tauri::command]
pub fn cancel_delayed_capture() -> Result<(), String> {
    if !DELAY_ACTIVE.load(Ordering::SeqCst) {
        return Err("没有进行中的延时截图".into());
    }
    DELAY_ABORT.store(true, Ordering::SeqCst);
    Ok(())
}

/// 倒计时剩余秒数 pull 通道（页面 mount 时补显，见 DELAY_REMAINING 注）：None = 无
/// 进行中的延时。与 push 事件互补，契约不变。
#[tauri::command]
pub fn countdown_remaining() -> Option<u64> {
    DELAY_ACTIVE
        .load(Ordering::SeqCst)
        .then(|| DELAY_REMAINING.load(Ordering::SeqCst))
}

// —— Task 4：方向键磁吸的窗口候选 ——

/// 磁吸候选窗口 rect（逻辑坐标，全 f64）。与 hawksnap_core::geometry::Rect（i32/u32）
/// 同形但独立定义：候选来自 CGWindowList（f64 逻辑坐标），且前端磁吸走自己的
/// 「最近单边」语义（geometry.ts），不经 Rust snap_to——两套语义并存、互不调用。
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct RectLog { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }

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
fn visible_window_rects() -> Result<Vec<RectLog>, String> {
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
fn visible_window_rects() -> Result<Vec<RectLog>, String> { Ok(vec![]) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_serializes_camel_case() {
        let p = DisplayPayloadForTest {
            display_id: 7, x: 0, y: 0, width: 100, height: 50, scale: 2.0,
            frame_url: "hawk-grab:///tmp/a.raw".into(),
            flow: 3,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"displayId\":7"));
        // Task 2b：冻结帧改为原始 RGBA 文件，payload 字段随之更名 pngUrl → frameUrl
        assert!(s.contains("\"frameUrl\":\"hawk-grab:///tmp/a.raw\""));
        // 修复（终审 wave）：payload 携带流程代数（单词字段 camelCase 无操作），
        // 前端按 flow 丢弃旧代迟到补发（代数守卫）
        assert!(s.contains("\"flow\":3"));
    }

    #[test]
    fn rect_log_serializes_flat_xywh_keys() {
        // Task 4 契约：snap_candidates 的 rect 为扁平 x/y/w/h 数字键（camelCase rename
        // 对单字母字段是无操作，但显式断言防止未来误改字段名或嵌套成 {x:{...}}）；
        // 前端 Rect 类型按同名结构消费。
        let r = RectLog { x: 1.5, y: 2.0, w: 300.0, h: 4.0 };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, r#"{"x":1.5,"y":2.0,"w":300.0,"h":4.0}"#);
    }

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

// —— Plan B Task 8：延时截图纯逻辑测试 ——
#[cfg(test)]
mod delay_tests {
    use super::*;

    #[test]
    fn delay_secs_validated() {
        assert_eq!(validate_delay_secs(3), Ok(3));
        assert!(validate_delay_secs(4).is_err());
        assert!(validate_delay_secs(0).is_err());
    }

    #[test]
    fn delay_gate_is_exclusive() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        assert!(try_begin_delay(&flag));
        assert!(!try_begin_delay(&flag)); // 进行中拒绝
        flag.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(try_begin_delay(&flag));
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
