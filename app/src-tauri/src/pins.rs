use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use tauri::AppHandle;

use crate::commands;

// —— 贴图 Pin 窗口句柄（平台门控） ——

/// macOS：platform-mac 的原生 NSWindow 句柄（无 webview）。非 macOS：map 类型退化为
/// `HashMap<u64, ()>` 且所有操作返回 Err——贴图的 Windows/Linux 适配归 Plan D CI 矩阵
/// （plan Global Constraints）。
#[cfg(target_os = "macos")]
type PinHandle = hawksnap_platform_mac::pin::PinWindow;
#[cfg(not(target_os = "macos"))]
type PinHandle = ();

#[cfg(target_os = "macos")]
static PINS: LazyLock<Mutex<HashMap<u64, PinHandle>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
#[cfg(not(target_os = "macos"))]
static PINS: LazyLock<Mutex<HashMap<u64, PinHandle>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// 贴图 id 分配器（也作级联序号：第 n 个贴图偏移 (n%8)*24）。
static PIN_SEQ: AtomicU64 = AtomicU64::new(0);

// —— 纯逻辑（可测；窗口路径经 PinWindow 句柄） ——

/// 级联偏移（C5 测试契约）：第 n 个贴图相对基准点（主屏中心 − 半尺寸）的对角偏移。
/// `(n % 8) * 24`：相邻两贴图错开 24pt，8 个一循环——9+ 贴图不会级联出屏。
pub(crate) fn next_pin_offset(n: u64) -> (f64, f64) {
    let k = (n % 8) as f64 * 24.0;
    (k, k)
}

/// 缩放目标 clamp：当前显示比 × factor 后截到 [`hawksnap_core::geometry`] 的贴图范围
/// （0.1..=3.2 = 10%–3200%，F3 验收口径；常量下沉后与 platform-mac 滚轮路径同一来源）。
pub(crate) fn clamp_zoom(cur: f64, factor: f64) -> f64 {
    use hawksnap_core::geometry::{PIN_ZOOM_MAX, PIN_ZOOM_MIN};
    (cur * factor).clamp(PIN_ZOOM_MIN, PIN_ZOOM_MAX)
}

/// 透明度 clamp：0.2..=1.0（20%–100%，F3 验收口径；常量单一来源同上）。
pub(crate) fn clamp_opacity(opacity: f64) -> f64 {
    use hawksnap_core::geometry::{PIN_OPACITY_MAX, PIN_OPACITY_MIN};
    opacity.clamp(PIN_OPACITY_MIN, PIN_OPACITY_MAX)
}

// —— 图像来源解析 ——

/// pin 图像来源（C5）：CAPTURES 最近一轮非空整帧，或剪贴板图像（scale=1.0）。
struct PinSource {
    /// 来源标签（探针/E2E 判别用）："captures" | "clipboard"。
    source: &'static str,
    rgba: std::sync::Arc<Vec<u8>>,
    w_px: u32,
    h_px: u32,
    scale: f64,
}

/// 来源顺序（plan C5）：① CAPTURES 最后一轮的 rgba（未释放时——release_captures 后为
/// 空 Arc，跳过）；② arboard 剪贴板图像（用户刚复制的任意图像）。两者皆无 → Err
/// 「剪贴板没有图像且无最近截图」。
fn resolve_source() -> Result<PinSource, String> {
    if let Some(c) = commands::latest_capture_rgba() {
        return Ok(PinSource { source: "captures", rgba: c.rgba, w_px: c.w_px, h_px: c.h_px, scale: c.scale });
    }
    let img = arboard::Clipboard::new()
        .and_then(|mut cb| cb.get_image())
        .map_err(|e| format!("剪贴板没有图像且无最近截图: {e}"))?;
    let (w, h) = (img.width as u32, img.height as u32);
    if w == 0 || h == 0 {
        return Err("剪贴板没有图像且无最近截图: empty image".into());
    }
    Ok(PinSource { source: "clipboard", rgba: std::sync::Arc::new(img.bytes.into_owned()), w_px: w, h_px: h, scale: 1.0 })
}

// —— 命令 —--

/// F2 / 托盘：把最近截图或剪贴板图像钉成贴图窗口。async 命令（涉及窗口操作，plan
/// 线程模型纪律）：图像解析在调用线程，窗口创建经 run_on_main_thread 交付主线程
/// （原生 NSWindow 必须 main thread；与 wry 窗口的「禁止主线程」相反——那是 webview
/// 窗口的消息循环约束，AppKit 原生窗口恰恰相反）。
#[tauri::command(async)]
pub fn pin_latest(app: AppHandle) -> Result<(), String> {
    do_pin_latest(&app)
}

#[cfg(target_os = "macos")]
fn do_pin_latest(app: &AppHandle) -> Result<(), String> {
    let src = resolve_source()?;
    spawn_pin_window(app, src.source, src.rgba, src.w_px, src.h_px, src.scale)?;
    Ok(())
}

