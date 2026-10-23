//! DBNet 检测后处理（M2 口径，见下「简化裁定」）：sigmoid 概率图 → 二值化 → 连通域
//! （4 邻接 BFS 标记）→ 每域「轴对齐外接框 + unclip 外扩」→ 原图坐标行。
//!
//! **M2 简化裁定（诚实记录，报告同文）**：paddleocr 的标准后处理用 Clipper 多边形
//! 偏移（probmap 上的等值线轮廓 → min-area-rect）。本版以「连通域外接框 + 标量
//! unclip 距离外扩」近似：距离公式与 paddle 相同（`d = area × unclip_ratio /
//! perimeter`），但对整个轴对齐框各边统一外扩，而非逐多边形段偏移。对截图场景
//! （水平排版、大段清晰文本）行为几乎一致；倾斜/环形文本会得到偏大的框。极小噪点
//! 由 min side/score 过滤。完整 DBNet 多边形留 M2.x。

/// 检出的文本框（原图坐标，顺序 TL→TR→BR→BL；M2 为轴对齐框）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextBox {
    pub pts: [(f32, f32); 4],
    /// 框内平均概率（paddle 的 box score 口径）。
    pub score: f32,
}

/// 默认参数：二值阈值 0.3、unclip 比率 1.5（v4 det mobile 训练口径）；框分阈值
/// 0.5（paddle 默认 0.6 的分母是未收缩多边形区域，本实现按「连通域像素均值」
/// 计分（语义更贴近真实文本置信度），0.5 为实测校准值，见 task-4-report.md）。
pub const DB_THRESH: f32 = 0.3;
pub const BOX_THRESH: f32 = 0.5;
pub const UNCLIP_RATIO: f32 = 1.5;
/// 最小边（概率图像素）：低于此的连通域视为噪点。
pub const MIN_SIDE: i32 = 3;

