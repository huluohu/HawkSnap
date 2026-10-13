mod commands;
mod overlay;
mod perf;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // 2.4.x 起插件不再提供自由函数 init()，改用 Builder::new().build()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::capture_screens, commands::finish_capture,
            overlay::cancel_capture, overlay::emit_capture_done,
            overlay::hotkey_error
        ])
        .setup(|app| {
            // Task 11 埋点：前端首帧画出冻结帧后回报 hawk://overlay-shown（无 payload）。
            // 多显示器时每窗口各发一次，mark_overlay_shown_once 只记每轮首个（评审 Finding 2a）。
            {
                use tauri::Listener;
                let _id = app.listen("hawk://overlay-shown", move |_event| {
                    crate::perf::mark_overlay_shown_once();
                });
            }

            // 热键注册失败不中止启动（spec §9：允许无热键启动，但必须给用户可见反馈）。
            if let Err(err) = overlay::register_hotkeys(app.handle()) {
                eprintln!("[hawksnap] 热键注册失败: {err}");
                use tauri::Emitter;
                let _ = app.emit("hawk://hotkey-error", err);
            }

            // 托盘常驻：截图入口 + 退出。
            {
                use tauri::{
                    menu::{Menu, MenuItem},
                    tray::TrayIconBuilder,
                };
                let shot = MenuItem::with_id(app, "shot", "截图 (F1)", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "退出 HawkSnap", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&shot, &quit])?;
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
                        "shot" => {
                            let app = app.clone();
                            std::thread::spawn(move || {
                                // 与 F1 热键路径同口径：失败走 hawk://capture-error，不静默吞掉
                                if let Err(e) = overlay::start_capture_flow(&app) {
                                    let _ = tauri::Emitter::emit(&app, "hawk://capture-error", e);
                                }
                            });
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .build(app)?;
            }

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
    app.run(|_app, event| {
        if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}
