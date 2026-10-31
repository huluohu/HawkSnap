//! N2 GIF 编码（`gif` crate）：录制旁路帧队列的消费侧。
//!
//! ## 分工（plan Task N2 裁定）
//!
//! 抽帧在平台侧（`hawksnap-platform-mac::record`：SCStream 视频回调 15fps 节拍 →
//! 最长边 ≤800 降采样 → 有界 32MB 队列丢最旧）；本模块只做**停止后编码**：
//! drain 队列 → `gif::Encoder` 逐帧 `Frame::from_rgba_speed`（内建 NeuQuant 量化）
//! → `write_frame` 落盘 `~/Movies/HawkSnap/<ts>.gif`。
//!
//! ## 编码器选型（2026-10-31：gifski → gif）
//!
//! HawkSnap 整体改 MIT 授权，原 gifski 是 AGPL-3.0（传染）→ 按许可证一致性替换为
//! `gif`（image-rs 生态，MIT / Apache-2.0）。差异与裁定：
//! - 量化：`from_rgba_speed` 内建 NeuQuant（color_quant 算法），`GIF_SPEED = 10`
//!   （1=最优最慢，30=最快；10 = 质量与耗时均衡，录屏动图与 gifski quality 90 的
//!   可感差异极小）。
//! - 时序：GIF delay 单位 10ms（u16）。帧 i 的显示时长 = `pts[i+1] - pts[i]`
//!   （末帧取前差），首帧不再置 0（delay 0 会让首图立即跳帧）；每帧钳到 ≥2（20ms）
//!   ——delay<2 触发浏览器「按 10s 解释」的历史 bug（GIF89a 实现约定）。
//! - 循环：`set_repeat(Repeat::Infinite)`（录屏动图语义，Snipaste/截图工具惯例）。
//! - 超时：gif crate 无 gifski 式 ProgressReporter 中止路径——改为**逐帧查时钟**，
//!   越线即 Err 返回（停止后同步编码，调用线程本来就是专用线程，无泄漏面）。
//! - 结构：gifski 需要 Collector/Writer 双线程闭环；gif 是同步顺序写——单线程
//!   直写，双线程协议整体退役。

// —— 非 macOS 平台放行约定（M4 P0 评审修复轮）——
//
// 本文件含 macOS 专属能力（录屏 / 画中画 / 贴图 / 按键显示 / 长截图 等）。非 macOS
// 构建下这些项**无人调用**——这是「能力缺席」的正常结果，不是代码缺陷。故按项逐个
// 放行：`#[cfg_attr(not(target_os = "macos"), allow(dead_code))]`（仅 macOS 分支才用
// 的局部变量用 `unused_variables`，仅 macOS 才用的导入用 `unused_imports`）。
//
// **逐项**放行而非 crate 级 blanket allow：crate 级 `allow(dead_code, unused_*)`
// 会把**未来新增**的同类问题一并吞掉；逐项放行后，新增项若在非 macOS 上真的无人用，
// 依然照报。macOS 构建下这些属性恒不生效（`not(target_os = "macos")` 为假）——
// macOS 的 lint 纪律零变化。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gif::{Encoder, Frame, Repeat};

use hawksnap_platform_mac::record::GifFrame;

/// NeuQuant 采样速度（from_rgba_speed 的 1-30 档，i32；10 = 质量优先的均衡档）。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const GIF_SPEED: i32 = 10;

/// 编码硬超时（F8 预算的 6 倍兜底：正常 150 帧 ≤10s，超时 = 编码异常 → 走
/// MP4 降级路径而不是卡死 stop）。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const GIF_ENCODE_DEADLINE: Duration = Duration::from_secs(60);

/// 编码产物（入库 + record_stop 返回值的数据源）。
#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct GifOutcome {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// GIF 总时长（秒）：末帧 PTS（与 history.duration 同口径）。
    pub duration: f64,
    pub frames: usize,
}

/// 帧 i 的显示时长（10ms 单位，u16）：`pts[i+1] - pts[i]`，末帧取前差。
/// 钳到 ≥2（20ms）——delay<2 触发浏览器「按 10s 解释」的历史 bug。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn frame_delay_10ms(pts: &[f64], i: usize) -> u16 {
    let delta = if i + 1 < pts.len() {
        pts[i + 1] - pts[i]
    } else if i > 0 {
        pts[i] - pts[i - 1]
    } else {
        1.0 / 15.0 // 单帧序列：给一个 15fps 缺省时长
    };
    ((delta.max(0.0) * 100.0).round() as u16).max(2)
}

