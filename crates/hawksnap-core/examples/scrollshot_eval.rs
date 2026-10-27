//! 长截图拼接 **eval 跑批器**（F10 自动验收层）。
//!
//! 用法：
//! ```text
//! cargo run --release -p hawksnap-core --example scrollshot_eval -- \
//!     scripts/scrollshot-eval/scenes.json
//! ```
//!
//! ## 它做什么
//!
//! 读 `scripts/scrollshot-eval/scenes.json`（20 个场景定义，与 HTML 页面生成器
//! **共用同一份** → 自动层与人工层场景口径一致），对每个场景：
//!
//! 1. 按 layout 合成一张**长页面**（正文/图片/表格/代码/聊天/固定头部/粘性侧栏/
//!    深色…各场景形态不同）；
//! 2. **模拟一次采集会话**：逐帧切视口 → 喂进**生产同一个**流式拼接器 → 用
//!    拼接器的**实测位移**决定下一拍间隔（与产品 `scrollshot.rs` 同款：测不出
//!    就拍密、静止守卫只在测不出时兜底）→ 收尾出图；
//! 3. 走**生产同一条**拼接路径（[`hawksnap_core::stitch::Stitcher`]）；
//! 4. 判定：输出高度是否等于期望（偏移链无漂移）、是否 ≥ 20000px、内容是否与
//!    原页面逐像素对齐（**无重影**的判定面）。
//!
//! ## 节奏模型与产品同源（评审 Minor）
//!
//! 旧版 eval 从**真值速度**推采集间隔 → 产品侧的 I1 改动（测量优先 / 测不出
//! 就拍密）**完全没有被覆盖**。现在间隔由**实测位移**决定（与产品同一个
//! 匹配器），产品改节奏 → eval 立刻跟着变。
//!
//! 报告：逐场景一行 + 总成功率 + 高度统计；退出码非 0 = 成功率未达 90%（18/20）。
//!
//! ## 为什么是「合成像素」而不是真浏览器
//!
//! 真浏览器滚动需要 GUI + 人工操作（人工层由控制器执行，见 `pages/` 目录）；
//! 自动层的价值在于**穷举地压测拼接算法本身**——20 个场景 × 40 步滚动 =
//! 800 次配对匹配，覆盖固定头部/侧栏/重复表格/深色等形态。合成像素可控、
//! 可复现、能给出**逐像素**的 ground truth（真浏览器截图做不到）。

use hawksnap_core::scrollshot::{resolve_shift, Pacing};
use hawksnap_core::stitch::{Frame, Stitched, Stitcher};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

// ---------------------------------------------------------------------------
// 场景定义（JSON）
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ScenesFile {
    frame_width: u32,
    frame_height: u32,
    min_output_height: u32,
    /// 基准采集间隔（ms）——与产品侧 `scrollshot.rs` 的 CAPTURE_INTERVAL 同源。
    capture_interval_ms: u64,
    /// 快速滚动时的加密间隔（ms）——产品侧 CAPTURE_INTERVAL_FAST。
    capture_interval_fast_ms: u64,
    /// 快/慢判定阈值（相对选区高度）——产品侧 FAST_SHIFT_RATIO / SLOW_SHIFT_RATIO。
    fast_shift_ratio: f64,
    slow_shift_ratio: f64,
    scenes: Vec<Scene>,
}

#[derive(Deserialize)]
struct Scene {
    id: String,
    layout: String,
    seed: u64,
    scroll: ScrollSpec,
    /// "standard" = F10 计入的 20 个标准场景；"limit" = 已知局限的专项场景
    /// （不计入成功率，只断言「无重影」——它测的是**边界**，不是合格线）。
    #[serde(default = "default_kind")]
    kind: String,
    /// **中途作废帧**的下标：该帧被替换成「无特征白屏」（渲染闪烁 / 弹窗 /
    /// 加载占位）→ 匹配失败 → 偏移链必须跳帧并**对上一张被接受的帧重新基准**
    /// （C1 的端到端形态：若沿用旧基准，之后每帧都少一个滚动步长 → 尾部整段
    /// 重叠 → **重影**）。`None` = 不注入（默认）。
    ///
    /// 这是「中段歧义/不可用帧」的真实形态：不是页面本身有洞，而是**某一帧**
    /// 拍废了——后续帧仍与最后一张好帧有重叠，链必须能恢复。
    #[serde(default)]
    flash_at: Option<usize>,
}

fn default_kind() -> String {
    "standard".into()
}

/// 滚动模型：**恒定滚动速度**（px/s）+ 产品的自适应采集间隔。
///
/// 步长不是直接给定的——它 = 速度 × 采集间隔，而采集间隔由产品侧的滞回规则
/// 决定（滚得快 → 加密到 200ms；滚得慢 → 回到 500ms）。故本 harness 复刻产品
/// 的间隔规则，得到的步长序列即**产品真实会拍到的序列**。
#[derive(Deserialize)]
struct ScrollSpec {
    count: u32,
    velocity: u32,
}

// ---------------------------------------------------------------------------
// 伪随机 + 画布
// ---------------------------------------------------------------------------

