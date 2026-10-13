import { useCallback, useEffect, useRef, useState } from "react";

export type SelRect = { x: number; y: number; w: number; h: number };
export type Phase = "idle" | "dragging" | "selected";
// 确认动作：Enter/双击缺省复制；保存是浮条上的显式按钮（Plan B Task 3 语义裁定）。
export type ConfirmAction = "copy" | "save";

type Point = { clientX: number; clientY: number };

export function useSelection(opts: {
  width: number; height: number;
  onConfirm?: (r: SelRect, action: ConfirmAction) => void; onCancel?: () => void;
}) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [rect, setRect] = useState<SelRect | null>(null);
  const start = useRef<Point | null>(null);
  // 确认在途标志：confirm 一旦触发即置位，防止 Enter 自动重复/双击在异步
  // finish_capture → emit_capture_done 链完成前再次确认（重复剪贴板写入、重复落盘）。
  // 覆盖层窗口常驻复用（hide 而非 close），故除 cancel() 外，新一轮 payload 到达时的
  // reset() 也必须清掉本标志——否则上一轮失败/中断的在途守卫会砖死下一轮确认。
  const confirmingRef = useRef(false);

  const norm = (a: Point, b: Point): SelRect => ({
    x: Math.max(0, Math.min(a.clientX, b.clientX)),
    y: Math.max(0, Math.min(a.clientY, b.clientY)),
    w: Math.abs(a.clientX - b.clientX),
    h: Math.abs(a.clientY - b.clientY),
  });

  const pointerHandlers = {
    // down 不清 rect：保留既有选区，dblclick（down#2→up#2 后触发）才能确认；
    // 新选区在首次 move 时由 norm() 覆盖旧 rect。
    onPointerDown: (e: Point) => { start.current = e; setPhase("dragging"); },
    onPointerMove: (e: Point) => {
      if (phase !== "dragging" || !start.current) return;
      setRect(norm(start.current, e));
    },
    onPointerUp: (_: Point) => {
      if (phase !== "dragging") return;
      setPhase(r => (r === "dragging" ? "selected" : r));
      start.current = null;
    },
  };

  // action 缺省 "copy"：Enter（keydown 监听）与 dblclick（canvas onDoubleClick）都不带
  // 动作参数，走复制；"save" 仅由浮条按钮显式传入。
  const confirm = useCallback((action: ConfirmAction = "copy") => {
    if (confirmingRef.current) return;
    if (phase !== "selected" || !rect || rect.w < 2 || rect.h < 2) return;
    confirmingRef.current = true;
    opts.onConfirm?.(rect, action);
  }, [phase, rect, opts]);

  const cancel = useCallback(() => {
    confirmingRef.current = false;
    setPhase("idle"); setRect(null); start.current = null; opts.onCancel?.();
  }, [opts]);

  // 新一轮捕获（overlay-ready 新 payload 到达）时由页面调用：回到 idle、清选区，
  // 并清 in-flight 守卫——与 cancel 的差异是不触发 onCancel（不关窗）。
  const reset = useCallback(() => {
    confirmingRef.current = false;
    start.current = null;
    setPhase("idle");
    setRect(null);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cancel();
      if (e.key === "Enter") confirm();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cancel, confirm]);

  return { phase, rect, pointerHandlers, confirm, cancel, reset };
}