/// 概率图 → 文本框（原图坐标）。`scale_x/scale_y` = 原图尺寸 / 概率图尺寸。
pub fn boxes_from_probmap(
    prob: &[f32],
    map_w: u32,
    map_h: u32,
    scale_x: f32,
    scale_y: f32,
    thresh: f32,
    box_thresh: f32,
    unclip_ratio: f32,
) -> Vec<TextBox> {
    let (mw, mh) = (map_w as usize, map_h as usize);
    if prob.len() != mw * mh || mw == 0 || mh == 0 {
        return Vec::new();
    }
    let mask: Vec<bool> = prob.iter().map(|&p| p >= thresh).collect();

    // 4 邻接 BFS 连通域标记。
    let mut label = vec![0u32; mw * mh];
    let mut boxes_raw: Vec<(i32, i32, i32, i32, f64, i64, i64)> = Vec::new(); // x0 y0 x1 y1 prob_sum area perimeter
    let mut queue: Vec<usize> = Vec::new();
    let mut next_label: u32 = 0;
    for start in 0..mw * mh {
        if !mask[start] || label[start] != 0 {
            continue;
        }
        next_label += 1;
        label[start] = next_label;
        queue.clear();
        queue.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (mw as i32, mh as i32, -1i32, -1i32);
        let mut prob_sum = 0f64;
        let mut area = 0i64;
        let mut qi = 0;
        while qi < queue.len() {
            let idx = queue[qi];
            qi += 1;
            let x = (idx % mw) as i32;
            let y = (idx / mw) as i32;
            prob_sum += prob[idx] as f64;
            area += 1;
            if x < x0 { x0 = x; }
            if y < y0 { y0 = y; }
            if x > x1 { x1 = x; }
            if y > y1 { y1 = y; }
            for nb in neighbors(idx, x as usize, y as usize, mw, mh) {
                if mask[nb] && label[nb] == 0 {
                    label[nb] = next_label;
                    queue.push(nb);
                }
            }
        }
        // 周长 ≈ 有 4 邻域出域的像素数（paddle 无此近似——Clipper 用轮廓长；此处
        // 作 unclip 距离的分子分母配对足够稳定）。
        let mut perimeter: i64 = 0;
        for &idx in &queue {
            let x = idx % mw;
            let y = idx / mw;
            let inside = |nx: usize, ny: usize| mask[ny * mw + nx];
            let edge = (x == 0 || !inside(x - 1, y))
                || (x + 1 == mw || !inside(x + 1, y))
                || (y == 0 || !inside(x, y - 1))
                || (y + 1 == mh || !inside(x, y + 1));
            if edge {
                perimeter += 1;
            }
        }
        boxes_raw.push((x0, y0, x1, y1, prob_sum, area, perimeter.max(1)));
    }

    #[cfg(feature = "ocr-debug")]
    for &(x0, y0, x1, y1, prob_sum, area, perimeter) in &boxes_raw {
        let sc = (prob_sum / area.max(1) as f64) as f32;
        eprintln!("[ocr-debug] raw blob ({x0},{y0})-({x1},{y1}) blob_score {sc:.3} area {area} perim {perimeter}");
    }
    let mut out = Vec::new();
    for (x0, y0, x1, y1, prob_sum, area, perimeter) in boxes_raw {
        let bw = x1 - x0 + 1;
        let bh = y1 - y0 + 1;
        if bw < MIN_SIDE || bh < MIN_SIDE {
            continue;
        }
        // 框分 = 连通域内概率均值（稀疏文本不被外扩稀释）。
        let score = (prob_sum / area.max(1) as f64) as f32;
        if score < box_thresh {
            continue;
        }
        // unclip：d = 外接框面积 × unclip_ratio / 周长（paddle 的轮廓多边形面积把
        // 字间空白也算在内，轴对齐近似取外接框面积；周长用连通域边界像素数近似）。
        let d = (bw * bh) as f32 * unclip_ratio / perimeter as f32;
        let fx0 = (x0 as f32 - d).max(0.0) * scale_x;
        let fy0 = (y0 as f32 - d).max(0.0) * scale_y;
        let fx1 = (x1 as f32 + 1.0 + d).min(map_w as f32) * scale_x;
        let fy1 = (y1 as f32 + 1.0 + d).min(map_h as f32) * scale_y;
        out.push(TextBox {
            pts: [(fx0, fy0), (fx1, fy0), (fx1, fy1), (fx0, fy1)],
            score,
        });
    }
    // 行序排序（M2：y 主序 + x 次序；paddle 用重叠行聚类——对单栏截图等价）。
    out.sort_by(|a, b| {
        let ay = a.pts[0].1;
        let by = b.pts[0].1;
        if (ay - by).abs() > 8.0 {
            ay.partial_cmp(&by).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            a.pts[0].0.partial_cmp(&b.pts[0].0).unwrap_or(std::cmp::Ordering::Equal)
        }
    });
    out
}

