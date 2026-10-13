import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { listen } from "@tauri-apps/api/event";
import { useSelection, type SelRect } from "./useSelection";

type Display = { displayId: number; x: number; y: number; width: number; height: number; scale: number; frameUrl: string };

// window.__LABEL__ 由 Rust 侧建窗后 eval 注入（overlay.rs），用于认领自己的 DisplayPayload
declare global {
  interface Window {
    __LABEL__?: string;
  }
}

// 冻结帧（原始物理 RGBA）加载：fetch 为主通道；若 WKWebView 对 asset 协议 URL 的 fetch
// 出现异常，降级 XHR responseType=arraybuffer（两者能力的最低公因数）。字节长度校验
// 由调用方按 w*h*4 断言，两通道共用同一防线。
async function fetchRaw(url: string): Promise<ArrayBuffer> {
  try {
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    return await resp.arrayBuffer();
  } catch (err) {
    console.warn("fetch raw frame failed, falling back to XHR", err);
    return await new Promise<ArrayBuffer>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open("GET", url);
      xhr.responseType = "arraybuffer";
      xhr.onload = () => {
        // 自定义 scheme 下 status 可能为 0；字节长度校验兜底，故 0 且有响应也算成功
        if ((xhr.status >= 200 && xhr.status < 300) || xhr.status === 0) {
          resolve(xhr.response as ArrayBuffer);
        } else {
          reject(new Error(`XHR HTTP ${xhr.status}`));
        }
      };
      xhr.onerror = () => reject(new Error("XHR network error"));
      xhr.send();
    });
  }
}