/// 确定性 LCG（与单测里的合成页面同一族）。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    /// [lo, hi] 闭区间。
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + self.next() % (hi - lo + 1)
    }
}

/// RGBA 画布（行优先）。
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32, bg: [u8; 4]) -> Self {
        let mut px = Vec::with_capacity((w as usize) * (h as usize) * 4);
        for _ in 0..(w as usize) * (h as usize) {
            px.extend_from_slice(&bg);
        }
        Self { w, h, px }
    }
    fn set(&mut self, x: u32, y: u32, c: [u8; 4]) {
        if x >= self.w || y >= self.h {
            return;
        }
        let i = ((y as usize) * (self.w as usize) + x as usize) * 4;
        self.px[i..i + 4].copy_from_slice(&c);
    }
    fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, c: [u8; 4]) {
        for yy in y..(y + h).min(self.h) {
            for xx in x..(x + w).min(self.w) {
                self.set(xx, yy, c);
            }
        }
    }
    /// 渐变块（图片占位：模拟真实图片的纹理，避免大片纯色）。
    ///
    /// 注：**必须带逐像素噪点**——纯渐变的归一化互相关对颜色仿射不变（两个不同
    /// 颜色的渐变块相关度仍 ≈1），合成器若画「完美渐变」，图库场景就会退化成
    /// 真歧义（实测：多个位移的二维分都是 0.99 → 匹配弃权）。真实照片有噪点与
    /// 细节，故此处加 ±14 的噪声纹理（与真实照片的判别力相当）。
    fn gradient_block(&mut self, x: u32, y: u32, w: u32, h: u32, rng: &mut Rng) {
        let base = [rng.range(40, 220) as u8, rng.range(40, 220) as u8, rng.range(40, 220) as u8, 255];
        for yy in y..(y + h).min(self.h) {
            let t = if h > 0 { (yy - y) as f64 / h as f64 } else { 0.0 };
            for xx in x..(x + w).min(self.w) {
                let s = if w > 0 { (xx - x) as f64 / w as f64 } else { 0.0 };
                let k = 0.55 + 0.45 * (1.0 - (t - s).abs());
                let n = rng.range(0, 28) as i32 - 14;
                self.set(
                    xx,
                    yy,
                    [
                        (base[0] as f64 * k) as i32 + n,
                        (base[1] as f64 * k) as i32 + n,
                        (base[2] as f64 * k) as i32 + n,
                        255,
                    ]
                    .map(|v| v.clamp(0, 255) as u8),
                );
            }
        }
    }
}

/// 一行「文字」：若干词块（词间留白 = 行内空白）。
fn draw_text_row(c: &mut Canvas, y: u32, x0: u32, x1: u32, rng: &mut Rng, ink: [u8; 3], size: u32) {
    let mut x = x0;
    while x + 20 < x1 {
        let word_w = rng.range(18, 90) as u32;
        if x + word_w > x1 {
            break;
        }
        let shade = rng.range(0, 90) as u8;
        let col = [
            ink[0].saturating_sub(shade),
            ink[1].saturating_sub(shade),
            ink[2].saturating_sub(shade),
            255,
        ];
        c.fill_rect(x, y, word_w, size, col);
        x += word_w + rng.range(8, 26) as u32;
    }
}

/// 段落块：若干行 + 行间距。返回结束 y。
fn draw_paragraphs(
    c: &mut Canvas,
    y: u32,
    x0: u32,
    x1: u32,
    rng: &mut Rng,
    ink: [u8; 3],
    line_h: u32,
    gap: u32,
    min_lines: u32,
    max_lines: u32,
) -> u32 {
    draw_paragraphs_until(c, y, x0, x1, rng, ink, line_h, gap, min_lines, max_lines, c.h)
}

/// 同 [`draw_paragraphs`]，但在 `limit`（画布行）处停笔（留白用）。
fn draw_paragraphs_until(
    c: &mut Canvas,
    y: u32,
    x0: u32,
    x1: u32,
    rng: &mut Rng,
    ink: [u8; 3],
    line_h: u32,
    gap: u32,
    min_lines: u32,
    max_lines: u32,
    limit: u32,
) -> u32 {
    let limit = limit.min(c.h);
    let mut y = y;
    while y + line_h < limit {
        let lines = rng.range(min_lines as u64, max_lines as u64) as u32;
        for _ in 0..lines {
            if y + line_h >= limit {
                break;
            }
            draw_text_row(c, y, x0 + rng.range(0, 26) as u32, x1, rng, ink, line_h);
            y += line_h + 2;
        }
        y += gap;
    }
    y
}

/// 表格块：网格线 + 单元格文字。
fn draw_table(c: &mut Canvas, mut y: u32, x0: u32, x1: u32, rows: u32, cols: u32, rng: &mut Rng, ink: [u8; 3]) -> u32 {
    let row_h = 26u32;
    let col_w = (x1 - x0) / cols.max(1);
    for _ in 0..rows {
        if y + row_h >= c.h {
            break;
        }
        // 行分隔线
        c.fill_rect(x0, y, x1 - x0, 1, [190, 195, 205, 255]);
        for col in 0..cols {
            let cx = x0 + col * col_w;
            if col > 0 {
                c.fill_rect(cx, y, 1, row_h, [200, 205, 215, 255]);
            }
            draw_text_row(c, y + 8, cx + 6, cx + col_w - 6, rng, ink, 10);
        }
        y += row_h;
    }
    y
}

