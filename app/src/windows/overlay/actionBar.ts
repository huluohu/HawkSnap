import type { SelRect } from "./useSelection";

// 浮条尺寸缺省常量：仅作「DOM 实测前的初始定位」兜底（index.tsx 在 useLayoutEffect
// 里按 offsetWidth/Height 实测后重定位，绘制前完成、无闪烁）。常量跟不上按钮文案
// （双语、七动作）的真实宽度——v1.1.3 的 BAR_W=252 就这样漂到实际 ~348px，右缘选区
// 的浮条被裁掉 90px；实测化之后这里只是给首帧一个合理近似（七动作约 440px）。
const BAR_W = 440, BAR_H = 40, MARGIN = 12;

// 选区确认浮条定位：默认右对齐选区、置于选区下方 12px；下方放不下（越出视口底）
// 则翻转到选区上方；水平方向夹在视口内（左右各留 MARGIN）。barW/barH 由调用方实测
// 传入（缺省回退常量）——宽度随语言/按钮集变化，写死常量必然漂移。
export function barPosition(
  rect: SelRect,
  viewW: number,
  viewH: number,
  barW: number = BAR_W,
  barH: number = BAR_H,
): { left: number; top: number } {
  const belowY = rect.y + rect.h + MARGIN;
  const top = belowY + barH > viewH ? Math.max(MARGIN, rect.y - barH - MARGIN) : belowY;
  const left = Math.min(
    Math.max(MARGIN, rect.x + rect.w - barW),
    Math.max(MARGIN, viewW - barW - MARGIN),
  );
  return { left, top };
}
