//! C5：贴图 Pin——原生 NSWindow + NSImageView 窗口族（无 webview）。
//!
//! 架构裁定（plan C5 + 控制器细化）：Snipaste 同款——每张贴图一个 borderless NSWindow，
//! 内容为铺满窗口的 NSImageView。8 张贴图零 webview 成本（对比每贴 ~30MB 的 webview
//! 方案直接击穿 G2 ≤80MB），缩放/透明度/拖拽全是窗口原生操作：
//! - 拖拽移动 = `isMovableByWindowBackground`（mouseDown 不覆写 → 父类默认路径保留拖拽）；
//! - 滚轮缩放 / Ctrl+滚轮透明度 = `HawkPinView` 覆写 `scrollWheel:`（define_class）；
//! - Esc 关闭聚焦贴图 = `HawkPinView` 覆写 `keyDown:`（仅 `isKeyWindow` 时），经
//!   [`set_on_escape`] 注入的 fn 指针回调 app 层（platform-mac 不反向依赖 app crate）；
//! - 双击 = `mouseUp:` 覆写仅 eprintln（编辑器归 M2；plan 的「闪烁反馈」有意省略）。
//!
//! 线程纪律：AppKit 窗口的一切消息只在主线程。创建由 app 层 `run_on_main_thread` 交付；
//! 滚轮/按键回调本就在主线程事件循环上；关闭（close/Drop）纪律见 [`PinWindow`]。
//! 注意这与 wry/tauri webview 窗口的「禁止主线程建窗」相反——那是 wry 消息循环的约束，
//! 原生 AppKit 窗口恰恰必须主线程。
//!
//! 绑定版本：objc2 0.6（define_class!，非 tao 0.37 的 ClassBuilder 命令式风格）、
//! objc2-app-kit 0.3.2、objc2-core-graphics 0.3.2。与 Task 5/13 的 extern/declare
//! 模式同源；`NSWindow.setActivationBehavior`（控制器提法的 NSWindowActivatable::
//! Accessory）在 objc2-app-kit 0.3.2 未生成绑定——本应用的 Accessory 激活策略
//! （lib.rs `ActivationPolicy::Accessory`）在应用层提供等价保证：贴图永不把 HawkSnap
//! 变成活跃应用、不抢菜单栏；窗口仍可成为 key（否则 Esc 不可达）。

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSFloatingWindowLevel, NSBackingStoreType, NSImage, NSImageScaling, NSImageView, NSEvent,
    NSEventModifierFlags, NSScreen, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo, CGBitmapInfo,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};

/// 缩放显示比范围：0.1..=3.2（= 10%–3200%，F3 验收）。1.0 = 图像原始尺寸
/// （1 物理像素/1 逻辑像素）；初始显示比 0.5（plan：半尺寸起步，Snipaste 式）。
const ZOOM_MIN: f64 = 0.1;
const ZOOM_MAX: f64 = 3.2;
/// 滚轮每档缩放/透明度步长（≈10%）。方向随 deltaY 符号。
const WHEEL_STEP: f64 = 1.1;
/// 透明度范围 0.2..=1.0（20%–100%，F3 验收）。
const OPACITY_MIN: f64 = 0.2;
/// Esc 的 keyCode（kVK_Escape = 53，Carbon/HIToolbox 沿用值）。
const KEYCODE_ESCAPE: i64 = 53;

// —— Esc 关闭钩子（platform-mac ← app 层注入，避免反向依赖） ——

/// Esc 关闭回调：`fn(u64)`（pin_id）。fn 指针 Copy + Send，语义固定为「按 id 移除」
/// （app 层 remove_pin_on_main），无需携带状态。仅在主线程被调用（keyDown 回调内）。
static ON_ESCAPE: Mutex<Option<fn(u64)>> = Mutex::new(None);