/// 概率图上单点 4 邻域（边界内）。
fn neighbors(idx: usize, x: usize, y: usize, w: usize, h: usize) -> Vec<usize> {
    let mut v = Vec::with_capacity(4);
    if x > 0 { v.push(idx - 1); }
    if x + 1 < w { v.push(idx + 1); }
    if y > 0 { v.push(idx - w); }
    if y + 1 < h { v.push(idx + w); }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probmap(w: u32, h: u32, fill: impl Fn(u32, u32) -> f32) -> Vec<f32> {
        let mut v = vec![0f32; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                v[(y * w + x) as usize] = fill(x, y);
            }
        }
        v
    }

    /// 两个分离的矩形热区 → 两个框，位置与热区一致（含 unclip 外扩）。
    #[test]
    fn finds_two_blobs_as_two_boxes() {
        // 40x40 图，热区 A: x 5..15, y 5..10；热区 B: x 20..30, y 20..25。
        let prob = probmap(40, 40, |x, y| {
            if (5..15).contains(&x) && (5..10).contains(&y) {
                0.9
            } else if (20..30).contains(&x) && (20..25).contains(&y) {
                0.8
            } else {
                0.0
            }
        });
        let boxes = boxes_from_probmap(&prob, 40, 40, 2.0, 2.0, DB_THRESH, BOX_THRESH, UNCLIP_RATIO);
        assert_eq!(boxes.len(), 2, "两热区两框: {boxes:?}");
        let a = &boxes[0];
        assert!(a.pts[0].0 < 12.0 && a.pts[0].1 < 12.0, "A 起点在热区左上附近（含外扩）: {a:?}");
        assert!(a.pts[2].0 > 28.0 && a.pts[2].1 > 20.0, "A 终点越过热区右下: {a:?}");
        let b = &boxes[1];
        assert!(b.pts[0].0 > 30.0 && b.pts[0].1 > 30.0, "B 在 A 之后（行序）: {b:?}");
        assert!(a.score > 0.85 && b.score > 0.75);
    }

    /// 低于 box_thresh 的弱热区被过滤。
    #[test]
    fn weak_blob_filtered_by_score() {
        let prob = probmap(20, 20, |x, y| if (2..8).contains(&x) && (2..8).contains(&y) { 0.4 } else { 0.0 });
        assert!(boxes_from_probmap(&prob, 20, 20, 1.0, 1.0, DB_THRESH, BOX_THRESH, UNCLIP_RATIO).is_empty(), "0.4 < 0.5 被过滤");
    }

    /// 噪点（单像素）不产框。
    #[test]
    fn single_pixel_noise_ignored() {
        let prob = probmap(10, 10, |x, y| if (x, y) == (3, 3) { 1.0 } else { 0.0 });
        assert!(boxes_from_probmap(&prob, 10, 10, 1.0, 1.0, DB_THRESH, BOX_THRESH, UNCLIP_RATIO).is_empty());
    }

    /// 坐标按 scale 回原图：2x 概率图上的热区 → 原图坐标翻倍。
    #[test]
    fn scale_maps_back_to_original() {
        let prob = probmap(20, 20, |x, y| if (4..10).contains(&x) && (4..10).contains(&y) { 1.0 } else { 0.0 });
        let boxes = boxes_from_probmap(&prob, 20, 20, 3.0, 3.0, DB_THRESH, BOX_THRESH, UNCLIP_RATIO);
        assert_eq!(boxes.len(), 1);
        let b = &boxes[0];
        assert!(
            (3.0..12.0).contains(&b.pts[0].0),
            "unclip 外扩后起点介于 3×0 与未外扩的 3×4 之间: {b:?}"
        );
        // 精确断言：框右下至少覆盖热区右下（30,30）。
        assert!(b.pts[2].0 >= 30.0 && b.pts[2].1 >= 30.0);
    }

    /// 行序排序：先上后下，同行先左后右。
    #[test]
    fn reading_order_sort() {
        let prob = probmap(60, 30, |x, y| {
            if (5..10).contains(&x) && (20..25).contains(&y) {
                1.0
            } else if (30..35).contains(&x) && (2..7).contains(&y) {
                1.0
            } else {
                0.0
            }
        });
        let boxes = boxes_from_probmap(&prob, 60, 30, 1.0, 1.0, DB_THRESH, BOX_THRESH, UNCLIP_RATIO);
        assert_eq!(boxes.len(), 2);
        assert!(boxes[0].pts[0].0 > 25.0, "上行的框在前（即便它偏右）: {boxes:?}");
    }

    /// 尺寸不符/空图防御。
    #[test]
    fn malformed_input_yields_no_boxes() {
        assert!(boxes_from_probmap(&[], 4, 4, 1.0, 1.0, DB_THRESH, BOX_THRESH, UNCLIP_RATIO).is_empty());
        assert!(boxes_from_probmap(&[0.9; 15], 4, 4, 1.0, 1.0, DB_THRESH, BOX_THRESH, UNCLIP_RATIO).is_empty());
    }
}
