import { describe, expect, it } from "vitest";
import {
  bboxOf,
  distanceToSegment,
  hitTest,
  translate,
  type MeasureFn,
  type Shape,
} from "./shapes";

// 文本量测桩（测试构造：bboxOf/hitTest 的 text 分支需要 ctx 量测，按注入函数解耦）。
const measure: MeasureFn = (text, size) => ({
  width: text.length * size * 0.5,
  height: size,
});

const rect: Shape = { kind: "rect", x: 10, y: 20, w: 100, h: 50, color: "#f00", width: 2 };
const ellipse: Shape = { kind: "ellipse", x: 0, y: 0, w: 40, h: 20, color: "#0f0", width: 2 };
const arrow: Shape = { kind: "arrow", x1: 0, y1: 0, x2: 30, y2: 40, color: "#00f", width: 3 };
const pen: Shape = {
  kind: "pen",
  points: [{ x: 0, y: 0 }, { x: 10, y: 0 }, { x: 10, y: 10 }],
  color: "#000", width: 2,
};
const highlighter: Shape = {
  kind: "highlighter",
  points: [{ x: 5, y: 5 }, { x: 25, y: 5 }],
  color: "#ff0", width: 8,
};
const text: Shape = { kind: "text", x: 7, y: 11, text: "abc", color: "#123", size: 16 };

describe("bboxOf", () => {
  it("rect bbox is the min-corner normalized box", () => {
    // 负宽高（反向拖拽的草稿）也归一化：包围盒恒为左上原点
    expect(bboxOf(rect)).toEqual({ x: 10, y: 20, w: 100, h: 50 });
    expect(bboxOf({ ...rect, x: 30, y: 40, w: -20, h: -10 })).toEqual({
      x: 10, y: 30, w: 20, h: 10,
    });
  });

  it("ellipse bbox is its box (normalized)", () => {
    expect(bboxOf(ellipse)).toEqual({ x: 0, y: 0, w: 40, h: 20 });
    expect(bboxOf({ ...ellipse, x: 40, y: 20, w: -40, h: -20 })).toEqual({
      x: 0, y: 0, w: 40, h: 20,
    });
  });

  it("arrow bbox spans both endpoints", () => {
    expect(bboxOf(arrow)).toEqual({ x: 0, y: 0, w: 30, h: 40 });
    expect(bboxOf({ ...arrow, x1: 50, y1: -5, x2: 30, y2: 40 })).toEqual({
      x: 30, y: -5, w: 20, h: 45,
    });
  });

  it("pen/highlighter bbox spans all points", () => {
    expect(bboxOf(pen)).toEqual({ x: 0, y: 0, w: 10, h: 10 });
    expect(bboxOf(highlighter)).toEqual({ x: 5, y: 5, w: 20, h: 0 });
  });

  it("text bbox uses the injected measure fn", () => {
    // 桩：width = len*size*0.5 = 24，height = size = 16
    expect(bboxOf(text, measure)).toEqual({ x: 7, y: 11, w: 24, h: 16 });
  });

  it("text bbox falls back to an estimate when no measure fn is passed", () => {
    const b = bboxOf(text);
    expect(b.x).toBe(7);
    expect(b.y).toBe(11);
    expect(b.w).toBeGreaterThan(0);
    expect(b.h).toBeGreaterThan(0);
  });
});

describe("distanceToSegment", () => {
  it("returns perpendicular distance for interior projection", () => {
    expect(distanceToSegment(5, 4, 0, 0, 10, 0)).toBeCloseTo(4);
  });

  it("returns endpoint distance when projection falls outside", () => {
    expect(distanceToSegment(-3, 0, 0, 0, 10, 0)).toBeCloseTo(3);
    expect(distanceToSegment(15, 0, 0, 0, 10, 0)).toBeCloseTo(5);
  });

  it("degenerate zero-length segment behaves as distance to the point", () => {
    expect(distanceToSegment(3, 4, 2, 3, 2, 3)).toBeCloseTo(Math.SQRT2);
  });
});

