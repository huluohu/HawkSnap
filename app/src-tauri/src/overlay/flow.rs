use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use super::delay::{DELAY_ABORT, DELAY_ACTIVE};
use crate::commands;

/// 截图流程代数（单调递增）：每轮 start_capture_flow 分配一个新代数并写入该轮全部
/// DisplayPayload.flow。overlay-ready 的 300ms 补发线程在每次发射前重查此值——已有
/// 更新一代启动则旧代补发自弃（否则快速连按 F1 时旧轮迟到补发会以陈旧 payload 覆盖
/// 新轮显示，且陈旧 raw 文件已被下一轮删除，画布冻结）。
static FLOW_SEQ: AtomicU64 = AtomicU64::new(0);

// —— Plan B Task 5：智能吸附 AX 悬停探测 ——

/// 悬停探测总闸：是否有任一覆盖层可见。start_capture_flow 在窗口就绪后置 true；
/// hide_all_overlays 隐藏全部覆盖层后落 false。悬停 handler 首行检查此门——
/// 覆盖层全隐时空转成本趋近于零（一次原子读即返回），不烧 CPU、不做 AX 查询。
pub(crate) static ANY_OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(false);

/// bench_mode 门控（G3 基准）：置 true 后覆盖层「幽灵化」——窗口仍要上屏，但
/// set_ignore_cursor_events(true) + 注入 CSS `visibility:hidden` 隐藏页面全部内容
/// （窗口本就 transparent(true)，故上屏后无任何可见像素）+ 绝不 set_focus。
///
/// 为什么不是纯 `visible(false)`：实测 macOS WKWebView 在从未上屏的窗口里不执行任何
/// JS（evaluateJavaScript 不运行、页面链路停摆），invisible 覆盖层永远等不到
/// overlay-shown。上屏但不可感知 = AppKit/WebKit 眼中全速运行 webview 全路径
/// （加载/取帧/绘制/overlay-shown 照常），用户侧零感知、零输入拦截。
pub static HIDE_WINDOWS: AtomicBool = AtomicBool::new(false);

// 仅测试用：镜像 commands::DisplayPayload 的序列化契约（camelCase），供下方契约测试断言
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPayloadForTest {
    pub display_id: u64, pub x: i32, pub y: i32,
    pub width: u32, pub height: u32, pub scale: f64, pub frame_url: String,
    pub flow: u64,
}

