//! 长截图的**纯策略**：采集节奏（间隔自适应）+ 会话收尾顺序。
//!
//! ## 为什么放 core（无平台依赖）
//!
//! 这两件事是**纯逻辑**（不碰 Tauri / macOS API）→ 放 core 就能**在 Linux 上
//! 单测**（app crate 在 Linux 不可编译 → 放那儿就永远只有「走查」）。
//!
//! 更重要的是**单一事实源**：产品侧（`app/src-tauri/src/scrollshot.rs`）与
//! eval 跑批器（`examples/scrollshot_eval.rs`）**共用同一份实现** → 规则改了
//! 两边一起变，不会出现「产品改了、eval 还按老模型跑」的盲区（这正是上一轮
//! 评审指出的问题：eval 从真值推间隔 → 产品侧的 I1 改动完全没被覆盖）。

use std::time::{Duration, Instant};

/// 采集节奏参数（产品侧常量与 eval 场景参数喂给同一套规则）。
#[derive(Debug, Clone, Copy)]
pub struct Pacing {
    /// 基准间隔（滚得慢时用）。
    pub base: Duration,
    /// 加密间隔（滚得快 / **测不准**时用）。
    pub fast: Duration,
    /// 「滚得快」判定阈值（位移 / 选区高度）。
    pub fast_ratio: f64,
    /// 「滚得慢」判定阈值（位移 / 选区高度）。
    pub slow_ratio: f64,
}

impl Pacing {
    /// 构造节奏参数。
    pub fn new(base: Duration, fast: Duration, fast_ratio: f64, slow_ratio: f64) -> Self {
        Self { base, fast, fast_ratio, slow_ratio }
    }

    /// 下一次采集的间隔（**滞回式自适应**）。
    ///
    /// 动因：选区高度 h、采集间隔 T ⇒ 用户滚动 v px/s 时，每帧位移 ≈ v·T。两帧的
    /// 重叠 = h − v·T；位移逼近 h 时重叠趋零 → **模板匹配无重叠可配**。
    ///
    /// 规则（带滞回，避免在两档之间抖动）：
    /// - 上次位移 > `fast_ratio`·h → 加密到 `fast`（滚得快 → 多拍几帧）；
    /// - 上次位移 < `slow_ratio`·h → 回到 `base`（滚得慢 → 省资源）；
    /// - 介于两者之间 → 保持当前间隔；
    /// - **测不出位移（None）→ 加密到 `fast`**。
    ///
    /// ## 为什么「测不出」要加密而不是保持
    ///
    /// 测不出有三种成因：位移太大（**无重叠**）、内容歧义、内容无特征。三者里
    /// 只有第一种是致命的——它会让后续帧**继续失配**（重叠永远追不回来）。
    /// 代价不对称：拍密一点只是多几帧（几百 ms 的 CPU）；拍疏了会丢掉重叠 →
    /// 内容断档。故**不确定就拍密**。
    pub fn next_interval(&self, current: Duration, shift: Option<i32>, region_h: u32) -> Duration {
        let Some(s) = shift else {
            return self.fast; // 不确定 → 拍密（保住重叠）
        };
        if region_h == 0 {
            return current;
        }
        let ratio = s as f64 / region_h as f64;
        if ratio > self.fast_ratio {
            self.fast
        } else if ratio < self.slow_ratio {
            self.base
        } else {
            current
        }
    }
}

/// 位移取值：**测量值优先**，静止守卫只在**测不出**时兜底。
///
/// 旧写法 `if still { Some(0) } else { measured }` 让静止守卫**覆盖真实测量**：
/// 准周期页面滚过**恰好一个周期**时行谱与上一帧逐字节相同 → 被判「静止」→
/// 位移强行归零 → 节奏退到基准间隔 → 重叠被吃掉 → 配对失配（丢帧）。
///
/// 现在：
/// - 测得出 → 用**测量值**（哪怕内容恰好重复了一个周期）；
/// - 测不出 + 内容确实没动 → `Some(0)`（静止页面不该被当成「不确定」而加密采集）；
/// - 测不出 + 内容在动 → `None`（不确定 → 拍密，见 [`Pacing::next_interval`]）。
pub fn resolve_shift(measured: Option<i32>, still: bool) -> Option<i32> {
    match measured {
        Some(d) => Some(d),
        None => {
            if still {
                Some(0)
            } else {
                None
            }
        }
    }
}

