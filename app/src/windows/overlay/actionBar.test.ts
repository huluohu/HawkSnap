import { describe, expect, it } from "vitest";
import { barPosition } from "./actionBar";

describe("barPosition", () => {
  it("places below the rect normally (explicit measured width)", () => {
    // 右对齐选区（左缘 = 选区右缘 300 - 浮条宽 252 = 48）、下方 12px。
    // 注：brief 原稿期望 180 与其自身实现常量 BAR_W=252 矛盾（180 仅当 BAR_W=120），
    // 以实现常量为准修正期望值；测试实质（右对齐 + 下方 12px）不变。
    // v1.2 起宽度由调用方按 DOM 实测传入（见 actionBar.ts 头注——常量跟不上双语
    // 七动作的真实宽度，252 漂移到 348 曾把右缘选区的浮条裁掉 90px），本用例显式
    // 传 252 保持原语义回归。
    expect(barPosition({ x: 100, y: 100, w: 200, h: 100 }, 1512, 982, 252, 40))
      .toEqual({ left: 48, top: 212 });
  });
  it("uses the default width only as the pre-measure fallback", () => {
    // 缺省宽度（实测前的兜底常量 440）：右缘 300 处按右对齐会越出左边界 → 夹取到 MARGIN。
    expect(barPosition({ x: 100, y: 100, w: 200, h: 100 }, 1512, 982))
      .toEqual({ left: 12, top: 212 });
  });
  it("flips above when the bar would overflow the bottom", () => {
    const p = barPosition({ x: 100, y: 900, w: 200, h: 70 }, 1512, 982);
    expect(p.top).toBeLessThan(900);
  });
  it("clamps left within the view", () => {
    const p = barPosition({ x: 10, y: 100, w: 20, h: 20 }, 1512, 982);
    expect(p.left).toBeGreaterThanOrEqual(12);
  });
});