// 必须在非主线程调用（register_hotkeys 已保证；Task 10 的托盘入口同样需要放到线程里）。
pub fn start_capture_flow(app: &AppHandle) -> Result<(), String> {
    // 修复（终审 wave）：手动 F1 撞上进行中的延时截图倒计时时，先置中止旗标再走本流程
    // ——延时 worker 的轮询循环每 ≤50ms 检查 DELAY_ABORT，见真即隐藏倒计时窗、清
    // DELAY_ACTIVE、直接返回（跳过它自己到点的 start_capture_flow）。否则倒计时归零与
    // 手动 F1 各触发一轮截图，用户连得两轮覆盖层。只置旗标不等待：中止由 worker 单点
    // 负责（与 cancel_delayed_capture 同款分工），本流程立即继续。
    if DELAY_ACTIVE.load(Ordering::SeqCst) {
        DELAY_ABORT.store(true, Ordering::SeqCst);
    }
    // Plan D Task D4（修 Plan B 裁定的静默壁纸缺陷）：抓屏前屏幕录制权限预检。macOS 14+
    // 未授权时抓屏 API 不报错、静默返回纯壁纸帧——用户看到"冻结的画面"却毫无解释。预检
    // 未过 → 尽力打开系统设置的屏幕录制面板 + 拒绝本轮截图（Err 经既有
    // hawk://capture-error → App.tsx 横幅链回流，前端零改动）。
    // bench 守卫（裁定 1）：bench 量链路延迟，与本机授权态无关（bench 宿主可能从未授权），
    // HIDE_WINDOWS 为真（bench 轮次进行中）时跳过预检，基准轮次照常。
    // 覆盖面：F1 / 媒体键（trigger_capture_flow）/ 托盘 / 面板按钮全部经本入口，
    // 一处预检全路径生效。
    if !HIDE_WINDOWS.load(Ordering::SeqCst)
        && !hawksnap_platform_mac::permissions::screen_recording_prechecked()
    {
        // 引导是尽力而为：spawn 不等待（open 手递 LaunchServices 即返回），失败仅记日志
        // ——拒绝横幅照发，用户仍可手动前往设置。
        if let Err(e) = hawksnap_platform_mac::permissions::open_screen_recording_settings() {
            eprintln!("[hawksnap] 打开屏幕录制权限设置失败: {e}");
        }
        // Rust 字面量（G6 i18n 范围裁定：Rust 侧文案不入 t() 字典），面向用户可读中文。
        return Err("屏幕录制权限未授予：已为你打开系统设置，授权后重新按 F1 即可".into());
    }
    // Task 11 埋点：热键/托盘共用此入口，首行即触发时刻（stage: hotkey）；
    // mark_hotkey 同时武装 overlay_shown 单发门闩（多显示器只记首个 shown，与 hotkey 1:1）
    crate::perf::mark_hotkey();
    let mut displays = tauri::async_runtime::block_on(commands::capture_screens(app.clone()))?;
    crate::perf::mark("grab_done"); // 抓屏成功返回
    // Plan B Task 1（G3-1）：覆盖层窗口常驻复用——首轮 build（webview 冷加载 ~267ms），
    // 之后每轮仅 set_position/set_size/show，页面由 overlay-ready 的新 payload 重绘，
    // 砍掉每轮的冷加载段。show/hide/set_position 都是投递给主线程事件循环的消息，
    // 非主线程调用安全（close 特殊，见 hide_all_overlays 注）。
    for d in &displays {
        let label = format!("overlay-{}", d.display_id);
        if let Some(w) = app.get_webview_window(&label) {
            // 复用：窗口与页面常驻，仅更新几何并显示。页面经 overlay-ready 的新 payload 重绘。
            let _ = w.set_position(tauri::LogicalPosition::new(d.x as f64, d.y as f64));
            let _ = w.set_size(tauri::LogicalSize::new(d.width as f64, d.height as f64));
            // bench_mode：上屏唤醒 WebKit（幽灵化在首建时已生效且跨轮持久：CSS 隐藏 +
            // 鼠标穿透），绝不 set_focus 抢焦点。
            if HIDE_WINDOWS.load(Ordering::SeqCst) {
                let _ = w.show();
            } else {
                let _ = w.show();
                let _ = w.set_focus();
            }
            continue;
        }
        // 首建路径（防御：复用模式下同 label 不会重建，除非窗口被外部强关）。
        // 旧实现的 close 旧窗 + 500ms 轮询等注销块已删除——复用模式下不存在“重建活 label”。
        // query 让前端 main.tsx 按 ?window=overlay 路由到覆盖层页面（dev 走 SPA fallback，
        // 生产 tauri 对 dist 亦以 index.html 兜底）
        let win = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("overlay?window=overlay".into()))
            .position(d.x as f64, d.y as f64)
            .inner_size(d.width as f64, d.height as f64)
            .decorations(false).transparent(true).always_on_top(true)
            .skip_taskbar(true).resizable(false)
            // bench_mode：不可见创建——窗口从不上屏，webview 全路径照常。
            .visible(!HIDE_WINDOWS.load(Ordering::SeqCst))
            .build().map_err(|e| e.to_string())?;
        // 注入窗口 label：页面加载前 wry 会把 eval 暂存到 pending_scripts，导航完成后执行，
        // 故此处注入不惧与首帧加载竞态；display_id 为 u64，无需转义。
        let _ = win.eval(&format!("window.__LABEL__='overlay-{}';", d.display_id));
        if HIDE_WINDOWS.load(Ordering::SeqCst) {
            // bench_mode 幽灵化：页面内容 CSS 隐藏（visibility 可继承，React 后续渲染的
            // 选区/浮条一并隐藏；首帧前的暂存期与页面背景均透明，上屏无闪烁）。在
            // show() 之前先建穿透，用户点击永不落入本窗。
            let _ = win.eval("document.documentElement.style.visibility='hidden';");
            let _ = win.set_ignore_cursor_events(true);
            let _ = win.show(); // 从未上屏的窗口里 WKWebView 不执行 JS（见 HIDE_WINDOWS 注）
        } else {
            let _ = win.set_focus();
        }
    }
    crate::perf::mark("overlay_created"); // 覆盖层窗口就绪（build 或复用 show 完成；每轮一次，便于脚本按索引配对）
    // Plan B Task 5：覆盖层已就绪 → 开启悬停探测总闸（handler 首行检查，见
    // ANY_OVERLAY_VISIBLE 注）；hide_all_overlays 隐藏全部覆盖层后落下。
    ANY_OVERLAY_VISIBLE.store(true, Ordering::SeqCst);
    // overlay-ready 在窗口 overlay 路由的前端挂载前可能先到（首次 emit 会丢）。
    // 简化实现：共发 3 次、间隔 150ms，前端以收到的最后一次为准（Task 9 契约）。
    // 重复发送放在独立线程，避免热键处理被拖慢 450ms。
    //
    // 修复（终审 wave，代数守卫）：本轮分配新代数写入全部 payload。补发线程 spawn 时
    // 捕获 my_flow，每次发射前重查 FLOW_SEQ——期间用户已触发新一轮（代数已推进）→
    // 旧代剩余补发自弃。若无此守卫，快速连按 F1 时旧轮迟到补发会以陈旧 payload 覆盖
    // 新轮显示，而陈旧 payload 的 raw 文件已被新一轮 capture_screens 删除（画布冻结），
    // 且 payload 到达触发的 reset() 会杀掉用户新一轮进行中的拖拽。前端按 payload.flow
    // 丢弃旧代（index.tsx 的 maxFlowRef），双端防线互补。
    let flow = FLOW_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    for d in displays.iter_mut() {
        d.flow = flow;
    }
    let app2 = app.clone();
    let repeats = displays.clone();
    let my_flow = flow;
    std::thread::spawn(move || {
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(150));
            if FLOW_SEQ.load(Ordering::SeqCst) != my_flow {
                return; // 已有更新一代流程启动：本线程的剩余补发全部作废
            }
            let _ = app2.emit("hawk://overlay-ready", repeats.clone());
        }
    });
    app.emit("hawk://overlay-ready", displays).map_err(|e| e.to_string())
}