/// 编码帧序列到 `out`（.gif）。帧需非空且按 PTS 升序（抽帧通道的天然产出）。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn encode_gif(frames: &[GifFrame], out: &Path) -> Result<GifOutcome, String> {
    let Some(first) = frames.first() else {
        return Err("GIF 编码无帧（录制过短或抽帧失败）".into());
    };
    let (w, h) = (first.width, first.height);
    let first_pts = first.pts;

    let file = std::fs::File::create(out)
        .map_err(|e| format!("创建 GIF 输出文件 {} 失败: {e}", out.display()))?;
    // gif crate 的宽高是 u16（抽帧通道已钳最长边 ≤800，必不溢出）
    let (w16, h16) = (u16::try_from(w).map_err(|_| "GIF 宽度超出 u16".to_string())?,
                     u16::try_from(h).map_err(|_| "GIF 高度超出 u16".to_string())?);
    let mut encoder = Encoder::new(std::io::BufWriter::new(file), w16, h16, &[])
        .map_err(|e| format!("GIF 编码器初始化失败: {e}"))?;
    encoder
        .set_repeat(Repeat::Infinite)
        .map_err(|e| format!("GIF 循环设置失败: {e}"))?;
    let deadline = Instant::now() + GIF_ENCODE_DEADLINE;

    // PTS 全序列归零一次（delay 映射 + 时长共用），避免逐帧 O(n²) 重建。
    let pts: Vec<f64> = frames.iter().map(|f| f.pts - first_pts).collect();
    for (i, f) in frames.iter().enumerate() {
        // 逐帧硬超时（gif crate 无 gifski 式 ProgressReporter 中止——同步写里查时钟）
        if Instant::now() > deadline {
            return Err(format!(
                "GIF 编码超时（>{GIF_ENCODE_DEADLINE:?}，帧 {i}/{}）——录制通道异常",
                frames.len()
            ));
        }
        // from_rgba_speed 需要 &mut（量化原地做）——每帧一次拷贝 ~1.4MB，
        // 相对量化编码成本可忽略。0.13 的该构造函数直接返回 Self（infallible）。
        let mut rgba = f.rgba.clone();
        let mut frame = Frame::from_rgba_speed(
            u16::try_from(f.width).map_err(|_| "GIF 帧宽超出 u16".to_string())?,
            u16::try_from(f.height).map_err(|_| "GIF 帧高超出 u16".to_string())?,
            &mut rgba,
            GIF_SPEED,
        );
        frame.delay = frame_delay_10ms(&pts, i);
        encoder
            .write_frame(&frame)
            .map_err(|e| format!("GIF 帧写入失败（帧 {i}）: {e}"))?;
    }

    let last = frames.last().expect("frames 非空已验");
    Ok(GifOutcome {
        path: out.to_path_buf(),
        width: w,
        height: h,
        duration: last.pts - first_pts,
        frames: frames.len(),
    })
}

