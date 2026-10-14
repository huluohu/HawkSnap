//! G3 基准模式：HAWKSNAP_BENCH=<轮数> 环境变量门控。
//! 启动后自动执行 N 轮真实捕获→确认链路（覆盖层「幽灵化」创建——webview 全路径照常：
//! 加载/取帧/绘制/overlay-shown，只是窗口内容 CSS 隐藏 + 鼠标穿透 + 不抢焦点），
//! 无键鼠注入、无可见窗口，跑完自动退出。
//! 副作用：剪贴板会被每轮覆盖（copy=true 是被测路径）。

use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::AppHandle;

use hawksnap_core::geometry::Rect;

/// HAWKSNAP_BENCH 解析：缺席/非 u64 → None（正常启动，零成本）。纯函数便于单测。
fn parse_rounds(v: Option<String>) -> Option<u64> {
    v.and_then(|s| s.trim().parse::<u64>().ok())
}

/// setup 首行调用（单实例守卫之后）：未开启基准模式时立即返回；开启则丢到独立线程
/// 跑完整链路（start_capture_flow 建窗禁主线程、其内部 block_on 亦要求非主线程，
/// 与热键/托盘入口同一纪律），不阻塞 setup。
pub fn maybe_run(app: &AppHandle) {
    let Some(rounds) = parse_rounds(std::env::var("HAWKSNAP_BENCH").ok()) else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || run_bench(app, rounds));
}

fn run_bench(app: AppHandle, rounds: u64) {
    // 等 setup 收尾（托盘/热键注册、资源就绪），不与启动竞态。
    std::thread::sleep(Duration::from_millis(1500));
    crate::overlay::HIDE_WINDOWS.store(true, Ordering::SeqCst);
    eprintln!("[bench] 基准模式：{rounds} 轮（覆盖层不可见，剪贴板将被覆盖 {rounds} 次）");
    for r in 1..=rounds {
        let baseline = crate::perf::overlay_shown_count();
        if let Err(e) = crate::overlay::start_capture_flow(&app) {
            eprintln!("[bench] 第 {r} 轮 start_capture_flow 失败: {e}");
            continue;
        }
        // 等本轮 overlay-shown（前端画出冻结帧后回报）：10ms 轮询、5s 封顶。超时仍继续
        // 确认链——该轮已污染，但跳过确认会缺 confirm/clipboard_done 行、继续确认会错位，
        // 两种结局 bench-latency.sh 的行数守护都会大声报错（宁可报错也不输出污染 P90）；
        // 选择继续走完，还能顺带暴露后续链路的真实故障点。
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while crate::perf::overlay_shown_count() == baseline {
            if std::time::Instant::now() >= deadline {
                eprintln!(
                    "[bench] 第 {r} 轮等待 overlay_shown 超时 5s——继续确认链，错位交由 bench 行数守护报出"
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // 确认链（复刻前端 onConfirm）：主显示器全帧内固定偏移取 300×200，越界钳回帧内。
        // primary_display 同源快照（id + 几何），DisplayFrame 刻意不含 id（见 commands.rs 注）。
        let Some((display_id, f)) = crate::commands::primary_display() else {
            eprintln!("[bench] 第 {r} 轮无可用捕获帧——跳过确认链（该轮将由行数守护报缺行）");
            let _ = crate::overlay::hide_all_overlays(&app);
            std::thread::sleep(Duration::from_millis(250));
            continue;
        };
        let w = (300.0_f64).min(f.w);
        let h = (200.0_f64).min(f.h);
        // 前端确认契约是 display 本地坐标（useSelection bounds 恒为原点；payload 的全局
        // x/y 混入会破坏副屏夹取）——本地 (300,200) 起固定 300×200，钳回帧内。
        let x = 300.0_f64.min(f.w - w).max(0.0);
        let y = 200.0_f64.min(f.h - h).max(0.0);
        let req = crate::commands::FinishReq {
            display_id,
            rect: Rect { x: x as i32, y: y as i32, w: w as u32, h: h as u32 },
            copy: true, // 被测路径：confirm → clipboard_done（副作用：每轮覆盖剪贴板）
            save: false,
        };
        // #[tauri::command] 保留原异步 fn，普通线程可直接 block_on 直呼（等价前端 invoke）。
        if let Err(e) =
            tauri::async_runtime::block_on(crate::commands::finish_capture(app.clone(), req))
        {
            eprintln!("[bench] 第 {r} 轮 finish_capture 失败: {e}");
        }
        let _ = crate::overlay::hide_all_overlays(&app);
        // 等 hide/资源释放落定，再开下一轮（hide 是投递主线程的异步消息）。
        std::thread::sleep(Duration::from_millis(250));
    }
    eprintln!("[bench] 完成 {rounds} 轮");
    app.exit(0); // code=Some(0)，越过 lib.rs 的「最后窗口关闭不退出」分支
}

#[cfg(test)]
mod tests {
    use super::parse_rounds;

    #[test]
    fn bench_mode_env_parses() {
        assert_eq!(parse_rounds(None), None, "环境变量缺席 → 正常启动");
        assert_eq!(parse_rounds(Some(String::new())), None, "空串无效");
        assert_eq!(parse_rounds(Some("abc".into())), None, "非数字无效");
        assert_eq!(parse_rounds(Some("-3".into())), None, "u64 拒绝负数");
        assert_eq!(parse_rounds(Some(" 5 ".into())), Some(5), "容忍首尾空白");
        assert_eq!(parse_rounds(Some("0".into())), Some(0), "0 为有效轮数（空跑即退）");
    }
}
