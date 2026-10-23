// 编辑器画布（M2 Task M1）：位图底（随图片加载绘制一次）+ 矢量层（shapes/draft
// 变化即全量重绘——M1 规模可接受，M2 优化）。指针流：down 建草稿 → move 更新 →
// up 校验并提交（过小/过短的形状丢弃）。drawShape 是 M1 的唯一绘制路径，M2 抽取
// 后作为导出共用（F5 单路径约束的雏形）。
import { useCallback, useEffect, useRef, useState } from "react";
import { bboxOf, type Shape } from "./shapes";
import { t } from "../../lib/i18n";

export type Tool = "rect" | "ellipse" | "arrow" | "pen" | "highlighter" | "text";

export interface EditorStyle { color: string; width: number; size: number }

interface EditorCanvasProps {
  imageUrl: string | null;
  shapes: Shape[];
  tool: Tool;
  style: EditorStyle;
  onCommit: (s: Shape) => void;
}

interface Pt { x: number; y: number }

// —— 绘制（预览用；M2 抽取为导出共用 drawShape）——
// 注：绘制不需要文本量测（fillText 直接画）；量测仅 bboxOf/hitTest 的 text 分支
// 需要（ctx.measureText 的注入点在矢量层重绘 effect 里）。

function drawShape(ctx: CanvasRenderingContext2D, shape: Shape): void {
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
      // 箭头头部：两翼沿主轴回折 ~30°
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
      ctx.textBaseline = "top";
      ctx.fillText(shape.text, shape.x, shape.y);
      break;
    }
  }
}

// —— 组件 ——

export default function EditorCanvas({ imageUrl, shapes, tool, style, onCommit }: EditorCanvasProps) {
  const baseRef = useRef<HTMLCanvasElement>(null);
  const vecRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  // 拖拽草稿：state 驱动矢量层实时预览（M1 规模：每 move 重绘全部形状可接受）
  const [draft, setDraft] = useState<Shape | null>(null);
  // 文本工具的内联输入：定位 + 草稿值
  const [textDraft, setTextDraft] = useState<{ x: number; y: number; value: string } | null>(null);
  const textInputRef = useRef<HTMLInputElement>(null);
  const dragRef = useRef(false);

  // 位图底：图片加载（每张一次）后按自然尺寸铺满 base canvas，并同步矢量层尺寸。
  useEffect(() => {
    if (!imageUrl) return;
    const img = new Image();
    img.onload = () => {
      const base = baseRef.current;
      const vec = vecRef.current;
      if (!base || !vec) return;
      base.width = img.naturalWidth;
      base.height = img.naturalHeight;
      vec.width = img.naturalWidth;
      vec.height = img.naturalHeight;
      const ctx = base.getContext("2d");
      if (ctx) ctx.drawImage(img, 0, 0);
    };
    img.src = imageUrl;
  }, [imageUrl]);

  // 矢量层：shapes/draft 任一变化即全量重绘（含空白兜底：图片未加载时也有干净画布）。
  useEffect(() => {
    const vec = vecRef.current;
    if (!vec) return;
    const ctx = vec.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, vec.width, vec.height);
    for (const s of shapes) drawShape(ctx, s);
    if (draft) drawShape(ctx, draft);
  }, [shapes, draft]);

  /** 事件坐标 → 画布物理坐标（CSS 缩放比换算）。 */
  const toCanvas = useCallback((e: React.PointerEvent): Pt => {
    const vec = vecRef.current;
    if (!vec) return { x: 0, y: 0 };
    const rect = vec.getBoundingClientRect();
    const sx = vec.width / (rect.width || 1);
    const sy = vec.height / (rect.height || 1);
    return { x: (e.clientX - rect.left) * sx, y: (e.clientY - rect.top) * sy };
  }, []);

  const onPointerDown = (e: React.PointerEvent) => {
    if (!imageUrl) return;
    const p = toCanvas(e);
    if (tool === "text") {
      // 文本工具：点击即落锚，内联输入 Enter/blur 提交、Esc 取消（index 层约定）
      setTextDraft({ x: p.x, y: p.y, value: "" });
      return;
    }
    dragRef.current = true;
    e.currentTarget.setPointerCapture(e.pointerId);
    switch (tool) {
      case "rect":
      case "ellipse":
        setDraft({ kind: tool, x: p.x, y: p.y, w: 0, h: 0, color: style.color, width: style.width });
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
    setDraft((d) => {
      if (!d) return d;
      switch (d.kind) {
        case "rect":
        case "ellipse":
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
          // 文本无拖拽草稿（点击落锚走内联输入）；防御分支保穷尽
          return d;
      }
    });
  };

  const onPointerUp = () => {
    if (!dragRef.current) return;
    dragRef.current = false;
    const d = draft;
    setDraft(null);
    if (!d) return;
    // 微形状丢弃（计划接口）：手写 <3 点、矩/椭 w*h<4、箭长 <4
    if (d.kind === "pen" || d.kind === "highlighter") {
      if (d.points.length < 3) return;
    } else if (d.kind === "rect" || d.kind === "ellipse") {
      if (d.w * d.h < 4) return;
    } else if (d.kind === "arrow") {
      if (Math.hypot(d.x2 - d.x1, d.y2 - d.y1) < 4) return;
    }
    onCommit(d);
  };

  // 文本提交/取消
  const commitText = () => {
    const td = textDraft;
    setTextDraft(null);
    if (!td || td.value.trim() === "") return;
    onCommit({ kind: "text", x: td.x, y: td.y, text: td.value, color: style.color, size: style.size });
  };

  // 文本内联输入的定位：画布物理坐标 → 容器 CSS 坐标（按矢量层实际缩放比换算）
  const textPos = (() => {
    const vec = vecRef.current;
    const wrap = wrapRef.current;
    if (!textDraft || !vec || !wrap) return null;
    const rect = vec.getBoundingClientRect();
    const wrapRect = wrap.getBoundingClientRect();
    return {
      left: rect.left - wrapRect.left + (textDraft.x * rect.width) / (vec.width || 1),
      top: rect.top - wrapRect.top + (textDraft.y * rect.height) / (vec.height || 1),
    };
  })();

  return (
    <div ref={wrapRef} style={{ position: "relative", flex: 1, overflow: "auto", background: "#2b2b2b" }}>
      <div
        style={{ position: "relative", margin: "0 auto", width: "fit-content" }}
        data-testid="editor-canvas-stack"
      >
        <canvas ref={baseRef} style={{ display: "block", maxWidth: "100%" }} />
        <canvas
          ref={vecRef}
          data-testid="editor-vector-canvas"
          style={{
            position: "absolute", inset: 0, width: "100%", height: "100%",
            cursor: tool === "text" ? "text" : "crosshair",
            touchAction: "none",
          }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
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
              e.stopPropagation(); // 不让 ⌘Z/Esc 冒泡到窗口级快捷键
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