/// 代码块：缩进 + 短 token 行（等宽感：词长更规整）。
fn draw_code(c: &mut Canvas, mut y: u32, x0: u32, x1: u32, lines: u32, rng: &mut Rng, dark: bool) -> u32 {
    let ink: [u8; 3] = if dark { [180, 190, 205] } else { [60, 70, 85] };
    let bg: [u8; 4] = if dark { [31, 36, 44, 255] } else { [244, 246, 249, 255] };
    for _ in 0..lines {
        if y + 18 >= c.h {
            break;
        }
        c.fill_rect(x0, y, x1 - x0, 18, bg);
        let indent = x0 + rng.range(0, 5) as u32 * 16;
        let mut x = indent;
        while x + 30 < x1 - 20 {
            let w = rng.range(24, 70) as u32;
            let shade = rng.range(0, 120) as u8;
            let col: [u8; 4] = [
                ink[0].saturating_sub(shade),
                ink[1].saturating_sub(shade),
                ink[2].saturating_sub(shade),
                255,
            ];
            c.fill_rect(x, y + 4, w, 9, col);
            x += w + 10;
        }
        y += 18;
    }
    y
}

/// 聊天块：左右气泡（宽度随机 → 行剖面非周期）。
fn draw_chat(c: &mut Canvas, mut y: u32, x0: u32, x1: u32, msgs: u32, rng: &mut Rng, ink: [u8; 3]) -> u32 {
    let bubble = [235, 240, 248, 255];
    let mine = [215, 240, 226, 255];
    for _ in 0..msgs {
        if y + 40 >= c.h {
            break;
        }
        let is_mine = rng.next() % 2 == 0;
        let bw = rng.range(120, (x1 - x0) as u64 / 2) as u32;
        let bh = rng.range(24, 70) as u32;
        let bx = if is_mine { x1 - bw - 16 } else { x0 + 16 };
        let col = if is_mine { mine } else { bubble };
        c.fill_rect(bx, y, bw, bh, col);
        let mut yy = y + 6;
        while yy + 12 < y + bh {
            draw_text_row(c, yy, bx + 8, bx + bw - 8, rng, ink, 9);
            yy += 14;
        }
        y += bh + 10;
    }
    y
}

/// 网格瓦片（图库）。
fn draw_grid(c: &mut Canvas, mut y: u32, x0: u32, x1: u32, cols: u32, tiles: u32, rng: &mut Rng) -> u32 {
    let tile_w = (x1 - x0) / cols.max(1);
    let tile_h = 120u32;
    for _ in 0..tiles {
        if y + tile_h + 12 >= c.h {
            break;
        }
        for col in 0..cols {
            let tx = x0 + col * tile_w;
            c.gradient_block(tx + 6, y, tile_w - 12, tile_h, rng);
            // 瓦片内的「照片内容」（真实图片各不相同；纯渐变瓦片会让整幅内容
            // 准周期化 → 匹配真歧义，属合成器的失真而非真实场景）。
            for _ in 0..3 {
                let ly = y + rng.range(10, (tile_h - 20) as u64) as u32;
                let lw = rng.range(20, (tile_w / 2) as u64) as u32;
                let lx = tx + 12 + rng.range(0, (tile_w / 3) as u64) as u32;
                let shade = rng.range(30, 200) as u8;
                c.fill_rect(lx, ly, lw.min(tile_w), 10, [shade, shade, shade, 255]);
            }
        }
        y += tile_h + 12;
    }
    y
}

/// 章节标题（大字 + 下方细线）。
fn draw_heading(c: &mut Canvas, y: u32, x0: u32, x1: u32, rng: &mut Rng, ink: [u8; 3]) -> u32 {
    let w = rng.range(120, 320) as u32;
    c.fill_rect(x0, y, w, 18, [ink[0], ink[1], ink[2], 255]);
    c.fill_rect(x0, y + 26, x1 - x0, 1, [210, 215, 225, 255]);
    y + 40
}

// ---------------------------------------------------------------------------
// 页面合成（按 layout）
// ---------------------------------------------------------------------------

/// 页面形态：固定件（固定头部 / 粘性侧栏）——**每帧都重绘**（滚动时不动）。
#[derive(Clone, Copy, PartialEq)]
enum Chrome {
    None,
    FixedHeader(u32),
    StickySidebar(u32),
}