// —— Task 9 薄命令 ——

/// 覆盖层取消：隐藏**全部**覆盖层（Task 7 多显示器语义收敛——任一屏 Esc，各屏覆盖层
/// 一致隐藏，不再残留他屏覆盖层；单显示器行为不变）。displayId 保留为前端 invoke 契约
/// （Tauri 命令按参数名匹配），隐藏语义不再限于发起窗口。
#[tauri::command]
pub async fn cancel_capture(app: AppHandle, display_id: u64) -> Result<(), String> {
    let _ = display_id; // 契约参数，语义见上
    hide_all_overlays(&app)
}

/// 覆盖层确认完成：广播 capture-done（payload = outcome，契约不变）后隐藏**全部**
/// 覆盖层（Task 7）。CaptureOutcome 不携带 displayId，无法走“setup 监听再关窗”链，
/// 故由本命令自带目标（Task 9 裁定 2，跳过 setup 监听）。
#[tauri::command]
pub async fn emit_capture_done(app: AppHandle, display_id: u64, outcome: commands::CaptureOutcome) -> Result<(), String> {
    app.emit("hawk://capture-done", &outcome).map_err(|e| e.to_string())?;
    let _ = display_id; // 契约参数，隐藏语义收敛为全部覆盖层（Task 7）
    hide_all_overlays(&app)
}

/// 常驻复用：隐藏而非关闭，窗口由 start_capture_flow 复用（Plan B Task 1）。命令在
/// 异步运行时线程执行（非主线程），hide() 是投递给主线程事件循环的消息，安全
/// （close 须避开主线程，见 register_hotkeys 注；此处本就非主线程，hide 更无此顾虑）。
///
/// 迭代安全（self-review 结论，无需先收集 label）：`webview_windows()` 返回自持的
/// owned HashMap，且 hide 只是投递异步消息——不在此调用内同步增删窗口集合，迭代器
/// 不失效；即便窗口被并发外部强关，手里仍是 owned 句柄，对其 hide 优雅返回 Err 而非
/// panic。单窗 hide 失败不中断其余窗口（记最后一个错误返回）。
///
/// 悬停总闸：全部 overlay-* 均已（请求）隐藏 → 直接落 false，无需再实查可见性。
/// 与 start_capture_flow 并发的极端竞态（新流程 store(true) 后旧取消才 store(false)）
/// 可自愈：下一轮流程会重新置 true。
pub(crate) fn hide_all_overlays(app: &AppHandle) -> Result<(), String> {
    let mut last_err: Option<String> = None;
    for (label, w) in app.webview_windows() {
        if label.starts_with("overlay-") {
            if let Err(e) = w.hide() {
                last_err = Some(e.to_string());
            }
        }
    }
    ANY_OVERLAY_VISIBLE.store(false, Ordering::SeqCst);
    // 修复（终审 wave，G2）：流程到此结束（本时刻起没有任何覆盖层能再确认），释放
    // CAPTURES 钉住的整帧 RGBA（~24-59MB/屏）——此前它常驻至下一轮覆盖，是空载残余
    // 内存的主力。frame_path 保留（下一轮 capture_screens 删除上一轮冻结帧仍需要）。
    // 释放后对已释放条目调 finish_capture 会拿到空 rgba 而报「display not captured」
    // 式错误——可接受：本函数恰在没有任何覆盖层能确认的时刻执行，唯一能走到该错误的
    // 是早已越过代际守卫的僵尸链。锁内只做 Arc 替换，无 IO，锁窗口极短。
    commands::release_captures();
    last_err.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_serializes_camel_case() {
        let p = DisplayPayloadForTest {
            display_id: 7, x: 0, y: 0, width: 100, height: 50, scale: 2.0,
            frame_url: "hawk-grab:///tmp/a.raw".into(),
            flow: 3,
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"displayId\":7"));
        // Task 2b：冻结帧改为原始 RGBA 文件，payload 字段随之更名 pngUrl → frameUrl
        assert!(s.contains("\"frameUrl\":\"hawk-grab:///tmp/a.raw\""));
        // 修复（终审 wave）：payload 携带流程代数（单词字段 camelCase 无操作），
        // 前端按 flow 丢弃旧代迟到补发（代数守卫）
        assert!(s.contains("\"flow\":3"));
    }
}