/// 停止收口 GIF（record_stop 的 gif 分支）：drain 旁路队列 → 编码 → .gif 落盘。
/// 产物路径 = MP4 路径换扩展名（同秒时间戳茎）；调用方在成功后删除 MP4 中间产物。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn finish_gif_recording(
    lane: Option<&Arc<hawksnap_platform_mac::record::GifLane>>,
    mp4_path: &Path,
) -> Result<GifOutcome, String> {
    let Some(lane) = lane else {
        return Err("GIF 通道缺失（会话状态异常）".into());
    };
    let frames = lane.drain();
    if frames.is_empty() {
        return Err("GIF 通道无帧（录制过短或抽帧已禁用）".into());
    }
    let out = mp4_path.with_extension("gif");
    let outcome = encode_gif(&frames, &out)?;
    eprintln!(
        "[hawksnap] GIF 导出完成: {} ({} 帧 {}×{}, {:.2}s)",
        outcome.path.display(),
        outcome.frames,
        outcome.width,
        outcome.height,
        outcome.duration
    );
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// 测试：F8 编码计时（#[ignore]，开发机直跑）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use hawksnap_platform_mac::record::GifLane;

    /// 合成帧（渐变色块：量化器对纯色序列会高度优化，渐变更接近真实
    /// 屏幕内容的熵——计时证据更诚实）。
    fn synth_frame(i: usize, w: u32, h: u32) -> GifFrame {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = ((x + y + (i as u32) * 7) % 256) as u8;
                rgba.extend_from_slice(&[v, v.wrapping_mul(3), v.wrapping_add(i as u8), 255]);
            }
        }
        GifFrame { pts: i as f64 / 15.0, width: w, height: h, rgba }
    }

    /// F8 编码计时（plan 验收口径）：合成 150 帧 800×600（=10s@15fps）→ 计时
    /// ≤10s + GIF 魔数 "GIF8" + 非空体积。debug 构建的计时失真 → 预算断言仅
    /// release 生效（F7 CPU 断言同口径先例），debug 只打印。
    #[test]
    #[ignore = "F8 计时基准；cargo test -p hawksnap --lib gif_encode --release -- --ignored 运行"]
    fn gif_encode_150_frames_800x600_within_f8_budget() {
        let frames: Vec<GifFrame> = (0..150).map(|i| synth_frame(i, 800, 600)).collect();
        let out = std::env::temp_dir().join(format!("hawksnap-n2-f8-{}.gif", std::process::id()));
        let started = Instant::now();
        let outcome = encode_gif(&frames, &out).expect("encode");
        let elapsed = started.elapsed();
        println!(
            "[n2] F8 encode: {} frames 800×600 in {:.2}s (budget 10s), file {} bytes",
            outcome.frames,
            elapsed.as_secs_f64(),
            std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0)
        );
        let data = std::fs::read(&out).expect("read gif");
        assert!(data.len() > 4 && &data[..3] == b"GIF", "GIF 魔数（GIF87a/GIF89a 前缀 GIF8 形态）");
        assert!(data.len() > 10_000, "150 帧产物应非空壳，实际 {} 字节", data.len());
        assert_eq!(outcome.frames, 150);
        assert!((outcome.duration - 149.0 / 15.0).abs() < 1e-9, "时长 = 末帧 PTS");
        if !cfg!(debug_assertions) {
            assert!(elapsed <= Duration::from_secs(10), "F8 预算：{elapsed:?} ≤10s");
        }
        let _ = std::fs::remove_file(&out);
    }

    /// 真实端到端（F8 现场证据）：800×600 区域 GIF 通道录 10s → stop → drain →
    /// 编码 → 文件魔数 + 编码计时 + 抽帧计数报告。需屏幕录制权限。
    #[test]
    #[ignore = "需要 macOS 屏幕录制权限 + 真实显示器；cargo test -p hawksnap --lib real_region_gif --release -- --ignored 运行"]
    fn real_region_gif_lane_10s_encodes_and_plays() {
        use hawksnap_platform_mac::record::{LogicalRect, ScreenRecorder};
        let dir = std::env::temp_dir().join(format!("hawksnap-n2-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mp4 = dir.join("n2-region.gif-test.mp4"); // 茎任意；编码路径走 with_extension
        let region = LogicalRect { x: 50.0, y: 50.0, w: 800.0, h: 600.0 };
        let lane = Arc::new(GifLane::new());
        let rec = ScreenRecorder::start_region_gif(
            mp4.clone(),
            hawksnap_platform_mac::record::main_display_id(),
            region,
            lane.clone(),
            None, // GIF 证据基线不含 PiP（N3 的 F9 端到端在 pip-f9-probe）
        )
        .expect("start_region_gif");
        std::thread::sleep(Duration::from_secs(10));
        let info = rec.stop().expect("stop");
        let started = Instant::now();
        let outcome = finish_gif_recording(Some(&lane), &mp4).expect("gif encode");
        let elapsed = started.elapsed();
        println!(
            "[n2] real 10s region gif: mp4 {}×{}, tapped {} frames (≤150), \
             encode {:.2}s (budget 10s), gif {} bytes, duration {:.2}s → {}",
            info.width,
            info.height,
            outcome.frames,
            elapsed.as_secs_f64(),
            std::fs::metadata(&outcome.path).map(|m| m.len()).unwrap_or(0),
            outcome.duration,
            outcome.path.display()
        );
        let data = std::fs::read(&outcome.path).expect("read gif");
        assert!(data.len() > 6 && &data[..3] == b"GIF", "GIF 魔数");
        if !cfg!(debug_assertions) {
            assert!(elapsed <= Duration::from_secs(10), "F8 预算：{elapsed:?} ≤10s");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 无进度报告器路径（NoProgress 常量性）：极小帧序列可编码（契约冒烟）。
    #[test]
    fn encode_tiny_sequence_smoke() {
        let frames = vec![
            GifFrame { pts: 0.0, width: 4, height: 4, rgba: vec![10; 4 * 4 * 4] },
            GifFrame { pts: 1.0 / 15.0, width: 4, height: 4, rgba: vec![200; 4 * 4 * 4] },
        ];
        let out = std::env::temp_dir().join(format!("hawksnap-n2-tiny-{}.gif", std::process::id()));
        let outcome = encode_gif(&frames, &out).expect("encode tiny");
        assert_eq!(outcome.frames, 2);
        let data = std::fs::read(&out).unwrap();
        assert!(&data[..3] == b"GIF");
        let _ = std::fs::remove_file(&out);
    }
}