fn render_page(scene: &Scene, w: u32, h: u32, band_top: Option<u32>) -> (Canvas, Chrome) {
    let dark = scene.layout == "dark";
    let bg = if dark { [20, 23, 28, 255] } else { [255, 255, 255, 255] };
    let ink: [u8; 3] = if dark { [216, 222, 233] } else { [29, 36, 48] };
    let mut c = Canvas::new(w, h, bg);
    let mut rng = Rng(scene.seed.wrapping_mul(2654435761).wrapping_add(12345));
    let x0 = 24u32;
    let x1 = w - 24;
    let mut y = 20u32;
    let mut chrome = Chrome::None;

    match scene.layout.as_str() {
        "text" => {
            draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 3, 6);
        }
        "text-dense" => {
            draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 7, 5, 6, 12);
        }
        "images" => {
            while y < h {
                y = draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 2, 5);
                y = y.max(0);
                if y >= h {
                    break;
                }
                let bh = rng.range(120, 260) as u32;
                c.gradient_block(x0, y, x1 - x0, bh, &mut rng);
                y += bh + 20;
            }
        }
        "tables" => {
            while y < h {
                y = draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 2, 4);
                y = draw_table(&mut c, y, x0, x1, 24, 6, &mut rng, ink);
                y += 24;
            }
        }
        "code" => {
            while y < h {
                y = draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 2, 4);
                y = draw_code(&mut c, y, x0, x1, 60, &mut rng, dark);
                y += 24;
            }
        }
        "chat" => {
            draw_chat(&mut c, y, x0, x1, h / 60, &mut rng, ink);
        }
        "fixed-header" => {
            chrome = Chrome::FixedHeader(48);
            draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 3, 6);
        }
        "sticky-sidebar" => {
            chrome = Chrome::StickySidebar(220);
            draw_paragraphs(&mut c, y, x0 + 240, x1, &mut rng, ink, 11, 14, 3, 6);
        }
        "dark" => {
            draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 3, 6);
        }
        "doc" => {
            while y < h {
                y = draw_heading(&mut c, y, x0, x1, &mut rng, ink);
                y = draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 3, 6);
                y = draw_table(&mut c, y, x0, x1, 14, 4, &mut rng, ink);
                y += 30;
            }
        }
        "gallery" => {
            draw_grid(&mut c, y, x0, x1, 3, h / 140, &mut rng);
        }
        "blog" => {
            while y < h {
                y = draw_heading(&mut c, y, x0, x1, &mut rng, ink);
                let bh = rng.range(90, 200) as u32;
                c.gradient_block(x0, y, (x1 - x0) / 2, bh, &mut rng);
                y = draw_paragraphs(&mut c, y, x0 + (x1 - x0) / 2 + 20, x1, &mut rng, ink, 10, 12, 2, 5);
                y += 30;
            }
        }
        "wiki" => {
            while y < h {
                y = draw_heading(&mut c, y, x0, x1, &mut rng, ink);
                y = draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 3, 6);
                y = draw_table(&mut c, y, x0, x1, 16, 3, &mut rng, ink);
                y += 30;
            }
        }
        "form" => {
            while y < h {
                for _ in 0..rng.range(1, 4) {
                    if y + 46 >= h {
                        break;
                    }
                    draw_text_row(&mut c, y, x0, x0 + 160, &mut rng, ink, 10);
                    c.fill_rect(x0, y + 18, 320, 20, [246, 248, 252, 255]);
                    c.fill_rect(x0, y + 18, 320, 1, [200, 206, 216, 255]);
                    y += 46;
                }
                y = draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 2, 5);
            }
        }
        "news" => {
            y = draw_heading(&mut c, y, x0, x1, &mut rng, ink);
            while y < h {
                y = draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 3, 6);
                let bh = rng.range(120, 240) as u32;
                c.gradient_block(x0, y, x1 - x0, bh, &mut rng);
                y += bh + 24;
            }
        }
        "forum" => {
            while y < h {
                y = draw_heading(&mut c, y, x0, x1, &mut rng, ink);
                y = draw_paragraphs(&mut c, y, x0 + 20, x1 - 20, &mut rng, ink, 10, 12, 2, 4);
                y += 24;
            }
        }
        "pdf-toolbar" => {
            chrome = Chrome::FixedHeader(40);
            draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 3, 6);
        }
        "spreadsheet" => {
            while y < h {
                y = draw_table(&mut c, y, x0, x1, 40, 8, &mut rng, ink);
                y += 20;
            }
        }
        "slides" => {
            while y < h {
                y = draw_heading(&mut c, y, x0, x1, &mut rng, ink);
                y = draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 12, 16, 2, 5);
                y += 200;
            }
        }
        // N5 回归场景：中段一大片空白（无特征）→ 跨越它的帧对匹配失败/歧义 →
        // 偏移链必须在中途跳帧后**重新基准**（C1 的端到端形态：跳帧后若沿用
        // 旧基准，后续每帧都少一个滚动步长 → 尾部整段重叠 → 重影）。
        "blank-band" => {
            // 页面**中段**留一大段空白（纯白、无任何特征，长度 ≈ 2.5 屏）→
            // 完全落在带内的帧「无特征」→ 与邻居匹配失败 → 偏移链必须在中途
            // 跳帧后**重新基准**（C1 的端到端形态）。
            //
            // `band_top` 由调用方给定（**跨迭代固定**）：页面高度会随「实测节奏」
            // 迭代变化，若空白带跟着高度漂移 → 场景本身在迭代中变形 → 固定点
            // 求解不稳。`None` = 兜底用 h/2。
            let band_top = band_top.unwrap_or(h / 2);
            draw_paragraphs_until(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 3, 6, band_top);
            let band_h = (h / 9).min(1000);
            for yy in band_top..(band_top + band_h).min(h) {
                for xx in x0..x1 {
                    let i = ((yy * w + xx) * 4) as usize;
                    c.px[i] = 255;
                    c.px[i + 1] = 255;
                    c.px[i + 2] = 255;
                    c.px[i + 3] = 255;
                }
            }
            draw_paragraphs(&mut c, band_top + band_h, x0, x1, &mut rng, ink, 11, 14, 3, 6);
        }
        "mixed" | _ => {
            while y < h {
                y = draw_paragraphs(&mut c, y, x0, x1, &mut rng, ink, 11, 14, 2, 5);
                let bh = rng.range(100, 200) as u32;
                c.gradient_block(x0, y, (x1 - x0) / 2, bh, &mut rng);
                y += bh + 20;
                y = draw_table(&mut c, y, x0, x1, 18, 5, &mut rng, ink);
                y = draw_code(&mut c, y, x0, x1, 40, &mut rng, dark);
                y += 24;
            }
        }
    }
    (c, chrome)
}

