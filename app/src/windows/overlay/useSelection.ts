import { useCallback, useEffect, useRef, useState } from "react";

export type SelRect = { x: number; y: number; w: number; h: number };
export type Phase = "idle" | "dragging" | "selected";

type Point = { clientX: number; clientY: number };

export function useSelection(opts: {
  width: number; height: number;
  onConfirm?: (r: SelRect) => void; onCancel?: () => void;
}) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [rect, setRect] = useState<SelRect | null>(null);
  const start = useRef<Point | null>(null);
  // 确认在途标志：confirm 一旦触发即置位，防止 Enter 自动重复/双击在异步
  // finish_capture → emit_capture_done 链完成前再次确认（重复剪贴板写入、重复落盘）。
  // 成功路径窗口随即关闭无需复位；失败/取消路径由 cancel() 复位。
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

  const confirm = useCallback(() => {
    if (confirmingRef.current) return;
    if (phase !== "selected" || !rect || rect.w < 2 || rect.h < 2) return;
    confirmingRef.current = true;
    opts.onConfirm?.(rect);
  }, [phase, rect, opts]);

  const cancel = useCallback(() => {
    confirmingRef.current = false;
    setPhase("idle"); setRect(null); start.current = null; opts.onCancel?.();
  }, [opts]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cancel();
      if (e.key === "Enter") confirm();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cancel, confirm]);

  return { phase, rect, pointerHandlers, confirm, cancel };
}