describe("hitTest", () => {
  it("rect is edge-inclusive", () => {
    expect(hitTest(rect, 10, 20, measure)).toBe(true); // 左上角边界;
    expect(hitTest(rect, 110, 70, measure)).toBe(true); // 右下角边界;
    expect(hitTest(rect, 60, 45, measure)).toBe(true); // 内部;
    expect(hitTest(rect, 9.9, 45, measure)).toBe(false);
    expect(hitTest(rect, 110.1, 45, measure)).toBe(false);
  });

  it("rect hit-test handles negative (reversed-drag) boxes", () => {
    const flipped = { ...rect, x: 110, y: 70, w: -100, h: -50 };
    expect(hitTest(flipped, 60, 45, measure)).toBe(true);
    expect(hitTest(flipped, 9, 45, measure)).toBe(false);
  });

  it("ellipse containment follows the ellipse equation, edge inclusive", () => {
    // 中心 (20,10)，rx=20，ry=10：方程值 (dx/rx)² + (dy/ry)²
    expect(hitTest(ellipse, 20, 10, measure)).toBe(true); // 中心（方程值 0）;
    expect(hitTest(ellipse, 0, 10, measure)).toBe(true); // 左顶点边界（方程值 1）;
    expect(hitTest(ellipse, 20, 0, measure)).toBe(true); // 上顶点边界（方程值 1）;
    expect(hitTest(ellipse, 20, 20.5, measure)).toBe(false); // dy=10.5 → 方程值 > 1;
  });

  it("arrow hits within width + 4 of the segment", () => {
    // 段 (0,0)-(30,40)：单位 (0.6,0.8)，法向 (0.8,-0.6)。
    // 段上点 (15,20) 沿法向偏移 6 → (19.8,16.4)，垂距 6 ≤ width+4=7 → 命中
    expect(hitTest(arrow, 19.8, 16.4, measure)).toBe(true);
    // 同点偏移 8 → (21.4,15.2)，垂距 8 > 7 → 不命中
    expect(hitTest(arrow, 21.4, 15.2, measure)).toBe(false);
  });

  it("pen hits by min distance over segments within width + 4", () => {
    expect(hitTest(pen, 5, 3, measure)).toBe(true); // 段 1 上方 3px < 2+4;
    expect(hitTest(pen, 13, 5, measure)).toBe(true); // 段 2 右侧 3px;
    expect(hitTest(pen, 20, 5, measure)).toBe(false); // 两端最近均为 ~10px > 6;
  });

  it("pen with a single point hits near that point", () => {
    const dot: Shape = { kind: "pen", points: [{ x: 0, y: 0 }], color: "#000", width: 2 };
    expect(hitTest(dot, 3, 4, measure)).toBe(true); // 距离 5 ≤ 6;
    expect(hitTest(dot, 6, 8, measure)).toBe(false); // 距离 10 > 6;
  });

  it("highlighter uses its (wider) width in the threshold", () => {
    // width=8 → 阈值 12：段 y=5，点 (15,16) 垂距 11 → 命中
    expect(hitTest(highlighter, 15, 16, measure)).toBe(true);
    expect(hitTest(highlighter, 15, 18, measure)).toBe(false); // 垂距 13 > 12;
  });

  it("text hits inside the measured bbox", () => {
    expect(hitTest(text, 8, 12, measure)).toBe(true);
    expect(hitTest(text, 7, 11, measure)).toBe(true); // 边界含;
    expect(hitTest(text, 31.1, 12, measure)).toBe(false); // x=7+24=31 之外;
    expect(hitTest(text, 8, 27.1, measure)).toBe(false); // y=11+16=27 之外;
  });
});

describe("translate", () => {
  it("translates point kinds and keeps the original untouched", () => {
    const moved = translate(rect, 5, -5);
    expect(moved).toEqual({ ...rect, x: 15, y: 15 });
    expect(rect.x).toBe(10); // 原形状不可变
  });

  it("translates ellipse and text", () => {
    expect(translate(ellipse, 1, 2)).toEqual({ ...ellipse, x: 1, y: 2 });
    expect(translate(text, -7, -11)).toEqual({ ...text, x: 0, y: 0 });
  });

  it("translates arrow endpoints", () => {
    expect(translate(arrow, 2, 3)).toEqual({ ...arrow, x1: 2, y1: 3, x2: 32, y2: 43 });
  });

  it("translates freehand points into a new array without mutating the original", () => {
    const moved = translate(pen, 10, 20);
    expect(moved).not.toBe(pen);
    if (moved.kind === "pen") {
      expect(moved.points[0]).toEqual({ x: 10, y: 20 });
      expect(moved.points[2]).toEqual({ x: 20, y: 30 });
      expect(moved.points).not.toBe(pen.points);
      expect(moved.points[2]).not.toBe(pen.points[2]); // 点对象也是新建的;
    } else {
      expect.unreachable("kind 必须保持 pen");
    }
    // 不可变：原 points 数组与点对象原样
    expect(pen.points[2]).toEqual({ x: 10, y: 10 });
  });
});
