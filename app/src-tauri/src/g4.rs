//! G4 仪器（plan D5）：HAWKSNAP_G4=1 环境变量门控的冷启动/主面板测量。
//! 与 bench_mode（HAWKSNAP_BENCH）同款思路：setup 收尾（托盘就绪）后自动拉起
//! 主面板 → 打点 panel_requested/panel_shown → stderr 输出汇总行
//! `[g4] cold_start=..ms panel=..ms` → app.exit(0)。
//! 口径：cold_start = tray_ready − process_start（进程级冷启动→托盘，目标 ≤800ms）；
//! panel = panel_shown − panel_requested（主面板打开，目标 ≤300ms）。
//! scripts/bench-g4.sh 消费 stderr 汇总行 ×5 次取中位/P90。
//! 副作用：面板会真实打开约 1 秒（面板即被测对象，不可避免——隐藏窗口无法
//! 度量「用户可见的面板就绪」）。

use std::time::{Duration, Instant};
use tauri::AppHandle;

/// HAWKSNAP_G4 解析：恰好为 "1" 才开启（其余取值视为关闭，正常启动）。纯函数便于单测。
fn enabled(v: Option<String>) -> bool {
    v.as_deref() == Some("1")
}

/// G4 模式是否被请求（enabled 的环境变量入口形态）。lib.rs run() **首行**置 perf
/// CSV 闸（M3 继承项④）时调用——process_start（G4 冷启动 t0）在 setup 之前打点，
/// 闸必须先于它置位，cold_start 两端行才完整。
pub fn requested() -> bool {
    enabled(std::env::var("HAWKSNAP_G4").ok())
}

/// lib.rs setup 托盘 build 之后调用（tray_ready 已打点）：G4 关闭时零成本返回；
/// 开启则丢独立线程跑「开面板 → 打点 → 汇总 → 退出」——ensure_main_panel 建
/// wry webview 窗口禁主线程，独立线程调用（与热键失败反馈/托盘面板入口同纪律），
/// 也不阻塞 setup 收尾。
pub fn maybe_run(app: &AppHandle) {
    if !enabled(std::env::var("HAWKSNAP_G4").ok()) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || run_g4(app));
}

fn run_g4(app: AppHandle) {
    eprintln!("[g4] 测量模式：自动打开主面板并退出（面板将短暂可见）");
    crate::perf::mark_panel_requested();
    match crate::panel::ensure_main_panel(&app) {
        Ok(_) => {
            crate::perf::mark_panel_shown();
            // 同 bench 的等待口径：轮询 panel_shown 计数确认打点完成（当前同线程
            // 打点，首查即中的快路径；保留轮询结构以兼容 panel_shown 未来改由
            // 前端首帧事件回报）。10ms 轮询、5s 封顶兜底，保证进程必然退出。
            let deadline = Instant::now() + Duration::from_secs(5);
            while crate::perf::panel_shown_count() == 0 {
                if Instant::now() >= deadline {
                    eprintln!("[g4] 等待 panel_shown 超时 5s");
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            eprintln!(
                "[g4] cold_start={} panel={}",
                fmt_ms(crate::perf::g4_cold_start_ms()),
                fmt_ms(crate::perf::g4_panel_ms()),
            );
        }
        Err(e) => {
            // 面板打开失败：panel 指标缺 honest 值 → 输出 n/a（bench-g4.sh 的
            // 正则匹配不上，该轮计为失败），冷启动照常汇报。
            eprintln!("[g4] 主面板打开失败: {e}");
            eprintln!(
                "[g4] cold_start={} panel=n/a",
                fmt_ms(crate::perf::g4_cold_start_ms()),
            );
        }
    }
    // code=Some(0)，越过 lib.rs 的「最后窗口关闭不退出」分支（同 bench_mode）。
    app.exit(0);
}

/// Some(ms) → "123ms"（汇总行格式）；None → "n/a"（正则不匹配 = 该轮失败）。
fn fmt_ms(v: Option<u64>) -> String {
    match v {
        Some(ms) => format!("{ms}ms"),
        None => "n/a".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{enabled, fmt_ms};

    #[test]
    fn g4_gate_requires_exact_one() {
        assert!(!enabled(None), "环境变量缺席 → 正常启动");
        assert!(!enabled(Some(String::new())), "空串关闭");
        assert!(!enabled(Some("0".into())), "0 关闭");
        assert!(!enabled(Some("true".into())), "非「1」值一律关闭");
        assert!(!enabled(Some(" 1".into())), "不做 trim 宽容：严格全等「1」");
        assert!(enabled(Some("1".into())), "恰为「1」才开启");
    }

    #[test]
    fn g4_summary_format_is_machine_readable() {
        assert_eq!(fmt_ms(Some(123)), "123ms", "命中 bench-g4.sh 的正则格式");
        assert_eq!(fmt_ms(None), "n/a", "失败轮正则不匹配");
    }
}