/// app 层在首个贴图创建前注入（幂等，重复覆盖无害）。
pub fn set_on_escape(f: fn(u64)) {
    *ON_ESCAPE.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

fn escape_hook() -> Option<fn(u64)> {
    *ON_ESCAPE.lock().unwrap_or_else(|e| e.into_inner())
}

// —— ObjC 子类 ——

/// `HawkPinView`（NSImageView 子类）的实例变量。Cell 包装：AppKit 回调以 `&self` 到达，
/// define_class 不允许 `&mut self` 覆写（objc2 0.6 interior mutability 纪律）。
struct PinViewIvars {
    /// 当前显示比（1.0 = 原始尺寸），滚轮缩放与命令缩放共用同一状态源。
    scale: Cell<f64>,
    /// 当前透明度，Ctrl+滚轮与命令设置共用。
    opacity: Cell<f64>,
}

define_class!(
    /// NSImageView 子类：滚轮缩放/Ctrl+滚轮透明度、Esc 关闭、双击反馈。
    /// mouseDown 刻意不覆写——父类默认路径保留 `isMovableByWindowBackground` 拖拽。
    #[unsafe(super(NSImageView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PinViewIvars]
    struct HawkPinView;

    impl HawkPinView {
        /// 成为 first responder：keyWindow 的按键事件才能沿响应链到达 keyDown。
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let dy = event.deltaY();
            if dy == 0.0 {
                return; // 纯横向滚动不参与
            }
            let step = if dy > 0.0 { WHEEL_STEP } else { 1.0 / WHEEL_STEP };
            let ivars = self.ivars();
            if event.modifierFlags().contains(NSEventModifierFlags::Control) {
                // Ctrl+滚轮：透明度（20%–100%）
                let next = (ivars.opacity.get() * step).clamp(OPACITY_MIN, 1.0);
                ivars.opacity.set(next);
                if let Some(win) = self.window() {
                    win.setAlphaValue(next);
                }
            } else {
                // 滚轮：以窗口中心缩放（10%–3200%）
                let cur = ivars.scale.get();
                let target = (cur * step).clamp(ZOOM_MIN, ZOOM_MAX);
                if target == cur {
                    return;
                }
                if let Some(win) = self.window() {
                    set_frame_around_center(&win, target / cur);
                }
                ivars.scale.set(target);
            }
            // 刻意不调 super：NSImageView/NSView 对 scrollWheel 无有意义默认行为
            // （误入响应链上游只会触发系统滚动噪声）。
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            // 控制器裁定：仅当本贴图是 key window 时 Esc 生效（keyDown 本就只在 key
            // window 投递，此处为双保险，防御未来响应链变更）。
            if event.keyCode() as i64 == KEYCODE_ESCAPE {
                let is_key = self.window().map(|w| w.isKeyWindow()).unwrap_or(false);
                if is_key {
                    if let Some(hook) = escape_hook() {
                        if let Some(id) = self.pin_id() {
                            hook(id);
                            return; // 已交由 app 层关闭，不再下行
                        }
                    }
                }
            }
            // SAFETY: 非 Esc（或钩子/窗口信息缺失）交还父类默认处理；self 是 AppKit
            // 投递的合法接收者，selector 与父类签名一致。
            unsafe { msg_send![super(self), keyDown: event] }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if event.clickCount() == 2 {
                // plan C5 的「双击闪烁反馈」简化为 eprintln（M1 无编辑器，M2 再议交互）。
                eprintln!("[hawksnap] 贴图双击：编辑器将在后续版本提供");
            }
            // SAFETY: 必须交还父类（保持 movableByWindowBackground 的点击语义完整）。
            unsafe { msg_send![super(self), mouseUp: event] }
        }
    }
);

impl HawkPinView {
    /// 所属贴图 id（经 window 下轔回 HawkPinWindow 读 ivar）；不在贴图窗口树 → None。
    fn pin_id(&self) -> Option<u64> {
        let win = self.window()?;
        win.downcast::<HawkPinWindow>().ok().map(|w| w.ivars().pin_id.get())
    }
}

/// `HawkPinWindow`（NSWindow 子类）的实例变量。
struct PinWinIvars {
    /// 所属贴图 id（app 层 PINS map 的键，Esc 关闭回调用）。
    pin_id: Cell<u64>,
}

define_class!(
    /// NSWindow 子类：唯一覆写 canBecomeKeyWindow——AppKit 默认对 borderless 窗口返回
    /// NO（NSWindow.h：`canBecomeKeyWindow` 对无 titlebar 窗口拒绝），Esc 将永不可达。
    /// 覆写为 YES 后贴图可成为 key window；应用是 Accessory 激活策略，成为 key 不会
    /// 抢走用户当前活跃应用的菜单栏/焦点（详见模块注释「不抢焦点」）。
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PinWinIvars]
    struct HawkPinWindow;

    impl HawkPinWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }
    }
);

// —— 对外句柄 —--

