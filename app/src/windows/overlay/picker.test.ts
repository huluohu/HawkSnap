import { describe, expect, it } from "vitest";
import { hexOf, rgbOf, rulerTicks, samplePixel } from "./picker";

// 2×2 测试帧（物理像素，RGBA 字节序）：行 0 = [1,2,3,4 | 5,6,7,8]，行 1 = [9.. | 13..]
const FRAME = new Uint8Array([
  1, 2, 3, 4, 5, 6, 7, 8,
  9, 10, 11, 12, 13, 14, 15, 16,
]).buffer;

describe("samplePixel", () => {
  it("reads the RGBA quad at the physical pixel index", () => {
    expect(samplePixel(FRAME, 2, 0, 0)).toEqual([1, 2, 3, 4]);
    expect(samplePixel(FRAME, 2, 1, 0)).toEqual([5, 6, 7, 8]);
    expect(samplePixel(FRAME, 2, 0, 1)).toEqual([9, 10, 11, 12]);
    expect(samplePixel(FRAME, 2, 1, 1)).toEqual([13, 14, 15, 16]);
  });
  it("rounds fractional cursor coordinates", () => {
    // 缩放屏上 逻辑×scale 可出小数；四舍五入到最近物理像素
    expect(samplePixel(FRAME, 2, 0.4, 0.4)).toEqual([1, 2, 3, 4]);
    expect(samplePixel(FRAME, 2, 0.6, 0.6)).toEqual([13, 14, 15, 16]);
  });
  it("clamps out-of-range coordinates into the buffer", () => {
    // 光标贴窗口右/下缘时 round 可能越界 1px；负坐标（不可能但防御）同夹取
    expect(samplePixel(FRAME, 2, 99, 99)).toEqual([13, 14, 15, 16]);
    expect(samplePixel(FRAME, 2, -3, -3)).toEqual([1, 2, 3, 4]);
  });
  it("returns null for degenerate width or undersized buffers", () => {
    expect(samplePixel(FRAME, 0, 0, 0)).toBeNull();
    expect(samplePixel(FRAME, -2, 0, 0)).toBeNull();
    expect(samplePixel(new ArrayBuffer(0), 2, 0, 0)).toBeNull();
    // 6 字节装不下 w=2 的一行（需 8）→ h=0 → null；w=1 则有一行可采
    const six = new Uint8Array(6).buffer;
    expect(samplePixel(six, 2, 0, 0)).toBeNull();
    expect(samplePixel(six, 1, 0, 0)).toEqual([0, 0, 0, 0]);
  });
});

describe("hexOf", () => {
  it("formats uppercase #RRGGBB", () => {
    expect(hexOf([0, 0, 0])).toBe("#000000");
    expect(hexOf([255, 255, 255])).toBe("#FFFFFF");
    expect(hexOf([16, 32, 64])).toBe("#102040");
    expect(hexOf([171, 205, 239])).toBe("#ABCDEF");
  });
});

describe("rgbOf", () => {
  it("formats the css rgb() label", () => {
    expect(rgbOf([1, 2, 3])).toBe("rgb(1, 2, 3)");
    expect(rgbOf([255, 0, 128])).toBe("rgb(255, 0, 128)");
  });
});

describe("rulerTicks", () => {
  it("emits ticks every 10px with majors every 50px", () => {
    expect(rulerTicks(100)).toEqual([
      { at: 0, major: true }, { at: 10, major: false }, { at: 20, major: false },
      { at: 30, major: false }, { at: 40, major: false }, { at: 50, major: true },
      { at: 60, major: false }, { at: 70, major: false }, { at: 80, major: false },
      { at: 90, major: false }, { at: 100, major: true },
    ]);
  });
  it("stops at the edge length (partial step dropped)", () => {
    expect(rulerTicks(45).map((t) => t.at)).toEqual([0, 10, 20, 30, 40]);
  });
  it("returns nothing below the 20px visibility threshold", () => {
    expect(rulerTicks(19)).toEqual([]);
    expect(rulerTicks(0)).toEqual([]);
    expect(rulerTicks(20).map((t) => t.at)).toEqual([0, 10, 20]);
  });
});
