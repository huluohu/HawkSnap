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

  const norm = (a: Point, b: Point): SelRect => ({
    x: Math.max(0, Math.min(a.clientX, b.clientX)),
    y: Math.max(0, Math.min(a.clientY, b.clientY)),
    w: Math.abs(a.clientX - b.clientX),
    h: Math.abs(a.clientY - b.clientY),
  });

  const pointerHandlers = {
    onPointerDown: (e: Point) => { start.current = e; setPhase("dragging"); setRect(null); },
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
    if (phase !== "selected" || !rect || rect.w < 2 || rect.h < 2) return;
    opts.onConfirm?.(rect);
  }, [phase, rect, opts]);

  const cancel = useCallback(() => {
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