/// 一张贴图窗口的 Rust 侧句柄：持有窗口 + 内容视图的 Retained 与关闭状态。
///
/// SAFETY（Send）: PinWindow 仅作为**句柄**跨线程存放（app 层 `PINS` map 需要静态
/// 容器里的字段满足 Send；NSWindow/NSImageView 是 MainThreadOnly，Retained 本身
/// 非 Send）。纪律：对句柄的一切解引用（AppKit 消息）只在主线程发生——
/// 创建/命令操作经 `run_on_main_thread`，Esc 路径在 AppKit 主线程回调内，
/// Drop（release）只发生在主线程的移除路径上（app 层 remove_pin_on_main /
/// run_on_main_thread 闭包内）。搬运本身不解引用，安全。
pub struct PinWindow {
    window: Retained<HawkPinWindow>,
    view: Retained<HawkPinView>,
    /// close() 幂等闸：置位后 Drop 不再触碰 AppKit（Esc 路径已先行 close）。
    closed: Cell<bool>,
}

// SAFETY: 见结构体注释——仅句柄搬运，解引用全部约束在主线程。
unsafe impl Send for PinWindow {}

impl PinWindow {
    /// 创建贴图窗口。**必须经 `run_on_main_thread` 调用**（内部断言主线程）。
    ///
    /// - `rgba`：物理像素 RGBA（premultiplied 语义，屏幕捕获缓冲即此格式）；
    /// - `top_left`：窗口**左上角**的全局逻辑坐标（左上原点，与 AX/CGEvent 同系；
    ///   内部翻转到 AppKit 左下原点屏幕坐标）；
    /// - `scale`：源显示器的逻辑/物理比（Retina 2.0），决定图像逻辑尺寸与初始半尺寸；
    /// - `pin_id`：app 层分配的贴图 id（Esc 回调回传）。
    pub fn create(
        rgba: Arc<Vec<u8>>,
        w_px: u32,
        h_px: u32,
        top_left: (f64, f64),
        scale: f64,
        pin_id: u64,
    ) -> Result<PinWindow, String> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| "PinWindow::create 必须在主线程调用（run_on_main_thread）".to_string())?;
        if w_px == 0 || h_px == 0 {
            return Err("贴图尺寸为空".into());
        }
        if rgba.len() != w_px as usize * h_px as usize * 4 {
            return Err(format!(
                "RGBA 长度不匹配: {} != {w_px}×{h_px}×4",
                rgba.len()
            ));
        }
        let scale = if scale > 0.0 { scale } else { 1.0 };
        // 图像逻辑尺寸（1 物理像素/1 逻辑像素的基线）与初始显示尺寸（半尺寸起步）。
        let logical_w = w_px as f64 / scale;
        let logical_h = h_px as f64 / scale;
        let init_w = logical_w * 0.5;
        let init_h = logical_h * 0.5;

        let cg_image = make_cg_image(&rgba, w_px, h_px)?;
        let image = NSImage::initWithCGImage_size(
            NSImage::alloc(),
            &cg_image,
            NSSize { width: logical_w, height: logical_h },
        );

        // 窗口：borderless + floating level + 全 Space 可见；不透明（内容本铺满）。
        let win_rect = NSRect {
            origin: NSPoint { x: top_left.0, y: top_left.1 - init_h },
            size: NSSize { width: init_w, height: init_h },
        };
        // SAFETY: set_ivars 先于父类 designated initializer（objc2 define_class 的
        // ivars 初始化契约：alloc → set_ivars → super init）；参数均为本地合法值。
        let this = HawkPinWindow::alloc(mtm).set_ivars(PinWinIvars {
            pin_id: Cell::new(pin_id),
        });
        let window: Retained<HawkPinWindow> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: win_rect,
                styleMask: NSWindowStyleMask::Borderless,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        };
        // floating level（=3）：悬浮于普通窗口之上、低于系统层（覆盖层/status）。plan
        // 给了 Floating/StatusGrid 两选，取 floating——贴图不该盖过本应用自己的覆盖层。
        window.setLevel(NSFloatingWindowLevel);
        // 拖拽移动：点按窗口任意处拖动（依赖 HawkPinView 不覆写 mouseDown）。
        window.setMovableByWindowBackground(true);
        // 全 Space 可见 + 允许出现在全屏应用之上（Snipaste 语义：贴图跟着桌面走）。
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        window.setAcceptsMouseMovedEvents(true);
        window.setOpaque(true);
        // SAFETY: 默认 true 会在 close() 时释放窗口对象，与 Rust 侧 Retained 形成
        // double-release；改 false 后 close() 仅离屏，对象生命周期完全由 PinWindow
        // 的 Retained 管理（Drop 统一 close + release）。
        unsafe { window.setReleasedWhenClosed(false) };

        // 内容视图：铺满窗口的 NSImageView，等比缩放（ProportionallyUpOrDown）。
        // SAFETY: 同上——alloc → set_ivars → super initWithFrame（NSView 的
        // designated initializer），帧与窗口内容区一致。
        let view_alloc = HawkPinView::alloc(mtm).set_ivars(PinViewIvars {
            scale: Cell::new(0.5),
            opacity: Cell::new(1.0),
        });
        let view: Retained<HawkPinView> = unsafe {
            msg_send![
                super(view_alloc),
                initWithFrame: NSRect {
                    origin: NSPoint { x: 0.0, y: 0.0 },
                    size: NSSize { width: init_w, height: init_h },
                },
            ]
        };
        view.setImage(Some(&image));
        view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        window.setContentView(Some(&view));

        // 位置：top_left（左上原点全局逻辑）→ AppKit 左下原点。翻转需主屏高度
        // （AppKit 屏幕坐标系原点在主屏左下）；主屏缺席（极端）时按值直用。
        if let Some(screen) = NSScreen::mainScreen(mtm) {
            let screen_h = screen.frame().size.height;
            window.setFrameTopLeftPoint(NSPoint { x: top_left.0, y: screen_h - top_left.1 });
        } else {
            window.setFrameTopLeftPoint(NSPoint { x: top_left.0, y: top_left.1 });
        }
        // 不抢焦点地显示（Accessory 应用 + orderFrontRegardless：不出现在焦点轮转里）。
        window.orderFrontRegardless();

        Ok(PinWindow { window, view, closed: Cell::new(false) })
    }

    /// 当前显示比（命令侧 clamp 的现状输入；滚轮缩放也写同一 ivar）。
    pub fn scale(&self) -> f64 {
        self.view.ivars().scale.get()
    }

    /// 绝对缩放到 target（0.1..=3.2 由调用方 clamp；滚轮路径内部自带同一 clamp）。
    pub fn zoom_to(&self, target: f64) {
        let cur = self.scale();
        if !(target > 0.0) || target == cur || cur <= 0.0 {
            return;
        }
        set_frame_around_center(&self.window, target / cur);
        self.view.ivars().scale.set(target);
    }

    /// 透明度（0.2..=1.0 由调用方 clamp）。ivar 同步：滚轮 Ctrl+滚轮路径读同一状态。
    pub fn set_opacity(&self, opacity: f64) {
        self.window.setAlphaValue(opacity);
        self.view.ivars().opacity.set(opacity);
    }

    /// 关闭（离屏 + close）。幂等：Esc 已关过则 no-op。仅主线程调用。
    pub fn close(&self) {
        if self.closed.get() {
            return;
        }
        self.closed.set(true);
        self.window.orderOut(None);
        // releasedWhenClosed=false（create 里设置）：close 仅离屏，对象由 Retained 管理。
        self.window.close();
    }
}

