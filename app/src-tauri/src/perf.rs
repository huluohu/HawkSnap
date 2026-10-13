// perf.rs —— 性能埋点（spec G3）：进程内打点，追加写 $TMPDIR/hawksnap-perf/{pid}.csv，
// 列固定 `stage,ts_ms`。阶段名是契约：hotkey → grab_done → overlay_created →
// overlay_shown → clipboard_done；scripts/bench-latency.sh 按同名目录消费统计 P90。
// 打点失败一律静默（let _ =），绝不阻塞/拖垮截图主路径。
// perf.rs
use std::io::Write;
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