/// 共用建窗管道（M0 重构：pin_latest 与 pin_from_path 同路）：Esc 钩子注入 → 主显示器
/// 中心 − 半尺寸 + 级联偏移定位 → run_on_main_thread 建 PinWindow 入册。返回贴图 id
/// （PIN_SEQ 分配；建窗失败仅 eprintln 降级，与既有 pin_latest 口径一致）。
#[cfg(target_os = "macos")]
fn spawn_pin_window(
    app: &AppHandle,
    source: &str,
    rgba: std::sync::Arc<Vec<u8>>,
    w_px: u32,
    h_px: u32,
    scale: f64,
) -> Result<u64, String> {
    // Esc 钩子（幂等注入）：贴图按键 Esc → remove_pin_on_main（主线程回调内直接执行）
    hawksnap_platform_mac::pin::set_on_escape(remove_pin_on_main);

    // 位置：主显示器中心 − 半尺寸 + 级联偏移。captured_display_frames 为全局逻辑坐标
    // （左上原点，ScreenFrame 同源）；无捕获几何（如纯剪贴板/历史文件来源）→ 1512×982
    // 逻辑分辨率中心的回退值。
    let (cw, ch) = primary_display_center();
    let logical_w = w_px as f64 / scale;
    let logical_h = h_px as f64 / scale;
    let id = PIN_SEQ.fetch_add(1, Ordering::SeqCst);
    let (ox, oy) = next_pin_offset(id);
    let top_left = (cw - logical_w * 0.5 + ox, ch - logical_h * 0.5 + oy);

    // 探针（常驻，廉价，每次贴图一行）：E2E 判别来源/落点是否合理。
    eprintln!(
        "[hawksnap] pin: id={id} src={source} {w_px}×{h_px} scale={scale} top_left=({:.0},{:.0})",
        top_left.0, top_left.1
    );
    app.run_on_main_thread(move || {
        match hawksnap_platform_mac::pin::PinWindow::create(rgba, w_px, h_px, top_left, scale, id) {
            Ok(win) => {
                PINS.lock().unwrap_or_else(|e| e.into_inner()).insert(id, win);
            }
            Err(e) => eprintln!("[hawksnap] 贴图窗口创建失败: {e}"),
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// 非 macOS：无原生贴图窗口（Plan D CI 矩阵），统一 Err——与 do_pin_latest 同口径。
#[cfg(not(target_os = "macos"))]
fn spawn_pin_window(
    _app: &AppHandle,
    _source: &str,
    _rgba: std::sync::Arc<Vec<u8>>,
    _w_px: u32,
    _h_px: u32,
    _scale: f64,
) -> Result<u64, String> {
    Err("unsupported platform".into())
}

/// 主显示器中心（全局逻辑坐标，左上原点）：captured_display_frames 里含原点 (0,0) 的
/// 帧视为主显示器（xcap/CGDisplayBounds 与 NSScreen 同一原点口径），否则取第一个；
/// 全空（未捕获过）→ (756.0, 491.0)（1512×982 逻辑分辨率的一半，主流 Mac 回退值）。
fn primary_display_center() -> (f64, f64) {
    let frames = commands::captured_display_frames();
    let f = frames
        .iter()
        .find(|f| f.x <= 0.0 && f.y <= 0.0)
        .or_else(|| frames.first());
    match f {
        Some(f) => (f.x + f.w / 2.0, f.y + f.h / 2.0),
        None => (756.0, 491.0),
    }
}

/// Esc 钩子（仅主线程调用——AppKit keyDown 回调内）：移除贴图。Drop（含 close）
/// 发生在本调用栈上 = 主线程（PinWindow 的 AppKit 纪律，见 pin.rs SAFETY 注）。
#[cfg(target_os = "macos")]
fn remove_pin_on_main(id: u64) {
    if let Some(p) = PINS.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
        p.close(); // 幂等：orderOut + close 后 closed 置位，Drop 不再触碰 AppKit
    }
}

#[cfg(not(target_os = "macos"))]
fn do_pin_latest(_app: &AppHandle) -> Result<(), String> {
    Err("unsupported platform".into())
}

/// M0 历史操作「贴图」：把历史条目的 PNG 文件钉成贴图窗口。async 命令：读文件 +
/// 解码（core png_to_rgba，几 MB 级）在运行时线程，建窗经 spawn_pin_window 交付
/// 主线程（同 pin_latest 纪律）。落盘文件即物理像素 → scale=1.0；返回贴图 id。
#[tauri::command(async)]
pub fn pin_from_path(app: AppHandle, path: String) -> Result<u64, String> {
    do_pin_from_path(&app, &path)
}

fn do_pin_from_path(app: &AppHandle, path: &str) -> Result<u64, String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    let png = std::fs::read(path).map_err(|e| format!("读取截图文件失败: {e}"))?;
    let (rgba, w_px, h_px) = hawksnap_core::crop::png_to_rgba(&png)
        .map_err(|e| format!("截图文件解码失败: {e}"))?;
    if w_px == 0 || h_px == 0 {
        return Err("截图文件为空图".into());
    }
    spawn_pin_window(app, "history", std::sync::Arc::new(rgba), w_px, h_px, 1.0)
}

#[tauri::command(async)]
pub fn pin_set_opacity(app: AppHandle, id: u64, opacity: f64) -> Result<(), String> {
    do_pin_set_opacity(&app, id, opacity)
}

#[cfg(target_os = "macos")]
fn do_pin_set_opacity(app: &AppHandle, id: u64, opacity: f64) -> Result<(), String> {
    let op = clamp_opacity(opacity);
    app.run_on_main_thread(move || {
        if let Some(p) = PINS.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
            p.set_opacity(op);
        }
    })
    .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "macos"))]
