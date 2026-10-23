// 编辑器画布（M2 Task M2）：位图底（随图片装载绘制）+ 矢量层 + 选择态。
// 指针流（按工具分派）：
// - 绘制工具：down 建草稿 → move 更新 → up 校验并提交（过小/过短的形状丢弃）；
//   一次拖拽 = 一次提交（up 时）。
// - 选择工具：down 命中测试（topmostShapeAt，描边带语义）→ 选中；按住拖动 =
//   移动预览（本地 dx/dy state，重绘层对选中形状套 translate）→ up 提交一次
//   onMoveShape（拖拽全程只入一档历史）；点空白 = 取消选中。
// - 马赛克：同 rect 拖拽（blockSize 固定 10px @自然尺度）；序号气泡：点击落位，
//   n = nextStepNumber(shapes)（创建时计算并存入形状，删除/撤销自然回落）。
// - 文本：点击落锚开内联输入（Enter/blur 提交、Esc 取消）；双击已提交文本形状
//   （选择工具下）重新打开输入，提交 = onReplaceText 替换（一档历史）。
//   工具切换时**结转**未提交文本（M1 遗留修复：原实现静默丢弃）。
// 绘制路径（F5）：矢量层重绘只经 drawShape(ctx, shape, bitmap)；选择框是编辑器
// UI，不进 drawShape、不进导出。bitmap 与导出同源（index.tsx 持有的 Image 元素）。
import { useCallback, useEffect, useRef, useState } from "react";
import { canvasMeasure, drawShape } from "./draw";
import {
  bboxOf,
  hitTest,
  nextStepNumber,
  topmostShapeAt,
  translate,
  withColor,
  withSize,
  withWidth,
  type MeasureFn,
  type Shape,
} from "./shapes";
import { t } from "../../lib/i18n";

export type Tool =
  | "select" | "rect" | "ellipse" | "arrow" | "pen" | "highlighter"
  | "mosaic" | "step" | "text";

export interface EditorStyle { color: string; width: number; size: number }

/** 马赛克块尺寸（px @图片自然尺度）——计划口径 ~10px。 */
export const MOSAIC_BLOCK_SIZE = 10;

interface EditorCanvasProps {
  /** 位图（index.tsx 装载的 Image 元素；马赛克采样与导出同源）。 */
  image: HTMLImageElement | null;
  shapes: Shape[];
  tool: Tool;
  style: EditorStyle;
  /** 选中形状下标（index.tsx 持有；删除/样式改按此定位）。 */
  selected: number | null;
  /** 选中形状的样式实时预览（滑杆拖动期间未提交）：非 null 时按此覆盖选中形状样式。 */
  previewStyle: EditorStyle | null;
  onSelect: (index: number | null) => void;
  onCommit: (s: Shape) => void;
  /** 拖动选中形状（pointerup 一次一档历史）。 */
  onMoveShape: (index: number, dx: number, dy: number) => void;
  /** 双击文本编辑提交（替换原形状文本）。 */
  onReplaceText: (index: number, text: string) => void;
}

interface Pt { x: number; y: number }

/** 选中形状样式实时预览：按工具栏当前样式覆盖（形状缺对应字段时为 no-op）。 */
function applyPreviewStyle(s: Shape, st: EditorStyle): Shape {
  return withColor(withWidth(withSize(s, st.size), st.width), st.color);
}

/** 选择框（编辑器 UI chrome）：虚线包围盒 + 8 把手。不进导出。 */
function drawSelection(ctx: CanvasRenderingContext2D, b: { x: number; y: number; w: number; h: number }): void {
  const HANDLE = 8;
  ctx.save();
  ctx.strokeStyle = "#4aa3ff";
  ctx.lineWidth = 1;
  ctx.setLineDash([4, 3]);
  ctx.strokeRect(b.x - 0.5, b.y - 0.5, b.w + 1, b.h + 1);
  ctx.setLineDash([]);
  ctx.fillStyle = "#ffffff";
  const xs = [b.x, b.x + b.w / 2, b.x + b.w];
  const ys = [b.y, b.y + b.h / 2, b.y + b.h];
  for (const hx of xs) {
    for (const hy of ys) {
      ctx.fillRect(hx - HANDLE / 2, hy - HANDLE / 2, HANDLE, HANDLE);
      ctx.strokeRect(hx - HANDLE / 2, hy - HANDLE / 2, HANDLE, HANDLE);
    }
  }
  ctx.restore();
}

