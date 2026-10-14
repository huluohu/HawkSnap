// perf.rs —— 性能埋点（spec G3）：进程内打点，追加写 $TMPDIR/hawksnap-perf/{pid}.csv，
// 列固定 `stage,ts_ms`。阶段契约共 6 个（Task 11 评审后）：hotkey → grab_done →
// overlay_created → overlay_shown（每轮仅记首个，见 mark_hotkey / mark_overlay_shown_once）
// → confirm → clipboard_done（仅 req.copy=true 路径）。scripts/bench-latency.sh 按同名
// 目录消费：h2o = hotkey→overlay_shown，c2c = confirm→clipboard_done。
// G4 仪器（plan D5）追加 4 个启动阶段：process_start（run() 首行，冷启动 t0）→
// tray_ready（托盘 build 完）→ panel_requested/panel_shown（仅 HAWKSNAP_G4=1 自动流程，
// 见 g4.rs）。CSV 行是 epoch-ms 绝对时刻（离线差值可用）；同时缓存进原子量供 G4
// 汇总进程内直接计算（不读 CSV，同 bench_mode 口径）——两处取时刻相差微秒级，
// 不影响毫秒口径。
// 打点失败一律静默（let _ =），绝不阻塞/拖垮截图主路径。
// perf.rs
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

// overlay_shown 单发门闩（评审 Finding 2a）：多显示器下每个覆盖层窗口各回报一次
// overlay-shown，全部记入会使 overlay_shown 行数多于 hotkey，bench 按行配对错位。
// hotkey 打点时置位，本轮首个 overlay_shown 打点时消费——保证两阶段行数 1:1。
static AWAIT_SHOWN: AtomicBool = AtomicBool::new(false);

// 已记录的 overlay_shown 轮数（bench_mode 轮推进门控）：与 CSV 的 overlay_shown 行数
// 严格 1:1——自增就在同一个 if 分支里，CSV 写几行它就计几轮（锁-free，纯原子）。
static SHOWN_COUNT: AtomicU32 = AtomicU32::new(0);

// 已记录的 clipboard_done 轮数（bench_mode 进程内 c2c 计时门控）：与 CSV 的
// clipboard_done 行数严格 1:1——自增与 CSV 追加同在 mark_clipboard_done 里
// （锁-free，纯原子）。
static CLIPBOARD_DONE_COUNT: AtomicU32 = AtomicU32::new(0);

/// hotkey 阶段打点并武装门闩（start_capture_flow 首行调用，热键/托盘共用）。
pub fn mark_hotkey() {
    mark("hotkey");
    AWAIT_SHOWN.store(true, Ordering::SeqCst);
}

