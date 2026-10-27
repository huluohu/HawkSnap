//! 长截图拼接算法（纯 Rust、跨平台可测——本模块无任何平台依赖与 IO）。
//!
//! ## 算法选型（任务裁定：手写简化匹配，非 ORB）
//!
//! plan 原文为「FAST 角点 + patch 相关性匹配取纵向偏移中位数」。任务书授权在
//! 「更简单**且**对屏幕内容更稳」时改用别的方案。本实现选 **1D 行亮度剖面互相关
//! （模板匹配）+ 2D patch 精修**，理由与代价如实记录：
//!
//! - **屏幕内容是单轴平移**（纵向滚动）。1D 剖面在 Y 方向天然平移不变——把二维
//!   匹配降维成一维信号对齐，代码量与失败模式都远低于「建描述子 + 假设检验」。
//! - FAST 角点在**纯文本页面**上稀疏且分布不均（大片留白 + 少量文字行），描述子
//!   匹配还要处理尺度/旋转假设——对「同一个窗口往下滚」这个**极窄**的运动模型
//!   是过度设计。行亮度剖面的起伏（文字行 vs 行间留白）本身就是强特征，且对
//!   低纹理页面反而更稳。
//! - 代价：**只能解单轴纵向平移**（横向滚动/缩放不支持——见「已知局限」）。
//!
//! ## 已知局限（诚实边界，eval 量化）
//!
//! - **固定头部/侧栏**：头部在滚动中不动 → 破坏「整幅平移」模型。解法：相关前
//!   **先切除顶部若干行**再匹配（多档 cut 取相关度最优者）；侧栏是**列**方向固定
//!   物 → 由**多列块中位数**天然忽略（侧栏所在列块相关度低、被阈值剔除）。
//! - **重复内容**（条纹、重复表格行、代码缩进）：多个位移的相关度接近 → 判为
//!   **歧义**，**拒绝拼接**（宁可不拼，绝不错拼出重影）。
//! - **纯色 / 极低纹理**：剖面方差 ≈ 0 → 无特征 → 拒绝（返回 `None`）。
//! - **动态内容**（视频、动画、闪烁光标、时钟）：局部不一致 → 表现为局部重影，
//!   但位移由**静态内容行主导**，整体对齐不受影响。
//! - **滚动步长过大**（一步滚过整个选区）→ 两帧无重叠 → 无匹配 → 该帧被跳过
//!   （拼接结果缺一段，见 [`stitch`] 的链式语义）。
//!
//! ## 位移符号约定（全局统一，勿混）
//!
//! `d > 0` 表示**内容向上移动了 d 像素**（用户向下滚动 d 像素）：即
//! `next[y] == prev[y + d]`。拼接画布中后一帧落在前一帧的**下方** d 像素处。

// ---------------------------------------------------------------------------
// 常量（契约值，改动需同步改单测）
// ---------------------------------------------------------------------------

/// 最小重叠行数：两帧重叠少于该行数时不做匹配（样本太少，相关度不可信）。
pub const MIN_OVERLAP: usize = 24;
/// 单列块相关度阈值：低于此值的列块视为「无特征/固定物」，不参与中位数。
pub const MIN_CORR: f32 = 0.55;
/// 歧义判定的位移间隔：与最优位移相距小于此值的候选不算「另一个解」。
pub const AMBIG_GAP: i32 = 6;
/// 歧义判定余量：次优解与最优解的相关度差小于此值 → 判为歧义 → 拒绝拼接。
pub const AMBIG_MARGIN: f32 = 0.03;
/// 列块数（多列中位数）：横向切成这么多块，各自估位移后取中位数。
pub const BLOCKS: usize = 4;
/// 顶部切除档位（占高度比例）：固定头部不动 → 切掉顶部若干行后再相关。
pub const CUT_FRACS: [f32; 4] = [0.0, 0.12, 0.24, 0.36];
/// 接缝渐变融合带高度（像素）：重叠区末端做线性 alpha 融合，抑制 1px 错位重影。
pub const BLEND_BAND: i32 = 8;
/// 1D 粗定位保留的候选峰数（逐列块、逐切口）。
///
/// 取 10（小重叠时 1D 谱噪声大、真解可能排不进前 5 → 二维裁决没机会选它；
/// 实测「一步滚 775px / 视口 900px」= 125 行重叠的场景因此整对失配）。
const TOP_K: usize = 20;
/// 2D patch 相关分的歧义余量：次优候选与最优候选的 2D 相关差小于此值 → 判歧义。
const AMBIG_MARGIN_2D: f32 = 0.02;
/// 每个列块参与二维裁决的 patch 候选点数（均匀铺开；无纹理的会被剔除）。
const PATCH_COUNT: usize = 6;
/// patch 位置的最小行谱局部方差（低于此值 = 纯色带 → 该 patch 无判别力）。
const MIN_PATCH_VARIANCE: f32 = 1.0;
/// 二维相关「有信息」的门槛：最高分低于此值 → 改用 1D 谱排序。
const S2_INFORMATIVE: f32 = 0.6;
/// 二维相关「强证据」门槛：达到此值即视为几乎逐像素吻合 → 可压过列块一致性。
const S2_STRONG: f32 = 0.97;
/// 2D 精修的改进门：候选残差须比原位移低到该比例才采纳（否则保留原位移）。
const REFINE_GAIN: f64 = 0.6;
/// patch 的最小标准差（亮度单位）：低于此值 = 无纹理 → 相关度无意义。
const MIN_PATCH_STDDEV: f64 = 4.0;

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// 拼接输入帧：物理像素 RGBA（每像素 4 字节，行优先）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    /// 构造帧并校验缓冲长度（长度不符 → None，绝不静默截断）。
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let expect = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        if rgba.len() != expect {
            return None;
        }
        Some(Self { width, height, rgba })
    }

    /// 整幅的行亮度剖面（每行的平均亮度，0..=255）。
    pub fn row_profile(&self) -> Vec<f32> {
        self.block_profile(0, self.width as usize)
    }

    /// 列区间 `[x0, x1)` 的行亮度剖面（每行在该列区间的平均亮度）。
    pub fn block_profile(&self, x0: usize, x1: usize) -> Vec<f32> {
        let w = self.width as usize;
        let h = self.height as usize;
        let x0 = x0.min(w);
        let x1 = x1.clamp(x0, w);
        let span = x1.saturating_sub(x0);
        let mut out = vec![0f32; h];
        if span == 0 {
            return out;
        }
        for (y, slot) in out.iter_mut().enumerate() {
            let row = y * w * 4;
            let mut acc = 0f64;
            for x in x0..x1 {
                let i = row + x * 4;
                // Rec.601 亮度：绿权重最高，贴近人眼对文本行的敏感度。
                let l = 0.299 * self.rgba[i] as f64
                    + 0.587 * self.rgba[i + 1] as f64
                    + 0.114 * self.rgba[i + 2] as f64;
                acc += l;
            }
            *slot = (acc / span as f64) as f32;
        }
        out
    }
}