export default function OverlayWindow() {
  const [display, setDisplay] = useState<Display | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // 覆盖层窗口常驻复用（Plan B Task 1）：页面生命周期跨越多轮捕获，此标志每轮 payload
  // 到达时复位，保证 hawk://overlay-shown 每轮捕获重发一次（bench 门闩按轮与 hotkey 配对）。
  const shownRef = useRef(false);
  // 捕获轮次（代际守卫）：每轮 payload 到达即自增。confirm 链（finish_crop 数百 ms）在途时
  // 用户新触发一轮会复用本窗口，旧链的迟到延续若不拦截，会用 emit_capture_done 把新一轮
  // 刚 show 的窗口藏掉（用户看到窗口闪现即消失），其失败也会误报在新一轮上。
  const flowRef = useRef(0);

  // cancel 的职责由 hook 内部的 Esc keydown 监听承担（会回调 onCancel），页面无需直接引用。
  // 声明须先于下方 listen effect：其依赖数组在渲染期取 reset（TDZ）。
  const { phase, rect, pointerHandlers, confirm, reset } = useSelection({
    width: display?.width ?? 0, height: display?.height ?? 0,
    onConfirm: async (r: SelRect) => {
      if (!display) return;
      const myFlow = flowRef.current;
      try {
        const outcome = await invoke<{ copied: boolean; savedTo: string | null }>("finish_capture", {
          req: { displayId: display.displayId, rect: r, copy: true, save: true },
        });
        // 代际守卫：finish_capture 在途期间新一轮 payload 已到（flowRef 已自增）→ 本链
        // 已被作废，绝不 emit_capture_done（否则 hide 掉新一轮刚显示的窗口）。
        if (flowRef.current !== myFlow) return;
        // 发 capture-done 并隐藏本窗口（Rust 侧按 displayId 隐藏，见 Task 9 裁定 2；
        // Plan B Task 1 起常驻复用：隐藏而非关闭，窗口由 start_capture_flow 复用）
        await invoke("emit_capture_done", { displayId: display.displayId, outcome });
      } catch (err) {
        // 旧轮链的失败静默吞掉：横幅是全局事件，迟到的失败会误导新一轮的用户
        if (flowRef.current !== myFlow) return;
        // spec §9：失败不允许静默。覆盖层窗口不是横幅宿主，经全局事件把错误送到
        // 主窗口的 capture-error 横幅（ACL core:default 含 event emit）。
        console.error("finish_capture failed", err);
        void emit("hawk://capture-error", String(err));
      }
    },
    onCancel: () => {
      // display 未到（overlay-ready 三连发全丢）时，从建窗注入的 __LABEL__（格式
      // overlay-{id}）同步解析 displayId，保证 Esc 始终能隐藏窗口，不留僵尸覆盖层。
      const labelId = window.__LABEL__?.match(/^overlay-(\d+)$/)?.[1];
      const id = display?.displayId ?? Number(labelId);
      if (!Number.isFinite(id)) return; // 无 display 且 label 缺失，无从取消
      void invoke("cancel_capture", { displayId: id });
    },
  });

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<Display[]>("hawk://overlay-ready", (e) => {
      const mine = e.payload.find((d) => `overlay-${d.displayId}` === window.__LABEL__);
      if (mine) {
        setDisplay(mine);          // 新对象身份必触发重绘 effect
        reset();                   // 新一轮捕获：清选区与 in-flight 守卫
        flowRef.current += 1;      // 代际自增：作废所有在途旧 confirm 链（见 flowRef 注）
        shownRef.current = false;  // 每轮捕获重发 overlay-shown（见上方 shownRef 注）
      }
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [reset]);

  // 冻结帧 + 遮罩 + 选区：rect/display 变化时重绘。常驻复用下每轮 payload 的 frameUrl
  // 携带后端版本化文件名（capture_screens 按时间戳命名，Plan B Task 1 cache-bust），
  // URL 变化即绕开 Webview 缓存，无需前端追加 query。Task 2b：帧为原始物理 RGBA 文件，
  // fetch 字节后 putImageData 直写，不再经 <img> 的 PNG 解码。
  // 原始字节按 frameUrl 缓存（记 Promise：首帧在途时选区已开始变化的重入共享同一次加载，
  // 拖拽重绘零重复请求）。
  const frameRef = useRef<{ url: string; load: Promise<ArrayBuffer> } | null>(null);
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !display) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    // 物理尺寸 = 逻辑 × scale（与 capture_screens/CaptureMeta 同一公式）
    const pw = Math.round(display.width * display.scale);
    const ph = Math.round(display.height * display.scale);
    let disposed = false;

    let load: Promise<ArrayBuffer>;
    if (frameRef.current && frameRef.current.url === display.frameUrl) {
      load = frameRef.current.load;
    } else {
      load = fetchRaw(convertFileSrc(display.frameUrl.replace("hawk-grab://", "")));
      frameRef.current = { url: display.frameUrl, load };
      // 加载失败清缓存项，下一轮 effect 可重试
      load.catch(() => {
        if (frameRef.current?.url === display.frameUrl) frameRef.current = null;
      });
    }

    const draw = (buf: ArrayBuffer) => {
      if (buf.byteLength !== pw * ph * 4) {
        console.error(`冻结帧字节数不符：got ${buf.byteLength}, expect ${pw * ph * 4}`);
        return;
      }
      const frame = new ImageData(new Uint8ClampedArray(buf), pw, ph);
      // 背衬 = 物理像素，CSS 尺寸 = 逻辑像素：Retina 下逐像素清晰（顺带修复旧实现
      // 逻辑画布被浏览器插值放大的发软问题）。重设 width/height 同时重置 ctx 状态。
      canvas.width = pw;
      canvas.height = ph;
      canvas.style.width = `${display.width}px`;
      canvas.style.height = `${display.height}px`;
      // putImageData 不受 transform 影响（恰好物理 1:1 直写）；遮罩/选区/标注仍用逻辑
      // 坐标，经 transform 映射到物理背衬。用 setTransform（而非 ctx.scale）保证 effect
      // 因 rect 变化重跑时 transform 绝对重设，绝不叠加。
      ctx.setTransform(display.scale, 0, 0, display.scale, 0, 0);
      ctx.putImageData(frame, 0, 0);
      ctx.fillStyle = "rgba(0,0,0,0.35)";
      ctx.fillRect(0, 0, display.width, display.height);
      if (rect && rect.w >= 2 && rect.h >= 2) {
        // 选区内恢复未加暗的原始像素（旧实现 drawImage 的等价物）：putImageData 的
        // dirty-rect 形式同为设备像素，坐标须按 scale 换算
        ctx.putImageData(frame, 0, 0,
          Math.round(rect.x * display.scale), Math.round(rect.y * display.scale),
          Math.round(rect.w * display.scale), Math.round(rect.h * display.scale));
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
      // 本轮冻结帧画出后回报一次（Rust 侧 mark_overlay_shown_once 只记每轮首个，
      // 门闩与 hotkey 1:1 配对）；shownRef 已在下一轮 payload 到达时复位
      if (!shownRef.current) {
        shownRef.current = true;
        void emit("hawk://overlay-shown");
      }
    };
    load.then((buf) => {
      if (!disposed) draw(buf);
    }).catch((err) => {
      if (!disposed) console.error("冻结帧加载失败", err);
    });
    return () => { disposed = true; };
  }, [rect, display, phase]);

  if (!display) return null;
  return (
    <canvas ref={canvasRef}
      style={{ position: "fixed", inset: 0, cursor: "crosshair" }}
      {...pointerHandlers}
      onDoubleClick={() => confirm()} />
  );
}
