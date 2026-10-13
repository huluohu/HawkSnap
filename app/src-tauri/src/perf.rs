// perf.rs —— 性能埋点（spec G3）：进程内打点，追加写 $TMPDIR/hawksnap-perf/{pid}.csv，
// 列固定 `stage,ts_ms`。阶段契约共 6 个（Task 11 评审后）：hotkey → grab_done →
// overlay_created → overlay_shown（每轮仅记首个，见 mark_hotkey / mark_overlay_shown_once）
// → confirm → clipboard_done（仅 req.copy=true 路径）。scripts/bench-latency.sh 按同名
// 目录消费：h2o = hotkey→overlay_shown，c2c = confirm→clipboard_done。
// 打点失败一律静默（let _ =），绝不阻塞/拖垮截图主路径。
// perf.rs
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

// overlay_shown 单发门闩（评审 Finding 2a）：多显示器下每个覆盖层窗口各回报一次
// overlay-shown，全部记入会使 overlay_shown 行数多于 hotkey，bench 按行配对错位。
// hotkey 打点时置位，本轮首个 overlay_shown 打点时消费——保证两阶段行数 1:1。
static AWAIT_SHOWN: AtomicBool = AtomicBool::new(false);

/// hotkey 阶段打点并武装门闩（start_capture_flow 首行调用，热键/托盘共用）。
pub fn mark_hotkey() {
    mark("hotkey");
    AWAIT_SHOWN.store(true, Ordering::SeqCst);
}

/// 仅记录本轮首个 overlay_shown；后续事件（多显示器第二屏起、迟到/游离事件）忽略。
pub fn mark_overlay_shown_once() {
    if AWAIT_SHOWN.swap(false, Ordering::SeqCst) {
        mark("overlay_shown");
    }
}

pub fn mark(stage: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dir = std::env::temp_dir().join("hawksnap-perf");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
        .open(dir.join(format!("{}.csv", std::process::id()))) {
        let _ = writeln!(f, "{stage},{ts}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_appends_row_to_pid_csv() {
        let path = std::env::temp_dir().join("hawksnap-perf").join(format!("{}.csv", std::process::id()));
        let before = std::fs::read_to_string(&path).map(|s| s.lines().count()).unwrap_or(0);
        mark("test_stage_task11");
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), before + 1, "mark 应恰好追加一行");
        let last = content.lines().last().unwrap();
        assert!(last.starts_with("test_stage_task11,"), "unexpected last row: {last}");
        let ts: u128 = last.split(',').nth(1).unwrap().parse().unwrap();
        assert!(ts > 0, "时间戳应为 UNIX 毫秒");
    }
}
