use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// 应用设置（C1）：JSON 持久化到 app_data_dir()/settings.json。serde camelCase 对齐
/// 前端命令边界；`default` 使旧版本文件缺字段时按 Default 补齐，升级不炸。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub hotkey_capture: String,   // 默认 "F1"
    pub hotkey_pin: String,       // 默认 "F2"
    pub hotkey_record: String,    // 默认 "F6"
    pub output_format: String,    // "png" | "jpeg"，默认 "png"
    pub jpeg_quality: u8,         // 1-100，默认 90
    pub save_dir: Option<String>, // None → ~/Pictures/HawkSnap
    pub launch_at_login: bool,    // 默认 false
    pub appearance: String,       // "system"|"light"|"dark"，默认 "system"
    pub language: String,         // "system"|"zh-CN"|"en-US"，默认 "system"（D1 i18n）
    pub update_enabled: bool,     // 默认 true；更新检查是 spec §5 唯一新增外联（D3）
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey_capture: "F1".into(),
            hotkey_pin: "F2".into(),
            hotkey_record: "F6".into(),
            output_format: "png".into(),
            jpeg_quality: 90,
            save_dir: None,
            launch_at_login: false,
            appearance: "system".into(),
            language: "system".into(),
            update_enabled: true,
        }
    }
}

/// settings.json 落点（create_dir_all 父目录由 save 做，此处只拼路径）。
pub fn settings_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("settings.json")
}

impl Settings {
    /// 读文件；不存在/解析失败 → Default（解析失败额外 eprintln，配置文件手改坏了
    /// 要有可见线索，但不能阻塞应用启动）。
    pub fn load(app: &AppHandle) -> Settings {
        let path = settings_path(app);
        Self::load_from(&path).unwrap_or_else(|e| {
            if path.exists() {
                eprintln!("[hawksnap] 设置文件解析失败，回退默认值: {e}");
            }
            Settings::default()
        })
    }

    /// C1 门面 API（计划接口）：落盘到 settings_path。C7 起 set_settings 走 apply_pipeline
    /// 的 save_to（路径注入），本包装保留为既定接口。
    #[allow(dead_code)]
    pub fn save(&self, app: &AppHandle) -> Result<(), String> {
        let path = settings_path(app);
        self.save_to(&path)
    }

    /// 内部可测对：与 AppHandle 解耦，测试用临时目录直调。
    pub(crate) fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    pub(crate) fn load_from(path: &std::path::Path) -> Result<Settings, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    /// 归一化：jpeg_quality clamp 1..=100；output_format 非 "jpeg" 一律 "png"；
    /// save_dir 空/全空白 → None（C7「恢复默认」清空为空串/null 的双形态归一）。
    /// 在保存边界统一执行，脏输入（前端/手改）永远落不出非法值。
    pub(crate) fn sanitized(mut self) -> Self {
        self.jpeg_quality = self.jpeg_quality.clamp(1, 100);
        if self.output_format != "jpeg" {
            self.output_format = "png".into();
        }
        if self.save_dir.as_deref().is_some_and(|d| d.trim().is_empty()) {
            self.save_dir = None;
        }
        // D1：语言白名单归一——未知值（旧文件手改/脏数据）一律回 "system"（跟随系统）。
        if self.language != "zh-CN" && self.language != "en-US" {
            self.language = "system".into();
        }
        self
    }
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    Settings::load(&app)
}

