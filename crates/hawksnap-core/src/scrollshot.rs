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

use std::time::Duration;

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
}
