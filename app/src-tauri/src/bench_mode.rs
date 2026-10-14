//! G3 基准模式：HAWKSNAP_BENCH=<轮数> 环境变量门控。
//! 启动后自动执行 N 轮真实捕获→确认链路（覆盖层「幽灵化」创建——webview 全路径照常：
//! 加载/取帧/绘制/overlay-shown，只是窗口内容 CSS 隐藏 + 鼠标穿透 + 不抢焦点），
//! 无键鼠注入、无可见窗口。计时全部在进程内完成（每轮四个 Instant 时刻），
//! 不依赖 CSV 行配对；CSV 打点照常写入（bench-latency.sh 注入流仍可用）。
//! 副作用：剪贴板会被每轮覆盖（copy=true 是被测路径）。

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::AppHandle;

use hawksnap_core::geometry::Rect;

/// bench 循环进行中：run_bench 的轮次循环开跑即置 true，此后从不清除（基准是一次性
/// 模式，跑完即 app.exit）。供外部触发点守卫用（D0-③，Plan C 终审）——见 [`is_running`]。
static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// bench 模式进行中？外部触发入口（F1/媒体键经 trigger_capture_flow、托盘截图、面板
/// 截图按钮）应在此为 true 时忽略触发：bench 轮次直接调 start_capture_flow、不走这些
/// 外部入口，用户/托盘中途混入的一轮会污染当轮计时。bench 线程自身不查此旗标。
pub fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// HAWKSNAP_BENCH 解析：缺席/非 u64 → None（正常启动，零成本）。纯函数便于单测。
fn parse_rounds(v: Option<String>) -> Option<u64> {
    v.and_then(|s| s.trim().parse::<u64>().ok())
}

/// 最近秩 P90：升序排序后取 sorted[ceil(0.9*n)-1]（纯函数便于单测）。
/// 纯整数式 ceil(0.9*n) = (9n+9)/10，避免浮点；空集返回 0（调用方守卫 n>0）。
fn p90_nearest_rank(mut xs: Vec<u64>) -> u64 {
    if xs.is_empty() {
        return 0;
    }
    xs.sort_unstable();
    let n = xs.len();
    let rank = (9 * n + 9) / 10;
    xs[rank - 1]
}

