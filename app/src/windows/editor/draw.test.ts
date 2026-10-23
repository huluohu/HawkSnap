import { describe, expect, it } from "vitest";
import { bitmapSize, drawMosaic, drawShape, type EditorBitmap, type TempCanvasFactory } from "./draw";
import type { Shape } from "./shapes";

// 伪 ctx：记录方法调用序 + 直接读属性断言（jsdom 无 2D 上下文——绘制语义以
// 「对 ctx 发出的原语序列」为单位断言；像素级一致性由浏览器人工验收单覆盖）。
interface FakeCtx {
  canvas: { width: number; height: number };
  imageSmoothingEnabled: boolean;
  /** 每次 drawImage 调用时刻的 imageSmoothingEnabled 快照（马赛克硬边断言用）。 */
  smoothingAtDraw: boolean[];
  [prop: string]: unknown;
}

function fakeCtx(): { ctx: FakeCtx; ops: Array<{ op: string; args: unknown[] }> } {
  const ops: Array<{ op: string; args: unknown[] }> = [];
  const methods = [
    "beginPath", "moveTo", "lineTo", "stroke", "fill", "arc", "ellipse",
    "fillRect", "strokeRect", "fillText", "drawImage", "clearRect",
  ];
  const ctx: FakeCtx = {
    canvas: { width: 0, height: 0 },
    imageSmoothingEnabled: true,
    smoothingAtDraw: [],
  };
  for (const m of methods) {
    if (m === "drawImage") {
      ctx.drawImage = (...args: unknown[]) => {
        ctx.smoothingAtDraw.push(ctx.imageSmoothingEnabled);
        ops.push({ op: m, args });
      };
    } else {
      ctx[m] = (...args: unknown[]) => ops.push({ op: m, args });
    }
  }
  return { ctx, ops };
}

/** 临时画布桩：width/height 可写，getContext 返回独立伪 ctx。 */
function fakeTempCanvas() {
  const { ctx: tctx, ops: tops } = fakeCtx();
  const tmp = { width: 0, height: 0, getContext: () => tctx };
  return { tmp, tctx, tops };
}

// 伪位图：无 naturalWidth（走 width/height 分支）；img 桩带 naturalWidth。
const fakeBitmap = { width: 100, height: 80 } as unknown as EditorBitmap;
const fakeImg = { naturalWidth: 200, naturalHeight: 120, width: 0, height: 0 } as unknown as EditorBitmap;

const rect: Shape = { kind: "rect", x: 30, y: 40, w: -20, h: -10, color: "#f00", width: 2 };
const highlighter: Shape = {
  kind: "highlighter",
  points: [{ x: 5, y: 5 }, { x: 25, y: 5 }],
  color: "#ff0", width: 8,
};
const text: Shape = { kind: "text", x: 7, y: 11, text: "hi", color: "#123", size: 16 };
const step: Shape = { kind: "step", x: 50, y: 60, n: 12, color: "#E5484D" };
const mosaic: Shape = { kind: "mosaic", x: 10, y: 10, w: 40, h: 20, blockSize: 10 };

describe("bitmapSize", () => {
  it("prefers naturalWidth for HTMLImageElement-like sources", () => {
    expect(bitmapSize(fakeImg)).toEqual({ w: 200, h: 120 });
    expect(bitmapSize(fakeBitmap)).toEqual({ w: 100, h: 80 });
  });
});

describe("drawShape", () => {
  it("rect: normalizes the box and sets stroke style on the ctx", () => {
    const { ctx, ops } = fakeCtx();
    drawShape(ctx as unknown as CanvasRenderingContext2D, rect);
    expect(ctx.strokeStyle).toBe("#f00");
    expect(ctx.lineWidth).toBe(2);
    // 反向拖拽的负宽高折正后画
    expect(ops[0]).toEqual({ op: "strokeRect", args: [10, 30, 20, 10] });
  });

  it("highlighter: paints at alpha 0.4 and restores to 1", () => {
    const { ctx, ops } = fakeCtx();
    drawShape(ctx as unknown as CanvasRenderingContext2D, highlighter);
    expect(ctx.globalAlpha).toBe(1); // 用毕还原
    expect(ops.filter((o) => o.op === "moveTo" || o.op === "lineTo")).toHaveLength(2);
    expect(ops[ops.length - 1]?.op).toBe("stroke");
  });

  it("text: sets left/top baseline and fills the string", () => {
    const { ctx, ops } = fakeCtx();
    drawShape(ctx as unknown as CanvasRenderingContext2D, text);
    expect(ctx.font).toBe("16px sans-serif");
    expect(ctx.fillStyle).toBe("#123");
    expect(ctx.textAlign).toBe("left");
    expect(ctx.textBaseline).toBe("top");
    expect(ops[ops.length - 1]).toEqual({ op: "fillText", args: ["hi", 7, 11] });
  });

  it("step: fills a disc in the shape color and the number in white, centered", () => {
    const { ctx, ops } = fakeCtx();
    drawShape(ctx as unknown as CanvasRenderingContext2D, step);
    // n=12 → 半径 23；圆心 (50,60)
    const arc = ops.find((o) => o.op === "arc");
    expect(arc?.args).toEqual([50, 60, 23, 0, Math.PI * 2]);
    expect(ops.filter((o) => o.op === "arc")).toHaveLength(1);
    expect(ctx.fillStyle).toBe("#FFFFFF");
    expect(ctx.textAlign).toBe("center");
    expect(ctx.textBaseline).toBe("middle");
    expect(ops[ops.length - 1]).toEqual({ op: "fillText", args: ["12", 50, 60] });
    expect(ctx.font).toBe("bold 21px sans-serif"); // round(23*0.9)
  });

  it("step text state does not leak into a following text shape (explicit per-shape state)", () => {
    const { ctx, ops } = fakeCtx();
    const c = ctx as unknown as CanvasRenderingContext2D;
    drawShape(c, step);
    drawShape(c, text);
    expect(ctx.textAlign).toBe("left");
    expect(ctx.textBaseline).toBe("top");
    expect(ops[ops.length - 1]).toEqual({ op: "fillText", args: ["hi", 7, 11] });
  });

  it("non-mosaic shapes ignore the bitmap argument (same ops with and without)", () => {
    const a = fakeCtx();
    const b = fakeCtx();
    drawShape(a.ctx as unknown as CanvasRenderingContext2D, rect, fakeBitmap);
    drawShape(b.ctx as unknown as CanvasRenderingContext2D, rect);
    expect(a.ops).toEqual(b.ops);
  });
});

