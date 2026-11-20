//! OCR 端到端自检（2026-11-20 真机验收战役）。
//!
//! **验什么**：OCR 是核心功能，且本轮动过它的**大图路径**（整图降采样 → 分块
//! 检测，见 pipeline.rs）。纯逻辑单测覆盖了分块与坐标链路，但**没有**跑过真
//! 模型 —— 「模型下载 → 装载 → 推理 → 坐标」这条链在 CI 里始终是空白。
//!
//! 本 example 自带**点阵字体**（5×7 位图）现场渲染一张带文字的图（白底黑字，
//! OCR 最易识别的形态），不依赖外部字体文件、不依赖网络图片：
//!   ① 小图（≤2400px）→ 单块路径；
//!   ② 大图（>2400px）→ **分块路径**（本轮新增）；
//!   ③ 断言：识别出至少一行非空文本、每行框坐标落在原图范围内。
//!
//! 用法：
//!   HAWKSNAP_OCR_MODELS=./target/ocr-models cargo run -p hawksnap-ocr --example ocr_e2e
//!
//! 退出码 0 = 通过；非 0 = 失败（CI 门禁判据）。

use hawksnap_ocr::pipeline::OcrEngine;

/// 5×7 点阵字形（只做本 example 需要的几个大写字母）。
/// 每行是 5 位（'1' = 墨点）。
fn glyph(c: char) -> [[u8; 5]; 7] {
    match c {
        'H' => [[1, 0, 1, 0, 1], [1, 0, 1, 0, 1], [1, 0, 1, 0, 1], [1, 1, 1, 1, 1],
                 [1, 0, 1, 0, 1], [1, 0, 1, 0, 1], [1, 0, 1, 0, 1]],
        'E' => [[1, 1, 1, 1, 1], [1, 0, 0, 0, 0], [1, 0, 0, 0, 0], [1, 1, 1, 1, 0],
                 [1, 0, 0, 0, 0], [1, 0, 0, 0, 0], [1, 1, 1, 1, 1]],
        'L' => [[1, 0, 0, 0, 0], [1, 0, 0, 0, 0], [1, 0, 0, 0, 0], [1, 0, 0, 0, 0],
                 [1, 0, 0, 0, 0], [1, 0, 0, 0, 0], [1, 1, 1, 1, 1]],
        'O' => [[1, 1, 1, 1, 1], [1, 0, 0, 0, 1], [1, 0, 0, 0, 1], [1, 0, 0, 0, 1],
                 [1, 0, 0, 0, 1], [1, 0, 0, 0, 1], [1, 1, 1, 1, 1]],
        'P' => [[1, 1, 1, 1, 0], [1, 0, 0, 0, 1], [1, 0, 0, 0, 1], [1, 1, 1, 1, 0],
                 [1, 0, 0, 0, 0], [1, 0, 0, 0, 0], [1, 0, 0, 0, 0]],
        _ => [[0, 0, 0, 0, 0], [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], [0, 0, 0, 0, 0],
              [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], [0, 0, 0, 0, 0]],
    }
}

/// 渲染一行文字：白底黑字，字高 ≈ 7*scale。
fn render_text(text: &str, scale: u32) -> (Vec<u8>, u32, u32) {
    let gw = 5u32 * scale;
    let gh = 7u32 * scale;
    let gap = scale; // 字间距
    let text_w = text.chars().count() as u32 * (gw + gap);
    let w = text_w.max(gw) + gap * 2;
    let h = gh + gap * 2;
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    let mut pen_x = gap;
    for ch in text.chars() {
        let g = glyph(ch);
        for (gy, row) in g.iter().enumerate() {
            for (gx, &ink) in row.iter().enumerate() {
                if ink == 0 {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        let x = pen_x + gx as u32 * scale + sx;
                        let y = gap + gy as u32 * scale + sy;
                        if x >= w || y >= h {
                            continue;
                        }
                        let i = ((y * w + x) * 4) as usize;
                        rgba[i] = 0;
                        rgba[i + 1] = 0;
                        rgba[i + 2] = 0;
                        rgba[i + 3] = 255;
                    }
                }
            }
        }
        pen_x += gw + gap;
    }
    (rgba, w, h)
}

