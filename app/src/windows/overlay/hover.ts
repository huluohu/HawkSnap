// Task 6：hover 帧本地化纯函数（无 React、无 Tauri 依赖，全部可单测）。
//
// Rust hawk://hover 的 payload 坐标系是全局逻辑坐标（top-left 原点，含义同
// hawksnap_platform_mac::ax::AxFrame）；覆盖层画布与选区用 display 本地坐标。
// 全局→本地换算、越界丢弃都在这里做（controller 裁定 5：前端全权负责 per-display
// 过滤，Rust 侧 clip_frame 保持未接线）。

import type { Rect } from "./geometry";

/** hawk://hover 事件 payload（Rust AxFrameJson；单词字段 camelCase rename 是无操作）。 */
export type HoverFrame = { x: number; y: number; w: number; h: number; role: string };

/** toLocalHover 所需的 display 子集（DisplayPayload 的字段，多传无害）。 */
export type HoverDisplay = { x: number; y: number; width: number; height: number };

/**
 * 全局逻辑坐标 hover 帧 → display 本地 rect。
 * probe 为 null（光标下无元素、窗口回退也未命中）→ null（清除高亮）；
 * 帧不完整落在本 display 内（光标在别的屏幕上，或元素跨界）→ null（本屏不高亮，
 * 丢弃而非裁剪——跨屏元素在两屏各画半截反而误导）。
 */
export function toLocalHover(frame: HoverFrame | null, display: HoverDisplay): Rect | null {
  if (!frame) return null;
  const x = frame.x - display.x;
  const y = frame.y - display.y;
  if (x < 0 || y < 0 || x + frame.w > display.width || y + frame.h > display.height) {
    return null;
  }
  return { x, y, w: frame.w, h: frame.h };
}
