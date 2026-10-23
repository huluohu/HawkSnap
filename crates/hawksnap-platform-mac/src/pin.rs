//! C5：贴图 Pin——原生 NSWindow + NSImageView 窗口族（无 webview）。
//!
//! 架构裁定（plan C5 + 控制器细化）：Snipaste 同款——每张贴图一个 borderless NSWindow，
//! 内容为铺满窗口的 NSImageView。8 张贴图零 webview 成本（对比每贴 ~30MB 的 webview
//! 方案直接击穿 G2 ≤80MB），缩放/透明度/拖拽全是窗口原生操作：
//! - 拖拽移动 = `isMovableByWindowBackground`（mouseDown 不覆写 → 父类默认路径保留拖拽）；
//! - 滚轮缩放 / Ctrl+滚轮透明度 = `HawkPinView` 覆写 `scrollWheel:`（define_class）；
//! - Esc 关闭聚焦贴图 = `HawkPinView` 覆写 `keyDown:`（仅 `isKeyWindow` 时），经
//!   [`set_on_escape`] 注入的 fn 指针回调 app 层（platform-mac 不反向依赖 app crate）；
//! - 双击进编辑器（M2 Task M3）= `mouseUp:` 覆写检测 `clickCount == 2`，经
//!   [`set_on_dblclick`] 注入的 fn 指针回调 app 层（同款反依赖模式）。mouseDown 刻意
//!   不覆写：`isMovableByWindowBackground` 的拖拽语义保留在父类默认路径，而 AppKit 对
//!   双击的第二击 mouseUp 照发且 clickCount=2，mouseUp 侧检测不干扰拖拽。
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

/// 滚轮每档缩放/透明度步长（≈10%）。方向随 deltaY 符号。
const WHEEL_STEP: f64 = 1.1;
// 缩放/透明度范围常量（D0-④ 下沉 hawksnap-core::geometry 的 PIN_ZOOM_* / PIN_OPACITY_*），
// 本文件经 use 引入；本地副本已删（命令侧 pins.rs 与滚轮路径同源单一值）。
use hawksnap_core::geometry::{PIN_OPACITY_MAX, PIN_OPACITY_MIN, PIN_ZOOM_MAX, PIN_ZOOM_MIN};
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

// —— 双击进编辑器钩子（M2 Task M3，模式同 ON_ESCAPE） ——

/// 双击回调：`fn(u64)`（pin_id）。fn 指针 Copy + Send，语义由 app 层裁定（M3：贴图
/// 当前帧落盘临时 PNG → editor_open）。仅在主线程被调用（mouseUp 回调内）。
static ON_DBLCLICK: Mutex<Option<fn(u64)>> = Mutex::new(None);