fn do_pin_set_opacity(_app: &AppHandle, _id: u64, _opacity: f64) -> Result<(), String> {
    Err("unsupported platform".into())
}

#[tauri::command(async)]
pub fn pin_zoom(app: AppHandle, id: u64, factor: f64) -> Result<(), String> {
    do_pin_zoom(&app, id, factor)
}

#[cfg(target_os = "macos")]
fn do_pin_zoom(app: &AppHandle, id: u64, factor: f64) -> Result<(), String> {
    app.run_on_main_thread(move || {
        if let Some(p) = PINS.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
            let target = clamp_zoom(p.scale(), factor);
            p.zoom_to(target);
        }
    })
    .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "macos"))]
fn do_pin_zoom(_app: &AppHandle, _id: u64, _factor: f64) -> Result<(), String> {
    Err("unsupported platform".into())
}

#[tauri::command(async)]
pub fn pin_close(app: AppHandle, id: u64) -> Result<(), String> {
    do_pin_close(&app, id)
}

#[cfg(target_os = "macos")]
fn do_pin_close(app: &AppHandle, id: u64) -> Result<(), String> {
    app.run_on_main_thread(move || remove_pin_on_main(id)).map_err(|e| e.to_string())
}

#[cfg(not(target_os = "macos"))]
fn do_pin_close(_app: &AppHandle, _id: u64) -> Result<(), String> {
    Err("unsupported platform".into())
}

#[tauri::command(async)]
pub fn pin_close_all(app: AppHandle) -> Result<(), String> {
    do_pin_close_all(&app)
}

#[cfg(target_os = "macos")]
fn do_pin_close_all(app: &AppHandle) -> Result<(), String> {
    // 托盘入口线程派发后到此处已在异步线程；close 必须主线程 → run_on_main_thread。
    // 先取 id 快照（锁不跨 run_on_main_thread 闭包），逐个 remove（闭包内主线程执行）。
    let ids: Vec<u64> = {
        let pins = PINS.lock().unwrap_or_else(|e| e.into_inner());
        pins.keys().copied().collect()
    };
    for id in ids {
        app.run_on_main_thread(move || remove_pin_on_main(id)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn do_pin_close_all(_app: &AppHandle) -> Result<(), String> {
    Err("unsupported platform".into())
}

#[cfg(test)]
mod pins_tests {
    use super::*;

    #[test]
    fn pin_offset_cascades_deterministically() {
        // 确定性：同 n 同结果
        assert_eq!(next_pin_offset(3), next_pin_offset(3));
        // 级联：前两个偏移不同，且按 24 递增
        let (x0, y0) = next_pin_offset(0);
        let (x1, y1) = next_pin_offset(1);
        assert_ne!((x0, y0), (x1, y1), "相邻贴图必须错开");
        assert_eq!((x1 - x0, y1 - y0), (24.0, 24.0));
        // 8 个一循环（9+ 贴图不级联出屏）
        assert_eq!(next_pin_offset(8), next_pin_offset(0));
        assert_eq!(next_pin_offset(17), next_pin_offset(1));
    }

    #[test]
    fn zoom_clamps_to_10_3200_percent() {
        assert_eq!(clamp_zoom(0.2, 0.5), 0.1, "下限 0.1（10%）");
        assert_eq!(clamp_zoom(2.0, 2.0), 3.2, "上限 3.2（3200%）");
        assert_eq!(clamp_zoom(1.0, 1.0), 1.0, "中性缩放不变");
        assert_eq!(clamp_zoom(0.5, 2.0), 1.0, "正常区间内直接乘");
    }

    #[test]
    fn opacity_clamps_to_20_100_percent() {
        assert_eq!(clamp_opacity(0.1), 0.2, "下限 0.2");
        assert_eq!(clamp_opacity(1.5), 1.0, "上限 1.0");
        assert_eq!(clamp_opacity(0.5), 0.5, "区间内不变");
    }
}
