// 编辑器形状模型（M2 Task M2）：判别联合 + 纯函数（包围盒 / 命中测试 / 平移 / 样式替换）。
// 全部函数无副作用、不修改入参（translate 显式不可变，测试断言守护）——矢量层的
// 所有几何语义单点收敛在本文件；预览/选中/导出复用同一套定义。
//
// 命中语义裁定（M1 遗留项，M2 落定）：
// - 本编辑器的形状全是**描边**（无填充），rect/ellipse 采用**描边带命中**：到边界
//   曲线的距离 ≤ width/2 + 4（描边半宽 + 4px 指击容差）——点击形状内部空腔不命中
//   （可穿透选中下层/取消选中），贴近边线才命中。M1 的「矩形包含/椭圆方程 ≤1」
//   全腹命中语义废弃。
// - 线状工具（arrow/pen/highlighter）沿用 M1 口径：到线段集最短距离 ≤ width + 4
//   （到描边中线更宽松，指击好命中）。arrow 命中域**含箭头两翼**（M1 遗留：头部
//   曾不可点选）——翼几何与 draw.ts 的绘制完全同参（±0.5 rad 回折、head 同长）。
// - mosaic 是不透明块：矩形全腹包含命中（贴块任意一点都能抓住）。
// - step 是实心圆气泡：到圆心距离 ≤ 半径 + 4。

/** 文本量测函数（bboxOf/hitTest 的 text 分支需要）：由调用方注入 ctx.measureText
 * 的包装，纯函数层不持有 canvas 依赖；未注入时走估算回退（0.6em 字宽 / 1.2em 行高）。 */
export type MeasureFn = (text: string, size: number) => { width: number; height: number };

export type Shape =
  | { kind: "rect"; x: number; y: number; w: number; h: number; color: string; width: number }
  | { kind: "ellipse"; x: number; y: number; w: number; h: number; color: string; width: number }
  | { kind: "arrow"; x1: number; y1: number; x2: number; y2: number; color: string; width: number }
  | { kind: "pen"; points: Array<{ x: number; y: number }>; color: string; width: number }
  | { kind: "highlighter"; points: Array<{ x: number; y: number }>; color: string; width: number }
  | { kind: "text"; x: number; y: number; text: string; color: string; size: number }
  /** 像素化方块：渲染 = 底图区域按 blockSize 分块降采样再放大（draw.ts drawMosaic） */
  | { kind: "mosaic"; x: number; y: number; w: number; h: number; blockSize: number }
  /** 序号气泡：实心圆 + 白字序号；n 在创建时按 nextStepNumber(shapes) 计算 */
  | { kind: "step"; x: number; y: number; n: number; color: string };

export type MosaicShape = Extract<Shape, { kind: "mosaic" }>;

export interface BBox { x: number; y: number; w: number; h: number }

/** 指击容差（px）：所有命中阈值共用的常量。 */
const HIT_TOL = 4;

/** 箭头头部：翼长（与 draw.ts 的 drawShape arrow 分支同参——命中域必须贴合所画）。 */
export function arrowHeadLen(width: number): number {
  return Math.max(10, width * 4);
}

/** 序号气泡半径：随位数增长（1 位 15px，2 位 23px，3 位 31px），绘制/命中/包围盒共用。 */
export function stepRadius(n: number): number {
  return Math.max(14, String(n).length * 8 + 7);
}

/** 下一个序号：已有 step 的最大 n + 1（无则 1）。删除/撤销后自然回落（n 存于形状）。 */
export function nextStepNumber(shapes: Shape[]): number {
  let max = 0;
  for (const s of shapes) if (s.kind === "step" && s.n > max) max = s.n;
  return max + 1;
}

/** 点到线段的最短距离（投影落在段外时取端点距离；零长段退化为点到点）。 */
export function distanceToSegment(
  px: number, py: number, x1: number, y1: number, x2: number, y2: number,
): number {
  const dx = x2 - x1;
  const dy = y2 - y1;
  const len2 = dx * dx + dy * dy;
  if (len2 === 0) return Math.hypot(px - x1, py - y1);
  // t = 投影参数 clamp 到 [0,1]
  const t = Math.max(0, Math.min(1, ((px - x1) * dx + (py - y1) * dy) / len2));
  return Math.hypot(px - (x1 + t * dx), py - (y1 + t * dy));
}

/** 包围盒（恒为左上原点归一化）：rect/ellipse/mosaic 容忍反向拖拽的负宽高，arrow 取
 * 两端点极值，pen/highlighter 取全部点极值，step = 外切方，text = 量测出的文本框。 */
