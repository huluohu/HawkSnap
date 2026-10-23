import { describe, expect, it } from "vitest";
import {
  arrowHeadLen,
  bboxOf,
  distanceToSegment,
  hitTest,
  nextStepNumber,
  stepRadius,
  topmostShapeAt,
  translate,
  withColor,
  withSize,
  withWidth,
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
const mosaic: Shape = { kind: "mosaic", x: 12, y: 24, w: 60, h: 30, blockSize: 10 };
const step: Shape = { kind: "step", x: 50, y: 60, n: 7, color: "#E5484D" };

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

  it("mosaic bbox is the normalized box", () => {
    expect(bboxOf(mosaic)).toEqual({ x: 12, y: 24, w: 60, h: 30 });
    expect(bboxOf({ ...mosaic, x: 72, y: 54, w: -60, h: -30 })).toEqual({
      x: 12, y: 24, w: 60, h: 30,
    });
  });

  it("step bbox is the square circumscribing stepRadius(n)", () => {
    const r = stepRadius(7);
    expect(bboxOf(step)).toEqual({ x: 50 - r, y: 60 - r, w: r * 2, h: r * 2 });
    expect(r).toBe(15); // 1 位序号
    expect(stepRadius(99)).toBe(23); // 2 位
    expect(stepRadius(100)).toBe(31); // 3 位
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
  it("rect uses stroke-band semantics (M2 裁定): edge band hits, deep interior passes through", () => {
    expect(hitTest(rect, 10, 20, measure)).toBe(true); // 边界上
    expect(hitTest(rect, 110, 70, measure)).toBe(true); // 右下角边界上
    expect(hitTest(rect, 60, 45, measure)).toBe(false); // 深腹：无填充 → 不命中
    expect(hitTest(rect, 60, 24, measure)).toBe(true); // 界内 4px < width/2+4=5 → 命中
    expect(hitTest(rect, 60, 23, measure)).toBe(true); // 界内 3px
  });

  it("rect stroke band extends 5px (width/2 + tol) on either side of the edge", () => {
    // width=2 → 半宽 1 + 指击 4 = 5
    expect(hitTest(rect, 60, 16, measure)).toBe(true); // 界外 4px
    expect(hitTest(rect, 60, 14, measure)).toBe(false); // 界外 6px
    expect(hitTest(rect, 60, 25, measure)).toBe(true); // 界内 5px（阈值含）
    expect(hitTest(rect, 60, 26, measure)).toBe(false); // 界内 6px
    expect(hitTest(rect, 7, 17, measure)).toBe(true); // 角外 (3,3)=4.24
    expect(hitTest(rect, 4, 14, measure)).toBe(false); // 角外 (6,6)=8.49
  });

  it("rect hit-test handles negative (reversed-drag) boxes", () => {
    const flipped = { ...rect, x: 110, y: 70, w: -100, h: -50 };
    expect(hitTest(flipped, 60, 25, measure)).toBe(true); // 下边描边带内
    expect(hitTest(flipped, 60, 45, measure)).toBe(false); // 深腹
    expect(hitTest(flipped, 9, 45, measure)).toBe(true); // 左边外 1px，仍在 5px 带内
    expect(hitTest(flipped, 3, 45, measure)).toBe(false); // 左边外 7px
  });

  it("ellipse uses ring-band semantics: center misses, boundary band hits", () => {
    // 中心 (20,10)，rx=20，ry=10；width=2 → 阈值 5
    expect(hitTest(ellipse, 20, 10, measure)).toBe(false); // 圆心：深腹不命中
    expect(hitTest(ellipse, 0, 10, measure)).toBe(true); // 左顶点边界（距 0）
    expect(hitTest(ellipse, 20, 0, measure)).toBe(true); // 上顶点边界
    expect(hitTest(ellipse, 20, 20.5, measure)).toBe(true); // 界外 0.5px → 命中
    expect(hitTest(ellipse, 20, 26, measure)).toBe(false); // 界外 6px
    expect(hitTest(ellipse, 20, 4, measure)).toBe(true); // 界内 4px（描边带内，详见下一 case）
  });

  it("ellipse band width matches width/2 + 4 on both sides of the curve", () => {
    // 顶点 (20,0)：界内 (20,4) 距边界 4 → 命中；(20,6) 距 6 → 不命中
    expect(hitTest(ellipse, 20, 4, measure)).toBe(true);
    expect(hitTest(ellipse, 20, 6, measure)).toBe(false);
    // 界外沿 y 轴：(20, -4) 距上顶点 4 → 命中
    expect(hitTest(ellipse, 20, -4, measure)).toBe(true);
    expect(hitTest(ellipse, 20, -6, measure)).toBe(false);
  });

  it("ellipse degenerates: zero-width → segment band, zero-size → point", () => {
    const line: Shape = { kind: "ellipse", x: 20, y: 0, w: 0, h: 20, color: "#0f0", width: 2 };
    expect(hitTest(line, 20, 10, measure)).toBe(true); // 线上
    expect(hitTest(line, 24, 10, measure)).toBe(true); // 距 4 ≤ 5
    expect(hitTest(line, 26, 10, measure)).toBe(false); // 距 6
    const dot: Shape = { kind: "ellipse", x: 5, y: 5, w: 0, h: 0, color: "#0f0", width: 2 };
    expect(hitTest(dot, 7, 8, measure)).toBe(true); // 距 ~3.6 ≤ 5
    expect(hitTest(dot, 15, 15, measure)).toBe(false); // 距 ~14
  });

  it("arrow hits within width + 4 of the segment", () => {
    // 段 (0,0)-(30,40)：单位 (0.6,0.8)，法向 (0.8,-0.6)。
    // 段上点 (15,20) 沿法向偏移 6 → (19.8,16.4)，垂距 6 ≤ width+4=7 → 命中
    expect(hitTest(arrow, 19.8, 16.4, measure)).toBe(true);
    // 同点偏移 8 → (21.4,15.2)，垂距 8 > 7 → 不命中
    expect(hitTest(arrow, 21.4, 15.2, measure)).toBe(false);
  });

  it("arrow hit region includes the drawn head wings (M1 遗留修复)", () => {
    // 与 draw.ts 同参：head = max(10, width*4) = 12，翼自尖端回折 ±(π−0.5)
    const ang = Math.atan2(40, 30);
    const head = arrowHeadLen(3);
    const da = Math.PI + 0.5;
    const wx = 30 + head * Math.cos(ang + da);
    const wy = 40 + head * Math.sin(ang + da);
    // 翼中点沿法向偏移 5（>距主线阈值会 miss 的位置）：翼线段距离 5 ≤ 7 → 命中
    const mx = (30 + wx) / 2;
    const my = (40 + wy) / 2;
    const dl = Math.hypot(wx - 30, wy - 40);
    const ux = (wx - 30) / dl;
    const uy = (wy - 40) / dl;
    const nx = -uy;
    const ny = ux;
    expect(hitTest(arrow, mx + nx * 5, my + ny * 5, measure)).toBe(true);
    expect(hitTest(arrow, mx + nx * 9, my + ny * 9, measure)).toBe(false);
    // 翼末端再沿翼向外 10px（超出 head 长 → 投影钳到翼端，距 10 > 7）：三段全不命中
    expect(hitTest(arrow, wx + ux * 10, wy + uy * 10, measure)).toBe(false);
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

  it("mosaic is a filled block: full-interior containment", () => {
    expect(hitTest(mosaic, 40, 40, measure)).toBe(true); // 深腹
    expect(hitTest(mosaic, 12, 24, measure)).toBe(true); // 角边界
    expect(hitTest(mosaic, 11.9, 40, measure)).toBe(false); // 界外
    expect(hitTest(mosaic, 72.1, 40, measure)).toBe(false);
  });

  it("step hits within radius + 4 of the center", () => {
    expect(hitTest(step, 50, 60, measure)).toBe(true); // 圆心
    expect(hitTest(step, 50 + 15, 60, measure)).toBe(true); // 圆周上
    expect(hitTest(step, 50 + 18, 60, measure)).toBe(true); // 半径+3 ≤ 19
    expect(hitTest(step, 50 + 20, 60, measure)).toBe(false); // 半径+5 > 19
  });

  it("text hits inside the measured bbox", () => {
    expect(hitTest(text, 8, 12, measure)).toBe(true);
    expect(hitTest(text, 7, 11, measure)).toBe(true); // 边界含;
    expect(hitTest(text, 31.1, 12, measure)).toBe(false); // x=7+24=31 之外;
    expect(hitTest(text, 8, 27.1, measure)).toBe(false); // y=11+16=27 之外;
  });
});

describe("topmostShapeAt", () => {
  it("returns the topmost (last) hit shape index, null when nothing hits", () => {
    const shapes = [rect, ellipse]; // 两形状无重叠，按点位区分归属
    expect(topmostShapeAt(shapes, 60, 25, measure)).toBe(0); // rect 下边描边带
    expect(topmostShapeAt(shapes, 0, 10, measure)).toBe(1); // ellipse 左顶点
    expect(topmostShapeAt(shapes, 500, 500, measure)).toBeNull();
  });

  it("later shapes win on overlap", () => {
    const onTop: Shape = { ...mosaic, x: 10, y: 20, w: 100, h: 50 };
    const shapes = [rect, onTop];
    expect(topmostShapeAt(shapes, 60, 45, measure)).toBe(1); // rect 深腹不命中，mosaic 命中
    const bothSolid = [mosaic, { ...mosaic, x: 20, y: 30 }];
    expect(topmostShapeAt(bothSolid, 30, 35, measure)).toBe(1);
  });
});

describe("nextStepNumber", () => {
  it("starts at 1 on empty / non-step shapes", () => {
    expect(nextStepNumber([])).toBe(1);
    expect(nextStepNumber([rect, text])).toBe(1);
  });

  it("increments past the max existing step number", () => {
    expect(nextStepNumber([step, { ...step, n: 3 }])).toBe(8);
    expect(nextStepNumber([{ ...step, n: 1 }, { ...step, n: 12 }])).toBe(13);
  });

  it("decrements naturally after delete/undo removes the max step", () => {
    const withNine = [step, { ...step, n: 8 }, { ...step, n: 9 }];
    expect(nextStepNumber(withNine)).toBe(10);
    expect(nextStepNumber(withNine.slice(0, -1))).toBe(9); // 撤销删除 n=9 后
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

  it("translates mosaic and step", () => {
    expect(translate(mosaic, 1, 2)).toEqual({ ...mosaic, x: 13, y: 26 });
    expect(translate(step, -5, -6)).toEqual({ ...step, x: 45, y: 54 });
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

describe("style replacement", () => {
  it("withColor replaces color on colored kinds and passes mosaic through", () => {
    expect(withColor(rect, "#00ff00")).toEqual({ ...rect, color: "#00ff00" });
    expect(withColor(step, "#00ff00")).toEqual({ ...step, color: "#00ff00" });
    expect(withColor(mosaic, "#00ff00")).toBe(mosaic); // 无 color 字段 → 原样（同引用）
  });

  it("withWidth replaces width on stroked kinds only", () => {
    expect(withWidth(rect, 9)).toEqual({ ...rect, width: 9 });
    expect(withWidth(arrow, 9)).toEqual({ ...arrow, width: 9 });
    expect(withWidth(text, 9)).toBe(text); // text 无 width
    expect(withWidth(step, 9)).toBe(step); // step 无 width
  });

  it("withSize applies to text only", () => {
    expect(withSize(text, 32)).toEqual({ ...text, size: 32 });
    expect(withSize(rect, 32)).toBe(rect);
  });
});