/// app 层在首个贴图创建前注入（幂等，重复覆盖无害）。
pub fn set_on_dblclick(f: fn(u64)) {
    *ON_DBLCLICK.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

fn dblclick_hook() -> Option<fn(u64)> {
    *ON_DBLCLICK.lock().unwrap_or_else(|e| e.into_inner())
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
                let next = (ivars.opacity.get() * step).clamp(PIN_OPACITY_MIN, PIN_OPACITY_MAX);
                ivars.opacity.set(next);
                if let Some(win) = self.window() {
                    win.setAlphaValue(next);
                }
            } else {
                // 滚轮：以窗口中心缩放（10%–3200%）
                let cur = ivars.scale.get();
                let target = (cur * step).clamp(PIN_ZOOM_MIN, PIN_ZOOM_MAX);
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
                // M2 Task M3：双击 → app 层钩子（贴图当前帧落盘 → editor_open）。钩子
                // 未注入/窗口树缺失时静默放行（父类路径继续，行为同单击）。
                if let (Some(hook), Some(id)) = (dblclick_hook(), self.pin_id()) {
                    hook(id);
                }
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
    /// 源图像保留（M2 Task M3，双击进编辑器 + 保存回写）：create 传入的整帧 RGBA
    /// Arc 原样持有（make_cg_image 已把字节**拷贝**进 data provider，此处只延长原
    /// 缓冲生命周期、不产生第三份拷贝）+ 物理 dims + 建窗时源显示器 scale（set_image
    /// 重算逻辑尺寸用）。编辑器保存回写经 [`PinWindow::set_image`] 整体替换。
    /// RefCell/Cell：方法签名保持 `&self`（AppKit 回调与 PINS 句柄 map 的 `get` 路径
    /// 只给共享引用；ivars 同款 interior mutability 纪律——读写全部约束在主线程）。
    image_rgba: std::cell::RefCell<std::sync::Arc<Vec<u8>>>,
    image_w_px: Cell<u32>,
    image_h_px: Cell<u32>,
    display_scale: f64,
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
        // 探针（常驻，廉价，每次创建一行）：E2E 判别「create 被调用 + 参数合理」。
        eprintln!(
            "[hawksnap] PinWindow::create: pin_id={pin_id} {w_px}×{h_px} top_left=({:.0},{:.0}) scale={scale}",
            top_left.0, top_left.1
        );
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
        // 探针（常驻，廉价，每次创建一行）：上屏状态立刻核对。0.3.2 无 isOnScreen
        // 绑定（仅 isOnActiveSpace，语义不同），isVisible = 已 order 上屏，够用。
        let frame = window.frame();
        eprintln!(
            "[hawksnap] PinWindow::create: frame=({:.0},{:.0} {:.0}×{:.0}) isVisible={}",
            frame.origin.x, frame.origin.y, frame.size.width, frame.size.height,
            window.isVisible()
        );

        Ok(PinWindow {
            window,
            view,
            closed: Cell::new(false),
            image_rgba: std::cell::RefCell::new(rgba),
            image_w_px: Cell::new(w_px),
            image_h_px: Cell::new(h_px),
            display_scale: scale,
        })
    }

    /// 调试/冒烟断言用：窗口当前是否可见（AppKit isVisible = 已 order 上屏）。
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
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

    /// 源图像快照（M2 Task M3）：`(rgba Arc 克隆, 宽 px, 高 px)`。仅主线程调用（
    /// PinWindow 句柄纪律：一切解引用在主线程）——app 层双击回调在 mouseUp 主线程栈内
    /// 取走 Arc 克隆后，编码/落盘/建编辑器窗全部在独立线程完成（rgba_to_png 数十 ms，
    /// 不得阻塞主线程事件循环）。设计说明：任务简报里的 `snapshot_png()`（句柄内直接
    /// 编码）与此处等价，但平台 crate 只保留 AppKit 纪律内的数据访问器，PNG 编码放
    /// app 层线程（hawksnap_core 纯函数）——同一个东西，挪了执行位置以守住线程纪律。
    pub fn image_rgba(&self) -> (std::sync::Arc<Vec<u8>>, u32, u32) {
        (
            self.image_rgba.borrow().clone(),
            self.image_w_px.get(),
            self.image_h_px.get(),
        )
    }

    /// 替换贴图图像（M2 Task M3，编辑器保存回写）：新 RGBA → CGImage（字节拷贝进
    /// provider，同 create）→ NSImageView 换图。窗口帧不动（当前缩放/位置保持），
    /// 图像等比缩放（ScaleProportionallyUpOrDown）铺满视图；逻辑尺寸按建窗时源显示
    /// 器 scale 重算（编辑器导出与原图同 dims，正常路径无可见变化）。仅主线程调用。
    pub fn set_image(&self, rgba: std::sync::Arc<Vec<u8>>, w_px: u32, h_px: u32) -> Result<(), String> {
        if w_px == 0 || h_px == 0 {
            return Err("贴图尺寸为空".into());
        }
        if rgba.len() != w_px as usize * h_px as usize * 4 {
            return Err(format!(
                "RGBA 长度不匹配: {} != {w_px}×{h_px}×4",
                rgba.len()
            ));
        }
        let cg_image = make_cg_image(&rgba, w_px, h_px)?;
        let logical_w = w_px as f64 / self.display_scale;
        let logical_h = h_px as f64 / self.display_scale;
        let image = NSImage::initWithCGImage_size(
            NSImage::alloc(),
            &cg_image,
            NSSize { width: logical_w, height: logical_h },
        );
        self.view.setImage(Some(&image));
        // 源图像快照同步替换：下一次双击进编辑器装载的是新图。
        *self.image_rgba.borrow_mut() = rgba;
        self.image_w_px.set(w_px);
        self.image_h_px.set(h_px);
        Ok(())
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

    /// M2 Task M3：双击钩子注入（幂等覆盖无害）。fn 指针静态 + Mutex，与 ON_ESCAPE
    /// 同款契约；窗口路径（mouseUp → hook(id)）需主线程 AppKit 环境，真实验证走
    /// examples/pin_smoke + 控制器 E2E，此处只验注入管道本身。
    #[test]
    fn dblclick_hook_injects_idempotently() {
        fn first(_: u64) {}
        fn second(_: u64) {}
        set_on_dblclick(first);
        assert!(dblclick_hook().is_some(), "注入后钩子必须可达");
        set_on_dblclick(second); // 重复注入不 panic（app 层每轮建窗都注入）
        assert!(dblclick_hook().is_some(), "重复注入后钩子仍可达");
    }

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