export function bboxOf(shape: Shape, measure?: MeasureFn): BBox {
  switch (shape.kind) {
    case "rect":
    case "ellipse":
    case "mosaic": {
      const x = Math.min(shape.x, shape.x + shape.w);
      const y = Math.min(shape.y, shape.y + shape.h);
      return { x, y, w: Math.abs(shape.w), h: Math.abs(shape.h) };
    }
    case "arrow":
      return {
        x: Math.min(shape.x1, shape.x2),
        y: Math.min(shape.y1, shape.y2),
        w: Math.abs(shape.x2 - shape.x1),
        h: Math.abs(shape.y2 - shape.y1),
      };
    case "pen":
    case "highlighter": {
      const xs = shape.points.map((p) => p.x);
      const ys = shape.points.map((p) => p.y);
      const x = Math.min(...xs);
      const y = Math.min(...ys);
      return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
    }
    case "step": {
      const r = stepRadius(shape.n);
      return { x: shape.x - r, y: shape.y - r, w: r * 2, h: r * 2 };
    }
    case "text": {
      // 有量测用量测；无量测走估算回退（CJK/西文混合的粗略均值，仅供纯几何上下文兜底）
      const m = measure
        ? measure(shape.text, shape.size)
        : { width: shape.text.length * shape.size * 0.6, height: shape.size * 1.2 };
      return { x: shape.x, y: shape.y, w: m.width, h: m.height };
    }
  }
}

/** 矩形描边带到点位的距离：界外 = 到矩形的距离；界内 = 到最近边的距离（0 即在边上）。 */
function rectBandDistance(px: number, py: number, b: BBox): number {
  const dx = Math.max(b.x - px, 0, px - (b.x + b.w));
  const dy = Math.max(b.y - py, 0, py - (b.y + b.h));
  const outside = Math.hypot(dx, dy);
  if (outside > 0) return outside;
  if (b.w === 0 || b.h === 0) return 0; // 退化（零宽/高）：整个包围盒都是描边
  return Math.min(px - b.x, b.x + b.w - px, py - b.y, b.y + b.h - py);
}

/**
 * 命中测试（选中/移动的基础）：
 * - rect：描边带（到矩形边界距离 ≤ width/2 + 4）；
 * - ellipse：描边带（沿中心连线近似到椭圆边界的像素距离 ≤ width/2 + 4）；
 * - arrow：主线段或箭头两翼的线段距离 ≤ width + 4；
 * - pen / highlighter：到线段集的最短距离 ≤ width + 4；
 * - mosaic：矩形全腹包含（不透明块）；
 * - step：到圆心距离 ≤ stepRadius + 4；
 * - text：量测 bbox 包含。
 */
export function hitTest(shape: Shape, px: number, py: number, measure?: MeasureFn): boolean {
  switch (shape.kind) {
    case "rect":
      return rectBandDistance(px, py, bboxOf(shape)) <= shape.width / 2 + HIT_TOL;
    case "ellipse": {
      const b = bboxOf(shape);
      const rx = b.w / 2;
      const ry = b.h / 2;
      if (rx === 0 && ry === 0) {
        return Math.hypot(px - b.x, py - b.y) <= shape.width / 2 + HIT_TOL; // 退化为点
      }
      if (rx === 0 || ry === 0) {
        // 退化为线段：rx=0 → 竖直线段；ry=0 → 水平线段
        const d = rx === 0
          ? distanceToSegment(px, py, b.x, b.y, b.x, b.y + b.h)
          : distanceToSegment(px, py, b.x, b.y, b.x + b.w, b.y);
        return d <= shape.width / 2 + HIT_TOL;
      }
      const cx = b.x + rx;
      const cy = b.y + ry;
      const vx = px - cx;
      const vy = py - cy;
      const vp = Math.hypot(vx, vy);
      if (vp === 0) return false; // 圆心是深腹，不在描边带上
      // 沿「中心→点位」射线与椭圆的交点半径（椭圆方程代入方向单位向量），
      // |点位距心 − 交点半径| ≈ 点到椭圆边界的距离（中低离心率下足够命中判定）。
      const ux = vx / vp;
      const uy = vy / vp;
      const rEdge = 1 / Math.sqrt((ux * ux) / (rx * rx) + (uy * uy) / (ry * ry));
      return Math.abs(vp - rEdge) <= shape.width / 2 + HIT_TOL;
    }
    case "arrow": {
      const threshold = shape.width + HIT_TOL;
      if (distanceToSegment(px, py, shape.x1, shape.y1, shape.x2, shape.y2) <= threshold) {
        return true;
      }
      // 箭头两翼：与 draw.ts 同参（自尖端沿主轴 ±(π−0.5) 回折 head 长）
      const ang = Math.atan2(shape.y2 - shape.y1, shape.x2 - shape.x1);
      const head = arrowHeadLen(shape.width);
      for (const da of [Math.PI - 0.5, Math.PI + 0.5]) {
        const wx = shape.x2 + head * Math.cos(ang + da);
        const wy = shape.y2 + head * Math.sin(ang + da);
        if (distanceToSegment(px, py, shape.x2, shape.y2, wx, wy) <= threshold) return true;
      }
      return false;
    }
    case "pen":
    case "highlighter": {
      const threshold = shape.width + HIT_TOL;
      if (shape.points.length === 1) {
        return Math.hypot(px - shape.points[0].x, py - shape.points[0].y) <= threshold;
      }
      for (let i = 1; i < shape.points.length; i++) {
        const a = shape.points[i - 1];
        const b = shape.points[i];
        if (distanceToSegment(px, py, a.x, a.y, b.x, b.y) <= threshold) return true;
      }
      return false;
    }
    case "mosaic": {
      const b = bboxOf(shape);
      return px >= b.x && px <= b.x + b.w && py >= b.y && py <= b.y + b.h;
    }
    case "step":
      return Math.hypot(px - shape.x, py - shape.y) <= stepRadius(shape.n) + HIT_TOL;
    case "text": {
      const b = bboxOf(shape, measure);
      return px >= b.x && px <= b.x + b.w && py >= b.y && py <= b.y + b.h;
    }
  }
}