#[tauri::command(async)]
pub fn set_settings(app: AppHandle, s: Settings) -> Result<Settings, String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    // 热键重注册第一步：注销旧组合（unregister_all 经插件 run_main_thread 交付主线程，
    // async 命令跑在 tokio 线程，安全）。注销本身失败同样视为注册侧失败 → Err 不落盘。
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| format!("注销旧热键失败: {e}"))?;
    let path = settings_path(&app);
    // 保存管线（可测内核）：sanitize → 按序注册热键（任一失败 → Err 且不落盘，文件保持
    // 旧值）→ 自启切换（best-effort，仅在与文件旧值不同时执行）→ 落盘 → 返回生效设置。
    // 运行态注：注册失败时旧热键已注销、新组合可能半数已注册（半新半旧），文件保持旧值
    // ——重启或下次成功保存自愈（reregister_from_settings 注，计划裁定不做回滚注册）。
    let effective = apply_pipeline(
        s,
        &path,
        |combo, kind| crate::overlay::register_one(&app, combo, kind),
        |enable| apply_launch_at_login(&app, enable),
    )?;
    // M0：save_dir 可能刚变更——重算 asset 协议放行（新目录的缩略图才能进网格）。
    // best-effort（内部 eprintln 降级），不影响保存成功返回。
    crate::commands::allow_asset_scope(&app);
    // 托盘菜单动态重建（M3 继承项⑦）：热键组合可能刚变更——用**生效设置**重建
    // 菜单并整体替换（TrayIcon::set_menu 只换菜单本体，on_menu_event 分派与菜单项
    // id 恒定不变）。best-effort：重建/替换失败仅 eprintln——菜单停留在旧文案
    // （旧组合可能已失效，但「主面板/退出」等恒定入口不受影响），不阻断保存成功
    // 返回；下次成功保存或重启自愈（setup 走同一 build_tray_menu）。
    if let Some(tray) = app.tray_by_id("main") {
        match crate::build_tray_menu(&app, &effective) {
            Ok(menu) => {
                if let Err(e) = tray.set_menu(Some(menu)) {
                    eprintln!("[hawksnap] 托盘菜单重建失败（保持旧菜单）: {e}");
                }
            }
            Err(e) => eprintln!("[hawksnap] 托盘菜单构建失败（保持旧菜单）: {e}"),
        }
    } else {
        eprintln!("[hawksnap] 托盘不可用（菜单未重建）");
    }
    Ok(effective)
}

/// set_settings 的可测内核（AppHandle-free）：sanitize → 按序注册热键（失败中止、不落盘）
/// → 自启（best-effort 回调，仅当与文件旧值不同才调用，终审 Fix 4）→ 落盘 → 返回生效
/// 设置。生产路径由 set_settings 注入真实注册（register_one）与自启
/// （apply_launch_at_login）闭包；单测注入 fake（见 tests）。
fn apply_pipeline(
    s: Settings,
    path: &std::path::Path,
    register_fn: impl FnMut(&str, crate::overlay::HotkeyKind) -> Result<(), String>,
    autostart_fn: impl FnOnce(bool),
) -> Result<Settings, String> {
    let s = s.sanitized();
    // Err 的 kind 归因（D0-⑤）只服务横幅；保存命令的 Err 口径保持纯文案 String 不变。
    crate::overlay::register_in_order(&s, register_fn).map_err(|e| e.message)?;
    // 自启 only-if-changed（终审 Fix 4）：此刻尚未落盘，load_from 读到的是文件中的旧值
    // ——仅当 launch_at_login 与旧值不同才切换登录项，省掉每次保存对 LaunchAgent 的
    // 无谓 enable/disable 往返。文件不存在/不可读（首次保存、手改损坏）无旧值可比，
    // 按需执行，行为与改动前一致；切换失败仍由 apply_launch_at_login 内 eprintln 降级。
    let old = Settings::load_from(path).ok();
    if old.map(|o| o.launch_at_login) != Some(s.launch_at_login) {
        autostart_fn(s.launch_at_login);
    }
    s.save_to(path)?;
    Ok(s)
}

