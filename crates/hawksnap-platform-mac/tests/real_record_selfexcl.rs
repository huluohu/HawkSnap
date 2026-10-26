//! 录屏自排除（终审修复 1）的端到端集成测试：以子进程跑
//! `src/bin/record-selfexcl-probe.rs`（主线程自建 NSWindow 后真实录制 3s），
//! 断言其退出码成功——即 `RecordingInfo.excluded_apps == 1`（excludingApplications
//! 恰含本应用，悬浮条/贴图不入画）。
//!
//! 为什么经子进程：NSWindow/NSApplication 是 MainThreadOnly，测试 harness 的
//! 主线程跑测试循环、测试体在任意池线程执行，无法在本进程内合法建窗；探针 bin
//! 的 main 即主线程，子进程是唯一合规载体。
//!
//! 运行（需屏幕录制权限）：
//! `cargo test -p hawksnap-platform-mac --test real_record_selfexcl -- --ignored`

#[test]
#[ignore = "需要 macOS 屏幕录制权限 + 真实显示器；-- --ignored 运行（子进程自建窗口后录 3s）"]
fn self_excl_probe_records_with_own_app_excluded() {
    let bin = env!("CARGO_BIN_EXE_record-selfexcl-probe");
    let out = std::process::Command::new(bin)
        .env("PROBE_RECORD_SECS", "3")
        .output()
        .expect("spawn record-selfexcl-probe");
    print!("{}", String::from_utf8_lossy(&out.stdout));
    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success(),
        "探针失败（exit={:?}）：自排除未生效或录制管线异常，见上方 [probe] 日志",
        out.status.code()
    );
    // 双重断言（防探针日志静默漂移）：stdout 必须含 PASS 标记与 excluded_apps=1。
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("PASS: excluded_apps=1"), "探针未报告自排除成功: {stdout}");
}
