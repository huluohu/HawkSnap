// draw.ts（M2 Task M2）：**预览与导出的唯一绘制路径**（F5 单路径约束）。
// EditorCanvas 的矢量层重绘与 export.ts 的离屏栅格化都只经 drawShape——每个形状的
// ctx 状态设置（strokeStyle/lineWidth/font/globalAlpha/textAlign）全部收进本函数，
// 调用方不做任何逐形状绘制。马赛克需要底图像素 → drawShape 增加可选 bitmap 参数；
// 预览与导出传入同一张位图，保证两条路径像素一致。
//
// 注：选择框（selection chrome）是编辑器 UI，不是形状——不经过 drawShape、不进导出
// （导出的是用户标注结果，不含编辑态装饰）。
import { bboxOf, stepRadius, type MeasureFn, type MosaicShape, type Shape } from "./shapes";

/** 马赛克/导出可用的位图来源（drawImage 接受的子集；HTMLImageElement 优先 naturalWidth）。 */
export type EditorBitmap = HTMLImageElement | HTMLCanvasElement | ImageBitmap | OffscreenCanvas;

/** 马赛克降采样用的临时画布工厂（默认 document.createElement；测试注入桩）。 */
export type TempCanvasFactory = () => HTMLCanvasElement | null;

/** 位图尺寸：HTMLImageElement 走 naturalWidth/Height（width 是布局属性，不可靠）。 */
export function bitmapSize(b: EditorBitmap): { w: number; h: number } {
  if ("naturalWidth" in b) {
    return { w: b.naturalWidth || b.width, h: b.naturalHeight || b.height };
  }
  return { w: b.width, h: b.height };
}

/** ctx.measureText 的 MeasureFn 注入（EditorCanvas 命中测试用；font 按字号设置）。 */
export function canvasMeasure(ctx: CanvasRenderingContext2D): MeasureFn {
  return (text, size) => {
    ctx.font = `${size}px sans-serif`;
    return { width: ctx.measureText(text).width, height: size * 1.2 };
  };
}

/**
 * 马赛克渲染：底图区域按 blockSize 分块像素化。
 * 路径（预览/导出同款）：位图区域 → 临时画布降到 ~1/blockSize 尺寸 → 关平滑放大回
 * 原尺寸贴回。临时画布由工厂注入（jsdom/测试可桩）；位图缺席时半透明灰块兜底
 * （理论不可达：绘制前必已装载位图——保留以防御并让无图测试可断言）。
 * 源矩形与位图边界求交（负源坐标/越界的 drawImage 行为跨浏览器不一致）。
 */
export function drawMosaic(
  ctx: CanvasRenderingContext2D,
  shape: MosaicShape,
  bitmap: EditorBitmap | null,
  newTempCanvas: TempCanvasFactory,
): void {
  const b = bboxOf(shape);
  if (!bitmap) {
    ctx.fillStyle = "rgba(127,127,127,0.5)";
    ctx.fillRect(b.x, b.y, b.w, b.h);
    return;
  }
  const blockSize = Math.max(2, Math.round(shape.blockSize));
  const { w: bw, h: bh } = bitmapSize(bitmap);
  const x0 = Math.max(0, b.x);
  const y0 = Math.max(0, b.y);
  const x1 = Math.min(bw, b.x + b.w);
  const y1 = Math.min(bh, b.y + b.h);
  const sw = x1 - x0;
  const sh = y1 - y0;
  if (sw <= 0 || sh <= 0) return; // 完全在位图外：无可采样区域
  const cols = Math.max(1, Math.round(sw / blockSize));
  const rows = Math.max(1, Math.round(sh / blockSize));
  const tmp = newTempCanvas();
  if (!tmp) return;
  tmp.width = cols;
  tmp.height = rows;
  const tctx = tmp.getContext("2d");
  if (!tctx) return;
  tctx.drawImage(bitmap, x0, y0, sw, sh, 0, 0, cols, rows); // 降采样（每块取均值色）
  const prev = ctx.imageSmoothingEnabled;
  ctx.imageSmoothingEnabled = false; // 放大关平滑 = 硬边像素块
  ctx.drawImage(tmp, 0, 0, cols, rows, b.x, b.y, b.w, b.h);
  ctx.imageSmoothingEnabled = prev;
}

function defaultTempCanvas(): HTMLCanvasElement {
  return document.createElement("canvas");
}

/**
 * 单一形状绘制（F5：预览与导出的唯一路径）。逐形状完整设置所需 ctx 状态
 * （不改全局假设；highlighter/马赛克的 globalAlpha/imageSmoothing 用毕还原）。
 * bitmap 供马赛克采样（其余形状忽略；预览与导出必须传同一张）。
 */
export function drawShape(
  ctx: CanvasRenderingContext2D,
  shape: Shape,
  bitmap?: EditorBitmap | null,
): void {
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  switch (shape.kind) {
    case "rect": {
      const b = bboxOf(shape);
      ctx.strokeStyle = shape.color;
      ctx.lineWidth = shape.width;
      ctx.strokeRect(b.x, b.y, b.w, b.h);
      break;
    }
    case "ellipse": {
      const b = bboxOf(shape);
      ctx.strokeStyle = shape.color;
      ctx.lineWidth = shape.width;
      ctx.beginPath();
      ctx.ellipse(b.x + b.w / 2, b.y + b.h / 2, b.w / 2, b.h / 2, 0, 0, Math.PI * 2);
      ctx.stroke();
      break;
    }
    case "arrow": {
      ctx.strokeStyle = shape.color;
      ctx.lineWidth = shape.width;
      ctx.beginPath();
      ctx.moveTo(shape.x1, shape.y1);
      ctx.lineTo(shape.x2, shape.y2);
      ctx.stroke();
      // 箭头头部：两翼沿主轴回折 ~30°（hitTest 的翼命中域与此同参）
      const ang = Math.atan2(shape.y2 - shape.y1, shape.x2 - shape.x1);
      const head = Math.max(10, shape.width * 4);
      for (const da of [Math.PI - 0.5, Math.PI + 0.5]) {
        ctx.beginPath();
        ctx.moveTo(shape.x2, shape.y2);
        ctx.lineTo(shape.x2 + head * Math.cos(ang + da), shape.y2 + head * Math.sin(ang + da));
        ctx.stroke();
      }
      break;
    }
    case "pen":
    case "highlighter": {
      ctx.globalAlpha = shape.kind === "highlighter" ? 0.4 : 1;
      ctx.strokeStyle = shape.color;
      ctx.lineWidth = shape.width;
      ctx.beginPath();
      const pts = shape.points;
      ctx.moveTo(pts[0].x, pts[0].y);
      for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i].x, pts[i].y);
      if (pts.length === 1) ctx.lineTo(pts[0].x + 0.01, pts[0].y); // 单点也出墨
      ctx.stroke();
      ctx.globalAlpha = 1;
      break;
    }
    case "text": {
      ctx.fillStyle = shape.color;
      ctx.font = `${shape.size}px sans-serif`;
      ctx.textAlign = "left";
      ctx.textBaseline = "top";
      ctx.fillText(shape.text, shape.x, shape.y);
      break;
    }
    case "mosaic": {
      drawMosaic(ctx, shape, bitmap ?? null, defaultTempCanvas);
      break;
    }
    case "step": {
      const r = stepRadius(shape.n);
      ctx.fillStyle = shape.color;
      ctx.beginPath();
      ctx.arc(shape.x, shape.y, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = "#FFFFFF";
      ctx.font = `bold ${Math.round(r * 0.9)}px sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(String(shape.n), shape.x, shape.y);
      break;
    }
  }
}