/// 开机自启（C7）：经 tauri-plugin-autostart 的 ManagerExt（AutoLaunchManager）切换
/// 登录项（macOS LaunchAgent，仅写本地）。裁定：best-effort——切换失败仅 eprintln，
/// 绝不影响整个保存的成败（登录项是旁路能力；失败可见性走日志，横幅留给热键冲突）。
fn apply_launch_at_login(app: &AppHandle, enable: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let result = if enable {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(e) = result {
        eprintln!("[hawksnap] 开机自启切换失败（enable={enable}）: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{register_in_order, HotkeyKind};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hs-settings-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_are_sane() {
        let s = Settings::default();
        assert_eq!(s.hotkey_capture, "F1");
        assert_eq!(s.hotkey_pin, "F2");
        assert_eq!(s.hotkey_record, "F6");
        assert_eq!(s.output_format, "png");
        assert_eq!(s.jpeg_quality, 90);
        assert_eq!(s.save_dir, None);
        assert!(!s.launch_at_login);
        assert_eq!(s.appearance, "system");
        assert_eq!(s.language, "system");
        assert!(s.update_enabled, "自动更新默认开启（spec §5，可关闭）");
    }

    #[test]
    fn roundtrip_preserves_fields() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("settings.json");
        let s = Settings {
            hotkey_capture: "F9".into(),
            hotkey_pin: "F10".into(),
            hotkey_record: "F11".into(),
            output_format: "jpeg".into(),
            jpeg_quality: 42,
            save_dir: Some("/tmp/shots".into()),
            launch_at_login: true,
            appearance: "dark".into(),
            language: "zh-CN".into(),
            update_enabled: false,
        };
        s.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert_eq!(s, loaded, "save→load 必须全等");
        assert!(!loaded.update_enabled, "update_enabled 必须随文件持久化（false 不被默认值吞掉）");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sanitized_clamps_quality_and_format() {
        let base = || Settings { jpeg_quality: 0, output_format: "webp".into(), ..Default::default() };
        assert_eq!(base().sanitized().jpeg_quality, 1, "0 → clamp 到 1");
        let high = Settings { jpeg_quality: 200, ..base() };
        assert_eq!(high.sanitized().jpeg_quality, 100, "200 → clamp 到 100");
        assert_eq!(base().sanitized().output_format, "png", "非 jpeg → png");
        let jpeg = Settings { output_format: "jpeg".into(), ..base() };
        assert_eq!(jpeg.sanitized().output_format, "jpeg", "合法 jpeg 保留");
        // D1：语言白名单——合法值保留，未知值归一 "system"。
        let zh = Settings { language: "zh-CN".into(), ..base() };
        assert_eq!(zh.sanitized().language, "zh-CN", "合法 zh-CN 保留");
        let en = Settings { language: "en-US".into(), ..base() };
        assert_eq!(en.sanitized().language, "en-US", "合法 en-US 保留");
        let dirty = Settings { language: "fr-FR".into(), ..base() };
        assert_eq!(dirty.sanitized().language, "system", "未知语言 → system");
    }

    // —— C7：set_settings 应用管线 ——

    #[test]
    fn apply_hotkey_failure_keeps_file() {
        // Plan C7 Step 1（计划裁定）：注册走注入而非真注册（AppHandle 不可造）。任一热键
        // 注册失败 → 返回 Err 且文件内容不变（设置不落盘，前端据此回显旧值）。自启以回调
        // 注入，测试绝不触碰真实登录项。
        let dir = temp_dir("apply-failure");
        let path = dir.join("settings.json");
        let original = Settings::default();
        original.save_to(&path).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        // 第三个（record = "Cmd+Q"）注册失败：验证失败发生在序列后段时同样中止保存
        let bad = Settings { hotkey_record: "Cmd+Q".into(), ..original.clone() };
        let mut autostart_calls: Vec<bool> = Vec::new();
        let result = apply_pipeline(
            bad,
            &path,
            |combo, _kind| if combo == "Cmd+Q" { Err("被占用".into()) } else { Ok(()) },
            |enable| autostart_calls.push(enable),
        );
        assert!(result.is_err(), "任一热键注册失败必须整体 Err");
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after, "注册失败时文件必须保持旧值（不落盘）");
        assert!(
            autostart_calls.is_empty(),
            "热键失败中止时不得执行自启切换（顺序：热键 → 自启 → 落盘）"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reregister_uses_settings_strings() {
        // 注入 register_fn 捕获收到的 (combo, kind)：顺序必须 capture → pin → record，
        // 字符串逐字取自设置（Shortcut::from_str 解析是插件域，不在此测）。
        let s = Settings {
            hotkey_capture: "Cmd+Shift+A".into(),
            hotkey_pin: "F9".into(),
            hotkey_record: "Ctrl+Alt+7".into(),
            ..Default::default()
        };
        let mut seen: Vec<(String, HotkeyKind)> = Vec::new();
        let result = register_in_order(&s, |combo, kind| {
            seen.push((combo.to_string(), kind));
            Ok(())
        });
        assert!(result.is_ok());
        assert_eq!(
            seen,
            vec![
                ("Cmd+Shift+A".to_string(), HotkeyKind::Capture),
                ("F9".to_string(), HotkeyKind::Pin),
                ("Ctrl+Alt+7".to_string(), HotkeyKind::Record),
            ],
            "注册序与字符串必须逐字来自设置"
        );
    }
}
