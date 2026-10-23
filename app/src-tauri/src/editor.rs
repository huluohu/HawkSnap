//! 标注编辑器窗口（M2 Task M1）：label "editor" 的 webview 窗口，?window=editor 路由。
//!
//! 单编辑器模型：整个应用只有一个编辑器窗口。`editor_open(path)` 建窗（首次）或
//! 复用并 show/focus（已在），随后 emit `hawk://editor-load { path }`；**编辑中再开
//! 新图 = 前端用新 path 替换当前图 + 清空撤销栈**（形状数组随旧图作废，语义见前端
//! index.tsx 的 loadFromPath）。
//!
//! 线程纪律（与 panel.rs 同款，**与贴图 pin 相反**）：wry webview 的建窗/关窗禁止
//! 在主线程做，`editor_open` 为 `#[tauri::command(async)]`——命令体在运行时线程执行，
//! 建窗合法；show/set_focus 是投递给主线程事件循环的消息，非主线程调用安全。

use base64::Engine;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// 编辑器窗口 label（与 capabilities/editor.json 的 `windows: ["editor"]` 对应）。
pub const EDITOR_LABEL: &str = "editor";

/// hawk://editor-load 载荷（serde camelCase 对齐前端 `e.payload.path`）。
#[derive(Clone, serde::Serialize)]
struct EditorLoadPayload {
    path: String,
}

/// M2 Task M1：打开标注编辑器装载指定 PNG。流程：校验（存在 + 可解码）→ 建窗或
/// 复用 → emit editor-load。emit 三连发（150ms 间隔，overlay-ready 同款冷加载竞态
/// 契约）：首建的 webview 冷加载期早于前端 listen 注册时首条 emit 必丢，前端以
/// **收到的最后一条**为准；补发在独立线程，命令立即返回。
#[tauri::command(async)]
pub fn editor_open(app: AppHandle, path: String) -> Result<(), String> {
    // 校验：文件存在 + PNG 可解码（ Hawksnap 编辑器只吃自家保存的 PNG；解码顺带
    // 验证非空/非损坏。几 MB 级解码在命令线程，不卡事件循环）。
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    let png = std::fs::read(&path).map_err(|e| format!("读取截图文件失败: {e}"))?;
    hawksnap_core::crop::png_to_rgba(&png).map_err(|e| format!("截图文件解码失败: {e}"))?;

    match app.get_webview_window(EDITOR_LABEL) {
        // 复用：窗口与页面常驻，新图经 editor-load 事件替换（单编辑器模型，见模块注）。
        Some(w) => {
            let _ = w.show();
            let _ = w.set_focus();
        }
        None => {
            WebviewWindowBuilder::new(
                &app,
                EDITOR_LABEL,
                WebviewUrl::App("editor?window=editor".into()),
            )
            .title("HawkSnap 编辑器") // 窗口标题静态（原生 titlebar 不走 t()；G6 范围外，注记）
            .inner_size(960.0, 640.0)
            .min_inner_size(720.0, 480.0)
            .resizable(true)
            .visible(true)
            .build()
            .map_err(|e| e.to_string())?;
        }
    }

    // emit 三连发：首条 + 独立线程 2 条补发 @150ms（覆盖层同款，无代数守卫——编辑器
    // 无流程代数概念，最后到的 path 即生效，重复 emit 同 path 幂等）。
    let payload = EditorLoadPayload { path };
    let app2 = app.clone();
    let repeats = payload.clone();
    std::thread::spawn(move || {
        for _ in 0..2 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let _ = app2.emit("hawk://editor-load", repeats.clone());
        }
    });
    app.emit("hawk://editor-load", payload).map_err(|e| e.to_string())?;
    Ok(())
}