/** 平移（不可变）：返回同 kind 新对象；自由手形状连点对象一起新建，绝不复用原数组。 */
export function translate(shape: Shape, dx: number, dy: number): Shape {
  switch (shape.kind) {
    case "rect":
    case "ellipse":
    case "mosaic":
    case "step":
    case "text":
      return { ...shape, x: shape.x + dx, y: shape.y + dy };
    case "arrow":
      return { ...shape, x1: shape.x1 + dx, y1: shape.y1 + dy, x2: shape.x2 + dx, y2: shape.y2 + dy };
    case "pen":
    case "highlighter":
      return { ...shape, points: shape.points.map((p) => ({ x: p.x + dx, y: p.y + dy })) };
  }
}

/** 顶层命中：自最上层向下找第一个命中的形状下标（无命中 → null）。 */
export function topmostShapeAt(
  shapes: Shape[], px: number, py: number, measure?: MeasureFn,
): number | null {
  for (let i = shapes.length - 1; i >= 0; i--) {
    if (hitTest(shapes[i], px, py, measure)) return i;
  }
  return null;
}

/**
 * 裁剪形状生存规则（M3 第 10 工具）：位图裁到 rect 后，形状按**包围盒与 rect 是否
 * 相交**二分——相交（含部分越界）→ `translate(-rect.x, -rect.y)` 平移进新坐标系
 * （越出新边界的部分由画布 overflow 裁剪与导出画布自然裁掉，形状本体保留）；
 * 完全在外 → 丢弃。全部在外时返回空数组（裁剪照常生效，形状层清空——与
 * `Option<shapes>` 语义对应的 TS 形态）。纯函数：不改入参、translate 显式新建。
 */
export function applyCrop(shapes: Shape[], rect: BBox): Shape[] {
  const out: Shape[] = [];
  for (const s of shapes) {
    const b = bboxOf(s);
    // 严格不等号：零面积搭接（边缘恰好相触）不算相交——不可见即不保留
    const intersects =
      b.x < rect.x + rect.w &&
      b.x + b.w > rect.x &&
      b.y < rect.y + rect.h &&
      b.y + b.h > rect.y;
    if (!intersects) continue;
    out.push(translate(s, -rect.x, -rect.y));
  }
  return out;
}

/** 样式替换（选中态改色/改粗细/改字号 = 不可变拷贝替换 + 一次历史提交）：
 * 形状没有对应字段时原样返回（mosaic 无 color/width；step 无 width/size；非 text 无 size）。 */
export function withColor(shape: Shape, color: string): Shape {
  return "color" in shape ? ({ ...shape, color } as Shape) : shape;
}

export function withWidth(shape: Shape, width: number): Shape {
  return "width" in shape ? ({ ...shape, width } as Shape) : shape;
}

export function withSize(shape: Shape, size: number): Shape {
  return shape.kind === "text" ? { ...shape, size } : shape;
}