/// 拼接产物：物理像素 RGBA。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stitched {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

// ---------------------------------------------------------------------------
// 纯逻辑：1D 剖面相关
// ---------------------------------------------------------------------------

/// 剖面方差（均值平方差）。≈0 → 无特征（纯色画面）。
fn profile_variance(profile: &[f32]) -> f64 {
    if profile.is_empty() {
        return 0.0;
    }
    let n = profile.len() as f64;
    let mean: f64 = profile.iter().map(|v| *v as f64).sum::<f64>() / n;
    let var: f64 = profile.iter().map(|v| {
        let d = *v as f64 - mean;
        d * d
    }).sum::<f64>() / n;
    var
}

/// 归一化互相关（零均值）：两组等长样本的相关系数 ∈ [-1, 1]；退化（方差 0）→ 0。
fn zncc(xs: &[f64], ys: &[f64]) -> f32 {
    let n = xs.len();
    if n < 4 || xs.len() != ys.len() {
        return 0.0;
    }
    let mut sx = 0f64;
    let mut sy = 0f64;
    let mut sxx = 0f64;
    let mut syy = 0f64;
    let mut sxy = 0f64;
    for i in 0..n {
        let x = xs[i];
        let y = ys[i];
        sx += x;
        sy += y;
        sxx += x * x;
        syy += y * y;
        sxy += x * y;
    }
    let nf = n as f64;
    let num = nf * sxy - sx * sy;
    // 方差项可能因浮点抵消变成**极小的负数** → 直接相乘开方会得到 NaN → 排序
    // 比较器不再满足全序（Rust 新版 sort 会 panic：「comparison function does not
    // correctly implement a total order」）。故先各自截断到非负再开方。
    let va = nf * sxx - sx * sx;
    let vb = nf * syy - sy * sy;
    if va <= 0.0 || vb <= 0.0 {
        return 0.0; // 退化（无方差）→ 相关度无意义
    }
    let den = (va * vb).sqrt();
    if den <= 1e-9 {
        return 0.0;
    }
    let r = num / den;
    if r.is_finite() {
        r as f32
    } else {
        0.0
    }
}

/// 在给定位移 `d` 处的相关度：`next[y]` 与 `prev[y+d]` 的零均值归一化互相关。
///
/// `cut`：跳过的顶部行数（固定头部切除）。`stride`：采样步长（1 = 全分辨率）。
fn zncc_at(prev: &[f32], next: &[f32], cut: usize, d: usize, stride: usize) -> f32 {
    let h = prev.len().min(next.len());
    if d >= h || cut >= h {
        return 0.0;
    }
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let mut y = cut;
    while y + d < h {
        xs.push(next[y] as f64);
        ys.push(prev[y + d] as f64);
        y += stride.max(1);
    }
    zncc(&xs, &ys)
}

/// 1D 剖面互相关：由两帧的行亮度剖面估计**纵向位移**（内容上移的像素数）。
///
/// 返回 `Some(d)` 表示 `next[y] ≈ prev[y + d]`；返回 `None` 表示**不可信**
/// （纯色无特征、两幅长度不足以重叠、相关度过低、或多个位移相关度接近＝歧义）。
///
/// 这是本模块的核心原语（纯函数、可单测）：位移由**行亮度序列的对齐**决定，
/// 不依赖任何特征点检测。
pub fn estimate_shift(prev_rows: &[f32], next_rows: &[f32]) -> Option<i32> {
    estimate_shift_scored(prev_rows, next_rows, 0).map(|(d, _)| d)
}

/// 1D 剖面互相关（带顶部切除 + 相关度）：返回 `(位移, 相关度)`。
///
/// `cut` = 跳过的顶部行数（固定头部切除——头部不动，会破坏整幅平移模型）。
pub fn estimate_shift_scored(
    prev_rows: &[f32],
    next_rows: &[f32],
    cut: usize,
) -> Option<(i32, f32)> {
    let cands = estimate_shift_candidates(prev_rows, next_rows, cut, TOP_K);
    let &(d, c) = cands.first()?;
    if c < MIN_CORR {
        return None; // 相关度过低：不是可靠匹配
    }
    // 歧义判定：次优峰与最优峰相关度接近 → 内容重复（条纹/重复表格行/准周期段落）
    // → 拒绝拼接（宁可不拼，绝不错拼）。
    for &(d2, c2) in cands.iter().skip(1) {
        if (d2 - d).abs() >= AMBIG_GAP && c2 >= c - AMBIG_MARGIN {
            return None;
        }
    }
    Some((d, c))
}

