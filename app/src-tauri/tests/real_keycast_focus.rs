//! keycast 小窗**焦点抢占**的端到端实证（M3 N4 评审修复 Important 的证据载体）：
//! 以子进程跑 `src/bin/keycast-focus-probe.rs`（真实 tauri app + 真实 NSWindow），
//! 断言其结论——focusable(false) 的 keycast 浮层「**仍可见**且**不能成为 key
//! window**」，而 Tauri 默认的 focusable(true) 窗口**能**成为 key window（在可
//! 激活的打包 app 中 show() 即夺走键盘焦点 = 本次修复的原始缺陷）。
//!
//! 为什么经子进程：NSWindow/NSApplication 是 MainThreadOnly——测试 harness 的
//! 主线程跑测试循环、测试体在池线程执行，无法在本进程内合法建窗；探针 bin 的
//! main 即主线程（tauri 事件循环），子进程是唯一合规载体（与
//! real_record_selfexcl 同款纪律）。
//!
//! 判据与环境无关：`canBecomeKeyWindow` 是**窗口自身的能力**（tao 的
//! focusable 覆写它），与「本进程能否被激活」无关——未打包的 CLI 探针的激活受
//! 系统限制，故不能靠观测 is_focused 判定。
//!
//! 运行：`cargo test -p hawksnap --test real_keycast_focus -- --ignored`

#[test]
#[ignore = "需要 macOS GUI 环境（真实 NSWindow + 事件循环）；-- --ignored 运行（子进程建窗实测）"]
fn keycast_window_is_visible_but_never_becomes_key_window() {
    let bin = env!("CARGO_BIN_EXE_keycast-focus-probe");
    let out = std::process::Command::new(bin)
        .output()
        .expect("spawn keycast-focus-probe");
    print!("{}", String::from_utf8_lossy(&out.stdout));
    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // 退出码语义（探针内定义）：0 = 修复组性质成立且对照组有判别力；
    // 2 = 修复组性质成立但该环境对照组无判别力（不算回归，仅证据弱）。
    let code = out.status.code();
    assert!(
        code == Some(0) || code == Some(2),
        "探针失败（exit={code:?}）：keycast 浮层抢焦点或不可见，见上方 [probe] 日志"
    );
    // 双重断言（防探针日志静默漂移）：关键性质行必须在场。
    assert!(
        stdout.contains("A-fix") && stdout.contains("B-fix"),
        "探针未输出修复组测量行: {stdout}"
    );
    assert!(
        stdout.contains("canBecomeKeyWindow=false"),
        "修复组仍能成为 key window（focusable(false) 未生效）: {stdout}"
    );
    assert!(
        stdout.contains("visible=true"),
        "修复组窗口不可见（focusable(false) 破坏了显示）: {stdout}"
    );
    if code == Some(2) {
        eprintln!("[test] 注意：对照组在该环境无判别力（exit=2）——修复组性质仍已断言通过");
    }
}
