import { describe, expect, it } from "vitest";
import { exportToPng, type CanvasFactory } from "./export";
import type { EditorBitmap } from "./draw";
import type { Shape } from "./shapes";

// 离屏栅格化的契约测试（jsdom 无 2D）：以「伪画布收到的调用序」断言——底图先画、
// 形状逐个 drawShape（马赛克拿到同一 bitmap）、toBlob 以 "image/png" 调用。
// 真正的像素一致性（F5）由浏览器人工验收单覆盖（报告附 checklist）。

function fakeCtx() {
  const ops: Array<{ op: string; args: unknown[] }> = [];
  const methods = [
    "beginPath", "moveTo", "lineTo", "stroke", "fill", "arc", "ellipse",
    "fillRect", "strokeRect", "fillText", "drawImage", "clearRect",
  ];
  const ctx: Record<string, unknown> = {
    canvas: { width: 0, height: 0 },
    imageSmoothingEnabled: true,
  };
  for (const m of methods) ctx[m] = (...args: unknown[]) => ops.push({ op: m, args });
  return { ctx, ops };
}

const bitmap = { naturalWidth: 200, naturalHeight: 120 } as unknown as EditorBitmap;
const rect: Shape = { kind: "rect", x: 10, y: 20, w: 30, h: 40, color: "#f00", width: 2 };
const step: Shape = { kind: "step", x: 50, y: 60, n: 1, color: "#00f" };

function makeFactory(canvasRef: { width: number; height: number }, ctx: Record<string, unknown>, blob: Blob | null, toBlobType: { v: string | null }): CanvasFactory {
  return () => {
    // width/height 用访问器属性写入 canvasRef（exportToPng 直接对 canvas 赋值）
    const canvas = {
      getContext: () => ctx,
      toBlob: (cb: (b: Blob | null) => void, type?: string) => {
        toBlobType.v = type ?? null;
        cb(blob);
      },
    };
    Object.defineProperty(canvas, "width", {
      get: () => canvasRef.width,
      set: (v: number) => { canvasRef.width = v; },
    });
    Object.defineProperty(canvas, "height", {
      get: () => canvasRef.height,
      set: (v: number) => { canvasRef.height = v; },
    });
    return canvas as unknown as HTMLCanvasElement;
  };
}

describe("exportToPng", () => {
  it("sets canvas size, draws the bitmap first, then every shape via drawShape (F5 single path)", async () => {
    const dims = { width: 0, height: 0 };
    const { ctx, ops } = fakeCtx();
    const blob = {} as Blob;
    const toBlobType = { v: null as string | null };
    const out = await exportToPng(bitmap, [rect, step], 200, 120, makeFactory(dims, ctx, blob, toBlobType));
    expect(out).toBe(blob);
    expect(dims).toEqual({ width: 200, height: 120 });
    // 底图最先画：drawImage(bitmap, 0, 0, w, h)
    expect(ops[0]).toEqual({ op: "drawImage", args: [bitmap, 0, 0, 200, 120] });
    // 形状原语跟在后面：rect 的 strokeRect、step 的 arc+fillText
    expect(ops.some((o) => o.op === "strokeRect" && o.args[0] === 10)).toBe(true);
    expect(ops.some((o) => o.op === "fillText" && o.args[0] === "1")).toBe(true);
    expect(toBlobType.v).toBe("image/png");
  });

  it("draws the same bitmap instance the preview uses (mosaic samples one source)", async () => {
    const { ctx, ops } = fakeCtx();
    const factory: CanvasFactory = () => ({
      width: 0,
      height: 0,
      getContext: () => ctx,
      toBlob: (cb: (b: Blob | null) => void) => cb({} as Blob),
    } as unknown as HTMLCanvasElement);
    await exportToPng(bitmap, [rect], 200, 120, factory);
    expect(ops[0].args[0]).toBe(bitmap); // 底图 = drawShape 收到的同一位图实例
    // 注：马赛克「临时画布降采样→放大」的两段 drawImage 同源断言在 draw.test.ts
    // （jsdom 的 document.createElement canvas getContext 为 null，故此处用无 mosaic 组）。
    expect(ops.filter((o) => o.op === "drawImage")).toHaveLength(1);
  });

  it("resolves null-bitmap exports (shapes only) and rejects when toBlob yields null", async () => {
    const { ctx, ops } = fakeCtx();
    const factory: CanvasFactory = () => ({
      width: 0,
      height: 0,
      getContext: () => ctx,
      toBlob: (cb: (b: Blob | null) => void) => cb({} as Blob),
    } as unknown as HTMLCanvasElement);
    await expect(exportToPng(null, [rect], 100, 100, factory)).resolves.toBeTruthy();
    expect(ops.filter((o) => o.op === "drawImage")).toHaveLength(0); // 无底图

    const nullFactory: CanvasFactory = () => ({
      width: 0,
      height: 0,
      getContext: () => ctx,
      toBlob: (cb: (b: Blob | null) => void) => cb(null),
    } as unknown as HTMLCanvasElement);
    await expect(exportToPng(bitmap, [rect], 100, 100, nullFactory)).rejects.toThrow("PNG 编码失败");
  });

  it("rejects when the 2d context is unavailable", async () => {
    const factory: CanvasFactory = () => ({
      width: 0,
      height: 0,
      getContext: () => null,
      toBlob: () => undefined,
    } as unknown as HTMLCanvasElement);
    await expect(exportToPng(bitmap, [], 10, 10, factory)).rejects.toThrow("无法创建离屏画布上下文");
  });
});