/// 每轮计时记录：h2o = t_shown−t_hotkey，c2c = t_clip−t_confirm（毫秒）。
/// failed=true 的轮（shown/clipboard 等待超时）无有效数值，统计排除。
struct RoundStat {
    r: u64,
    h2o_ms: Option<u64>,
    c2c_ms: Option<u64>,
    failed: bool,
    exclude_why: &'static str,
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
    // D0-③：循环开跑即宣告 bench 进行中，外部触发点（热键/托盘/面板）自此忽略触发。
    RUNNING.store(true, Ordering::SeqCst);
    crate::overlay::HIDE_WINDOWS.store(true, Ordering::SeqCst);
    eprintln!("[bench] 基准模式：{rounds} 轮（覆盖层不可见，剪贴板将被覆盖 {rounds} 次）");
    let mut stats: Vec<RoundStat> = Vec::new();
    for r in 1..=rounds {
        let shown_baseline = crate::perf::overlay_shown_count();
        // t_hotkey：本轮计时起点，紧贴 start_capture_flow（其首行 mark_hotkey）。
        let t_hotkey = Instant::now();
        if let Err(e) = crate::overlay::start_capture_flow(&app) {
            eprintln!("[bench] 第 {r} 轮 start_capture_flow 失败: {e}");
            continue;
        }
        // 等本轮 overlay-shown（前端画出冻结帧后回报）：10ms 轮询、5s 封顶，观测到
        // 计数越过基线的瞬间记 t_shown（首查即中的快路径同一入口）。超时仍继续确认链
        // ——该轮已污染计为失败、统计排除（t_clip 独立门控，不受此影响），但走完链路
        // 能保持 CSV 行数整齐并暴露故障点。
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut t_shown = None;
        loop {
            if crate::perf::overlay_shown_count() != shown_baseline {
                t_shown = Some(Instant::now());
                break;
            }
            if Instant::now() >= deadline {
                eprintln!("[bench] 第 {r} 轮等待 overlay_shown 超时 5s——该轮计为失败，统计排除；继续确认链");
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // 确认链（复刻前端 onConfirm）：主显示器全帧内固定偏移取 300×200，越界钳回帧内。
        // primary_display 同源快照（id + 几何），DisplayFrame 刻意不含 id（见 commands.rs 注）。
        let Some((display_id, f)) = crate::commands::primary_display() else {
            eprintln!("[bench] 第 {r} 轮无可用捕获帧——跳过确认链（该轮计为失败，统计排除）");
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
        // t_confirm：紧贴 finish_capture（其首行 mark("confirm")）；clipboard 基线同刻
        // 读取，供下方 t_clip 轮询做「越过基线」判定。
        let clip_baseline = crate::perf::clipboard_done_count();
        let t_confirm = Instant::now();
        // #[tauri::command] 保留原异步 fn，普通线程可直接 block_on 直呼（等价前端 invoke）。
        if let Err(e) =
            tauri::async_runtime::block_on(crate::commands::finish_capture(app.clone(), req))
        {
            eprintln!("[bench] 第 {r} 轮 finish_capture 失败: {e}");
        }
        // t_clip：轮询 clipboard_done_count 越过本轮基线的瞬间（10ms 轮询、5s 封顶；
        // finish_capture 内部打点常先于返回，首查即中的快路径同一入口）。
        // 超时 → 该轮计为失败、统计排除，但照常进入下一轮，应用继续跑。
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut t_clip = None;
        loop {
            if crate::perf::clipboard_done_count() != clip_baseline {
                t_clip = Some(Instant::now());
                break;
            }
            if Instant::now() >= deadline {
                eprintln!("[bench] 第 {r} 轮等待 clipboard_done 超时 5s——该轮计为失败，统计排除");
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let failed = t_shown.is_none() || t_clip.is_none();
        let exclude_why: &'static str = if t_shown.is_none() {
            "overlay_shown 超时"
        } else {
            "clipboard_done 超时"
        };
        let stat = RoundStat {
            r,
            h2o_ms: t_shown.map(|t| (t - t_hotkey).as_millis() as u64),
            c2c_ms: t_clip.map(|t| (t - t_confirm).as_millis() as u64),
            failed,
            exclude_why,
        };
        let _ = crate::overlay::hide_all_overlays(&app);
        // 等 hide/资源释放落定，再开下一轮（hide 是投递主线程的异步消息）。
        std::thread::sleep(Duration::from_millis(250));
        stats.push(stat);
    }
    eprintln!("[bench] 完成 {rounds} 轮");
    // —— 统计汇总（进程内计时，不读 CSV）——
    // 第 1 轮无条件排除：webview 冷启动（覆盖层首次创建/加载/取帧路径未热），
    // 其 h2o 是预热成本而非稳态延迟，计入会抬高 P90 造成误读。
    let mut h2o: Vec<u64> = Vec::new();
    let mut c2c: Vec<u64> = Vec::new();
    for s in &stats {
        let (label, reason) = if s.r == 1 {
            ("排除（冷启动）", "")
        } else if s.failed {
            ("排除", s.exclude_why)
        } else {
            ("计入", "")
        };
        match (s.h2o_ms, s.c2c_ms) {
            (Some(h), Some(c)) => {
                eprintln!("[bench] round {}: h2o={h}ms c2c={c}ms {label}{}",
                    s.r, if reason.is_empty() { String::new() } else { format!("（{reason}）") });
            }
            _ => eprintln!("[bench] round {}: 失败 排除（{}）", s.r, s.exclude_why),
        }
        if s.r != 1 && !s.failed {
            if let (Some(h), Some(c)) = (s.h2o_ms, s.c2c_ms) {
                h2o.push(h);
                c2c.push(c);
            }
        }
    }
    if h2o.is_empty() {
        eprintln!("[bench] 无有效样本，无法计算 P90");
    } else {
        let (n_h2o, n_c2c) = (h2o.len(), c2c.len());
        eprintln!("[bench] h2o P90 = {}ms (n={})", p90_nearest_rank(h2o), n_h2o);
        eprintln!("[bench] c2c P90 = {}ms (n={})", p90_nearest_rank(c2c), n_c2c);
    }
    app.exit(0); // code=Some(0)，越过 lib.rs 的「最后窗口关闭不退出」分支
}

#[cfg(test)]
mod tests {
    use super::{parse_rounds, p90_nearest_rank};

    #[test]
    fn bench_mode_env_parses() {
        assert_eq!(parse_rounds(None), None, "环境变量缺席 → 正常启动");
        assert_eq!(parse_rounds(Some(String::new())), None, "空串无效");
        assert_eq!(parse_rounds(Some("abc".into())), None, "非数字无效");
        assert_eq!(parse_rounds(Some("-3".into())), None, "u64 拒绝负数");
        assert_eq!(parse_rounds(Some(" 5 ".into())), Some(5), "容忍首尾空白");
        assert_eq!(parse_rounds(Some("0".into())), Some(0), "0 为有效轮数（空跑即退）");
    }

    #[test]
    fn p90_nearest_rank_picks_ceil_09n() {
        assert_eq!(p90_nearest_rank((1..=10).collect()), 9, "[1..=10] → 第 9 个");
        assert_eq!(p90_nearest_rank(vec![1, 2, 3]), 3, "[1,2,3] → ceil(2.7)=3 → 3");
        assert_eq!(p90_nearest_rank(vec![7]), 7, "单样本 → 自身");
    }
}