impl Drop for PinWindow {
    fn drop(&mut self) {
        // 兜底：未显式 close 的移除路径在此离屏。纪律：Drop 只发生在主线程（见
        // Send 的 SAFETY 注——app 层所有 remove 都在 run_on_main_thread / 主线程
        // 回调内执行）。Retained 逐字段释放 → NSWindow/NSImageView dealloc。
        self.close();
    }
}

/// 以窗口中心为锚缩放：新帧 = 旧帧尺寸 × ratio，中心不变。仅主线程。
fn set_frame_around_center(win: &NSWindow, ratio: f64) {
    let frame = win.frame();
    let cx = frame.origin.x + frame.size.width / 2.0;
    let cy = frame.origin.y + frame.size.height / 2.0;
    let nw = frame.size.width * ratio;
    let nh = frame.size.height * ratio;
    win.setFrame_display(
        NSRect {
            origin: NSPoint { x: cx - nw / 2.0, y: cy - nh / 2.0 },
            size: NSSize { width: nw, height: nh },
        },
        true,
    );
}

// —— CGImage 构造（objc2-core-graphics） —--

/// 从原始 RGBA 构造 CGImage。字节**拷贝**进 data provider（控制器裁定的 SAFEST 方案）：
/// 调用方 `Arc<Vec<u8>>`（CAPTURES 缓存）可能在贴图存活期间被 `release_captures` 清空，
/// provider 必须自持数据。所有权链：`Box<[u8]>` → `CGDataProviderCreateWithData` 的
/// info/data 指针 → release 回调 [`drop_provider_bytes`] 按 `Box::from_raw` 收回，
/// 与分配严格配对（provider 释放时恰好一次）。
fn make_cg_image(rgba: &[u8], w_px: u32, h_px: u32) -> Result<CFRetained<CGImage>, String> {
    // 双层 Box：外层 Box<Box<[u8]>> 是瘦指针（可安全经 c_void 往返），内层持有缓冲。
    let boxed: Box<Box<[u8]>> = Box::new(rgba.to_vec().into_boxed_slice());
    let len = boxed.len();
    let ptr = Box::into_raw(boxed);
    // SAFETY: ptr 是上一行 Box::into_raw 的产物（非空、对齐合法）；data 指向其内层缓冲；
    // release 回调 drop_provider_bytes 与该 Box 配对（CG 契约：provider 释放时恰调一次）。
    let provider = unsafe {
        CGDataProvider::with_data(
            ptr as *mut c_void,
            (**ptr).as_ptr() as *const c_void,
            len,
            Some(drop_provider_bytes),
        )
    }
    .ok_or("CGDataProvider 创建失败")?;
    let space = CGColorSpace::new_device_rgb().ok_or("CGColorSpace 创建失败")?;
    // SAFETY: 各参数均为刚构造的合法 CF 对象或本地标量。位图格式：8 bpc / 32 bpp /
    // bytesPerRow = w×4 / premultiplied-last RGBA（屏幕捕获缓冲语义）/ DeviceRGB /
    // 无 decode 表 / 插值开 / 默认渲染意图。CGImage 失败（理论仅内存耗尽）→ Err。
    unsafe {
        CGImage::new(
            w_px as usize,
            h_px as usize,
            8,
            32,
            w_px as usize * 4,
            Some(&space),
            CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0),
            Some(&provider),
            std::ptr::null(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .ok_or_else(|| "CGImage 创建失败".into())
}

/// `CGDataProviderReleaseDataCallback`：收回 make_cg_image 交给 provider 的
/// `Box<Box<[u8]>>`。SAFETY: info 必须是 make_cg_image 里 Box::into_raw 的指针
/// （CG 保证恰调一次、此后不再读 data）；from_raw 与 into_raw 的类型严格一致。
unsafe extern "C-unwind" fn drop_provider_bytes(
    info: *mut c_void,
    _data: NonNull<c_void>,
    _size: usize,
) {
    // SAFETY: 见函数注释——info 是 make_cg_image 配对存入的 Box 指针，此处恰收回一次。
    unsafe { drop(Box::from_raw(info as *mut Box<[u8]>)) };
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    /// 真·窗口创建/关闭（需 macOS 窗口环境 + **进程主线程**）。libtest 的测试线程
    /// 永远不是进程主线程（含 --test-threads=1），故本测试在 cargo test 下只会以
    /// 「请在主线程运行」失败——真实验证走 examples/pin_smoke.rs（main 在主线程）：
    /// `cargo run -p hawksnap-platform-mac --example pin_smoke`。CI 不跑（控制器裁定）。
    #[test]
    #[ignore = "需窗口环境与进程主线程；libtest 线程非主线程——真实验证走 cargo run -p hawksnap-platform-mac --example pin_smoke"]
    fn real_pin_window_creates_and_closes() {
        assert!(
            MainThreadMarker::new().is_some(),
            "测试线程不是进程主线程（libtest 限制）：请运行 cargo run -p hawksnap-platform-mac --example pin_smoke"
        );
        let mut rgba = vec![0u8; 100 * 100 * 4];
        for px in rgba.chunks_exact_mut(4) {
            px[0] = 255; // R
            px[3] = 255; // A
        }
        let pin = PinWindow::create(Arc::new(rgba), 100, 100, (120.0, 120.0), 2.0, 42)
            .expect("创建贴图窗口");
        assert_eq!(pin.scale(), 0.5, "初始显示比为半尺寸");
        pin.zoom_to(1.0);
        assert_eq!(pin.scale(), 1.0);
        pin.set_opacity(0.5);
        std::thread::sleep(std::time::Duration::from_millis(200));
        pin.close();
        pin.close(); // 幂等
    }
}
