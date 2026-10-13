use hawksnap_core::capture::ScreenGrabber;
use hawksnap_core::geometry::Rect;
use hawksnap_core::{crop::{crop_rgba, rgba_to_png}, naming::{expand, NamingCtx}};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPayload {
    pub display_id: u64, pub x: i32, pub y: i32,
    pub width: u32, pub height: u32, pub scale: f64, pub png_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishReq { pub display_id: u64, pub rect: Rect, pub copy: bool, pub save: bool }

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOutcome { pub copied: bool, pub saved_to: Option<String> }

pub fn to_physical_rect(logical: &Rect, scale: f64) -> Rect {
    Rect {
        x: (logical.x as f64 * scale).round() as i32,
        y: (logical.y as f64 * scale).round() as i32,
        w: (logical.w as f64 * scale).round() as u32,
        h: (logical.h as f64 * scale).round() as u32,
    }
}

pub fn save_path_for(dir: &std::path::Path, app_name: &str) -> std::path::PathBuf {
    let now = chrono::Local::now();
    let ctx = NamingCtx {
        app: app_name.to_string(),
        date: now.format("%Y-%m-%d").to_string(),
        time: now.format("%H%M%S").to_string(),
    };
    let name = expand("{app} {date} {time}", &ctx, |n| dir.join(format!("{n}.png")).exists());
    dir.join(format!("{name}.png"))
}

#[tauri::command]
pub async fn capture_screens(app: AppHandle) -> Result<Vec<DisplayPayload>, String> {
    let mut grabber = hawksnap_platform_mac::XcapGrabber::new();
    let frames = grabber.grab_all().map_err(|e| e.to_string())?;
    let cache = app.path().cache_dir().map_err(|e| e.to_string())?.join("hawksnap/grabs");
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(frames.len());
    let mut captures = CAPTURES.lock().unwrap_or_else(|e| e.into_inner());
    for f in frames {
        // Plan B Task 1 cache-bust：覆盖层窗口常驻复用后页面不再重载，同路径覆盖写会命中
        // Webview 的 URL 缓存而显示上一轮旧帧，故文件名带毫秒时间戳强制每轮重新加载
        // （asset 协议对 query 参数不可靠，版本化文件名是确定性方案；仍在 $CACHE/** scope 内）。
        // 写入前删除上一轮同 display 的冻结帧，缓存目录不随轮数增长。
        if let Some(prev) = captures.get(&f.display_id) {
            let _ = std::fs::remove_file(&prev.png_path);
        }
        let file = cache.join(format!(
            "{}-{}.png",
            f.display_id,
            chrono::Local::now().timestamp_millis()
        ));
        std::fs::write(&file, &f.png).map_err(|e| e.to_string())?;
        // raw-RGBA 捕获缓存（Plan B Task 2）：物理尺寸按 (width*scale).round() 计算，
        // finish_capture 直通行复制裁剪与剪贴板，全程不再读 PNG 文件。PNG 仅供前端
        // asset 协议显示冻结帧；png_path 仅供下一轮删除上一轮冻结帧。
        let (pw, ph) = (
            (f.width as f64 * f.scale).round() as u32,
            (f.height as f64 * f.scale).round() as u32,
        );
        captures.insert(f.display_id, CaptureMeta {
            scale: f.scale, width_px: pw, height_px: ph,
            rgba: std::sync::Arc::new(f.rgba.clone()), png_path: file.clone(),
        });
        out.push(DisplayPayload {
            display_id: f.display_id, x: f.x, y: f.y,
            width: f.width, height: f.height, scale: f.scale,
            png_url: format!("hawk-grab://{}", file.display()),
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn finish_capture(req: FinishReq) -> Result<CaptureOutcome, String> {
    // Task 11 评审 Finding 1：前端 invoke 即确认时刻，在函数首行打点（stage: confirm）。
    // G3 的确认→剪贴板耗时 = clipboard_done - confirm；旧口径 overlay_shown→clipboard_done
    // 混入了人工框选时间，无法作为 ≤100ms 目标的证据。
    crate::perf::mark("confirm");
    // raw-RGBA 直通（Plan B Task 2）：不再读冻结 PNG。锁内只做查找 + Arc clone 并立即
    // 释放；行复制裁剪与剪贴板全程无锁（单用户场景总耗时 <50ms，无需 spawn_blocking）。
    let meta = { CAPTURES.lock().unwrap_or_else(|e| e.into_inner()).get(&req.display_id).cloned() }
        .ok_or("display not captured")?;
    let physical = to_physical_rect(&req.rect, meta.scale);
    let cropped = crop_rgba(&meta.rgba, meta.width_px, meta.height_px, &physical)
        .map_err(|e| e.to_string())?;
    // 3) 剪贴板：直接送 RGBA，省去裁剪 PNG 重编码 + 再解码两个环节
    let mut copied = false;
    if req.copy {
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        // clone：req.save 同时为 true 时 cropped 还要经 rgba_to_png 落盘
        cb.set_image(arboard::ImageData {
            width: physical.w as usize,
            height: physical.h as usize,
            bytes: cropped.clone().into(),
        })
            .map_err(|e| e.to_string())?;
        copied = true;
        // Task 11 埋点：clipboard_done 仅在 req.copy=true 路径打点（M1 流程恒为 true）。
        // 若未来支持“仅保存”，confirm 与 clipboard_done 行数将不等，bench 长度校验会
        // 报“数据不齐”——属预期守护而非缺陷。
        crate::perf::mark("clipboard_done");
    }
    // 4) 保存：PNG 编码只发生在保存路径
    let mut saved_to = None;
    if req.save {
        let dir = dirs_picture_dir().ok_or("no picture dir")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = save_path_for(&dir, &frontmost_app_name().unwrap_or_else(|| "HawkSnap".into()));
        let png = rgba_to_png(&cropped, physical.w, physical.h).map_err(|e| e.to_string())?;
        std::fs::write(&path, &png).map_err(|e| e.to_string())?;
        saved_to = Some(path.to_string_lossy().into_owned());
    }
    Ok(CaptureOutcome { copied, saved_to })
}

use std::sync::{LazyLock, Mutex};
use std::collections::HashMap;
// raw-RGBA 捕获缓存（Plan B Task 2，取代 FRAME_META）：每 display 存冻结帧的 scale、
// 物理尺寸与整帧 RGBA（Arc 共享，finish_capture 锁外裁剪）。png_path 仅供下一轮
// capture_screens 删除上一轮冻结帧文件（Task 1 版本化文件名方案仍生效）。
// HashMap::new 非 const fn，static 用 LazyLock 包装。
static CAPTURES: LazyLock<Mutex<HashMap<u64, CaptureMeta>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
#[derive(Clone)]
struct CaptureMeta {
    scale: f64,
    width_px: u32,
    height_px: u32,
    rgba: std::sync::Arc<Vec<u8>>,
    png_path: std::path::PathBuf, // 仅 capture_screens 删除上一轮冻结帧用
}

fn dirs_picture_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Pictures/HawkSnap"))
}

fn frontmost_app_name() -> Option<String> { None /* Plan B 接 window 检测后替换 */ }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_rect_maps_to_physical() {
        // 逻辑 (10,20,100,50) @2x → 物理 (20,40,200,100)
        let p = to_physical_rect(
            &hawksnap_core::geometry::Rect { x: 10, y: 20, w: 100, h: 50 }, 2.0);
        assert_eq!((p.x, p.y, p.w, p.h), (20, 40, 200, 100));
    }

    #[test]
    fn save_uses_template_and_avoids_collision() {
        let dir = std::env::temp_dir().join(format!("hs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = save_path_for(&dir, "Safari");
        assert!(first.to_string_lossy().contains("Safari"));
        std::fs::write(&first, b"1").unwrap();
        let second = save_path_for(&dir, "Safari");
        assert_ne!(first, second, "重名时必须生成不同文件名");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