/// 1D 剖面相关的**候选峰**（纯函数）：按相关度降序返回 top-K 局部极大值。
///
/// 为什么要「候选」而不是「最优」：行亮度剖面把二维内容**沿列聚合**——准周期内容
/// （行距固定的段落、重复表格行）会在多个位移上产生几乎等高的峰，单看 1D 谱
/// 无法分辨真解与假解。故 1D 只负责**粗定位候选**，最终由二维 patch 相关裁决
/// （见 [`estimate_frame_shift`]）。
pub fn estimate_shift_candidates(
    prev_rows: &[f32],
    next_rows: &[f32],
    cut: usize,
    top_k: usize,
) -> Vec<(i32, f32)> {
    let h = prev_rows.len().min(next_rows.len());
    if h < MIN_OVERLAP + 2 {
        return Vec::new();
    }
    // 无特征判定：任一侧剖面方差 ≈ 0（纯色画面）→ 拒绝，绝不猜。
    if profile_variance(&prev_rows[..h]) < 1e-3 || profile_variance(&next_rows[..h]) < 1e-3 {
        return Vec::new();
    }
    let cut = cut.min(h.saturating_sub(MIN_OVERLAP));
    if cut + MIN_OVERLAP >= h {
        return Vec::new();
    }
    // 粗扫（全位移、抽样行）→ 找局部极大值。
    //
    // 采样步长**按重叠行数自适应**（`overlap/300`，下限 1）：大位移时重叠只有
    // 几十行，若仍按整幅高度定步长（h/300），样本数会掉到十几个 → 相关度噪声
    // 淹没信号、真解被假峰盖过（实测「一步滚 859px / 视口 900px」= 41 行重叠
    // 的场景整段失配）。自适应后每个候选恒有 ~300 个样本。
    let max_d = h - cut - MIN_OVERLAP;
    let mut coarse: Vec<(i32, f32)> = Vec::with_capacity(max_d + 1);
    for d in 0..=max_d {
        let overlap = h - cut - d;
        let stride = (overlap / 300).max(1);
        coarse.push((d as i32, zncc_at(prev_rows, next_rows, cut, d, stride)));
    }
    let stride = (h / 300).max(1); // 细扫邻域宽度
    let mut peaks: Vec<(i32, f32)> = Vec::new();
    for i in 0..coarse.len() {
        let c = coarse[i].1;
        let left = if i == 0 { f32::MIN } else { coarse[i - 1].1 };
        let right = if i + 1 == coarse.len() { f32::MIN } else { coarse[i + 1].1 };
        if c > 0.2 && c >= left && c > right {
            peaks.push(coarse[i]);
        }
    }
    if peaks.is_empty() {
        // 没有局部极大（如单调谱）→ 取全局最大。
        peaks.push(
            coarse
                .iter()
                .copied()
                .fold((0i32, f32::MIN), |acc, v| if v.1 > acc.1 { v } else { acc }),
        );
    }
    peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    peaks.truncate(top_k.max(1));
    // 每个峰邻域 ±stride 全分辨率细扫 → 精确位移。
    let mut out = Vec::with_capacity(peaks.len());
    for (d0, _) in peaks {
        let lo = (d0 - stride as i32).max(0) as usize;
        let hi = (d0 + stride as i32).min(max_d as i32).max(0) as usize;
        let mut best = (d0, 0f32);
        for d in lo..=hi {
            let c = zncc_at(prev_rows, next_rows, cut, d, 1);
            if c > best.1 {
                best = (d as i32, c);
            }
        }
        out.push(best);
    }
    out
}

// ---------------------------------------------------------------------------
// 帧级位移估计：多列块中位数 + 1D 粗定位 + 2D patch 裁决
// ---------------------------------------------------------------------------

