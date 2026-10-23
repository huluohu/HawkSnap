//! 真模型集成测试（#[ignore]：需网络 + 官方模型/dylib 下载，CI 跳过，本地跑）。
//!
//! 运行：`cargo test -p hawksnap-ocr --test real_models -- --ignored --nocapture`
//!
//! 覆盖：models::ensure_models 全链（官方 release 下载 + SHA256 门）→ OcrEngine
//! 三模型推理（det/cls/rec）→ 合成文本图识别断言（fixture.png：Helvetica
//! "HawkSnap OCR 2026" + Hiragino "中文识别测试"）→ 1080p 计时（F6 预算 ≤1.5s）。

use hawksnap_ocr::{ensure_models, OcrEngine};

fn data_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

#[test]
#[ignore]
fn real_models_download_and_recognize() {
    // 模型目录走临时目录（不污染 app_data_dir；重复跑复用下载缓存无妨）。
    let dir = std::env::temp_dir().join(format!("hs-ocr-real-{}", std::process::id()));
    let mut progress_seen = 0;
    let paths = {
        let mut cb = |p: hawksnap_ocr::models::Progress| {
            progress_seen += 1;
            println!("progress: {p:?}");
        };
        ensure_models(&dir, &mut cb).expect("ensure_models（真实网络下载）")
    };
    assert!(progress_seen > 0, "下载必须发进度事件");
    for f in [paths.det.as_path(), paths.cls.as_path(), paths.rec.as_path(), paths.dict.as_path(), paths.ort_dylib.as_path()] {
        assert!(f.is_file(), "{} 缺失", f.display());
    }
    // 第二次调用：全量短路（无新进度事件 = 零下载）。
    let mut seen2 = 0;
    {
        let mut cb = |_: hawksnap_ocr::models::Progress| seen2 += 1;
        ensure_models(&dir, &mut cb).unwrap();
    }
    assert_eq!(seen2, 0, "已就绪文件必须零下载短路");

    let mut engine = OcrEngine::load(&paths).expect("引擎装载");

    // fixture：900x300 双语两行。
    let png = std::fs::read(data_dir().join("fixture.png")).unwrap();
    let (rgba, w, h) = hawksnap_core::crop::png_to_rgba(&png).unwrap();
    let t0 = std::time::Instant::now();
    let lines = engine.recognize(&rgba, w, h).expect("识别 fixture");
    println!("fixture recognize: {:?}, {} 行", t0.elapsed(), lines.len());
    for l in &lines {
        println!("  [{:.2}] {:?}", l.score, l.text);
    }
    assert!(!lines.is_empty(), "fixture 至少一行");
    let joined: String = lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join("\n");
    assert!(joined.contains("HawkSnap"), "英文行必须命中，实际: {joined:?}");
    assert!(joined.contains("2026"), "数字必须命中: {joined:?}");
    assert!(joined.contains("测试"), "中文行必须命中: {joined:?}");
    assert!(lines.iter().all(|l| l.score > 0.5 && l.score <= 1.0));

    // 1080p 计时（F6 ≤1.5s；release 口径——debug 构建计时无意义，仅记录）。
    let png = std::fs::read(data_dir().join("bench_1080p.png")).unwrap();
    let (rgba, w, h) = hawksnap_core::crop::png_to_rgba(&png).unwrap();
    let t1 = std::time::Instant::now();
    let lines = engine.recognize(&rgba, w, h).expect("识别 1080p");
    let elapsed = t1.elapsed();
    println!("1080p recognize: {elapsed:?}, {} 行（F6 预算 1.5s）", lines.len());
    assert_eq!(lines.len(), 24, "24 行文本全部检出（第 23 行可能轻微空格损失）");
    let joined: String = lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join("\n");
    assert!(joined.contains("line 0:") && joined.contains("line 22:"), "首尾行文本正确");
    assert!(
        elapsed.as_secs_f64() < 1.5,
        "F6：1080p 识别必须 <1.5s，实测 {elapsed:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