/// 降级装载（上下文裁定：仅当 asset 协议加载失败时前端调用）：读 PNG → base64
/// dataURL。8MP PNG → ~10MB 字符串过 IPC，故只作降级路径，不作常规装载。
#[tauri::command(async)]
pub fn editor_image_dataurl(path: String) -> Result<String, String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    let png = std::fs::read(&path).map_err(|e| format!("读取截图文件失败: {e}"))?;
    hawksnap_core::crop::png_to_rgba(&png).map_err(|e| format!("截图文件解码失败: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(format!("data:image/png;base64,{b64}"))
}

/// PNG 文件魔数（RFC 2083 8 字节签名）：editor_save 的载荷校验（前端 canvas.toBlob
/// 产物必带；防误写其他字节流）。
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// 保存编辑结果（M2 Task M2）：前端 exportToPng（F5 单路径栅格化）产出的 PNG 字节
/// 以 base64 过 IPC → 校验（目标为已存在的 .png 文件 + PNG 魔数）→ 覆盖写回原路径。
///
/// 路径安全裁定（上下文预授权）：path 只来自编辑器装载过的历史/临时文件路径
/// （hawk://editor-load 原样传回），属第一方受控输入——不做目录白名单校验，仅校验
/// 「文件已存在 + 扩展名 .png（大小写不敏感）」，杜绝借编辑器写任意路径/任意格式。
#[tauri::command(async)]
pub fn editor_save(path: String, png_base64: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    let p = std::path::Path::new(&path);
    let is_png_ext = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png_ext {
        return Err("编辑器仅支持保存 PNG 文件".into());
    }
    if !p.is_file() {
        return Err(format!("目标文件不存在: {path}"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64.trim())
        .map_err(|e| format!("PNG 数据解码失败: {e}"))?;
    if bytes.len() < PNG_MAGIC.len() || bytes[..PNG_MAGIC.len()] != PNG_MAGIC {
        return Err("不是有效的 PNG 数据".into());
    }
    std::fs::write(p, bytes).map_err(|e| format!("写入截图文件失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod editor_save_tests {
    use super::*;

    /// 每测试独立临时目录（进程 id + 用例 tag 隔离，免共享状态互踩）。
    fn temp_png(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hawksnap-editor-save-{}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("shot.png")
    }

    #[test]
    fn writes_valid_png_bytes_over_the_original() {
        let path = temp_png("roundtrip");
        // 起始文件必须存在（编辑器只覆盖自家装载过的路径）
        std::fs::write(&path, b"stale-bytes").unwrap();
        let png = hawksnap_core::crop::rgba_to_png(&[10, 20, 30, 255, 40, 50, 60, 255], 2, 1).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        editor_save(path.to_string_lossy().into(), b64).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), png); // 原样落盘（不再转码——F5 像素一致）
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn rejects_bad_base64() {
        let path = temp_png("badb64");
        std::fs::write(&path, b"stale").unwrap();
        let err = editor_save(path.to_string_lossy().into(), "!!!not-base64!!!".into()).unwrap_err();
        assert!(err.contains("PNG 数据解码失败"), "got: {err}");
        // 失败不落盘（原文件保持原样）
        assert_eq!(std::fs::read(&path).unwrap(), b"stale");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn rejects_payload_without_png_magic() {
        let path = temp_png("magic");
        std::fs::write(&path, b"stale").unwrap();
        let not_png = base64::engine::general_purpose::STANDARD.encode(b"JPEG data, honestly");
        let err = editor_save(path.to_string_lossy().into(), not_png).unwrap_err();
        assert!(err.contains("不是有效的 PNG 数据"), "got: {err}");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn rejects_missing_target_and_non_png_extension() {
        let dir = std::env::temp_dir().join(format!("hawksnap-editor-save-{}-missing", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("gone.png");
        let err = editor_save(missing.to_string_lossy().into(), String::new()).unwrap_err();
        assert!(err.contains("目标文件不存在"), "got: {err}");
        // 已存在但扩展名非 .png：拒绝（防借编辑器写任意格式路径）
        let txt = dir.join("shot.txt");
        std::fs::write(&txt, b"x").unwrap();
        let png = hawksnap_core::crop::rgba_to_png(&[0, 0, 0, 255], 1, 1).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let err = editor_save(txt.to_string_lossy().into(), b64).unwrap_err();
        assert!(err.contains("仅支持保存 PNG"), "got: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_empty_path() {
        assert_eq!(editor_save("  ".into(), String::new()).unwrap_err(), "路径为空");
    }
}
