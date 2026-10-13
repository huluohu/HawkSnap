//! 空闲覆盖层 webview 回收看护（Plan B Task 9 应急步骤，G2 空载内存预算）。
//!
//! 动机：Plan B Task 1 为压 P90 延迟把覆盖层 webview 常驻（取消/完成只 hide 不
//! close），实测每个空闲覆盖层 webview 常驻约 ~30MB；G2 预算（spec §5：空载
//! ≤80MB）被这些常驻 webview 吃穿（fresh-idle 实测 100.7MB）。处置：看护线程每
//! 30s 扫一轮，隐藏持续满 5 分钟（IDLE_TTL）未复用的 overlay-*/countdown 窗口
//! close()；下次截图走 start_capture_flow 的既有首建分支重建，代价是一轮 webview
//! 冷加载（~267ms）——落在基准 P90 口径外的一轮离群值，不影响达标。
//!
//! 安全性：可见窗口绝不回收（可见/已消失不在隐藏集合 → 登记簿条目即删；close 前
//! 二次复核 is_visible）；close() 对已消失窗口是 best-effort（Err 吞掉）；登记簿不
//! 无界增长（expire_labels 每轮删除可见/已消失条目 + reap_tick 兜底清理已不存在
//! 窗口的条目）。close() 从非主线程调用安全（与 overlay.rs 建窗/关窗同款约束，本
//! 看护为普通 std::thread）。

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

/// 空闲阈值：隐藏持续满 5 分钟未复用即回收（G2 应急裁定值）。
const IDLE_TTL: Duration = Duration::from_secs(5 * 60);

/// 看护轮询周期：30s。回收延迟上界 = IDLE_TTL + 一个周期。
const TICK: Duration = Duration::from_secs(30);

/// 空闲登记簿（进程级）：label → 首次观测到隐藏的时刻。HashMap::new 非 const fn，
/// static 用 LazyLock 包装（与 commands::CAPTURES 同款）；锁中毒容忍同
/// overlay::HOTKEY_ERROR（unwrap_or_else into_inner）。
static IDLE_BOOK: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 每轮扫描的空闲状态机（纯逻辑，可单测）——首次观测插入、老化回收、未满保留、
/// 可见/消失重置，四态全部收敛在本函数（契约原文：「callers see the full state
/// machine in one function」）。
///
/// `visible`（参数名沿用任务契约签名）：本轮观测到「窗口仍存在且当前隐藏」的候选
/// label 集合，即空闲回收候选。集合缺席 = 窗口当前可见或已不存在，不属空闲态——
/// 对应契约规则「可见 → 从登记簿删除（不计时）」：登记簿里的残留条目一律删除，
/// 下次再隐藏时按首次观测重新起算。（契约同时要求本函数完成隐藏首见条目的插入，
/// 单个集合参数故只能承载隐藏候选——缺席即可见/不存在，二者在本状态机里同义。）
///
/// 规则（对每轮调用）：
/// - 集合中的 label 不在登记簿 → 以 `now` 插入（首次观测，空闲计时起点）；
/// - 登记簿中的 label 在集合中且 `now - 条目` ≥ `ttl` → 计入返回值并删除条目；
/// - 登记簿中的 label 在集合中但未满 `ttl` → 保留（继续累计）；
/// - 登记簿中的 label 不在集合中 → 删除（可见/已消失，状态重置）。
///
/// 返回：本轮满期待销毁的 label 列表（顺序随 HashMap 迭代序，不保证稳定）。
pub fn expire_labels(
    book: &mut HashMap<String, Instant>,
    visible: &HashSet<String>,
    now: Instant,
    ttl: Duration,
) -> Vec<String> {
    // 首次观测：隐藏但尚未登记 → 以 now 起算（新条目 elapsed=0，必走「未满」分支）。
    for label in visible {
        book.entry(label.clone()).or_insert(now);
    }
    let mut expired = Vec::new();
    book.retain(|label, seen_since| {
        if !visible.contains(label) {
            return false; // 可见或已不存在：不属空闲态，重置（出簿）
        }
        if now.duration_since(*seen_since) >= ttl {
            expired.push(label.clone());
            return false; // 满期待销毁：出簿（交调用方 close）
        }
        true // 隐藏中未满：继续累计
    });
    expired
}

/// 看护线程入口：setup 末尾（托盘之后）spawn，进程生命周期常驻。先睡一轮再扫
/// （启动期覆盖层尚未建，首轮空转无意义）。轮内失败一律 eprintln 吞掉，绝不影响
/// 截图主链路。
pub fn spawn_watchdog(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        reap_tick(&app);
    });
}