/// 「完成」的**结果** → 收尾处置（**顺序契约**）。
///
/// 旧实现的顺序是「先注销热键 + 关提示条，再拼接」→ 失败反馈**无处可显**：
/// 提示条正在销毁 → 前端 `setError` 渲染进虚空；会话已被消费；托盘兜底答
/// 「没有进行中的长截图」→ **主出口彻底死路，用户零反馈**。
///
/// 现在：**先拼接、成功后才收尾**。失败 → 会话与提示条都留着（错误可见、
/// 可重试/放弃）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// 拼接失败（无可靠匹配）→ **保留**会话与提示条。
    StitchFailed,
    /// 落盘失败（画布已取出 → 不可重试）→ 消费会话 + 横幅告知。
    SaveFailed,
    /// 成功 → 消费会话（关条 + 注销热键 + 清槽位）。
    Saved,
}

impl StopOutcome {
    /// 是否**消费会话**（清槽位 + 停采集 + 注销热键 + 关提示条）。
    pub fn consumes_session(self) -> bool {
        matches!(self, StopOutcome::SaveFailed | StopOutcome::Saved)
    }

    /// 是否**关闭提示条**。拼接失败时必须为 `false`——错误要有地方显示。
    pub fn closes_bar(self) -> bool {
        self.consumes_session()
    }
}

/// **停滞检测**（画布不再生长 → 匹配链已断 → 长图被静默截断）。
///
/// ## 判据（评审 Minor：动效页误报）
///
/// 三个条件**同时**成立才算停滞：
/// 1. 画布**没长**（新帧没被接受）；
/// 2. 页面**确实在动**（行谱变了 → 用户在滚）；
/// 3. 内容**测不出**（匹配器给不出位移 —— 无特征 / 歧义 / 无重叠）。
///
/// 条件 3 是「内容**真的不可测量**」的判据：动效页（GIF 横幅、加载动画）上
/// 主体内容仍可对齐（测得出位移）→ 不算停滞。
///
/// ## 为什么用**墙钟窗口**而不是帧数
///
/// 采集间隔是自适应的（200ms 加密 / 500ms 基准）→ 「连续 N 帧」对应的**真实
/// 时长**随节奏漂移（10 帧 = 2s ~ 5s）→ 慢速滚动（每帧几 px、匹配器跟不上）
/// 会被误判。墙钟窗口（≥3s）与节奏**无关** → 判据稳定。
#[derive(Debug, Clone)]
pub struct StallDetector {
    /// 判定停滞所需的**连续不可测量时长**（墙钟）。
    window: Duration,
    /// 连续不可测量的**起点**（None = 当前不处于停滞窗口内）。
    since: Option<Instant>,
}

impl StallDetector {
    /// 构造检测器（窗口 = 连续多久「不可测量」才算停滞）。
    pub fn new(window: Duration) -> Self {
        Self { window, since: None }
    }

