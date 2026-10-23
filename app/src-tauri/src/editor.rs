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
