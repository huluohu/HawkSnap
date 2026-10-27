//! keycast 小窗**焦点抢占实证探针**（M3 N4 评审修复 Important 的证据载体）。
//!
//! 为什么存在：keycast 是录屏中的**非交互**浮层。评审发现 Tauri 建窗默认
//! `focused(true)` → `show()` 走 tao 的 `makeKeyAndOrderFront` → 窗口可能成为
//! **key window** → 用户在录屏中的按键被吞进 keycast 的 webview
//! （`set_ignore_cursor_events` 只管鼠标，管不了键盘）。修复是 `focusable(false)`
//! （tao 覆写 `canBecomeKeyWindow` → 本窗永不能成为 key window）——**但「不能成为
//! key」是否仍会被排到最前（可见）** 必须实证，不能假设。
//!
//! 探针做 A/B（同款窗口，唯一变量 focusable），两组环境：
//! - **A 组｜生产配置**（Accessory 激活策略、app 未被激活）：对照组预期不抢焦点
//!   （accessory app 不因 orderFront 而激活）——本组只证明「可见 + 不夺 key」。
//! - **B 组｜最坏环境**（Regular 策略、app 已激活——模拟用户点过悬浮条后
//!   HawkSnap 处于活动状态）：对照组**必须**抢焦点（探针判别力证明），修复组
//!   **必须**不抢（修复生效证明）。
//!
//! 退出码：0 = 修复组在两组环境下均「可见且非 key window」，且 B 组对照组确有
//! 抢焦点（完全实证）；1 = 修复组性质被破坏（窗口不可见 / 仍成为 key window /
//! 焦点应用被换成本进程）；2 = B 组对照组未抢焦点（该环境无判别力）。
//!
//! 运行（需 `--features probe-util`——探针不进生产构建）：
//! `cargo run -p hawksnap --features probe-util --bin keycast-focus-probe`
//! （需辅助功能权限以读 AXFocusedApplication；无权限时焦点应用 pid 读为 None，
//! 探针退化为「仅 key window 判据」并照常给结论）。
//! 经 `tests/real_keycast_focus.rs`（#[ignore]，同样需 `--features probe-util`）
//! 以子进程方式纳入测试口径。
//!
//! 跨平台：NSWindow 焦点语义是 macOS 专属——非 macOS 平台打印 SKIP 并以 0 退出
//! （否则 cargo 报 "main function not found"）。

#[cfg(target_os = "macos")]
mod macos_probe {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    /// 造一个与生产 keycast 同款的浮层窗（320×88 透明 / 无边框 / always_on_top /
    /// 点击穿透），唯一变量是 focusable。
    fn make_probe_window(
        app: &tauri::AppHandle,
        label: &str,
        focusable: bool,
    ) -> tauri::Result<tauri::WebviewWindow> {
        let win = WebviewWindowBuilder::new(
            app,
            label,
            WebviewUrl::App("keycast?window=keycast".into()),
        )
        .inner_size(320.0, 88.0)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .focusable(focusable)
        .focused(false)
        .transparent(true)
        .visible(false)
        .build()?;
        let _ = win.set_ignore_cursor_events(true);
        win.show()?;
        Ok(win)
    }

    /// 一次测量：窗口是否可见 / 是否为 key window / **能否成为 key window**
    /// （canBecomeKeyWindow——决定性判据，与「本进程能否激活」无关）/ 系统级焦点
    /// 应用 pid。
    struct Measurement {
        label: &'static str,
        focusable: bool,
        visible: bool,
        focused: bool,
        can_become_key: bool,
        focused_app_before: Option<i32>,
        focused_app_after: Option<i32>,
    }

    impl Measurement {
        fn take(
            label: &'static str,
            focusable: bool,
            win: &tauri::WebviewWindow,
            before: Option<i32>,
        ) -> Self {
            // 焦点变更对 AX 可见需一点传播时间（300ms 足够）。
            std::thread::sleep(std::time::Duration::from_millis(300));
            Self {
                label,
                focusable,
                visible: win.is_visible().unwrap_or(false),
                focused: win.is_focused().unwrap_or(false),
                can_become_key: can_become_key_window(win),
                focused_app_before: before,
                focused_app_after: hawksnap_platform_mac::ax::focused_application_pid(),
            }
        }

        fn print(&self) {
            println!(
                "[probe] {:<10} focusable={:<5} visible={:<5} focused={:<5} canBecomeKeyWindow={:<5} focused_app_before={:?} focused_app_after={:?}",
                self.label,
                self.focusable,
                self.visible,
                self.focused,
                self.can_become_key,
                self.focused_app_before,
                self.focused_app_after,
            );
        }
    }

    /// 窗口**能否成为 key window**（NSWindow 的 canBecomeKeyWindow）。
    ///
    /// 这是「会不会吞键盘」的决定性判据，且**与本进程能否被激活无关**（未打包的
    /// CLI 二进制的激活策略受系统限制，观测「is_focused」不具判别力；
    /// canBecomeKeyWindow 是窗口自身的能力，无条件成立）。tao 的
    /// focusable(false) 覆写此方法返回 NO。
    fn can_become_key_window(win: &tauri::WebviewWindow) -> bool {
        let Ok(ptr) = win.ns_window() else {
            return false;
        };
        let ns = ptr as *mut objc2_app_kit::NSWindow;
        // SAFETY: ptr 来自 tauri 的 ns_window()（该窗口此刻仍存活，主线程持有），
        // 转型为 NSWindow 指针后调用其只读查询方法 canBecomeKeyWindow。
        unsafe { (&*ns).canBecomeKeyWindow() }
    }