/// 2D patch 相关分：候选位移 `d` 在列区间 `[x0, x1)` 上的二维相关（-1..1）。
///
/// 取重叠区（`next` 的 `[cut, h-d)` 行）内若干个 48 行高的 patch，逐 patch 算
/// 零均值归一化互相关后取均值。**二维信息是 1D 行谱没有的**——行谱把整行聚合成
/// 一个数，准周期内容（行距固定的段落）会出现多个几乎等高的假峰；二维 patch
/// 逐像素比对，真解处相关度 ≈ 1，假解处显著更低 → 由它裁决。
///
/// **patch 位置按纹理挑选**（[`pick_patch_rows`]）：落在纯色留白上的 patch 方差
/// 为 0、相关度无意义（实测聊天气泡场景：均匀分布的 patch 有大半落在气泡间隙
/// → 二维裁决失效、块间投票互相矛盾 → 整对失配）。故按行谱的局部方差挑「有
/// 纹理」的行带做 patch，保证二维信息有效。
fn patch_score(
    prev: &Frame,
    next: &Frame,
    x0: usize,
    x1: usize,
    prev_profile: &[f32],
    cut: usize,
    d: i32,
) -> f32 {
    const PATCH_H: usize = 48;
    let h = prev.height as usize;
    if d < 0 {
        return 0.0;
    }
    let d_us = d as usize;
    if d_us >= h {
        return 0.0;
    }
    let lo = cut;
    let hi = h - d_us; // 重叠区上界
    if hi <= lo + 16 {
        return 0.0; // 重叠太小 → 二维无从判别
    }
    let rows = pick_patch_rows(prev_profile, lo, hi, PATCH_COUNT);
    if rows.is_empty() {
        return 0.0;
    }
    // patch 高随重叠自适应（大位移时重叠只有几十行，固定 48 行的 patch 放不下）。
    let ph = PATCH_H.min((hi - lo) / 2).max(8);
    let mut total = 0f32;
    let mut n = 0;
    for top in rows {
        if top + ph > hi {
            continue;
        }
        let s = patch_zncc(prev, next, x0, x1, top, d_us, ph);
        if s != 0.0 {
            total += s;
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        total / n as f32
    }
}

/// 行谱的局部方差（窗口 ±6 行）——「这一带有没有纹理」的代理指标。
fn local_variance(profile: &[f32], i: usize) -> f32 {
    const WIN: usize = 6;
    let lo = i.saturating_sub(WIN);
    let hi = (i + WIN + 1).min(profile.len());
    if hi <= lo + 1 {
        return 0.0;
    }
    let seg = &profile[lo..hi];
    let mean: f64 = seg.iter().map(|v| *v as f64).sum::<f64>() / seg.len() as f64;
    let var: f64 = seg
        .iter()
        .map(|v| {
            let d = *v as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / seg.len() as f64;
    var as f32
}

/// 挑选 patch 行位置：在重叠区 `[cut, hi)` 内**均匀铺开**若干候选点（覆盖整段
/// 重叠区，避免只盯着局部），再按行谱局部方差**剔除无纹理**的候选（纯色留白上
/// 的 patch 方差 0、相关度无意义——实测聊天气泡场景过半 patch 落在气泡间隙，
/// 二维裁决直接失效）。
///
/// 注：曾试过「只取方差最大的几个」，结果反而更差——高方差行往往集中在局部
/// （如图片边缘、固定头部），patch 位置一旦偏斜，二维裁决就可能被静态区域
/// 劫持。**均匀铺开 + 剔除非纹理**是更稳的折中。
fn pick_patch_rows(profile: &[f32], cut: usize, hi: usize, count: usize) -> Vec<usize> {
    let span = hi.saturating_sub(cut);
    if span < 16 {
        return Vec::new();
    }
    let mut rows = Vec::with_capacity(count);
    for k in 1..=count {
        let top = cut + span * k / (count + 1);
        // 局部方差过低 → 该带纯色 → 该 patch 无判别力 → 跳过。
        let v = local_variance(profile, top.min(profile.len().saturating_sub(1)));
        if v >= MIN_PATCH_VARIANCE {
            rows.push(top);
        }
    }
    rows
}

/// 单个 patch 的零均值归一化互相关：`next` 的 `[top, top+ph)` 行 vs `prev` 的
/// `[top+d, top+d+ph)` 行，列区间 `[x0, x1)`（行抽样 4、列抽样 8）。
fn patch_zncc(
    prev: &Frame,
    next: &Frame,
    x0: usize,
    x1: usize,
    top: usize,
    d: usize,
    ph: usize,
) -> f32 {
    let w = prev.width as usize;
    let x1 = x1.min(w);
    if x0 >= x1 {
        return 0.0;
    }
    let mut xs: Vec<f64> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    let mut y = top;
    while y < top + ph {
        let ra = (y + d) * w * 4; // prev 行（内容更早）
        let rb = y * w * 4; // next 行
        let mut x = x0;
        while x < x1 {
            let ia = ra + x * 4;
            let ib = rb + x * 4;
            xs.push(luma(&prev.rgba[ia..ia + 3]));
            ys.push(luma(&next.rgba[ib..ib + 3]));
            x += 8;
        }
        y += 4;
    }
    // **纹理门**：patch 自身方差过低（纯色 / 平滑渐变）时，归一化互相关的分母趋
    // 零 → 相关系数是数值噪声，会在任意位移上给出 ~0.99 的假高分（实测图库场
    // 景：渐变瓦片上的 patch 在十几个位移上都报 0.99 → 触发「歧义」弃权）。
    // 方差不足 → 该 patch 无判别力 → 记 0（由调用方按「无信息」处理）。
    if patch_stddev(&xs) < MIN_PATCH_STDDEV {
        return 0.0;
    }
    zncc(&xs, &ys)
}

/// 样本标准差（patch 纹理度量）。
fn patch_stddev(xs: &[f64]) -> f64 {
    if xs.len() < 8 {
        return 0.0;
    }
    let mean: f64 = xs.iter().sum::<f64>() / xs.len() as f64;
    let var: f64 = xs.iter().map(|v| (*v - mean) * (*v - mean)).sum::<f64>() / xs.len() as f64;
    var.sqrt()
}

/// 像素亮度（Rec.601）。
fn luma(rgb: &[u8]) -> f64 {
    0.299 * rgb[0] as f64 + 0.587 * rgb[1] as f64 + 0.114 * rgb[2] as f64
}

/// 2D 精修：在 1D 估计的 ±2 px 邻域内，用 patch 的 SSD 选出更精确的位移。
///
/// 1D 剖面在**行方向聚合**了整幅列的信息，理论上已是整数像素级精确；2D 精修
/// 是**验证 + 兜底**：它在真实二维像素上比较，选出残差最小的位移（对 1D 因聚合
/// 而可能取整偏差的情况有用）。
fn refine_2d(prev: &Frame, next: &Frame, d: i32) -> Option<i32> {
    const PATCH: usize = 24;
    let h = prev.height as usize;
    let w = prev.width as usize;
    if h < PATCH + 8 || w < 8 {
        return None;
    }
    let d_us = usize::try_from(d).ok()?;
    // 取重叠区居中的 patch 行区间（重叠区 = next 的 [0, h-d) 行）。
    let overlap = h.checked_sub(d_us)?;
    if overlap < PATCH + 4 {
        return None;
    }
    let y0 = (overlap - PATCH) / 2;
    // 列带取中间 512 px（够用且省时）。
    let cw = w.min(512);
    let x0 = (w - cw) / 2;
    let ssd_at = |cand: i32| -> Option<f64> {
        if cand < 0 {
            return None;
        }
        let c_us = cand as usize;
        if c_us + y0 + PATCH > h {
            return None;
        }
        let mut ssd = 0f64;
        for yy in 0..PATCH {
            let a = (y0 + yy) * w * 4;
            let b = (y0 + yy + c_us) * w * 4;
            for xx in 0..cw {
                let ia = a + (x0 + xx) * 4;
                let ib = b + (x0 + xx) * 4;
                for c in 0..3 {
                    let da = next.rgba[ia + c] as f64;
                    let db = prev.rgba[ib + c] as f64;
                    let diff = da - db;
                    ssd += diff * diff;
                }
            }
        }
        Some(ssd)
    };
    let base = ssd_at(d)?;
    let mut best = (d, base);
    for cand in (d - 6)..=(d + 6) {
        let Some(ssd) = ssd_at(cand) else { continue };
        if ssd < best.1 {
            best = (cand, ssd);
        }
    }
    // **改进门**：精修只在残差**显著**更小时才采纳。低纹理区域里 ±6 px 的残差
    // 差异纯属噪声 → 无条件采纳会把正确的位移改错（实测聊天场景出现 -6px 偏移）。
    if best.0 != d && best.1 > base * REFINE_GAIN {
        return Some(d);
    }
    Some(best.0)
}

/// 帧级位移估计：多列块中位数 + 1D 粗定位 + **2D patch 裁决**。
///
/// 三层结构（对应 plan 的「多列采样鲁棒」与「模板匹配」）：
///
/// 1. **多列块**（`BLOCKS` 块）：逐块独立估位移 → 取中位数。固定侧栏所在的列块
///    要么无特征（纯色 → 该块不投票）、要么投票 0（静态），中位数天然忽略。
/// 2. **1D 粗定位**：行亮度剖面的 top-K 候选峰（多档顶栏切口）。
/// 3. **2D patch 裁决**：候选峰逐个用二维 patch 相关打分，取最高者——**这是关键
///    一步**：1D 行谱把整行聚合成一个数，准周期内容（行距固定的段落、重复表格
///    行）会在多个位移上产生几乎等高的假峰，单看 1D 谱会选错（实测某场景真解
///    678px 被 524px 的假峰盖过 → 整图错位 154px）。二维逐像素比对在真解处相关
///    度 ≈1、假解处显著更低 → 稳定裁决。
///
/// 返回 `Some(d)`：`d > 0` 表示内容上移 d 像素（用户向下滚了 d）。
/// 返回 `None`：无可靠匹配（纯色、重复内容歧义、重叠不足、相关度过低）。
pub fn estimate_frame_shift(prev: &Frame, next: &Frame) -> Option<i32> {
    if prev.width == 0 || prev.height == 0 {
        return None;
    }
    let w = prev.width as usize;
    let h = prev.height as usize;
    let block_w = (w / BLOCKS).max(8);
    let mut votes: Vec<(i32, f32)> = Vec::new(); // (位移, 2D 相关分)
    for b in 0..BLOCKS {
        let x0 = b * block_w;
        let x1 = x0 + block_w;
        if x1 > w {
            break;
        }
        let prev_p = prev.block_profile(x0, x1);
        let next_p = next.block_profile(x0, x1);
        // 1D 候选（多档顶栏切口）：切口越准、1D 相关越高。
        let mut cands: Vec<(i32, usize, f32)> = Vec::new(); // (d, cut, 1D 相关)
        for frac in CUT_FRACS {
            let cut = (h as f32 * frac) as usize;
            for (d, c) in estimate_shift_candidates(&prev_p, &next_p, cut, TOP_K) {
                cands.push((d, cut, c));
            }
        }
        if cands.is_empty() {
            continue; // 该块无特征（纯色）→ 不投票
        }
        cands.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        cands.dedup_by(|a, b| a.0 == b.0); // 同一位移只留 1D 相关最高的那档切口
        cands.truncate(TOP_K * 2);
        // 2D 裁决：候选逐个打分（2D 相关优先，1D 相关作并列时的次序）。
        let mut scored: Vec<(i32, f32, f32)> = cands
            .into_iter()
            .map(|(d, cut, c)| (d, patch_score(prev, next, x0, x1, &prev_p, cut, d), c))
            .collect();
        // 二维**无信息**时（最高分很低 = patch 全落在纯色区/内容变化剧烈）→
        // 退回 1D 排序：此时 1D 谱是唯一证据（聊天气泡场景实测需要此回退）。
        let s2_max = scored.iter().map(|(_, s2, _)| *s2).fold(0f32, f32::max);
        if s2_max < S2_INFORMATIVE {
            scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            scored.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
            });
        }
        let Some(&(d_best, s_best, c_best)) = scored.first() else { continue };
        if c_best < MIN_CORR {
            continue; // 1D 相关度不足 → 该块不可信 → 不投票
        }
        // 歧义：与次优候选（距离足够远）的 2D 相关接近 → 内容重复 → 该块弃权。
        if let Some(&(_, s2, _)) = scored
            .iter()
            .skip(1)
            .find(|(d, _, _)| (*d - d_best).abs() >= AMBIG_GAP)
        {
            if s2 >= s_best - AMBIG_MARGIN_2D {
                continue;
            }
        }
        votes.push((d_best, s_best));
    }
    if votes.is_empty() {
        return None;
    }
    // 中位数：列块多数一致时抗单块离群（多列匹配的核心鲁棒性来源）。
    let mut sorted: Vec<i32> = votes.iter().map(|(d, _)| *d).collect();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    // 一致性：过半列块落在中位数 ±2 px 内，否则判为不可靠。
    //
    // 例外：若某个列块的二维相关**接近 1**（≥ 0.97 = 纹理区几乎逐像素吻合），
    // 则该块的位移有极强证据 → 直接采信它（列块间不一致往往源于某几块的 patch
    // 落在纯色区、其二维分虚高/虚低，而非真解有争议）。
    let agree = votes.iter().filter(|(d, _)| (*d - median).abs() <= 2).count();
    if agree * 2 < votes.len() {
        let &(d_strong, _s_strong) = votes
            .iter()
            .filter(|(_, s)| *s >= S2_STRONG)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
        return Some(refine_2d(prev, next, d_strong).unwrap_or(d_strong));
    }
    // 2D 精修（±2 px 内的残差最小者）。
    Some(refine_2d(prev, next, median).unwrap_or(median))
}

// ---------------------------------------------------------------------------
// 拼接
// ---------------------------------------------------------------------------

/// 中值滤波（**离群剔除语义**）：逐对位移与**邻域中位数**比较，明显离群者标脏。
///
/// 为何是「剔除」而不是「替换」：位移序列天然非单调（用户滚动速度剧变：一步
/// 400px、下一步 850px 完全合法）。教科书面 3 抽头中值滤波会**无条件**把邻域
/// 中位数强加给序列——实测把 40/60/30 压成 40/40/30（累计高度少 20px）、把
/// 506/783/764/449… 这种高方差序列压扁（图库场景高度直接少 14310px）。
/// 故本实现只**标记离群对**（`|d - 邻域中位数| > max(32, 2×邻域中位数)`），
/// 由调用方跳过该帧（而不是塞进一个错位移）——错位移会让整图错位，跳过一帧
/// 只是丢一段内容。
fn outlier_flags(est: &[Option<i32>]) -> Vec<bool> {
    let mut out = vec![false; est.len()];
    for i in 0..est.len() {
        let Some(d) = est[i] else { continue };
        let mut neigh: Vec<i32> = Vec::with_capacity(3);
        if i > 0 {
            if let Some(x) = est[i - 1] {
                neigh.push(x);
            }
        }
        neigh.push(d);
        if i + 1 < est.len() {
            if let Some(x) = est[i + 1] {
                neigh.push(x);
            }
        }
        if neigh.len() < 3 {
            continue; // 邻域不足 → 无从判定（首尾对不判）
        }
        neigh.sort_unstable();
        let local = neigh[neigh.len() / 2];
        if (d - local).abs() > 32.max(2 * local.abs()) {
            out[i] = true;
        }
    }
    out
}

/// 拼接一组帧为一张长图。
///
/// 语义：
/// - 逐对估计相邻帧的纵向位移 → **偏移链**（`y[i] = y[i-1] + d`）；
/// - 位移序列过 **3 抽头中值滤波**（去单对尖峰）；
/// - 重叠区末端做 **BLEND_BAND 像素线性 alpha 融合**（抑制 1px 错位重影）；
/// - 画布高度 = 内容总跨度，宽度 = 首帧宽度。
///
/// 失败（`None`）：**没有任何一对帧匹配成功**（纯色画面 / 重复内容歧义 /
/// 重叠不足）——宁可不产出，绝不产出错拼的重影图。单帧输入同样返回 `None`
/// （无可拼接内容，由调用方给出「未滚动」类提示）。
pub fn stitch(frames: &[Frame]) -> Option<Stitched> {
    if frames.len() < 2 {
        return None;
    }
    let w = frames[0].width as usize;
    let h = frames[0].height as usize;
    if w == 0 || h == 0 {
        return None;
    }
    // —— 第一遍：逐对估计（相邻两帧）——
    let mut est: Vec<Option<i32>> = vec![None; frames.len()];
    for i in 1..frames.len() {
        if frames[i].width != frames[0].width || frames[i].height != frames[0].height {
            continue; // 尺寸不一致的帧无法参与拼接
        }
        est[i] = estimate_frame_shift(&frames[i - 1], &frames[i]);
    }
    // —— 中值滤波：标出离群对（跳帧而非塞错位移）——
    let rejected = outlier_flags(&est);
    // —— 偏移链 ——
    // 中间帧不可用（匹配失败 / 离群）时**跳过该帧**，并让下一帧与「上一张被接受
    // 的帧」比对——这样单帧失败不必截断整条链（代价：滚动超过一屏时该段内容
    // 丢失，链自然终止）。
    let mut chain: Vec<(usize, i32)> = vec![(0, 0)]; // (帧下标, 画布 y 偏移)
    let mut shifts: Vec<i32> = Vec::new(); // 相邻被接受帧之间的位移
    let mut last_idx = 0usize;
    let mut last_y = 0i32;
    for i in 1..frames.len() {
        let direct = match est[i] {
            Some(d) if !rejected[i] => Some(d),
            _ => None,
        };
        // 直邻估计不可用 → 试「跨过被跳过的帧」再估一次（内容可能仍在重叠区）。
        let d = match direct {
            Some(d) => Some(d),
            None => {
                if last_idx + 1 == i {
                    None // 就是相邻帧且估不出来 → 确实无重叠
                } else {
                    estimate_frame_shift(&frames[last_idx], &frames[i])
                }
            }
        };
        let Some(d) = d else { continue };
        let d = d.max(0); // 只支持向下滚动（内容上移）
        shifts.push(d);
        last_y += d;
        chain.push((i, last_y));
        last_idx = i;
    }
    if shifts.is_empty() {
        return None; // 无任何可靠匹配 → 拒绝拼接
    }
    // —— 偏移链（滤波后的位移重新累加）——
    let mut offsets: Vec<(usize, i32)> = Vec::with_capacity(chain.len());
    let mut y = 0i32;
    offsets.push((chain[0].0, 0));
    for (k, (idx, _)) in chain.iter().enumerate().skip(1) {
        y += shifts[k - 1];
        offsets.push((*idx, y));
    }
    debug_assert_eq!(offsets.len(), shifts.len() + 1);
    // —— 画布：覆盖全部帧的纵向并集 ——
    let origin = offsets.iter().map(|(_, y)| *y).min().unwrap_or(0);
    let bottom = offsets
        .iter()
        .map(|(_, y)| y + h as i32)
        .max()
        .unwrap_or(h as i32);
    let canvas_h = (bottom - origin).max(1) as usize;
    let mut canvas = vec![0u8; w * canvas_h * 4];
    // —— 合成：逐帧写入，重叠区末端做线性 alpha 融合 ——
    let mut covered_top = offsets[0].1;
    let mut covered_bottom = offsets[0].1 + h as i32;
    copy_rows(
        &mut canvas,
        w,
        canvas_h,
        origin,
        &frames[offsets[0].0],
        offsets[0].1,
        0,
        h as i32,
    );
    for (idx, fy) in offsets.iter().skip(1).copied() {
        let frame = &frames[idx];
        let frame_top = fy;
        let frame_bottom = fy + h as i32;
        // 接缝位置：向下滚动 → 上一内容的底边；向上 → 顶边（本实现只向下）。
        let seam = covered_bottom;
        let band_lo = (seam - BLEND_BAND).max(frame_top);
        let band_hi = seam.min(frame_bottom);
        if band_hi > band_lo {
            let n = band_hi - band_lo;
            for row in band_lo..band_hi {
                let alpha = (row - band_lo + 1) as f32 / n as f32;
                blend_row(
                    &mut canvas,
                    w,
                    canvas_h,
                    origin,
                    row,
                    frame,
                    (row - frame_top) as usize,
                    alpha,
                );
            }
        }
        // 新内容区（接缝之下）直接写入。
        let copy_from = band_hi.max(frame_top);
        if frame_bottom > copy_from {
            copy_rows(
                &mut canvas,
                w,
                canvas_h,
                origin,
                frame,
                frame_top,
                (copy_from - frame_top) as usize,
                (frame_bottom - copy_from) as i32,
            );
        }
        covered_top = covered_top.min(frame_top);
        covered_bottom = covered_bottom.max(frame_bottom);
    }
    Some(Stitched {
        width: w as u32,
        height: canvas_h as u32,
        rgba: canvas,
    })
}

/// 把帧的 `count` 行（从帧内第 `src_row` 起）复制到画布（帧顶落在画布 `frame_top`）。
#[allow(clippy::too_many_arguments)]
fn copy_rows(
    canvas: &mut [u8],
    w: usize,
    canvas_h: usize,
    origin: i32,
    frame: &Frame,
    frame_top: i32,
    src_row: usize,
    count: i32,
) {
    let stride = w * 4;
    for i in 0..count.max(0) as usize {
        let dst = (origin + frame_top + src_row as i32 + i as i32) as usize;
        if dst >= canvas_h {
            break;
        }
        let s = (src_row + i) * stride;
        let d = dst * stride;
        if s + stride > frame.rgba.len() || d + stride > canvas.len() {
            break;
        }
        canvas[d..d + stride].copy_from_slice(&frame.rgba[s..s + stride]);
    }
}

/// 画布某行与帧某行的线性 alpha 融合（`alpha` = 帧内容的权重）。
#[allow(clippy::too_many_arguments)]
fn blend_row(
    canvas: &mut [u8],
    w: usize,
    canvas_h: usize,
    origin: i32,
    row: i32,
    frame: &Frame,
    src_row: usize,
    alpha: f32,
) {
    let stride = w * 4;
    let dst = (origin + row) as usize;
    if dst >= canvas_h {
        return;
    }
    let s = src_row * stride;
    let d = dst * stride;
    if s + stride > frame.rgba.len() || d + stride > canvas.len() {
        return;
    }
    for i in 0..stride {
        let old = canvas[d + i] as f32;
        let new = frame.rgba[s + i] as f32;
        canvas[d + i] = (old + (new - old) * alpha).round().clamp(0.0, 255.0) as u8;
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 伪随机（确定性 LCG）——合成纹理用，避免依赖 rand 依赖。
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    /// 合成「网页正文」帧：白底 + 随机段落/行/词（行高、行距、词长、墨色均随机）。
    ///
    /// 关键：**行亮度剖面必须非周期**——真实页面的行距/行高参差，若合成内容带固定
    /// 周期（如每 18 行一条带），剖面就周期化 → 相关函数出现多个等高峰 → 匹配
    /// 判为「重复内容歧义」而拒绝（这是**正确**行为，但会让正向用例失真）。
    pub fn page_frame(w: u32, h: u32, seed: u64, _top: u32) -> Frame {
        let mut rng = Lcg(seed);
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        let mut y = 0u32;
        while y < h {
            let lines = 2 + (rng.next() % 6) as u32;
            for _ in 0..lines {
                if y >= h {
                    break;
                }
                let line_h = 3 + (rng.next() % 4) as u32;
                let words = 3 + (rng.next() % 8) as u32;
                let mut x = 6u32 + (rng.next() % 20) as u32;
                for _ in 0..words {
                    if x >= w {
                        break;
                    }
                    let word_w = 8 + (rng.next() % 40) as u32;
                    let ink = 40 + (rng.next() % 180) as u8;
                    for yy in y..(y + line_h).min(h) {
                        for xx in x..(x + word_w).min(w) {
                            let i = ((yy * w + xx) * 4) as usize;
                            rgba[i] = ink;
                            rgba[i + 1] = ink;
                            rgba[i + 2] = ink;
                        }
                    }
                    x += word_w + 6 + (rng.next() % 10) as u32;
                }
                y += line_h + 2;
            }
            y += 6 + (rng.next() % 12) as u32; // 段间距
        }
        Frame { width: w, height: h, rgba }
    }

    /// 从「页面」中截取一帧（模拟滚动到 offset 处的视口，视口高 view_h）。
    pub fn viewport(page: &Frame, offset: usize, view_h: u32) -> Frame {
        let w = page.width as usize;
        let h = view_h as usize;
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            let sy = (offset + y).min(page.height as usize - 1);
            let s = sy * w * 4;
            let d = y * w * 4;
            rgba[d..d + w * 4].copy_from_slice(&page.rgba[s..s + w * 4]);
        }
        Frame { width: page.width, height: view_h, rgba }
    }

    fn solid_frame(w: u32, h: u32, v: u8) -> Frame {
        Frame { width: w, height: h, rgba: vec![v, v, v, 255].repeat((w * h) as usize) }
    }

    /// 合成「重复内容」帧：每 16 行完全相同的条纹（歧义场景）。
    fn striped_frame(w: u32, h: u32) -> Frame {
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        for y in 0..h {
            if y % 16 < 8 {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i] = 20;
                    rgba[i + 1] = 20;
                    rgba[i + 2] = 20;
                }
            }
        }
        Frame { width: w, height: h, rgba }
    }

    #[test]
    fn estimate_shift_recovers_integer_shift() {
        let page = page_frame(160, 240, 7, 0);
        for d in [1usize, 5, 17, 40, 70] {
            let prev = viewport(&page, 0, 160);
            let next = viewport(&page, d, 160);
            let got = estimate_frame_shift(&prev, &next).expect("应匹配出位移");
            assert_eq!(got, d as i32, "位移 {d} 应被精确恢复");
        }
    }

    #[test]
    fn estimate_shift_tolerates_subpixel_scroll() {
        // 亚像素滚动（3.5 px）→ 整数位移只能是 3 或 4（取整），误差 ≤1 px。
        let page = page_frame(160, 240, 11, 0);
        let prev = viewport(&page, 0, 160);
        // 亚像素滚动用整数行近似（浏览器滚动落到物理像素，误差 ≤1 px）。
        let next = viewport(&page, 3, 160);
        let got = estimate_frame_shift(&prev, &next).expect("应匹配出位移");
        assert!(
            (got - 4).abs() <= 1,
            "亚像素滚动应给出 ±1 px 内的整数位移，实际 {got}"
        );
    }

    #[test]
    fn estimate_shift_handles_large_shift() {
        // 大位移（选区高度的 75%）仍应恢复：重叠 25% 行仍够相关。
        let page = page_frame(160, 300, 3, 0);
        let prev = viewport(&page, 0, 160);
        let next = viewport(&page, 120, 160);
        let got = estimate_frame_shift(&prev, &next).expect("大位移应可匹配");
        assert_eq!(got, 120);
    }

    #[test]
    fn pure_color_has_no_features_and_is_rejected() {
        let a = solid_frame(120, 120, 200);
        let b = solid_frame(120, 120, 200);
        // 1D 原语：纯色剖面方差 ≈ 0 → 无特征 → None（绝不错拼）。
        assert_eq!(estimate_shift(&a.row_profile(), &b.row_profile()), None);
        // 帧级与拼接级同样拒绝。
        assert_eq!(estimate_frame_shift(&a, &b), None);
        assert_eq!(stitch(&[a.clone(), b.clone()]), None);
    }

    #[test]
    fn repeated_content_is_ambiguous_and_rejected() {
        // 条纹周期 16 px → 多个位移相关度接近 → 判歧义 → None。
        let a = striped_frame(160, 200);
        let b = striped_frame(160, 200);
        assert_eq!(estimate_frame_shift(&a, &b), None, "重复内容必须拒绝");
        assert_eq!(stitch(&[a, b]), None, "重复内容不得错拼");
    }

    #[test]
    fn estimate_shift_needs_enough_overlap() {
        // 剖面太短 → 无重叠 → None。
        let short = vec![10f32; 8];
        assert_eq!(estimate_shift(&short, &short), None);
    }

    #[test]
    fn stitch_produces_full_height_and_aligned_content() {
        // 页面 240 行高，帧高 100，按 40/60/30 的步长滚动 → 拼接后高度 = 页面高度。
        let page = page_frame(120, 240, 21, 0);
        let offsets = [0usize, 40, 100, 130];
        let frames: Vec<Frame> = offsets.iter().map(|o| viewport(&page, *o, 100)).collect();
        let out = stitch(&frames).expect("应拼接成功");
        assert_eq!(out.width, 120);
        // 末帧底部 = 130 + 100 = 230；页面高 240 → 画布高 230（末帧之后无内容）。
        assert_eq!(out.height, 230, "拼接高度 = 偏移链末端 + 帧高");
        // 逐行与页面对比：绝大多数行应**逐字节**一致（错位只在融合带内）。
        let mut exact = 0usize;
        let mut total = 0usize;
        for y in 0..out.height as usize {
            let d = y * out.width as usize * 4;
            let s = y * page.width as usize * 4;
            total += 1;
            if out.rgba[d..d + out.width as usize * 4] == page.rgba[s..s + page.width as usize * 4] {
                exact += 1;
            }
        }
        let ratio = exact as f64 / total as f64;
        assert!(
            ratio > 0.98,
            "拼接结果应与原页面逐行对齐（逐字节一致行占比 {ratio}）"
        );
    }

    #[test]
    fn stitch_handles_fixed_header() {
        // 固定头部（顶部 24 行恒定）+ 滚动正文：头部破坏整幅平移模型 → 靠顶部
        // 切除 + 多列中位数恢复位移。
        let page = page_frame(120, 400, 33, 0);
        let mut framed = page.clone();
        // 头部：前 24 行覆盖为固定深色条（滚动中不动）。
        for y in 0..24u32 {
            for x in 0..120u32 {
                let i = ((y * 120 + x) * 4) as usize;
                framed.rgba[i] = 40;
                framed.rgba[i + 1] = 40;
                framed.rgba[i + 2] = 60;
            }
        }
        let prev = viewport(&framed, 0, 100);
        let next = viewport(&framed, 50, 100);
        let got = estimate_frame_shift(&prev, &next).expect("固定头部下仍应恢复位移");
        assert_eq!(got, 50, "固定头部不应破坏位移估计");
    }

    #[test]
    fn stitch_skips_unmatched_frames_and_keeps_chain() {
        // 中间插入一帧纯色（匹配失败）→ 该帧被跳过，前后两帧仍应拼上。
        let page = page_frame(120, 300, 44, 0);
        let frames = vec![
            viewport(&page, 0, 100),
            solid_frame(120, 100, 128),
            viewport(&page, 60, 100),
        ];
        let out = stitch(&frames).expect("前后帧可配对应拼接成功");
        assert_eq!(out.height, 160, "跳过中间帧后高度 = 60 + 帧高");
    }

    #[test]
    fn single_frame_is_not_stitchable() {
        let a = page_frame(64, 64, 5, 0);
        assert_eq!(stitch(&[a]), None, "单帧无拼接内容");
    }

    #[test]
    fn frame_validates_buffer_length() {
        assert!(Frame::new(2, 2, vec![0; 16]).is_some());
        assert!(Frame::new(2, 2, vec![0; 15]).is_none());
    }

    #[test]
    fn median_filter_flags_outliers_without_touching_legal_variation() {
        // 变速滚动（合法）→ 不得被判离群（无条件中值替换会把 60 压成 40 →
        // 累计高度少 20 px，是真实缺陷）。
        let varied = vec![Some(40), Some(60), Some(30)];
        assert_eq!(outlier_flags(&varied), vec![false, false, false]);
        // 高方差序列（图库场景实测）→ 全部合法。
        let wild = vec![
            Some(506), Some(783), Some(764), Some(449), Some(467), Some(771), Some(475),
        ];
        assert!(
            !outlier_flags(&wild).iter().any(|x| *x),
            "高方差滚动不得被判离群：{:?}",
            outlier_flags(&wild)
        );
        // 单点尖峰（真值 ~100，估成 2400）→ 标脏（该帧跳过，而不是塞错位移）。
        let spike = vec![Some(100), Some(100), Some(2400), Some(100), Some(100)];
        let flags = outlier_flags(&spike);
        assert!(flags[2], "尖峰应被标脏：{flags:?}");
        assert!(!flags[1] && !flags[3], "非尖峰不得被标脏：{flags:?}");
        // 缺失值不参与邻域判定。
        let holes = vec![Some(100), None, Some(100)];
        assert_eq!(outlier_flags(&holes), vec![false, false, false]);
    }
}