describe("drawMosaic", () => {
  it("downsamples the bitmap region on the temp canvas and upscales blocky on the target", () => {
    const main = fakeCtx();
    const { tmp, tctx, tops } = fakeTempCanvas();
    const factory: TempCanvasFactory = () => tmp as unknown as HTMLCanvasElement;
    drawMosaic(main.ctx as unknown as CanvasRenderingContext2D, mosaic, fakeBitmap, factory);
    // 临时画布尺寸 = 区域 / blockSize（(40,20)/10 → 4×2 块）
    expect(tmp.width).toBe(4);
    expect(tmp.height).toBe(2);
    // 降采样在临时 ctx：源区 (10,10,40,20) → 目标 (0,0,4,2)
    expect(tops).toHaveLength(1);
    expect(tops[0].op).toBe("drawImage");
    expect(tops[0].args).toEqual([fakeBitmap, 10, 10, 40, 20, 0, 0, 4, 2]);
    // 放大在目标 ctx：临时画布整体 → 原区域，期间 smoothing 关闭、用毕还原
    expect(main.ops).toHaveLength(1);
    expect(main.ops[0].op).toBe("drawImage");
    expect(main.ops[0].args).toEqual([tmp, 0, 0, 4, 2, 10, 10, 40, 20]);
    expect(main.ctx.smoothingAtDraw).toEqual([false]);
    expect(main.ctx.imageSmoothingEnabled).toBe(true);
    expect(tctx.imageSmoothingEnabled).toBe(true);
  });

  it("clamps the sampled region to the bitmap bounds", () => {
    const main = fakeCtx();
    const { tmp, tops } = fakeTempCanvas();
    // 区域左上在位图外（-10,-5），右下 (30,15) 在内 → 源区钳到 (0,0,30,15)
    const out: Shape = { kind: "mosaic", x: -10, y: -5, w: 40, h: 20, blockSize: 10 };
    drawMosaic(main.ctx as unknown as CanvasRenderingContext2D, out, fakeBitmap, () => tmp as unknown as HTMLCanvasElement);
    expect(tmp.width).toBe(3);
    expect(tmp.height).toBe(2);
    expect(tops[0].args).toEqual([fakeBitmap, 0, 0, 30, 15, 0, 0, 3, 2]);
    expect(main.ops[0].args).toEqual([tmp, 0, 0, 3, 2, -10, -5, 40, 20]);
  });

  it("fully outside the bitmap draws nothing", () => {
    const main = fakeCtx();
    const out: Shape = { kind: "mosaic", x: 500, y: 500, w: 40, h: 20, blockSize: 10 };
    drawMosaic(main.ctx as unknown as CanvasRenderingContext2D, out, fakeBitmap, () => null);
    expect(main.ops).toHaveLength(0);
  });

  it("null temp canvas factory aborts without drawing (defensive)", () => {
    const main = fakeCtx();
    drawMosaic(main.ctx as unknown as CanvasRenderingContext2D, mosaic, fakeBitmap, () => null);
    expect(main.ops).toHaveLength(0);
  });

  it("drawShape routes mosaic through the same path with the default factory (bitmap present)", () => {
    const main = fakeCtx();
    // 默认工厂走 document.createElement——jsdom 的 createElement 可用但 getContext 为 null
    // → tmp 存在、tctx 为 null → 中止不画；此 case 只断言「不炸 + 无形状原语落到主 ctx」。
    drawShape(main.ctx as unknown as CanvasRenderingContext2D, mosaic, fakeBitmap);
    expect(main.ops).toHaveLength(0);
  });

  it("drawShape without a bitmap falls back to a translucent gray block", () => {
    const main = fakeCtx();
    drawShape(main.ctx as unknown as CanvasRenderingContext2D, mosaic, null);
    expect(main.ctx.fillStyle).toBe("rgba(127,127,127,0.5)");
    expect(main.ops).toHaveLength(1);
    expect(main.ops[0]).toEqual({ op: "fillRect", args: [10, 10, 40, 20] });
  });
});
