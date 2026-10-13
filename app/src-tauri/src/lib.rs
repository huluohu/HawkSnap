mod commands;
mod overlay;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // 2.4.x 起插件不再提供自由函数 init()，改用 Builder::new().build()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::capture_screens, commands::finish_capture,
            overlay::cancel_capture, overlay::emit_capture_done
        ])
        .setup(|app| {
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
                                let _ = overlay::start_capture_flow(&app);
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
