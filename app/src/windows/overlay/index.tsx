import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { listen } from "@tauri-apps/api/event";
import { useSelection, type SelRect } from "./useSelection";

type Display = { displayId: number; x: number; y: number; width: number; height: number; scale: number; pngUrl: string };

// window.__LABEL__ 由 Rust 侧建窗后 eval 注入（overlay.rs），用于认领自己的 DisplayPayload
declare global {
  interface Window {
    __LABEL__?: string;
  }
}

export default function OverlayWindow() {
  const [display, setDisplay] = useState<Display | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const shownRef = useRef(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<Display[]>("hawk://overlay-ready", (e) => {
      const mine = e.payload.find((d) => `overlay-${d.displayId}` === window.__LABEL__);
      if (mine) setDisplay(mine);
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // cancel 的职责由 hook 内部的 Esc keydown 监听承担（会回调 onCancel），页面无需直接引用
  const { phase, rect, pointerHandlers, confirm } = useSelection({
    width: display?.width ?? 0, height: display?.height ?? 0,
    onConfirm: async (r: SelRect) => {
      if (!display) return;
      try {
        const outcome = await invoke<{ copied: boolean; savedTo: string | null }>("finish_capture", {
          req: { displayId: display.displayId, rect: r, copy: true, save: true },
        });
        // 发 capture-done 并关闭本窗口（Rust 侧按 displayId 直接关，见 Task 9 裁定 2）
        await invoke("emit_capture_done", { displayId: display.displayId, outcome });
      } catch (err) {
        // spec §9：失败不允许静默。覆盖层窗口不是横幅宿主，经全局事件把错误送到
        // 主窗口的 capture-error 横幅（ACL core:default 含 event emit）。
        console.error("finish_capture failed", err);
        void emit("hawk://capture-error", String(err));
      }
    },
    onCancel: () => {
      // display 未到（overlay-ready 三连发全丢）时，从建窗注入的 __LABEL__（格式
      // overlay-{id}）同步解析 displayId，保证 Esc 始终能关窗，不留僵尸覆盖层。
      const labelId = window.__LABEL__?.match(/^overlay-(\d+)$/)?.[1];
      const id = display?.displayId ?? Number(labelId);
      if (!Number.isFinite(id)) return; // 无 display 且 label 缺失，无从取消
      void invoke("cancel_capture", { displayId: id });
    },
  });

  // 冻结帧 + 遮罩 + 选区：rect/display 变化时重绘
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !display) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    canvas.width = display.width;
    canvas.height = display.height;
    const img = new Image();
    img.onload = () => {
      ctx.drawImage(img, 0, 0, display.width, display.height);
      ctx.fillStyle = "rgba(0,0,0,0.35)";
      ctx.fillRect(0, 0, display.width, display.height);
      if (rect && rect.w >= 2 && rect.h >= 2) {
        ctx.drawImage(img, rect.x, rect.y, rect.w, rect.h, rect.x, rect.y, rect.w, rect.h);
        ctx.strokeStyle = "#FFB020";
        ctx.lineWidth = 2;
        ctx.strokeRect(rect.x + 1, rect.y + 1, rect.w - 2, rect.h - 2);
        const label = `${rect.w}×${rect.h}`;
        ctx.font = "13px system-ui, sans-serif";
        const tw = ctx.measureText(label).width + 12;
        ctx.fillStyle = "#FFFFFF";
        ctx.fillRect(rect.x + rect.w - tw, rect.y + rect.h + 6, tw, 20);
        ctx.fillStyle = "#0D1522";
        ctx.fillText(label, rect.x + rect.w - tw + 6, rect.y + rect.h + 20);
      }
      // 首次画出冻结帧后回报一次（Rust 侧暂忽略，Task 11 接性能打点）
      if (!shownRef.current) {
        shownRef.current = true;
        void emit("hawk://overlay-shown");
      }
    };
    img.src = convertFileSrc(display.pngUrl.replace("hawk-grab://", ""));
  }, [rect, display, phase]);

  if (!display) return null;
  return (
    <canvas ref={canvasRef}
      style={{ position: "fixed", inset: 0, cursor: "crosshair" }}
      {...pointerHandlers}
      onDoubleClick={() => confirm()} />
  );
}
