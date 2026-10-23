// export.ts（M2 Task M2）：导出栅格化——位图 + 全部形状重绘到**离屏 canvas**（图片
// 自然尺寸 1:1，无 DPR 放大），PNG Blob 交 index.tsx 经 editor_save 写回原路径。
// F5 单路径约束：形状绘制只经 drawShape（与预览同一函数、同一 bitmap 参数）——本
// 文件不含任何逐形状绘制代码，仅负责「建画布 → 画底图 → 逐形状 drawShape → 编码」。
import { drawShape, type EditorBitmap } from "./draw";
import type { Shape } from "./shapes";

/** 离屏画布工厂（测试注入桩；默认 document.createElement）。 */
export type CanvasFactory = () => HTMLCanvasElement;

export function defaultCanvasFactory(): HTMLCanvasElement {
  return document.createElement("canvas");
}

/**
 * 栅格化导出：底图按 (0,0,w,h) 铺满（调用方传图片自然尺寸）→ 逐形状 drawShape
 * → toBlob("image/png")。toBlob 回调给 null 时 reject（编码失败）。
 */
export async function exportToPng(
  bitmap: EditorBitmap | null,
  shapes: Shape[],
  w: number,
  h: number,
  createCanvas: CanvasFactory = defaultCanvasFactory,
): Promise<Blob> {
  const canvas = createCanvas();
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("无法创建离屏画布上下文");
  if (bitmap) ctx.drawImage(bitmap, 0, 0, w, h);
  for (const s of shapes) drawShape(ctx, s, bitmap);
  return await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("PNG 编码失败"))), "image/png");
  });
}
