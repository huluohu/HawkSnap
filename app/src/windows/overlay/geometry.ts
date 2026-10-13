// Task 4：方向键微调 + 8px 磁吸的纯几何逻辑（无 React、无 Tauri 依赖，全部可单测）。
//
// 语义裁定（Plan B Task 4 controller）：本文件的 snapRect 采用「最近单边」语义——对
// 所有候选的所有边算 |delta|，取全距 < threshold 中的最小者，只吸一条边；与
// hawksnap_core::geometry::Rect::snap_to 的「四 if 独立判断」语义并存、互不调用：
// Rust 侧 snap_to 供未来后端路径使用，前端磁吸一律走这里。

/** 与 useSelection 的 SelRect 结构全同（结构类型互通）；独立定义避免模块循环依赖。 */
export type Rect = { x: number; y: number; w: number; h: number };

/** r 是否完整落在 bounds 内（nudgeRect 的夹取保证）。 */
export function inBounds(r: Rect, bounds: Rect): boolean {
  return r.x >= bounds.x && r.y >= bounds.y
    && r.x + r.w <= bounds.x + bounds.w && r.y + r.h <= bounds.y + bounds.h;
}

/** 平移 (dx, dy) 后夹取回 bounds（选区必须完整留在 display 内）。 */
export function nudgeRect(r: Rect, dx: number, dy: number, bounds: Rect): Rect {
  const x = Math.min(Math.max(bounds.x, r.x + dx), bounds.x + bounds.w - r.w);
  const y = Math.min(Math.max(bounds.y, r.y + dy), bounds.y + bounds.h - r.h);
  return { ...r, x, y };
}

/** 每条候选边的 (距离, 吸附后的目标坐标)：显式数值对，不用闭包（避免循环内捕获旧 r）。 */
type EdgeHit = { d: number; axis: "x" | "y"; target: number };

/**
 * 最近单边磁吸：选区的每条边可对齐到候选窗口的同侧或对侧边（8 种组合/候选——含
 * 「选区紧贴候选右侧」这类跨边对齐，brief 用例 906→900 即选区左边 √ 候选右边），
 * 算 |delta|，取所有 0 < d < threshold 中的最小者，只移动对应的一条边；并列取首个
 * （稳定）。d == 0 的组合即「已精确对齐」，吸附它是恒等操作，必须排除——否则一条
 * 恰好对齐的边（delta 0）会以 0 距离赢下「最近」竞争，挡住同一 rect 上真正需要吸附
 * 的另一条边（brief 原用例正是此场景）。
 * candidates 为选区所在 display 的本地坐标（index.tsx 已做全局→本地换算）；
 * 吸附可能把选区推出 display 至多 threshold 像素（候选贴边且选区更宽时），由调用方
 * nudgeRect(…, 0, 0, bounds) 夹回（见 useSelection 键盘分支）。
 */
export function snapRect(r: Rect, candidates: Rect[], threshold: number): Rect {
  let best: EdgeHit | null = null;
  for (const c of candidates) {
    const xr = c.x + c.w, yb = c.y + c.h;
    const edges: EdgeHit[] = [
      { d: Math.abs(r.x - c.x), axis: "x", target: c.x },             // 选区左 → 候选左
      { d: Math.abs(r.x - xr), axis: "x", target: xr },               // 选区左 → 候选右
      { d: Math.abs(r.x + r.w - c.x), axis: "x", target: c.x - r.w }, // 选区右 → 候选左
      { d: Math.abs(r.x + r.w - xr), axis: "x", target: xr - r.w },   // 选区右 → 候选右
      { d: Math.abs(r.y - c.y), axis: "y", target: c.y },             // 选区上 → 候选上
      { d: Math.abs(r.y - yb), axis: "y", target: yb },               // 选区上 → 候选下
      { d: Math.abs(r.y + r.h - c.y), axis: "y", target: c.y - r.h }, // 选区下 → 候选上
      { d: Math.abs(r.y + r.h - yb), axis: "y", target: yb - r.h },   // 选区下 → 候选下
    ];
    for (const e of edges) {
      if (e.d > 0 && e.d < threshold && (!best || e.d < best.d)) best = e;
    }
  }
  if (!best) return r;
  return best.axis === "x" ? { ...r, x: best.target } : { ...r, y: best.target };
}
