//! G2 OCR 常驻内存实测（M4 遗留 Important #2 的 M7 收口；`#[ignore]`：需网络 +
//! 真实模型下载，CI 跳过，本地跑）。
//!
//! 运行：`scripts/sample-ocr-mem.sh`
//! （内部 `cargo test -p hawksnap-ocr -- --ignored ocr_resident_memory --nocapture`）
//!
//! 口径：`ps -o rss= -p <self pid>` 采样——引擎初始化**前**（模型已就绪、fixture 已
//! 解码，下载/哈希噪声排除）、初始化**后**、首次推理**后**、再连续 3 次推理观察
//! 稳态（ORT arena 热身跨两次推理，单看第二次会误判泄漏）。
//!
//! 判定（任务简报 + M7 实测校准）：
//! - 原预算「init+首推理 <300MB」被实测**击穿**（release 365.9MB / debug 353.8MB，
//!   2026-10-24，M2 0.2.0 closeout）→ 按裁定线 >150MB **idle 卸载已随本任务实现**
//!   （app 层 ENGINE 空闲 10 分钟即 drop，见 app/src-tauri/src/ocr.rs）；ORT arena
//!   调优（会话构建选项裁剪）列 M2.1 候选。断言护栏按实测校准为 **<500MB**（防 2x
//!   级回归，非预算声明）。
//! - 稳态增量（第 3→4 次推理）**<50MB**：持续增长 = 泄漏形态，硬失败。实测 release
//!   稳态 RSS ~566-583MB（前两次推理 +293/+193MB 后持平），debug 高水位 ~540MB 后
//!   有 malloc 归还（-135MB）——热身形态，非泄漏。

use hawksnap_ocr::{ensure_models, OcrEngine};

fn data_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

/// 本进程当前 RSS（KB）。ps 在 macOS/Linux 均可用；失败即 panic（测量是本测试本体）。
fn rss_kb() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps 可用");
    assert!(out.status.success(), "ps 退出码非零: {}", out.status);
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .expect("RSS 输出为整数 KB")
}

fn mb(kb: u64) -> String {
    format!("{:.1}MB", kb as f64 / 1024.0)
}

#[test]
#[ignore]
fn ocr_resident_memory() {
    // 模型缓存目录固定名（不带 pid）：重复运行免 ~60MB 重下；与 real_models 的
    // pid 目录天然隔离（测试二进制串行执行，无并发写冲突）。
    let dir = std::env::temp_dir().join("hs-ocr-mem-cache");
    let paths = {
        let mut n = 0;
        let paths = ensure_models(&dir, &mut |_| n += 1).expect("模型就绪（首次运行需网络下载）");
        if n > 0 {
            println!("[mem] 下载了 {n} 个进度事件（缓存未命中）");
        }
        paths
    };

    // fixture（900x300 双语两行）先解码进内存：before 采样只反映「引擎尚未装载」。
    let png = std::fs::read(data_dir().join("fixture.png")).unwrap();
    let (rgba, w, h) = hawksnap_core::crop::png_to_rgba(&png).unwrap();

    let before = rss_kb();
    let mut engine = OcrEngine::load(&paths).expect("引擎装载");
    let after_init = rss_kb();
    let lines = engine.recognize(&rgba, w, h).expect("首次推理");
    assert!(!lines.is_empty(), "fixture 至少识别一行（保证推理真实发生）");
    let after_first = rss_kb();

    // 稳态观察：再连跑 3 次（#2/#3/#4）。ORT arena 热身跨两次推理，末两次的差值
    // 才是稳态增量（第 2 次的 +193MB release 实测属热身，不是泄漏证据）。
    let mut warm = Vec::with_capacity(3);
    for _ in 0..3 {
        let l = engine.recognize(&rgba, w, h).expect("热身推理");
        assert_eq!(l.len(), lines.len(), "同输入推理结果行数稳定");
        warm.push(rss_kb());
    }
    let steady_delta = warm[2].saturating_sub(warm[1]);

    let init_growth = after_init.saturating_sub(before);
    let infer_growth = after_first.saturating_sub(after_init);
    let total = after_first.saturating_sub(before);

    println!("[mem] rss before-init     = {} ({})", before, mb(before));
    println!("[mem] rss after-init      = {} ({})  init Δ {} ({})", after_init, mb(after_init), init_growth, mb(init_growth));
    println!("[mem] rss after-first-inf = {} ({})  infer Δ {} ({})", after_first, mb(after_first), infer_growth, mb(infer_growth));
    println!("[mem] rss warm #2/#3/#4   = {} / {} / {}", warm[0], warm[1], warm[2]);
    println!("[mem] init+first 合计增长 = {} ({})", total, mb(total));
    println!("[mem] 稳态增量（#3→#4）  = {} ({})", steady_delta, mb(steady_delta));
    println!(
        "[mem] idle-unload 裁定线 150MB：{}（{}）→ 裁定见 task-7-report.md",
        if total > 150 * 1024 { "超线" } else { "未超线" },
        mb(total)
    );

    // 护栏（按 M7 实测校准，见模块注）：init+首推理 <500MB（实测 release 365.9MB）。
    assert!(
        total < 500 * 1024,
        "OCR init+首推理常驻增长异常：{}（{}），护栏 500MB（实测基线见 task-7-report.md）",
        total,
        mb(total)
    );
    // 泄漏护栏：热身完成后（末两次推理之间）不再持续增长。
    assert!(
        steady_delta < 50 * 1024,
        "稳态仍增长 {}（{}）——疑似泄漏，需人工判定",
        steady_delta,
        mb(steady_delta)
    );
}