/// 从页面切一帧（模拟一次抓取），并叠加「固定件」（固定头部/粘性侧栏每帧重绘）。
fn viewport(page: &Canvas, offset: usize, vh: u32, chrome: Chrome) -> Frame {
    let w = page.w;
    let mut px = Vec::with_capacity((w as usize) * (vh as usize) * 4);
    for y in 0..vh as usize {
        let sy = (offset + y).min(page.h as usize - 1);
        let s = sy * (w as usize) * 4;
        px.extend_from_slice(&page.px[s..s + (w as usize) * 4]);
    }
    // 固定件覆盖
    match chrome {
        Chrome::None => {}
        Chrome::FixedHeader(hh) => {
            for y in 0..hh.min(vh) {
                for x in 0..w {
                    let i = ((y as usize) * (w as usize) + x as usize) * 4;
                    px[i] = 238;
                    px[i + 1] = 242;
                    px[i + 2] = 247;
                    px[i + 3] = 255;
                }
                // 头部左侧一段「logo 文字」（静态 → 破坏「整幅平移」模型，
                // 考验顶栏切除能力；纯色带则无特征、切不切都一样）。
                if y > 12 && y < 30 {
                    for x in 20..260 {
                        let i = ((y as usize) * (w as usize) + x as usize) * 4;
                        px[i] = 40;
                        px[i + 1] = 45;
                        px[i + 2] = 55;
                        px[i + 3] = 255;
                    }
                }
            }
        }
        Chrome::StickySidebar(sw) => {
            for y in 0..vh {
                for x in 0..sw.min(w) {
                    let i = ((y as usize) * (w as usize) + x as usize) * 4;
                    px[i] = 244;
                    px[i + 1] = 246;
                    px[i + 2] = 249;
                    px[i + 3] = 255;
                }
            }
        }
    }
    Frame { width: w, height: vh, rgba: px }
}

// ---------------------------------------------------------------------------
// 评估
// ---------------------------------------------------------------------------

struct SceneResult {
    id: String,
    layout: String,
    ok: bool,
    reason: String,
    height: u32,
    expected_height: u32,
    align: f64,
    /// 输入帧数 / 实际进入偏移链的帧数（后者 < 前者 = 中途跳帧 → 触发 C1 重基准）。
    frames: (usize, usize),
    /// 是否为 F10 计入的标准场景（false = 专项局限场景）。
    is_standard: bool,
    /// **迭代是否收敛**（步长 ← 实测节奏 ← 帧 ← 页面 ← 页高 ← 步长 的不动点）。
    /// false = 该场景的 eval 模型本身不稳定 → **判失败**（不得静默按另一个页高
    /// 重新渲染后当作「测过了」）。
    converged: bool,
}

/// 内容对齐率：输出与原页面逐像素比较（跳过固定件覆盖区）。
/// 低于 0.95 即视为出现**重影/错位**。
fn alignment(out: &hawksnap_core::stitch::Stitched, page: &Canvas, chrome: Chrome, offsets: &[usize]) -> f64 {
    let w = out.width as usize;
    let mut total = 0usize;
    let mut same = 0usize;
    let mut y = 0usize;
    while y < out.height as usize {
        // 固定头部带：这些行在输出里是「头部」而非页面内容 → 跳过。
        let in_header = match chrome {
            Chrome::FixedHeader(hh) => offsets.iter().any(|o| y >= *o && y < *o + hh as usize),
            _ => false,
        };
        let sy = y.min(page.h as usize - 1);
        let srow = sy * (page.w as usize) * 4;
        let drow = y * w * 4;
        let mut x = 0usize;
        while x < w {
            // 粘性侧栏列：输出里是侧栏而非页面内容 → 跳过。
            let skip_col = matches!(chrome, Chrome::StickySidebar(sw) if x < sw as usize);
            if !in_header && !skip_col {
                let si = srow + x * 4;
                let di = drow + x * 4;
                total += 1;
                if out.rgba[di..di + 3] == page.px[si..si + 3] {
                    same += 1;
                }
            }
            x += 7; // 列抽样（每 7 列取 1 列，够统计又不慢）
        }
        y += 3; // 行抽样
    }
    if total == 0 {
        return 0.0;
    }
    same as f64 / total as f64
}

