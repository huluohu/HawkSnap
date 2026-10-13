use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale(pub f64);

impl Rect {
    pub fn from_points(ax: i32, ay: i32, bx: i32, by: i32) -> Self {
        let (x0, x1) = if ax <= bx { (ax, bx) } else { (bx, ax) };
        let (y0, y1) = if ay <= by { (ay, by) } else { (by, ay) };
        Rect { x: x0, y: y0, w: (x1 - x0) as u32, h: (y1 - y0) as u32 }
    }

    pub fn right(&self) -> i32 { self.x + self.w as i32 }
    pub fn bottom(&self) -> i32 { self.y + self.h as i32 }

    /// 半开区间 [x, x+w) × [y, y+h) 点命中：右/下边缘上的点不属于该矩形（标准
    /// hit-test，相邻矩形不双重命中）。供窗口/元素选区的点包含判定。
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn clamp_to(self, bounds: Rect) -> Rect {
        let x = self.x.clamp(bounds.x, bounds.right() - 1);
        let y = self.y.clamp(bounds.y, bounds.bottom() - 1);
        let w = (self.w as i32).clamp(1, bounds.right() - x) as u32;
        let h = (self.h as i32).clamp(1, bounds.bottom() - y) as u32;
        Rect { x, y, w, h }
    }

    pub fn nudge(self, dx: i32, dy: i32) -> Rect { Rect { x: self.x + dx, y: self.y + dy, ..self } }

    /// 任一边缘与某候选边缘距离 < threshold 时吸附到该候选。
    pub fn snap_to(self, candidates: &[Rect], threshold: i32) -> Rect {
        let mut r = self;
        for c in candidates {
            let pairs = [(r.x, c.x), (r.right(), c.right()), (r.y, c.y), (r.bottom(), c.bottom())];
            let deltas = pairs.map(|(a, b)| b - a);
            if deltas.iter().any(|d| d.abs() < threshold) {
                if (c.x - r.x).abs() < threshold { r.x = c.x; }
                if (c.right() - r.right()).abs() < threshold { r.x = c.right() - r.w as i32; }
                if (c.y - r.y).abs() < threshold { r.y = c.y; }
                if (c.bottom() - r.bottom()).abs() < threshold { r.y = c.bottom() - r.h as i32; }
            }
        }
        r
    }
}

/// f64 域的半开区间点命中 [x, x+w) × [y, y+h)，与 [`Rect::contains_point`] 同一语义。
///
/// 为何是自由函数而非 Rect 方法：悬停回退链的候选（CGWindowList 的 RectLog、显示器全帧
/// DisplayFrame）与光标坐标天然是 f64 逻辑坐标，Rect 自身是 i32/u32——若把 f64 入参挂成
/// &self 方法，调用方必须先做有损的 f64→i32 矩形转换（小数边缘被吞、半开语义漂移）；
/// 自由函数让 i32 Rect 与 f64 候选共用同一份经测试的边缘判定，零转换。
pub fn contains_point_f64(px: f64, py: f64, x: f64, y: f64, w: f64, h: f64) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

impl Scale {
    pub fn to_physical(self, v: i32) -> i32 { (v as f64 * self.0).round() as i32 }
    pub fn to_logical(self, v: i32) -> i32 { (v as f64 / self.0).round() as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_points_normalizes_reverse_drag() {
        // 从右下往左上拖拽也要得到正的宽高
        let r = Rect::from_points(300, 200, 100, 50);
        assert_eq!((r.x, r.y, r.w, r.h), (100, 50, 200, 150));
    }

    #[test]
    fn clamp_keeps_rect_inside_bounds() {
        let r = Rect { x: -20, y: -10, w: 100, h: 50 };
        let b = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        let c = r.clamp_to(b);
        assert_eq!((c.x, c.y), (0, 0));
        assert!(c.x + c.w as i32 <= 1920 && c.y + c.h as i32 <= 1080);
    }

    #[test]
    fn nudge_moves_origin_only() {
        let r = Rect { x: 10, y: 10, w: 100, h: 100 };
        let n = r.nudge(5, -3);
        assert_eq!((n.x, n.y, n.w, n.h), (15, 7, 100, 100));
    }

    #[test]
    fn snap_pulls_edge_within_threshold() {
        let sel = Rect { x: 103, y: 10, w: 50, h: 50 };
        let target = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        // 左边缘距 target 左边缘 103px，仅当阈值 ≥103 时吸附
        let s = sel.snap_to(&[target], 8);
        assert_eq!(s.x, 103);
        let s2 = sel.snap_to(&[target], 120);
        assert_eq!(s2.x, 0);
    }

    #[test]
    fn scale_roundtrip_physical_logical() {
        let s = Scale(2.0); // Retina
        assert_eq!(s.to_physical(100), 200);
        assert_eq!(s.to_logical(200), 100);
        let s125 = Scale(1.25);
        assert_eq!(s125.to_physical(100), 125);
        assert_eq!(s125.to_logical(125), 100);
    }

    #[test]
    fn contains_point_bounds_checked() {
        let r = Rect { x: 10, y: 10, w: 50, h: 50 };
        assert!(r.contains_point(10, 10));
        assert!(r.contains_point(59, 59));
        assert!(!r.contains_point(60, 60));
        assert!(!r.contains_point(9, 30));
    }

    #[test]
    fn contains_point_f64_half_open() {
        // 与 i32 contains_point 同一半开语义（[x, x+w) × [y, y+h)），f64 域：
        // 供 overlay 的 window_hit / display_frame_at（候选与光标天然是 f64 逻辑坐标，
        // 光标坐标取整会把负半轴上的点错误折进左/上边缘，故命中判定必须保 f64）。
        let (x, y, w, h) = (10.0, 10.0, 50.0, 50.0);
        assert!(contains_point_f64(10.0, 10.0, x, y, w, h)); // 左/上边缘（含）
        assert!(contains_point_f64(59.9, 59.9, x, y, w, h)); // 右/下边缘内侧
        assert!(!contains_point_f64(60.0, 30.0, x, y, w, h)); // 右边缘（不含）
        assert!(!contains_point_f64(30.0, 60.0, x, y, w, h)); // 下边缘（不含）
        assert!(!contains_point_f64(9.5, 30.0, x, y, w, h)); // 左外侧（含小数负偏）
        assert!(!contains_point_f64(30.0, -0.5, x, y, w, h)); // 上外侧
    }
}
