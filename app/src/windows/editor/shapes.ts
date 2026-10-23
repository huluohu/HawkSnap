// 编辑器形状模型（M2 Task M1）：判别联合 + 纯函数（包围盒 / 命中测试 / 平移）。
// 全部函数无副作用、不修改入参（translate 显式不可变，测试断言守护）——矢量层
// 的所有几何语义单点收敛在本文件，M2 的选中/移动/导出复用同一套定义。

/** 文本量测函数（bboxOf/hitTest 的 text 分支需要）：由调用方注入 ctx.measureText
 * 的包装，纯函数层不持有 canvas 依赖；未注入时走估算回退（0.6em 字宽 / 1.2em 行高）。 */
export type MeasureFn = (text: string, size: number) => { width: number; height: number };

export type Shape =
  | { kind: "rect"; x: number; y: number; w: number; h: number; color: string; width: number }
  | { kind: "ellipse"; x: number; y: number; w: number; h: number; color: string; width: number }
  | { kind: "arrow"; x1: number; y1: number; x2: number; y2: number; color: string; width: number }
  | { kind: "pen"; points: Array<{ x: number; y: number }>; color: string; width: number }
  | { kind: "highlighter"; points: Array<{ x: number; y: number }>; color: string; width: number }
  | { kind: "text"; x: number; y: number; text: string; color: string; size: number };

export interface BBox { x: number; y: number; w: number; h: number }

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

/** 包围盒（恒为左上原点归一化）：rect/ellipse 容忍反向拖拽的负宽高，arrow 取两端
 * 点极值，pen/highlighter 取全部点极值，text = 量测出的文本框（缺量测时估算）。 */
export function bboxOf(shape: Shape, measure?: MeasureFn): BBox {
  switch (shape.kind) {
    case "rect":
    case "ellipse": {
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
    case "text": {
      // 有量测用量测；无量测走估算回退（CJK/西文混合的粗略均值，仅供纯几何上下文兜底）
      const m = measure
        ? measure(shape.text, shape.size)
        : { width: shape.text.length * shape.size * 0.6, height: shape.size * 1.2 };
      return { x: shape.x, y: shape.y, w: m.width, h: m.height };
    }
  }
}

/**
 * 命中测试（选中/移动的基础，M2 消费）：
 * - rect：归一化盒的边含包含（x ≤ px ≤ x+w）；
 * - ellipse：椭圆方程值 ≤ 1（边界含）；零宽/高退化为线段判定（退化为矩形包含）；
 * - arrow / pen / highlighter：到线段集的最短距离 ≤ width + 4（4px 为指击容差）；
 * - text：量测 bbox 包含。
 */
export function hitTest(shape: Shape, px: number, py: number, measure?: MeasureFn): boolean {
  switch (shape.kind) {
    case "rect": {
      const b = bboxOf(shape);
      return px >= b.x && px <= b.x + b.w && py >= b.y && py <= b.y + b.h;
    }
    case "ellipse": {
      const b = bboxOf(shape);
      if (b.w === 0 || b.h === 0) {
        // 零尺寸退化：按矩形包含（避免除零把一切判成未命中）
        return px >= b.x && px <= b.x + b.w && py >= b.y && py <= b.y + b.h;
      }
      const nx = (px - (b.x + b.w / 2)) / (b.w / 2);
      const ny = (py - (b.y + b.h / 2)) / (b.h / 2);
      return nx * nx + ny * ny <= 1;
    }
    case "arrow":
      return distanceToSegment(px, py, shape.x1, shape.y1, shape.x2, shape.y2) <= shape.width + 4;
    case "pen":
    case "highlighter": {
      const threshold = shape.width + 4;
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
      return { ...shape, x: shape.x + dx, y: shape.y + dy };
    case "arrow":
      return { ...shape, x1: shape.x1 + dx, y1: shape.y1 + dy, x2: shape.x2 + dx, y2: shape.y2 + dy };
    case "pen":
    case "highlighter":
      return { ...shape, points: shape.points.map((p) => ({ x: p.x + dx, y: p.y + dy })) };
    case "text":
      return { ...shape, x: shape.x + dx, y: shape.y + dy };
  }
}
