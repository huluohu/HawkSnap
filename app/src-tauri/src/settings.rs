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

    /// 归一化：jpeg_quality clamp 1..=100；output_format 非 "jpeg" 一律 "png"。
    /// 在保存边界统一执行，脏输入（前端/手改）永远落不出非法值。
    fn sanitized(mut self) -> Self {
        self.jpeg_quality = self.jpeg_quality.clamp(1, 100);
        if self.output_format != "jpeg" {
            self.output_format = "png".into();
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
    let s = s.sanitized();
    s.save(&app)?;
    // C7: apply hotkeys & autostart
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        s.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert_eq!(s, loaded, "save→load 必须全等");
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
    }
}