/// 仅记录本轮首个 overlay_shown；后续事件（多显示器第二屏起、迟到/游离事件）忽略。
pub fn mark_overlay_shown_once() {
    if AWAIT_SHOWN.swap(false, Ordering::SeqCst) {
        mark("overlay_shown");
        SHOWN_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

/// bench_mode 轮推进门控：当前进程已记录的 overlay_shown 轮数。bench 每轮记下轮前
/// 基线、轮询此值越过基线来确认「前端已画出冻结帧」，再发确认链。
pub fn overlay_shown_count() -> u32 {
    SHOWN_COUNT.load(Ordering::SeqCst)
}

/// clipboard_done 阶段打点（copy=true 路径专用，commands.rs finish_capture 调用）：
/// CSV 追加与 CLIPBOARD_DONE_COUNT 自增在同一函数里，两者严格 1:1。
pub fn mark_clipboard_done() {
    mark("clipboard_done");
    CLIPBOARD_DONE_COUNT.fetch_add(1, Ordering::SeqCst);
}

/// bench_mode 进程内 c2c 计时门控：当前进程已记录的 clipboard_done 轮数。bench 每轮
/// 记下 finish_capture 前的基线、轮询此值越过基线的瞬间作为 t_clip。
pub fn clipboard_done_count() -> u32 {
    CLIPBOARD_DONE_COUNT.load(Ordering::SeqCst)
}

// —— G4 仪器（plan D5）——
// 四个启动时刻的进程内缓存（epoch-ms，0 = 未打点）：CSV 落行供离线差值，
// 原子量供 g4.rs 的汇总线程直接计算（同 bench_mode「进程内计时」口径）。
static PROCESS_START_MS: AtomicU64 = AtomicU64::new(0);
static TRAY_READY_MS: AtomicU64 = AtomicU64::new(0);
static PANEL_REQUESTED_MS: AtomicU64 = AtomicU64::new(0);
static PANEL_SHOWN_MS: AtomicU64 = AtomicU64::new(0);

// panel_shown 轮数（G4 等待门控）：同 overlay_shown_count 模式——轮询计数越过
// 基线确认打点完成（当前同线程打点首查即中，保持接口以便 panel_shown 未来
// 改由异步事件回报时 g4.rs 无需改结构）。
static PANEL_SHOWN_COUNT: AtomicU32 = AtomicU32::new(0);

/// 当前 UNIX 毫秒（mark 与 G4 缓存共用一个取时刻入口）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 非负差值，任一端未打点（0）→ None（纯函数便于单测）。
fn delta_ms(from: u64, to: u64) -> Option<u64> {
    (from != 0 && to != 0).then(|| to.saturating_sub(from))
}

/// 进程起点打点（lib.rs run() 首行、先于 Builder）：G4 冷启动 t0。
pub fn mark_process_start() {
    let ts = now_ms();
    PROCESS_START_MS.store(ts, Ordering::SeqCst);
    mark("process_start");
}

/// 托盘就绪打点（lib.rs setup 托盘 build 之后）：G4 冷启动终点。
pub fn mark_tray_ready() {
    let ts = now_ms();
    TRAY_READY_MS.store(ts, Ordering::SeqCst);
    mark("tray_ready");
}

/// 面板请求打点（仅 G4 自动流程，g4.rs 调 ensure_main_panel 前）。
pub fn mark_panel_requested() {
    let ts = now_ms();
    PANEL_REQUESTED_MS.store(ts, Ordering::SeqCst);
    mark("panel_requested");
}

/// 面板就绪打点（仅 G4 自动流程，ensure_main_panel 成功返回后）。
pub fn mark_panel_shown() {
    let ts = now_ms();
    PANEL_SHOWN_MS.store(ts, Ordering::SeqCst);
    PANEL_SHOWN_COUNT.fetch_add(1, Ordering::SeqCst);
    mark("panel_shown");
}

/// G4 等待门控：当前进程已记录的 panel_shown 轮数（0/1）。
pub fn panel_shown_count() -> u32 {
    PANEL_SHOWN_COUNT.load(Ordering::SeqCst)
}

/// G4 冷启动 = tray_ready − process_start（毫秒；任一未打点 → None）。
pub fn g4_cold_start_ms() -> Option<u64> {
    delta_ms(
        PROCESS_START_MS.load(Ordering::SeqCst),
        TRAY_READY_MS.load(Ordering::SeqCst),
    )
}

/// G4 主面板 = panel_shown − panel_requested（毫秒；任一未打点 → None）。
pub fn g4_panel_ms() -> Option<u64> {
    delta_ms(
        PANEL_REQUESTED_MS.load(Ordering::SeqCst),
        PANEL_SHOWN_MS.load(Ordering::SeqCst),
    )
}

pub fn mark(stage: &str) {
    let ts = now_ms();
    let dir = std::env::temp_dir().join("hawksnap-perf");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
        .open(dir.join(format!("{}.csv", std::process::id()))) {
        // 整行先格式化、单次 write_all：write_fmt 按格式分段发多次 write(2)，
        // 并发打点线程之间会撕裂出行（G4 双写者单测实测）；O_APPEND 单次 write
        // 在常规文件上原子定位，小行写入互不穿插。
        let _ = f.write_all(format!("{stage},{ts}\n").as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_appends_row_to_pid_csv() {
        let path = std::env::temp_dir().join("hawksnap-perf").join(format!("{}.csv", std::process::id()));
        // 只统计本阶段行（g4_panel_metrics_follow_marks 并发写同 PID 文件的其他
        // 阶段行不算；单次 write_all 保证行不撕裂，见 mark 注）。
        let count = |s: &str| {
            s.lines().filter(|l| l.starts_with("test_stage_task11,")).count()
        };
        let before = std::fs::read_to_string(&path).map(|s| count(&s)).unwrap_or(0);
        mark("test_stage_task11");
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(count(&content), before + 1, "mark 应恰好追加一行本阶段");
        let last = content.lines()
            .filter(|l| l.starts_with("test_stage_task11,")).last().unwrap();
        let ts: u128 = last.split(',').nth(1).unwrap().parse().unwrap();
        assert!(ts > 0, "时间戳应为 UNIX 毫秒");
    }

    #[test]
    fn delta_ms_needs_both_marks_and_saturates() {
        assert_eq!(delta_ms(0, 123), None, "起点未打点 → None");
        assert_eq!(delta_ms(123, 0), None, "终点未打点 → None");
        assert_eq!(delta_ms(100, 250), Some(150), "正常差值");
        assert_eq!(delta_ms(100, 100), Some(0), "同毫秒 → 0（saturating 不回退）");
    }

    #[test]
    fn g4_panel_metrics_follow_marks() {
        // 进程级静态量：panel_requested/panel_shown 的写入方只有本测试与
        // g4.rs（后者仅在 HAWKSNAP_G4=1 真跑进程出现，单测进程不会），故断言
        // 只依赖「打点前后单调」，与测试执行顺序解耦。
        let panel_before = g4_panel_ms();
        let count_before = panel_shown_count();
        mark_panel_requested();
        assert_eq!(
            g4_panel_ms().is_some(),
            panel_before.is_some(),
            "只打 requested 不应让 panel 指标从 None 变 Some"
        );
        mark_panel_shown();
        assert_eq!(panel_shown_count(), count_before + 1, "shown 打点应恰好 +1");
        let panel_after = g4_panel_ms().expect("requested+shown 齐备后 panel 指标应为 Some");
        if let Some(p0) = panel_before {
            assert!(panel_after >= p0, "panel 指标应随打点单调不减");
        }
    }
}
