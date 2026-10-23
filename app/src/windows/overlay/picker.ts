// M2 Task M5：取色器 + 像素标尺的纯逻辑（无 React、无 Tauri 依赖，全部可单测）。
//
// 采样路径零 IPC（plan 约束）：冻结帧原始物理 RGBA 已在覆盖层前端
// （index.tsx 经 fetchRaw 装入 frameBufRef），samplePixel 直接按 (y*w+x)*4
// 索引字节——不经过任何 invoke。

export type Rgb = [number, number, number];
/** RGBA 四元组（0-255）。alpha 原样返回；HEX 不含 alpha（冻结帧恒不透明）。 */
export type Rgba = [number, number, number, number];

/**
 * 从原始 RGBA 缓冲取物理像素 (x, y) 的颜色。
 * 坐标先取整再夹取进缓冲边界（光标可停在窗口最右/最下缘，逻辑×scale 四舍五入后
 * 可能越界 1px）；w 非法或缓冲装不下完整一行返回 null（调用方按 null 跳过浮签更新，
 * 绝不把脏字节当颜色展示）。字节序按 RGBA（capture_screens 冻结帧口径，与
 * putImageData 同源）。
 */
export function samplePixel(buf: ArrayBuffer, w: number, x: number, y: number): Rgba | null {
  if (!(w > 0)) return null;
  const bytes = new Uint8Array(buf);
  const h = Math.floor(bytes.length / (w * 4));
  if (h <= 0) return null;
  const px = Math.min(Math.max(Math.round(x), 0), w - 1);
  const py = Math.min(Math.max(Math.round(y), 0), h - 1);
  const i = (py * w + px) * 4;
  return [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]];
}

/** rgb → "#RRGGBB"（大写；点击复制的就是这个串）。 */
export function hexOf(rgb: Rgb): string {
  return "#" + rgb.map((v) => v.toString(16).padStart(2, "0")).join("").toUpperCase();
}

/** rgb → "rgb(r, g, b)"（浮签第二行）。 */
export function rgbOf(rgb: Rgb): string {
  return `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
}

/**
 * 像素标尺刻度（选区一条边内的偏移序列）：从 0 起每 10px 一格，每 50px 为长刻度
 * （major）。len 为选区边长（逻辑 px）；len < 20 返回空——过短的选区画刻度只剩噪音，
 * 调用方（index.tsx 重绘 effect）据此整体跳过上下两组刻度。
 */
export function rulerTicks(len: number): { at: number; major: boolean }[] {
  if (len < 20) return [];
  const out: { at: number; major: boolean }[] = [];
  for (let at = 0; at <= len; at += 10) out.push({ at, major: at % 50 === 0 });
  return out;
}