/// 把文字块贴到指定画布上（白底），返回整图 RGBA。
fn canvas_with_text(text: &str, scale: u32, canvas_w: u32, canvas_h: u32, origin_x: u32, origin_y: u32) -> (Vec<u8>, u32, u32) {
    let (mut rgba, tw, th) = render_text(text, scale);
    let mut out = vec![255u8; (canvas_w * canvas_h * 4) as usize];
    for y in 0..th.min(canvas_h.saturating_sub(origin_y)) {
        for x in 0..tw.min(canvas_w.saturating_sub(origin_x)) {
            let si = ((y * tw + x) * 4) as usize;
            let di = (((origin_y + y) * canvas_w) + origin_x + x) as usize * 4;
            out[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
        }
    }
    (out, canvas_w, canvas_h)
}

fn main() {
    let dir = std::env::var("HAWKSNAP_OCR_MODELS")
        .unwrap_or_else(|_| "./target/ocr-models".to_string());
    let dir = std::path::PathBuf::from(dir);
    eprintln!("[ocr-e2e] 模型目录: {}", dir.display());

    eprintln!("[ocr-e2e] 准备模型（首次会下载 ~100MB）…");
    let paths = match hawksnap_ocr::ensure_models(&dir, &mut |p| {
        eprintln!("[ocr-e2e] 下载中: file={} {}/{}", p.file, p.downloaded, p.total);
    }) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[ocr-e2e] 失败：模型准备失败: {e}");
            std::process::exit(1);
        }
    };

    let mut engine = match OcrEngine::load(&paths) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[ocr-e2e] 失败：引擎装载失败: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("[ocr-e2e] 引擎就绪");

    let mut failures = 0usize;

    // —— ① 小图（单块路径）——
    let (rgba, w, h) = canvas_with_text("HELLO", 18, 900, 300, 40, 60);
    match engine.recognize(&rgba, w, h) {
        Ok(lines) => {
            eprintln!("[ocr-e2e] 小图（{w}×{h}）识别到 {} 行:", lines.len());
            for l in &lines {
                eprintln!("[ocr-e2e]   {:?} score={:.3}", l.text, l.score);
            }
            if lines.is_empty() {
                eprintln!("[ocr-e2e] 失败：小图未识别出任何文本行");
                failures += 1;
            } else {
                for l in &lines {
                    if l.text.trim().is_empty() {
                        eprintln!("[ocr-e2e] 失败：存在空文本行");
                        failures += 1;
                    }
                    for p in &l.points {
                        if p.x < 0.0 || p.y < 0.0 || p.x > w as f32 || p.y > h as f32 {
                            eprintln!("[ocr-e2e] 失败：框坐标越界 {:?}（图 {w}×{h}）", p);
                            failures += 1;
                        }
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("[ocr-e2e] 失败：小图识别出错: {e}");
            failures += 1;
        }
    }

    // —— ② 大图（分块路径，本轮新增）：文字落在**单块内**（x=500，第一块），
    //       保证识别稳定；分块路径本身被走到，框坐标必须仍落在原图范围内 ——
    //       这是「分块后坐标没有跑偏」的关键断言。
    let (rgba, w, h) = canvas_with_text("HELLO", 18, 3200, 900, 500, 300);
    match engine.recognize(&rgba, w, h) {
        Ok(lines) => {
            eprintln!("[ocr-e2e] 大图（{w}×{h}，分块路径）识别到 {} 行:", lines.len());
            for l in &lines {
                eprintln!(
                    "[ocr-e2e]   {:?} score={:.3} box0=({:.0},{:.0})",
                    l.text, l.score, l.points[0].x, l.points[0].y
                );
            }
            if lines.is_empty() {
                eprintln!("[ocr-e2e] 失败：大图未识别出任何文本行");
                failures += 1;
            }
            for l in &lines {
                for p in &l.points {
                    if p.x < 0.0 || p.y < 0.0 || p.x > w as f32 || p.y > h as f32 {
                        eprintln!("[ocr-e2e] 失败：大图框坐标越界 {:?}（图 {w}×{h}）", p);
                        failures += 1;
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("[ocr-e2e] 失败：大图识别出错: {e}");
            failures += 1;
        }
    }

    if failures > 0 {
        eprintln!("[ocr-e2e] 失败项: {failures}");
        std::process::exit(1);
    }
    eprintln!("[ocr-e2e] 通过：小图单块路径 + 大图分块路径均识别出文本且坐标在界内");
}
