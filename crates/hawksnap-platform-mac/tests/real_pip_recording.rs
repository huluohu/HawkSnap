//! N3 F9 端到端集成测试（#[ignore]）：子进程跑 `pip-f9-probe`（动窗驱动 + 相机/
//! 注入源合成 → 无 PiP 基线 5s + PiP 10s 两段真实录制），随后：
//! ① probe 自身断言 pts 连续性（stts 带内 ≥95%）+ PiP 产物大于基线（exit code 双重闸）；
//! ② swift 抽帧工具（pip_region_motion.swift）程序化断言 PiP 区域运动熵显著高于
//!    无 PiP 基线同区域（贴片可见的程序化证据）；KEEP=1 时另抽 3 帧目检 PNG。
//!
//! 运行（需屏幕录制权限 + 相机权限或接受注入模式）：
//! `cargo test -p hawksnap-platform-mac --test real_pip_recording -- --ignored`
//! HAWKSNAP_RECORD_KEEP=1 保留产物与目检帧。

#[test]
#[ignore = "需要 macOS 屏幕录制权限 + 真实显示器；-- --ignored 运行（动窗 + 两段真实录制）"]
fn real_pip_recording_pts_continuous_and_pip_visible() {
    let bin = env!("CARGO_BIN_EXE_pip-f9-probe");
    let out = std::process::Command::new(bin)
        .env("HAWKSNAP_RECORD_KEEP", "1")
        .output()
        .expect("spawn pip-f9-probe");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    print!("{stdout}");
    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success() && stdout.contains("PASS"),
        "probe 失败（exit={:?}）：pts 连续性或体积增长断言未过，见上方 [probe] 日志",
        out.status.code()
    );

    // —— 机读协议行解析。 ——
    let mut pip_path = String::new();
    let mut nopip_path = String::new();
    let mut rect = (0u32, 0u32, 0u32);
    for line in stdout.lines() {
        if let Some(p) = line.strip_prefix("[probe] pip=") {
            pip_path = p.to_string();
        }
        if let Some(p) = line.strip_prefix("[probe] nopip=") {
            nopip_path = p.to_string();
        }
        if let Some(rest) = line.strip_prefix("[probe] dims=") {
            // dims=1600x1200 scale=2.00 rect=1072,672,480 mode=...
            if let Some(r) = rest.split(" rect=").nth(1) {
                let parts: Vec<&str> = r.splitn(2, ' ').next().unwrap_or("").split(',').collect();
                if parts.len() == 3 {
                    rect = (
                        parts[0].parse().unwrap_or(0),
                        parts[1].parse().unwrap_or(0),
                        parts[2].parse().unwrap_or(0),
                    );
                }
            }
        }
    }
    assert!(!pip_path.is_empty() && !nopip_path.is_empty(), "probe 协议行缺失");
    assert_ne!(rect.2, 0, "PiP rect 解析失败");

    // —— 程序化「贴片可见」：PiP 区域在两帧间有运动（相机/注入贴片是活动内容），
    //     无 PiP 基线同区域（同动窗、同位置）应近静态。阈值：pip motion ≥ 0.5
    //     且 ≥ 基线的 5×（相机传感器噪声 ~0.2-1，静态桌面 <0.05）。
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/pip_region_motion.swift");
    let run_motion = |mp4: &str, t1: f64, t2: f64| -> f64 {
        let out = std::process::Command::new("swift")
            .args([
                script,
                mp4,
                &rect.0.to_string(),
                &rect.1.to_string(),
                &rect.2.to_string(),
                &format!("{t1}"),
                &format!("{t2}"),
            ])
            .output()
            .expect("spawn pip_region_motion.swift");
        let so = String::from_utf8_lossy(&out.stdout).into_owned();
        print!("[motion] {mp4}: {so}");
        assert!(out.status.success(), "motion 探针失败: {}", String::from_utf8_lossy(&out.stderr));
        so.strip_prefix("motion=")
            .and_then(|v| v.trim().parse::<f64>().ok())
            .expect("motion 数值解析")
    };
    // 取 t1=1.0、t2=1.2（probe 打印的 rect 是**默认位**；中途 t=3s 的换位发生在
    // 其后——运动熵采样必须在默认位窗口内）。基线 = 无 PiP 录像同区域（桌面内容）。
    let pip_motion = run_motion(&pip_path, 1.0, 1.2);
    let nopip_motion = run_motion(&nopip_path, 2.0, 2.2);
    println!(
        "[n3] PiP 区域运动熵: pip={pip_motion:.4} vs 基线={nopip_motion:.4}（阈值 pip≥10 且 ≥5× 基线）"
    );
    assert!(
        pip_motion >= 10.0,
        "PiP 区域近乎静止（{pip_motion:.4}）——贴片未合成或为静态内容（注入信号帧间差应 ~60）"
    );
    assert!(
        pip_motion >= nopip_motion * 5.0,
        "PiP 区域运动熵 ({pip_motion:.4}) 未显著高于无 PiP 基线 ({nopip_motion:.4})"
    );

    // —— 目检帧（×3 时间戳；控制器/评审用）。 ——
    let frames = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/pip-frames");
    let frames_script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/pip_frames.swift");
    let out = std::process::Command::new("swift")
        .args([frames_script, &pip_path, frames, "1.0", "5.0", "9.0"])
        .output()
        .expect("spawn pip_frames.swift");
    print!("{}", String::from_utf8_lossy(&out.stdout));
    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "抽帧失败");
    if std::env::var("HAWKSNAP_RECORD_KEEP").is_err() {
        let _ = std::fs::remove_dir_all(std::path::Path::new(&pip_path).parent().unwrap().parent().unwrap());
    } else {
        println!("[n3] 产物与目检帧保留（KEEP=1）: {pip_path} / {frames}");
    }
}
