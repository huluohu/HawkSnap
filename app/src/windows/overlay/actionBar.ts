import type { SelRect } from "./useSelection";

// 浮条尺寸常量：与 index.tsx 的实际样式（padding/border/gap 折算）保持一致，
// 供定位计算使用；改动样式时须同步这里。
const BAR_W = 252, BAR_H = 40, MARGIN = 12;

// 选区确认浮条定位：默认右对齐选区、置于选区下方 12px；下方放不下（越出视口底）
// 则翻转到选区上方；水平方向夹在视口内（左右各留 MARGIN）。
export function barPosition(rect: SelRect, viewW: number, viewH: number): { left: number; top: number } {
  const belowY = rect.y + rect.h + MARGIN;
  const top = belowY + BAR_H > viewH ? Math.max(MARGIN, rect.y - BAR_H - MARGIN) : belowY;
  const left = Math.min(
    Math.max(MARGIN, rect.x + rect.w - BAR_W),
    Math.max(MARGIN, viewW - BAR_W - MARGIN),
  );
  return { left, top };
}
