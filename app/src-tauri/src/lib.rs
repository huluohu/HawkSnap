mod commands;
mod history;
mod idle_reaper;
mod overlay;
mod panel;
mod perf;
mod pins;
mod settings;
mod single_instance;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // 2.4.x 起插件不再提供自由函数 init()，改用 Builder::new().build()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // C7 开机自启：autostart 2.7.0 同样无自由函数 init()（计划所写
        // init(MacosLauncher::LaunchAgent, None) 为旧版签名），Builder.build() 适配；
        // macOS 用 LaunchAgent（登录项 plist，仅写本地）。macos_launcher 仅 macOS 编译，
        // 非 macOS 用 Builder 默认值（文档默认即 LaunchAgent，跨平台语义不变）。
        .plugin({
            let builder = tauri_plugin_autostart::Builder::new();
            #[cfg(target_os = "macos")]
            let builder = builder.macos_launcher(tauri_plugin_autostart::MacosLauncher::LaunchAgent);
            builder.build()
        })
        .invoke_handler(tauri::generate_handler![
            commands::capture_screens, commands::finish_capture,
            overlay::cancel_capture, overlay::emit_capture_done,
            overlay::hotkey_error, overlay::snap_candidates,
            overlay::start_delayed_capture, overlay::cancel_delayed_capture,
            overlay::countdown_remaining,
            settings::get_settings, settings::set_settings,
            history::history_list, history::history_delete,
            pins::pin_latest, pins::pin_set_opacity, pins::pin_zoom,
            pins::pin_close, pins::pin_close_all,
            panel::panel_trigger
        ])
        .setup(|app| {
            // 单实例守卫（Task 13）：setup 第一步。锁文件在临时目录，flock 独占；
            // 守卫泄漏为 'static，锁覆盖进程全生命周期（进程退出由内核释放）。
            // 第二实例拿不到锁 → 提示并立即退出，不再初始化任何窗口/托盘。
            match single_instance::acquire_lock_at(&std::env::temp_dir().join("hawksnap.lock")) {
                Ok(guard) => std::mem::forget(guard),
                Err(_) => {
                    eprintln!("[hawksnap] 另一实例正在运行");
                    app.handle().exit(0);
                    return Ok(());
                }
            }

            // Task 11 埋点：前端首帧画出冻结帧后回报 hawk://overlay-shown（无 payload）。
            // 多显示器时每窗口各发一次，mark_overlay_shown_once 只记每轮首个（评审 Finding 2a）。
            {
                use tauri::Listener;
                let _id = app.listen("hawk://overlay-shown", move |_event| {
                    crate::perf::mark_overlay_shown_once();
                });
            }

            // 历史库（C3）：旁路功能，init 失败仅记录不阻塞启动（裁定：不 `?` 退出）；
            // 此后各 history 命令自行开连接，失败以 Err 返回给前端。
            if let Err(e) = history::init(app.handle()) {
                eprintln!("[hawksnap] 历史库初始化失败: {e}");
            }

            // 热键注册失败不中止启动（spec §9：允许无热键启动，但必须给用户可见反馈）。
            if let Err(err) = overlay::register_hotkeys(app.handle()) {
                eprintln!("[hawksnap] 热键注册失败: {err}");
                use tauri::Emitter;
                let _ = app.emit("hawk://hotkey-error", err);
                // spec §9 收口（C6）：启动零窗口后，此处的 emit 无窗可显（push 通道
                // 且前端尚未挂载）——立即拉起主面板让横幅有处可显。setup 运行在主线程
                // （register_hover_probe 的 AppKit 监听依赖此事实），而 webview 建窗
                // 禁止主线程（与 start_capture_flow 同一纪律，panel.rs 模块注）——
                // 故既不能直调也不能 run_on_main_thread，spawn 独立线程直呼。
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    if let Err(e) = panel::ensure_main_panel(&handle) {
                        eprintln!("[hawksnap] 热键失败反馈面板拉起失败: {e}");
                    }
                });
            }

            // 媒体键兼容（Task 13）：macOS 默认 F1=亮度键（fnState 未开启时物理 F1 不发
            // keycode 122），监听 systemDefined 的亮度下键走与 F1 相同的截图流程；
            // 监听不可用（未授予辅助功能）时降级并按需经 hotkey-error 通道发「提示：」，
            // 绝不阻塞启动。
            #[cfg(target_os = "macos")]
            if let Err(err) = overlay::register_media_key_f1(app.handle()) {
                eprintln!("[hawksnap] 媒体键监听注册失败: {err}");
            }

            // 智能吸附悬停探测（Plan B Task 5）：辅助功能未授予时整条链路缺席（不装监听、
            // 不弹窗，拖拽框选不受影响）；注册失败只 eprintln，绝不阻塞启动。
            #[cfg(target_os = "macos")]
            if let Err(err) = overlay::register_hover_probe(app.handle()) {
                eprintln!("[hawksnap] 悬停探测注册失败: {err}");
            }

            // 托盘常驻：截图入口 + 延时截图子菜单（Plan B Task 8）+ 退出。
            {
                use tauri::{
                    menu::{Menu, MenuItem, Submenu},
                    tray::TrayIconBuilder,
                };
                // C6 主面板：启动零窗口后唯一的常驻面板入口（热键失败路径会自动拉起，
                // 但用户主动打开面板的常规路径在这里）。
                let panel =
                    MenuItem::with_id(app, "panel", "主面板", true, None::<&str>)?;
                let shot = MenuItem::with_id(app, "shot", "截图 (F1)", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "退出 HawkSnap", true, None::<&str>)?;
                // C5 贴图：关闭全部贴图恒可用（无贴图时调用为空操作，不做动态
                // enable/disable——省去菜单项状态的跨线程同步，与延时子菜单同口径）。
                let pin_close_all =
                    MenuItem::with_id(app, "pin-close-all", "关闭全部贴图", true, None::<&str>)?;
                // spec §3.1：延时截图 1/2/3/5 秒可选 + 「取消倒计时」恒可用（非活动期
                // 调用返回 Err，调用方 eprintln 吞掉——不做动态 enable/disable，省去
                // 菜单项状态的跨线程同步）。
                let delay_1 = MenuItem::with_id(app, "delay-1", "1 秒", true, None::<&str>)?;
                let delay_2 = MenuItem::with_id(app, "delay-2", "2 秒", true, None::<&str>)?;
                let delay_3 = MenuItem::with_id(app, "delay-3", "3 秒", true, None::<&str>)?;
                let delay_5 = MenuItem::with_id(app, "delay-5", "5 秒", true, None::<&str>)?;
                let delay_cancel =
                    MenuItem::with_id(app, "delay-cancel", "取消倒计时", true, None::<&str>)?;
                let delay_menu = Submenu::with_id_and_items(
                    app, "delay-submenu", "延时截图", true,
                    &[&delay_1, &delay_2, &delay_3, &delay_5, &delay_cancel],
                )?;
                let menu = Menu::with_items(app, &[&panel, &shot, &pin_close_all, &delay_menu, &quit])?;
                TrayIconBuilder::with_id("main")
                    .icon(
                        app.default_window_icon()
                            .expect("bundle icons missing: tauri.conf must provide a window icon")
                            .clone(),
                    )
                    .menu(&menu)
                    // on_menu_event 在主线程回调；start_capture_flow 建窗/关窗不能在主线程做，
                    // 必须丢到独立线程（其内部自行 block_on），保持本回调体最小。
                    .on_menu_event(|app, ev| match ev.id.as_ref() {
                        // C6 主面板：线程派发同 shot——on_menu_event 在主线程回调，而
                        // ensure_main_panel 建 wry webview 窗口禁主线程（与贴图原生
                        // NSWindow 的 run_on_main_thread 相反，见 panel.rs 模块注）。
                        "panel" => {
                            let app = app.clone();
                            std::thread::spawn(move || {
                                if let Err(e) = panel::ensure_main_panel(&app) {
                                    eprintln!("[hawksnap] 主面板打开失败: {e}");
                                }
                            });
                        }
                        "shot" => {
                            let app = app.clone();
                            std::thread::spawn(move || {
                                // 与 F1 热键路径同口径：失败走 hawk://capture-error，不静默吞掉
                                if let Err(e) = overlay::start_capture_flow(&app) {
                                    let _ = tauri::Emitter::emit(&app, "hawk://capture-error", e);
                                }
                            });
                        }
                        // 延时入口：线程派发同 shot（start_delayed_capture 建窗禁主线程）；
                        // 失败（门闩占用/建窗失败）与 shot 同口径走 capture-error。
                        id @ ("delay-1" | "delay-2" | "delay-3" | "delay-5") => {
                            let secs = id.trim_start_matches("delay-").parse::<u64>().unwrap_or(0);
                            let app = app.clone();
                            std::thread::spawn(move || {
                                if let Err(e) = overlay::start_delayed_capture(app.clone(), secs) {
                                    let _ =
                                        tauri::Emitter::emit(&app, "hawk://capture-error", e);
                                }
                            });
                        }
                        "delay-cancel" => {
                            // 纯原子置位，主线程直调安全（不建窗/关窗）；非活动期为良性
                            // 竞态（裁定 5），eprintln 吞掉。
                            if let Err(e) = overlay::cancel_delayed_capture() {
                                eprintln!("[hawksnap] 取消倒计时: {e}");
                            }
                        }
                        // C5 贴图：关闭全部贴图。线程派发同 shot——on_menu_event 在主线程
                        // 回调，而 pin_close_all 内部经 run_on_main_thread 触碰窗口对象，
                        // 派发到独立线程保持回调体最小（与 shot 同一纪律）。
                        "pin-close-all" => {
                            let app = app.clone();
                            std::thread::spawn(move || {
                                if let Err(e) = pins::pin_close_all(app.clone()) {
                                    eprintln!("[hawksnap] 关闭全部贴图失败: {e}");
                                }
                            });
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .build(app)?;
            }

            // Plan B Task 9 应急（G2）：空闲覆盖层 webview 看护线程。Plan B Task 1 的
            // 常驻复用让每个空闲 overlay-*/countdown webview 常驻 ~30MB，fresh-idle
            // 实测 100.7MB 吃穿 G2 预算（spec §5 ≤80MB）——隐藏满 5 分钟未复用即
            // close()，下次截图经 start_capture_flow 首建分支重建（一轮 webview 冷
            // 加载离群值，不计入基准 P90 口径）。看护自身吞错，绝不影响截图主链路。
            idle_reaper::spawn_watchdog(app.handle().clone());

            // macOS 专属 API：引用点与 API 同门控，非 macOS 平台无激活策略概念。
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // 托盘常驻：最后一个窗口关闭（用户点红绿灯叉）只应关窗，不应退出整个应用。
    // ExitRequested 的 code=None 表示“最后一个窗口关闭”这类用户交互触发的退出请求，
    // 一律阻止；托盘“退出”走 app.exit(0)，以 code=Some(0) 到达，不匹配本分支，正常退出。
    // （ExitRequested 标注 #[non_exhaustive]，模式需带 `..`。）
    // C6 零窗口启动口径：code=None 仅由「最后一个窗口销毁」引发——启动期无窗口不会
    // 自发产生（托盘 + 事件循环独立于窗口存活）；用户关掉懒创建的主面板时恰由本分支
    // 兜住（进程存活，下次托盘「主面板」重建）。
    app.run(|_app, event| {
        if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}