    /// 观察一次采集结果 → `true` = 判定停滞（调用方应停止采集并告知用户）。
    ///
    /// - `grew`：画布高度是否增长（新帧被接受）；
    /// - `still`：内容是否**完全没动**（行谱逐字节相同 → 用户没滚）；
    /// - `measured`：匹配器测出的位移（`None` = 测不出 = 内容不可测量）。
    pub fn observe(
        &mut self,
        grew: bool,
        still: bool,
        measured: Option<i32>,
        now: Instant,
    ) -> bool {
        // 画布长了 / 页面没动 / 内容仍可测量 → **不处于停滞状态** → 重置窗口。
        if grew || still || measured.is_some() {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        now.duration_since(since) >= self.window
    }

    /// 当前是否处于「持续不可测量」的窗口内（诊断用）。
    pub fn pending(&self) -> bool {
        self.since.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自适应采集间隔（滞回规则，纯函数）：
    /// - 位移 > 0.4h → 加密到 200ms（滚得快）；
    /// - 位移 < 0.2h → 回到 500ms（滚得慢）；
    /// - 中间区间 → 保持当前间隔（不抖动）。
    #[test]
    fn next_interval_adapts_with_hysteresis() {
        let p = Pacing::new(
            Duration::from_millis(500),
            Duration::from_millis(200),
            0.4,
            0.2,
        );
        let h = 900u32;
        // 800/900 = 0.89 > 0.4 → 加密
        assert_eq!(p.next_interval(Duration::from_millis(500), Some(800), h), Duration::from_millis(200));
        // 400/900 = 0.44 > 0.4 → 仍算快 → 加密
        assert_eq!(p.next_interval(Duration::from_millis(500), Some(400), h), Duration::from_millis(200));
        // 300/900 = 0.33 落在滞回带内 → 保持当前间隔
        assert_eq!(p.next_interval(Duration::from_millis(500), Some(300), h), Duration::from_millis(500));
        // 100/900 = 0.11 < 0.2 → 滚得慢 → 回到基准间隔
        assert_eq!(p.next_interval(Duration::from_millis(200), Some(100), h), Duration::from_millis(500));
        // 滞回带内保持加密（不来回抖动）
        assert_eq!(p.next_interval(Duration::from_millis(200), Some(250), h), Duration::from_millis(200));
        // 位移测不出（None）→ **加密**（不确定就拍密：拍疏了会丢掉重叠 → 断档）。
        assert_eq!(p.next_interval(Duration::from_millis(200), None, h), Duration::from_millis(200));
        assert_eq!(p.next_interval(Duration::from_millis(500), None, h), Duration::from_millis(200));
    }

    /// 位移取值：**测量优先**，静止守卫只在测不出时兜底（评审 Minor）。
    #[test]
    fn measured_shift_wins_over_static_guard() {
        // 测得出 → 用测量值（即使内容恰好重复了一个周期，行谱与上一帧相同）。
        assert_eq!(resolve_shift(Some(280), false), Some(280));
        assert_eq!(
            resolve_shift(Some(280), true),
            Some(280),
            "静止守卫不得覆盖真实测量（准周期页面滚过一个周期 → 丢帧）"
        );
        // 测不出 + 页面确实没动 → 0（静止页面不该被当成「不确定」而加密采集）。
        assert_eq!(resolve_shift(None, true), Some(0));
        // 测不出 + 页面在动 → None（不确定 → 拍密）。
        assert_eq!(resolve_shift(None, false), None);
    }

    /// **顺序契约**（评审 I4 残留必修）：拼接失败**不得**消费会话、**不得**关提示条
    /// ——否则错误回传到正在销毁的 webview（渲染进虚空）+ 会话已被消费 → 用户零反馈。
    #[test]
    fn stitch_failure_keeps_session_and_bar() {
        assert!(
            !StopOutcome::StitchFailed.consumes_session(),
            "拼接失败必须保留会话（可重试/放弃）"
        );
        assert!(
            !StopOutcome::StitchFailed.closes_bar(),
            "拼接失败必须保留提示条（错误要有地方显示）"
        );
        // 成功/不可重试 → 消费会话并关条。
        assert!(StopOutcome::Saved.consumes_session());
        assert!(StopOutcome::Saved.closes_bar());
        assert!(StopOutcome::SaveFailed.consumes_session());
        assert!(StopOutcome::SaveFailed.closes_bar());
    }

    /// 停滞检测：**墙钟窗口**（≥3s）+ **内容确实不可测量**才判定（评审 Minor：
    /// 动效页误报）。旧判据是「连续 10 帧画布没长」→ 加密节奏下 10 帧只有 2s
    /// → 慢速滚动 / 动效页被误判为停滞。
    #[test]
    fn stall_needs_a_wall_clock_window_and_unmeasurable_content() {
        let window = Duration::from_secs(3);
        let t0 = Instant::now();
        let mut d = StallDetector::new(window);

        // ① 帧数很多但时间不够（10 帧 × 200ms = 2s < 3s）→ **不判定**停滞。
        for i in 0..10 {
            let now = t0 + Duration::from_millis(200 * i);
            assert!(
                !d.observe(false, false, None, now),
                "墙钟窗口未满（{i} 帧 / {}ms）不得判定停滞",
                200 * i
            );
        }
        // ② 窗口满（≥3s 连续不可测量）→ 判定停滞。
        assert!(d.observe(false, false, None, t0 + Duration::from_secs(3)));
        assert!(d.pending(), "窗口内");

        // ③ 画布长了 → 重置（不再是停滞）。
        let mut d2 = StallDetector::new(window);
        assert!(!d2.observe(false, false, None, t0));
        assert!(!d2.observe(true, false, None, t0 + Duration::from_secs(1)));
        assert!(!d2.pending(), "画布增长后窗口应重置");
        assert!(!d2.observe(false, false, None, t0 + Duration::from_secs(5)));

        // ④ 页面根本没动（用户没滚）→ 不算停滞（不得打扰用户）。
        let mut d3 = StallDetector::new(window);
        assert!(!d3.observe(false, true, None, t0));
        assert!(!d3.observe(false, true, None, t0 + Duration::from_secs(10)));
        assert!(!d3.pending(), "页面静止 → 无停滞窗口");

        // ⑤ **动效页**：内容在动但匹配器**测得出**位移 → 不是「不可测量」→ 不判停滞
        //    （这是评审点名的误报形态）。
        let mut d4 = StallDetector::new(window);
        for i in 0..30 {
            let now = t0 + Duration::from_millis(200 * i);
            assert!(
                !d4.observe(false, false, Some(120), now),
                "内容可测量（动效页）不得判停滞"
            );
        }
        assert!(!d4.pending(), "可测量 → 无停滞窗口");
    }
}