export default function EditorCanvas({
  image, shapes, tool, style, selected, previewStyle,
  onSelect, onCommit, onMoveShape, onReplaceText,
}: EditorCanvasProps) {
  const baseRef = useRef<HTMLCanvasElement>(null);
  const vecRef = useRef<HTMLCanvasElement>(null);
  // 内层 stack div（文本内联输入的包含块；评审修复 1）
  const stackRef = useRef<HTMLDivElement>(null);
  // 绘制草稿：state 驱动矢量层实时预览（每 move 全量重绘——M2 规模仍可接受：
  // 重绘成本 ~几十个形状的矢量指令；马赛克的临时画布降采样只在含 mosaic 时发生）
  const [draft, setDraft] = useState<Shape | null>(null);
  // 移动草稿：{ 选中下标, 起点, 累计位移 }（up 时一次提交）
  const [moveDraft, setMoveDraft] = useState<{ index: number; start: Pt; dx: number; dy: number } | null>(null);
  // 文本内联输入：落锚 + 草稿值 + 编辑目标（editIndex=null = 新建）
  const [textDraft, setTextDraft] = useState<{ x: number; y: number; value: string; editIndex: number | null } | null>(null);
  const textInputRef = useRef<HTMLInputElement>(null);
  const dragRef = useRef(false);
  // 最新文本草稿镜像（effect/事件回调读取用，规避闭包过期）
  const textDraftRef = useRef(textDraft);
  textDraftRef.current = textDraft;

  // 位图底：image 变化（每张一次）后按自然尺寸铺满 base canvas，并同步矢量层尺寸。
  useEffect(() => {
    const base = baseRef.current;
    const vec = vecRef.current;
    if (!base || !vec) return;
    if (!image) {
      // 无图兜底：小占位画布（清掉旧图残影）
      base.width = 0;
      base.height = 0;
      vec.width = 0;
      vec.height = 0;
      return;
    }
    base.width = image.naturalWidth;
    base.height = image.naturalHeight;
    vec.width = image.naturalWidth;
    vec.height = image.naturalHeight;
    const ctx = base.getContext("2d");
    if (ctx) {
      ctx.clearRect(0, 0, base.width, base.height);
      ctx.drawImage(image, 0, 0);
    }
  }, [image]);

  // 换图即清草稿/移动/文本编辑（旧图的未提交输入不能提交进新图会话）。
  useEffect(() => {
    setDraft(null);
    setMoveDraft(null);
    setTextDraft(null);
  }, [image]);

  // 矢量层重绘：shapes/draft/移动/选中态任一变化即全量重绘。F5：形状只经 drawShape
  // （与导出同一函数、同一 bitmap）；选择框紧随其后（仅预览层）。
  useEffect(() => {
    const vec = vecRef.current;
    if (!vec) return;
    const ctx = vec.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, vec.width, vec.height);
    const measure = canvasMeasure(ctx);
    for (let i = 0; i < shapes.length; i++) {
      let s = shapes[i];
      if (moveDraft && i === moveDraft.index) s = translate(s, moveDraft.dx, moveDraft.dy);
      else if (i === selected && previewStyle) s = applyPreviewStyle(s, previewStyle);
      drawShape(ctx, s, image);
    }
    if (draft) drawShape(ctx, draft, image);
    if (selected != null && shapes[selected]) {
      const s = moveDraft && moveDraft.index === selected
        ? translate(shapes[selected], moveDraft.dx, moveDraft.dy)
        : shapes[selected];
      drawSelection(ctx, bboxOf(s, measure));
    }
  }, [shapes, draft, moveDraft, selected, previewStyle, image]);

  /** 事件坐标 → 画布物理坐标（CSS 缩放比换算）。 */
  const toCanvas = useCallback((e: { clientX: number; clientY: number }): Pt => {
    const vec = vecRef.current;
    if (!vec) return { x: 0, y: 0 };
    const rect = vec.getBoundingClientRect();
    const sx = vec.width / (rect.width || 1);
    const sy = vec.height / (rect.height || 1);
    return { x: (e.clientX - rect.left) * sx, y: (e.clientY - rect.top) * sy };
  }, []);

  /** 命中测试量测（文本 bbox 需 ctx.measureText；jsdom/无 ctx 时回退估算）。 */
  const getMeasure = useCallback((): MeasureFn | undefined => {
    const ctx = vecRef.current?.getContext("2d");
    return ctx ? canvasMeasure(ctx) : undefined;
  }, []);

  /** 提交（或丢弃）挂起的内联文本。工具切换/画布点击前调用（M1 遗留修复：切换
   * 工具结转而非丢弃；编辑态替换、新建态追加，各成一档历史）。 */
  const flushText = useCallback(() => {
    const td = textDraftRef.current;
    if (!td) return;
    setTextDraft(null);
    const value = td.value;
    if (value.trim() === "") return; // 空文本：静默丢弃（新建与编辑同口径）
    if (td.editIndex != null) onReplaceText(td.editIndex, value);
    else onCommit({ kind: "text", x: td.x, y: td.y, text: value, color: style.color, size: style.size });
  }, [onCommit, onReplaceText, style.color, style.size]);

  // 工具切换：结转未提交文本（含双击编辑中的）。
  useEffect(() => {
    flushText();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 仅工具变化时结转
  }, [tool]);

  const onPointerDown = (e: React.PointerEvent) => {
    if (!image) return;
    flushText(); // 任何画布点击前先结转挂起文本（blur 时序竞态兜底）
    const p = toCanvas(e);
    const measure = getMeasure();
    if (tool === "select") {
      const idx = topmostShapeAt(shapes, p.x, p.y, measure);
      onSelect(idx);
      if (idx != null) {
        dragRef.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        setMoveDraft({ index: idx, start: p, dx: 0, dy: 0 });
      }
      return;
    }
    if (tool === "text") {
      setTextDraft({ x: p.x, y: p.y, value: "", editIndex: null });
      return;
    }
    if (tool === "step") {
      // 点击落位；n 创建时算好存进形状（删除/撤销后重算自然回落）
      onCommit({ kind: "step", x: p.x, y: p.y, n: nextStepNumber(shapes), color: style.color });
      return;
    }
    dragRef.current = true;
    e.currentTarget.setPointerCapture(e.pointerId);
    switch (tool) {
      case "rect":
      case "ellipse":
        setDraft({ kind: tool, x: p.x, y: p.y, w: 0, h: 0, color: style.color, width: style.width });
        break;
      case "mosaic":
        setDraft({ kind: "mosaic", x: p.x, y: p.y, w: 0, h: 0, blockSize: MOSAIC_BLOCK_SIZE });
        break;
      case "arrow":
        setDraft({ kind: "arrow", x1: p.x, y1: p.y, x2: p.x, y2: p.y, color: style.color, width: style.width });
        break;
      case "pen":
      case "highlighter":
        setDraft({ kind: tool, points: [p], color: style.color, width: style.width });
        break;
    }
  };

  const onPointerMove = (e: React.PointerEvent) => {
    if (!dragRef.current) return;
    const p = toCanvas(e);
    if (moveDraft) {
      setMoveDraft({ ...moveDraft, dx: p.x - moveDraft.start.x, dy: p.y - moveDraft.start.y });
      return;
    }
    setDraft((d) => {
      if (!d) return d;
      switch (d.kind) {
        case "rect":
        case "ellipse":
        case "mosaic":
          // 归一化：负宽高（反向拖拽）在此折正，bboxOf 兜底
          return { ...d, x: Math.min(d.x, p.x), y: Math.min(d.y, p.y), w: Math.abs(p.x - d.x), h: Math.abs(p.y - d.y) };
        case "arrow":
          return { ...d, x2: p.x, y2: p.y };
        case "pen":
        case "highlighter":
          // 距上一点 <1px 不追点（mousemove 高频，压点密度）
          if (d.points.length) {
            const last = d.points[d.points.length - 1];
            if (Math.hypot(p.x - last.x, p.y - last.y) < 1) return d;
          }
          return { ...d, points: [...d.points, p] };
        case "text":
        case "step":
          // 无拖拽草稿（点击分派）；防御分支保穷尽
          return d;
      }
    });
  };

  const onPointerUp = () => {
    if (!dragRef.current) return;
    dragRef.current = false;
    if (moveDraft) {
      const { index, dx, dy } = moveDraft;
      setMoveDraft(null);
      // <1px 视为点击（选中）而非拖动，不入历史——一次拖拽 = 一档历史
      if (Math.hypot(dx, dy) >= 1) onMoveShape(index, dx, dy);
      return;
    }
    const d = draft;
    setDraft(null);
    if (!d) return;
    // 微形状丢弃（计划接口）：手写 <3 点、矩/椭/马 w*h<4、箭长 <4
    if (d.kind === "pen" || d.kind === "highlighter") {
      if (d.points.length < 3) return;
    } else if (d.kind === "rect" || d.kind === "ellipse" || d.kind === "mosaic") {
      if (d.w * d.h < 4) return;
    } else if (d.kind === "arrow") {
      if (Math.hypot(d.x2 - d.x1, d.y2 - d.y1) < 4) return;
    }
    onCommit(d);
  };

  // 双击（选择工具）：重新打开已提交文本形状的内联输入（预填原文，提交=替换）。
  const onDoubleClick = (e: React.MouseEvent) => {
    if (tool !== "select" || !image) return;
    const p = toCanvas(e);
    const measure = getMeasure();
    for (let i = shapes.length - 1; i >= 0; i--) {
      const s = shapes[i];
      if (s.kind === "text" && hitTest(s, p.x, p.y, measure)) {
        onSelect(i);
        setTextDraft({ x: s.x, y: s.y, value: s.text, editIndex: i });
        return;
      }
    }
  };

  // 文本提交/取消（内联输入自身的 Enter/blur/Esc）
  const commitText = () => {
    const td = textDraft;
    setTextDraft(null);
    if (!td) return;
    if (td.value.trim() === "") return;
    if (td.editIndex != null) onReplaceText(td.editIndex, td.value);
    else onCommit({ kind: "text", x: td.x, y: td.y, text: td.value, color: style.color, size: style.size });
  };

  // 文本内联输入的定位：画布物理坐标 → **stack 内** CSS 坐标（评审修复 1：input 的
  // position:absolute 以内层 stack div（position:relative）为包含块，而非外层滚动容器
  // wrap——原实现以 wrap 为原点，非满宽图会右偏 canvas 在 stack 内的居中偏移量。
  // 矢量层 inset:0 充满 stack，stackRect 即缩放原点；换算按矢量层实际缩放比）。
  const textPos = (() => {
    const vec = vecRef.current;
    const stack = stackRef.current;
    if (!textDraft || !vec || !stack) return null;
    const rect = vec.getBoundingClientRect();
    const stackRect = stack.getBoundingClientRect();
    return {
      left: rect.left - stackRect.left + (textDraft.x * rect.width) / (vec.width || 1),
      top: rect.top - stackRect.top + (textDraft.y * rect.height) / (vec.height || 1),
    };
  })();

  const cursor = tool === "select" ? "default" : tool === "text" ? "text" : "crosshair";

  return (
    <div style={{ position: "relative", flex: 1, overflow: "auto", background: "#2b2b2b" }}>
      <div
        ref={stackRef}
        style={{ position: "relative", margin: "0 auto", width: "fit-content" }}
        data-testid="editor-canvas-stack"
      >
        <canvas ref={baseRef} style={{ display: "block", maxWidth: "100%" }} />
        <canvas
          ref={vecRef}
          data-testid="editor-vector-canvas"
          style={{
            position: "absolute", inset: 0, width: "100%", height: "100%",
            cursor,
            touchAction: "none",
          }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onDoubleClick={onDoubleClick}
        />
        {textDraft && textPos && (
          <input
            ref={textInputRef}
            autoFocus
            data-testid="editor-text-input"
            aria-label={t("editor.textPlaceholder")}
            placeholder={t("editor.textPlaceholder")}
            value={textDraft.value}
            onChange={(e) => setTextDraft({ ...textDraft, value: e.target.value })}
            onBlur={commitText}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitText();
              if (e.key === "Escape") setTextDraft(null);
              e.stopPropagation(); // 不让 ⌘Z/Esc/Delete 冒泡到窗口级快捷键
            }}
            style={{
              position: "absolute", left: textPos.left, top: textPos.top,
              font: `${style.size}px sans-serif`, color: style.color,
              background: "rgba(255,255,255,0.85)", border: "1px dashed #888",
              minWidth: 120, padding: "0 2px", outline: "none",
            }}
          />
        )}
      </div>
    </div>
  );
}