/// 采集节奏参数（产品与 eval **共用 core 的同一份规则** → 不会各改各的）。
///
/// 评审 Minor：旧 eval 从**真值**推间隔 → 产品侧的 I1 改动（测量优先 / 测不出
/// 就拍密）**完全没被覆盖**。现在间隔由**实测位移**决定（与产品同一个匹配器
/// —— [`Stitcher::measured_shift`]），且规则本体与产品同源。
fn pacing_from_spec(spec: &ScenesFile) -> Pacing {
    Pacing::new(
        Duration::from_millis(spec.capture_interval_ms),
        Duration::from_millis(spec.capture_interval_fast_ms),
        spec.fast_shift_ratio,
        spec.slow_shift_ratio,
    )
}

/// 一次「模拟采集 + 拼接」的结果。
struct Simulation {
    /// 步长序列（每步 = 用户在一拍间隔内滚过的距离）。
    steps: Vec<u32>,
    /// 每帧的页面偏移（真值，用于对齐率比对）。
    offsets: Vec<usize>,
    /// 拼接产物（`None` = 拼接失败：无可靠匹配）。
    out: Option<Stitched>,
    /// 输入帧数（= steps.len() + 1）。
    frames_in: usize,
    /// 进入偏移链的帧数。
    frames_used: usize,
    /// 触到画布上界被截断。
    truncated: bool,
}

/// **模拟一次采集会话**（与产品同款节奏）：逐帧切视口 → 喂进**同一个**流式
/// 拼接器 → 用拼接器的**实测位移**决定下一拍间隔 → 最后收尾出图。
///
/// 与产品 `spawn_capture_thread` 的逐步同构：
/// 1. 抓帧 → 推入拼接器；
/// 2. 读 `measured_shift()`（当前待决帧相对上一张**被接受**帧的估计）；
/// 3. 按滞回规则定下一拍间隔（测不出 → 拍密 200ms）；
/// 4. 用户在下一拍间隔里滚过 `速度 × 间隔`（带抖动）→ 下一帧。
fn simulate_capture(
    scene: &Scene,
    spec: &ScenesFile,
    page: &Canvas,
    chrome: Chrome,
    fw: u32,
    fh: u32,
) -> Simulation {
    let mut rng = Rng(scene.seed.wrapping_mul(6364136223846793005).wrapping_add(1));
    let jitter = |r: &mut Rng| -> f64 { 0.85 + r.range(0, 30) as f64 / 100.0 };
    let pacing = pacing_from_spec(spec);
    let mut st = Stitcher::new(fw as usize);
    let mut steps: Vec<u32> = Vec::with_capacity(scene.scroll.count as usize);
    let mut offsets: Vec<usize> = Vec::with_capacity(scene.scroll.count as usize + 1);
    // 上一帧的行亮度剖面（产品侧同款：判定「页面完全没动」）。
    let mut last_profile: Option<Vec<f32>>;
    let mut off = 0usize;
    let mut interval = pacing.base;

    // 首帧（offset 0）→ 入链 → 测不出位移 → **拍密**（与产品同款）。
    let first = viewport(page, off, fh, chrome);
    last_profile = Some(first.row_profile());
    st.push(first);
    offsets.push(off);
    interval = pacing.next_interval(interval, resolve_shift(st.measured_shift(), false), fh);

    for i in 0..scene.scroll.count as usize {
        let step = (scene.scroll.velocity as f64 * interval.as_millis() as f64 / 1000.0
            * jitter(&mut rng))
            .round()
            .max(1.0) as u32;
        off += step as usize;
        let mut frame = viewport(page, off, fh, chrome);
        // 注入「中段作废帧」（C1 回归场景）：某一帧拍成无特征白屏 → 匹配失败 →
        // 偏移链必须跳帧并对上一张被接受的帧**重新基准**（否则尾部整段重叠 → 重影）。
        if scene.flash_at == Some(i + 1) {
            frame = Frame {
                width: fw,
                height: fh,
                rgba: vec![247u8; (fw as usize) * (fh as usize) * 4],
            };
        }
        let profile = frame.row_profile();
        let still = last_profile.as_ref().map(|p| *p == profile).unwrap_or(false);
        last_profile = Some(profile);
        st.push(frame);
        offsets.push(off);
        steps.push(step);
        // 节奏自适应：读**当前帧**的实测位移（与链上同一个匹配器）。
        let shift = resolve_shift(st.measured_shift(), still);
        interval = pacing.next_interval(interval, shift, fh);
    }

    let frames_in = steps.len() + 1;
    let truncated = st.truncated();
    let out = st.finish();
    // `frames_used` 必须在 `finish()` **之后**读（收尾帧也进链）。
    let frames_used = st.frames_used();
    Simulation { steps, offsets, out, frames_in, frames_used, truncated }
}

