import { describe, expect, it } from "vitest";
import { inBounds, nudgeRect, snapRect } from "./geometry";

const screen = { x: 0, y: 0, w: 1512, h: 982 };

describe("nudgeRect", () => {
  it("moves by step and clamps to bounds", () => {
    expect(nudgeRect({ x: 10, y: 10, w: 50, h: 50 }, 5, -3, screen))
      .toEqual({ x: 15, y: 7, w: 50, h: 50 });
    expect(nudgeRect({ x: 0, y: 0, w: 50, h: 50 }, -10, 0, screen))
      .toEqual({ x: 0, y: 0, w: 50, h: 50 });
  });
});

describe("snapRect", () => {
  const candidates = [{ x: 0, y: 0, w: 1512, h: 982 }, { x: 500, y: 300, w: 400, h: 300 }];
  it("does not snap beyond threshold", () => {
    expect(snapRect({ x: 103, y: 500, w: 100, h: 100 }, candidates, 8).x).toBe(103);
  });
  it("snaps the nearest single edge within threshold", () => {
    // 左边缘距候选窗口右边缘 (900) 6px → 吸附到 900
    expect(snapRect({ x: 906, y: 500, w: 100, h: 100 }, candidates, 8).x).toBe(900);
    // 左边缘距屏幕左边缘 3px → 吸附 0
    expect(snapRect({ x: 3, y: 500, w: 100, h: 100 }, candidates, 8).x).toBe(0);
  });
  it("picks the nearest edge when several are in range", () => {
    // 距 900 差 2、距 500+400=900 右…同点：距屏幕右 1512-1006=506 不在阈值
    expect(snapRect({ x: 902, y: 500, w: 100, h: 100 }, candidates, 8).x).toBe(900);
  });
  it("breaks exact ties deterministically (first edge wins)", () => {
    // 左边缘 96 距 c1 左 90 与 c2 左 102 同为 6px：取首个（稳定、可预期）
    const two = [{ x: 90, y: 0, w: 100, h: 100 }, { x: 102, y: 0, w: 100, h: 100 }];
    expect(snapRect({ x: 96, y: 500, w: 100, h: 100 }, two, 8).x).toBe(90);
  });
  it("only moves one edge by at most the threshold", () => {
    // 吸附位移必须 < threshold，且宽高不变
    const snapped = snapRect({ x: 905, y: 500, w: 100, h: 100 }, candidates, 8);
    expect(snapped.x).toBe(900); // 位移 5
    expect(snapped.w).toBe(100);
    expect(snapped.h).toBe(100);
    expect(snapped.y).toBe(500);
  });
});

describe("inBounds", () => {
  it("reports full containment inside bounds", () => {
    expect(inBounds({ x: 0, y: 0, w: 50, h: 50 }, screen)).toBe(true);
    expect(inBounds({ x: 1462, y: 932, w: 50, h: 50 }, screen)).toBe(true); // 右下贴边
    expect(inBounds({ x: -1, y: 0, w: 50, h: 50 }, screen)).toBe(false);
    expect(inBounds({ x: 0, y: 0, w: 1513, h: 50 }, screen)).toBe(false);
    expect(inBounds({ x: 0, y: 933, w: 50, h: 50 }, screen)).toBe(false);
  });
});