/// 单轮回收：快照候选窗口可见性 → expire_labels → 逐个 close（best-effort）。
fn reap_tick(app: &tauri::AppHandle) {
    // 快照：overlay-* + countdown 为候选（与 hide_all_overlays 过滤同口径；tauri.conf
    // 的主窗口不在列）。webview_windows() 返回 owned HashMap，is_visible 只是查询，
    // 循环内不增删窗口，迭代安全（同 hide_all_overlays 的 self-review 结论）。
    let mut hidden = HashSet::new();
    let mut existing = HashSet::new();
    for (label, w) in app.webview_windows() {
        if !(label.starts_with("overlay-") || label == "countdown") {
            continue;
        }
        existing.insert(label.clone());
        // 查询 Err（窗口恰在关闭中）按「不在集合」处理：登记簿条目被删、不回收；
        // 下轮若窗口仍在且隐藏则按首见重新起算——保守方向，绝不误伤。
        if matches!(w.is_visible(), Ok(false)) {
            hidden.insert(label);
        }
    }
    let expired = {
        let mut book = IDLE_BOOK.lock().unwrap_or_else(|e| e.into_inner());
        let expired = expire_labels(&mut book, &hidden, Instant::now(), IDLE_TTL);
        // 兜底清理：label 已不存在的条目（expire_labels 已按「集合缺席」删除，此
        // retain 为任务契约要求的显式防线，防未来集合语义调整后漏删导致无界增长）。
        book.retain(|label, _| existing.contains(label));
        expired
    };
    for label in expired {
        let Some(w) = app.get_webview_window(&label) else { continue };
        // 二次复核：快照与 close 之间窗口可能重新可见（新截图流程恰在此毫秒级窗口
        // 启动极罕见，但「绝不回收可见窗口」是无条件红线）。Err 视为不可确认，跳过。
        if matches!(w.is_visible(), Ok(false)) {
            // 修复（终审 wave，close vs 在途 show 竞态）：overlay-* 额外要求悬停总闸
            // 为 false 才准 close——总闸只在 start_capture_flow 窗口就绪后置 true、
            // hide_all_overlays 后落 false，恰补住「is_visible 快照读作隐藏，但新一轮
            // 流程刚 post 了 show、尚未置总闸/尚未被系统渲染为可见」的毫秒级窗口。在途
            // show 时 close 会与新流程赛跑（竞态销毁刚复用的窗口）。countdown 前缀不同，
            // 不受此门影响，仍按 ttl 原样回收。
            if label.starts_with("overlay-")
                && crate::overlay::ANY_OVERLAY_VISIBLE.load(std::sync::atomic::Ordering::SeqCst)
            {
                continue;
            }
            let _ = w.close(); // 已消失/关闭失败均 best-effort，下轮按需重走首建
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用短阈值：偏移取毫秒级，`now - TTL` 只需亚秒时钟余量（Instant::now()
    /// 为系统启动起的单调时钟，开机即跑测试也不 panic），测试无时序脆弱性。
    const TEST_TTL: Duration = Duration::from_millis(100);

    #[test]
    fn first_sighting_inserts_hidden_label() {
        // 首次观测：隐藏 label 尚未入簿 → 以 now 插入（计时起点），无回收
        let now = Instant::now();
        let mut book = HashMap::new();
        let hidden = HashSet::from(["overlay-1".to_string()]);
        let expired = expire_labels(&mut book, &hidden, now, TEST_TTL);
        assert!(expired.is_empty());
        assert_eq!(book.get("overlay-1"), Some(&now));
    }

    #[test]
    fn visible_label_is_removed_from_book() {
        // 可见（不在隐藏集合）→ 登记簿条目删除、不计时（条目即使很「新」也删，
        // 与「未满保留」分支区分开）
        let now = Instant::now();
        let mut book = HashMap::from([("overlay-1".to_string(), now)]);
        let hidden = HashSet::new();
        let expired = expire_labels(&mut book, &hidden, now, TEST_TTL);
        assert!(expired.is_empty());
        assert!(book.is_empty());
    }

    #[test]
    fn ttl_elapsed_label_is_collected() {
        // 满期（>= 含恰等边界）：回收 + 出簿，返回值逐项精确
        let now = Instant::now();
        let mut book = HashMap::from([("countdown".to_string(), now - TEST_TTL)]);
        let hidden = HashSet::from(["countdown".to_string()]);
        let expired = expire_labels(&mut book, &hidden, now, TEST_TTL);
        assert_eq!(expired, vec!["countdown".to_string()]);
        assert!(book.is_empty());
    }

    #[test]
    fn young_hidden_label_is_kept() {
        // 隐藏未满：原条目原时刻保留，无回收
        let now = Instant::now();
        let mut book = HashMap::from([("overlay-2".to_string(), now)]);
        let hidden = HashSet::from(["overlay-2".to_string()]);
        let expired = expire_labels(&mut book, &hidden, now, TEST_TTL);
        assert!(expired.is_empty());
        assert_eq!(book.get("overlay-2"), Some(&now));
    }

    #[test]
    fn mixed_set_exercises_all_four_transitions() {
        // 混合集合：可见重置 / 未满保留 / 满期回收 / 首见插入，一次覆盖
        let now = Instant::now();
        let mut book = HashMap::from([
            ("overlay-vis".to_string(), now - TEST_TTL), // 又可见 → 重置
            ("overlay-young".to_string(), now),          // 隐藏未满 → 保留
            ("overlay-old".to_string(), now - TEST_TTL), // 隐藏满 → 回收
        ]);
        let hidden = HashSet::from([
            "overlay-young".to_string(),
            "overlay-old".to_string(),
            "overlay-new".to_string(), // 首次观测 → 插入
        ]);
        let mut expired = expire_labels(&mut book, &hidden, now, TEST_TTL);
        expired.sort();
        assert_eq!(expired, vec!["overlay-old".to_string()]);
        assert_eq!(book.len(), 2);
        assert_eq!(book.get("overlay-new"), Some(&now));   // 插入
        assert_eq!(book.get("overlay-young"), Some(&now)); // 保留
        assert!(!book.contains_key("overlay-vis"));        // 重置
        assert!(!book.contains_key("overlay-old"));        // 出簿
    }
}