/// 旧模型（**真值**速度推间隔）算出的步长序列——只作为**不动点迭代的初值**
/// （页面的初始高度依赖它）。
fn ground_truth_steps(scene: &Scene, spec: &ScenesFile, fh: u32) -> Vec<u32> {
    let mut rng = Rng(scene.seed.wrapping_mul(6364136223846793005).wrapping_add(1));
    let jitter = |r: &mut Rng| -> f64 { 0.85 + r.range(0, 30) as f64 / 100.0 };
    let mut steps: Vec<u32> = Vec::with_capacity(scene.scroll.count as usize);
    let mut interval_ms = spec.capture_interval_ms;
    for _ in 0..scene.scroll.count {
        let step = (scene.scroll.velocity as f64 * interval_ms as f64 / 1000.0 * jitter(&mut rng))
            .round()
            .max(1.0) as u32;
        steps.push(step);
        let ratio = step as f64 / fh as f64;
        interval_ms = if ratio > spec.fast_shift_ratio {
            spec.capture_interval_fast_ms
        } else if ratio < spec.slow_shift_ratio {
            spec.capture_interval_ms
        } else {
            interval_ms
        };
    }
    steps
}

fn run_scene(scene: &Scene, spec: &ScenesFile, fw: u32, fh: u32) -> SceneResult {
    let min_output_height = spec.min_output_height;
    // —— 不动点迭代：步长 ← 实测节奏 ← 帧 ← 页面 ← 页面高度 ← 步长 ——
    //
    // 页面的**内容**自上而下绘制 → 同样的高度区间内容一致；但**页面高度**本身
    // 由步长总和决定 → 「实测节奏」反过来改页面高度 → 迭代到稳定为止
    // （通常 2 轮：第 1 轮用旧步长渲染页面，第 2 轮用实测步长渲染 → 稳定）。
    let mut steps = ground_truth_steps(scene, spec, fh);
    // 空白带位置**跨迭代固定**（否则场景在迭代中变形 → 固定点不稳）。
    let band_anchor: Option<u32> = Some((fh + steps.iter().sum::<u32>()) / 2);
    let mut sim: Option<Simulation> = None;
    let mut page = Canvas::new(fw, 0, [0, 0, 0, 255]);
    let mut chrome = Chrome::None;
    let mut page_h_used = 0u32;
    let mut converged = false;
    for _ in 0..4 {
        let total: u32 = steps.iter().sum();
        page_h_used = fh + total;
        let (p, c) = render_page(scene, fw, page_h_used, band_anchor);
        let s = simulate_capture(scene, spec, &p, c, fw, fh);
        let is_fixed = s.steps == steps;
        page = p;
        chrome = c;
        sim = Some(s);
        if is_fixed {
            converged = true;
            break;
        }
        steps = sim.as_ref().unwrap().steps.clone();
    }
    let sim = sim.expect("至少迭代一轮");
    let total: u32 = sim.steps.iter().sum();
    // **未收敛**（评审 Minor：不得静默换个页高重渲染）：
    // 步长 ← 实测节奏 ← 帧 ← 页面 ← 页高 ← 步长 构成循环，4 轮内没到稳定点 →
    // 说明该场景的 eval 模型本身不稳定（基准页高与实际页高分离）→ **判失败**，
    // 而不是「按最终步长重新渲染页面」后当作测过了（那样得到的高度差是
    // 假象：比对基准被换掉了，缺口的来源被掩盖）。
    let expected_height = fh + total;
    let reference_height = if converged { expected_height } else { page_h_used };

    match sim.out {
        None => SceneResult {
            id: scene.id.clone(),
            layout: scene.layout.clone(),
            ok: false,
            reason: "拼接失败（无可靠匹配：歧义/无重叠/纯色）".into(),
            height: 0,
            expected_height,
            align: 0.0,
            frames: (sim.frames_in, 0),
            is_standard: scene.kind == "standard",
            converged,
        },
        Some(out) => {
            let drift = (out.height as i64 - reference_height as i64).abs();
            let align = alignment(&out, &page, chrome, &sim.offsets);
            let mut problems: Vec<String> = Vec::new();
            let mut note = String::new();
            // **未收敛 → 判失败**（评审 Minor）：eval 模型（步长 ↔ 页高的不动点）
            // 没稳定 → 该场景的结论不可信 → 如实判失败，不静默换基准。
            if !converged {
                problems.push(format!(
                    "eval 迭代未收敛（页高 {page_h_used}px 与实测步长推出的 {expected_height}px 分离）"
                ));
            }
            // F10 三项判据：无重影 / 输出 ≥20000px / 内容完整（无缺口）。
            if align < 0.95 {
                problems.push(format!("内容错位（对齐率 {:.3}）", align));
            }
            if scene.kind == "limit" {
                // 专项局限场景：只断言「无重影」（它测的是**边界**，不是合格线）。
                // 内容被截断是**预期结果**（空白带内无任何可匹配特征 → 链无法
                // 跨越——这是所有匹配方案共有的固有限制，ORB 亦然），故不判失败，
                // 只在结论里如实记一句。
                if drift > 0 {
                    note = format!("内容截断 {}px（预期：空白带不可跨越）", drift);
                }
            } else {
                if out.height < min_output_height {
                    problems.push(format!(
                        "输出高度不足（{} < {}）",
                        out.height, min_output_height
                    ));
                }
                // 内容完整：允许极小的漂移（1%），超出即判「有内容缺口」。
                let drift_limit = (reference_height as f64 * 0.01).max(8.0) as i64;
                if drift > drift_limit {
                    problems.push(format!("内容缺口 {}px（漂移上限 {drift_limit}）", drift));
                }
            }
            if sim.truncated {
                problems.push("触到画布内存上界被截断".into());
            }
            SceneResult {
                id: scene.id.clone(),
                layout: scene.layout.clone(),
                ok: problems.is_empty(),
                reason: if !problems.is_empty() {
                    problems.join("；")
                } else if note.is_empty() {
                    "OK".into()
                } else {
                    format!("OK（{note}）")
                },
                height: out.height,
                expected_height,
                align,
                frames: (sim.frames_in, sim.frames_used),
                is_standard: scene.kind == "standard",
                converged,
            }
        }
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "scripts/scrollshot-eval/scenes.json".to_string());
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读不到场景定义 {path}: {e}"));
    let spec: ScenesFile = serde_json::from_str(&raw).unwrap_or_else(|e| panic!("场景定义解析失败: {e}"));

    println!(
        "[scrollshot-eval] 场景数 {} · 帧高 {}px · 最低输出高度 {}px · 采集间隔 {}/{}ms（自适应）",
        spec.scenes.len(), spec.frame_height, spec.min_output_height,
        spec.capture_interval_ms, spec.capture_interval_fast_ms
    );
    println!("[scrollshot-eval] {:<16} {:<16} {:>8} {:>9} {:>8}  {}", "场景", "形态", "输出高", "期望高", "对齐率", "结论");

    let mut results: Vec<SceneResult> = Vec::new();
    for scene in &spec.scenes {
        let r = run_scene(scene, &spec, spec.frame_width, spec.frame_height);
        println!(
            "[scrollshot-eval] {:<16} {:<16} {:>8} {:>9} {:>8.3}  {:>3}/{:<3}帧  {}{}",
            r.id, r.layout, r.height, r.expected_height, r.align,
            r.frames.1, r.frames.0, r.reason,
            if r.converged { "" } else { "  ⚠ eval 未收敛" }
        );
        results.push(r);
    }

    let standard: Vec<&SceneResult> = results.iter().filter(|r| r.is_standard).collect();
    let limit: Vec<&SceneResult> = results.iter().filter(|r| !r.is_standard).collect();
    let total = standard.len();
    let passed = standard.iter().filter(|r| r.ok).count();
    // F10 字面判据：无重影（对齐率 ≥0.95）且输出 ≥20000px。
    let no_ghost = standard.iter().filter(|r| r.align >= 0.95).count();
    let long_ok = standard
        .iter()
        .filter(|r| r.height >= spec.min_output_height)
        .count();
    let f10_ok = standard
        .iter()
        .filter(|r| r.align >= 0.95 && r.height >= spec.min_output_height)
        .count();
    let rate = passed as f64 / total.max(1) as f64;
    // 高度统计只看标准场景（专项局限场景按设计会被截断）。
    let heights: Vec<u32> = standard.iter().map(|r| r.height).collect();
    let min_h = heights.iter().copied().min().unwrap_or(0);
    let max_h = heights.iter().copied().max().unwrap_or(0);

    println!();
    println!("[scrollshot-eval] 成功率 {passed}/{total} = {:.1}%（严格判据：无重影 + 输出≥{}px + 无内容缺口；门槛 ≥90%）", rate * 100.0, spec.min_output_height);
    println!("[scrollshot-eval] F10 字面判据（无重影 {no_ghost}/{total}，输出≥{}px {long_ok}/{total}）：{f10_ok}/{total} = {:.1}%", spec.min_output_height, f10_ok as f64 / total.max(1) as f64 * 100.0);
    println!("[scrollshot-eval] 输出高度：最小 {min_h}px · 最大 {max_h}px");
    if !limit.is_empty() {
        println!(
            "[scrollshot-eval] 专项局限场景 {} 个（不计入成功率，只断言无重影）：",
            limit.len()
        );
        for r in &limit {
            println!(
                "  {:<16} {:<16} {:>8}px  对齐 {:.3}  用帧 {}/{}  {}",
                r.id, r.layout, r.height, r.align, r.frames.1, r.frames.0, r.reason
            );
        }
    }
    println!("[scrollshot-eval] 逐场景：");
    for r in &standard {
        println!(
            "  {:<16} {:<16} {:>8}px  对齐 {:.3}  用帧 {}/{}  {}",
            r.id, r.layout, r.height, r.align, r.frames.1, r.frames.0, r.reason
        );
    }

    // 机器可读摘要（供 run-eval.sh 汇总）。
    let mut by_id: HashMap<&str, (bool, u32, f64)> = HashMap::new();
    for r in &results {
        by_id.insert(r.id.as_str(), (r.ok, r.height, r.align));
    }
    println!("[scrollshot-eval] SUMMARY passed={passed} total={total} rate={:.3} f10={f10_ok} min_h={min_h} max_h={max_h}", rate);

    if rate < 0.9 {
        std::process::exit(1);
    }
}