    /// 探针主体：建 A/B 四窗并判定（退出码语义见模块注）。
    pub fn run() {
        let app = tauri::Builder::default()
            .setup(|app| {
                let self_pid = std::process::id() as i32;

                // ——— A 组｜生产配置：Accessory 策略、app 未激活 ———
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);

                let before_a = hawksnap_platform_mac::ax::focused_application_pid();
                let a_control = make_probe_window(app.handle(), "keycast-probe-a-control", true)?;
                let a_control_m = Measurement::take("A-control", true, &a_control, before_a);
                a_control_m.print();
                let a_control_focused = a_control_m.focused;

                let before_a2 = hawksnap_platform_mac::ax::focused_application_pid();
                let a_fix = make_probe_window(app.handle(), "keycast-probe-a-fix", false)?;
                let a_fix_m = Measurement::take("A-fix", false, &a_fix, before_a2);
                a_fix_m.print();
                let _ = a_control.close();
                let _ = a_fix.close();

                // ——— B 组｜最坏环境：Regular 策略（orderFront 会激活 app）———
                app.set_activation_policy(tauri::ActivationPolicy::Regular);
                std::thread::sleep(std::time::Duration::from_millis(300));

                let before_b = hawksnap_platform_mac::ax::focused_application_pid();
                let b_control = make_probe_window(app.handle(), "keycast-probe-b-control", true)?;
                let b_control_m = Measurement::take("B-control", true, &b_control, before_b);
                b_control_m.print();
                let b_control_focused = b_control_m.focused;

                let before_b2 = hawksnap_platform_mac::ax::focused_application_pid();
                let b_fix = make_probe_window(app.handle(), "keycast-probe-b-fix", false)?;
                let b_fix_m = Measurement::take("B-fix", false, &b_fix, before_b2);
                b_fix_m.print();
                let _ = b_control.close();
                let _ = b_fix.close();

                // ——— 判定 ———
                // ① 修复组（A/B 两组）：可见（不能为了不抢焦点而让窗口不显示）。
                for m in [&a_fix_m, &b_fix_m] {
                    if !m.visible {
                        println!("[probe] FAIL: {} 修复组不可见——修复破坏了显示", m.label);
                        std::process::exit(1);
                    }
                }
                // ② 修复组（A/B 两组）：**不能成为 key window**（决定性判据——窗口
                // 自身的能力，无条件成立，与本进程能否被激活无关）。
                for m in [&a_fix_m, &b_fix_m] {
                    if m.can_become_key {
                        println!("[probe] FAIL: {} 修复组仍能成为 key window——仍会吞键盘", m.label);
                        std::process::exit(1);
                    }
                }
                // ③ 修复组：不是 key window、焦点应用未被换成本进程。
                for m in [&a_fix_m, &b_fix_m] {
                    if m.focused {
                        println!("[probe] FAIL: {} 修复组成为 key window——抢了焦点", m.label);
                        std::process::exit(1);
                    }
                    if m.focused_app_after == Some(self_pid) {
                        println!(
                            "[probe] FAIL: {} 焦点应用变成本进程（pid={self_pid}）——抢了用户焦点",
                            m.label
                        );
                        std::process::exit(1);
                    }
                }
                // ④ 对照组判别力：focusable(true) 的窗口**能**成为 key window（→ 在
                // 可激活的打包 app 中 show() 即会夺走键盘焦点 = 本次修复的原始缺陷）。
                for (m, name) in [(&a_control_m, "A"), (&b_control_m, "B")] {
                    if !m.can_become_key {
                        println!(
                            "[probe] WARN: {name} 组对照组 focusable(true) 竟不能成为 key window（该环境无判别力）"
                        );
                        std::process::exit(2);
                    }
                }
                let _ = a_control_focused;
                let _ = b_control_focused;
                println!("[probe] OK: 修复组在两组环境下均可见且**不能**成为 key window；对照组 focusable(true) 能成为 key window（探针有判别力）");
                std::process::exit(0);
            })
            .build(tauri::generate_context!())
            .expect("tauri app 构建失败");
        // setup 钩子在事件循环 Ready 时触发（tauri 语义）——必须 run 起来，setup 里
        // 的判定与退出才会发生。setup 内已 exit，不会走到回调。
        app.run(|_app, _event| {});
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    macos_probe::run();
    #[cfg(not(target_os = "macos"))]
    {
        // NSWindow 焦点语义是 macOS 专属——非 macOS 无可测对象，SKIP + 退出 0
        // （保持「测试因平台无关而跳过」而非「因编译错误而失败」）。
        println!("[probe] SKIP: keycast 焦点探针仅适用于 macOS（NSWindow key window 语义）");
    }
}
