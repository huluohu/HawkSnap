import { useCallback, useEffect, useRef, useState } from "react";
import { nudgeRect, snapRect, type Rect } from "./geometry";

// 与 geometry::Rect 结构全同；保留 SelRect 名字供既有页面/测试引用。
export type SelRect = Rect;
export type Phase = "idle" | "dragging" | "selected";
// 确认动作：Enter/双击缺省复制；保存是浮条上的显式按钮（Plan B Task 3 语义裁定）。
export type ConfirmAction = "copy" | "save";

type Point = { clientX: number; clientY: number };

export function useSelection(opts: {
  width: number; height: number;
  // Task 4：磁吸候选（display 本地坐标）与夹取边界。bounds 缺省（display 未到）时
  // 方向键不移动选区；candidatesRef 缺省时只 nudge 不磁吸。
  candidatesRef?: { current: Rect[] };
  bounds?: Rect;
  onConfirm?: (r: SelRect, action: ConfirmAction) => void; onCancel?: () => void;
  // M2 Task M5 取色模式（Snipaste 式）：置位时 Esc/Enter 由拾色器消费——Esc 只退出
  // 拾色（第一次 Esc 不该连截图一起取消），Enter 不触发确认复制。方向键不动（选区
  // 微调与拾色不冲突）。opts 每渲染新对象 → effect 随 pickerMode 重挂，闭包必新。
  keysSuppressed?: boolean;
  // N1 录制模式整屏快捷确认：无选区（或选区过小）时 Enter/双击以此 rect 确认
  // （= 选区即整屏，rect 为本 display 全边界）。缺省（截图模式）保持「无选区不
  // 确认」不变——截图侧零行为变化。
  emptyConfirmRect?: Rect;
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
    if (phase === "selected" && rect && rect.w >= 2 && rect.h >= 2) {
      confirmingRef.current = true;
      // HiDPI 分数坐标修正（0.1 hotfix）：缩放屏上 clientX/Y 可为小数（如 428.75），norm()
      // 原样保留 → Rust 侧 Rect 是 i32/u32，serde 在 IPC 层直接拒绝浮点 → finish_capture
      // 失败。出口统一取整（round(w≥2) ≥ 2，既有 w/h≥2 守卫取整后仍成立，无需复检）。
      const r = {
        x: Math.round(rect.x), y: Math.round(rect.y),
        w: Math.round(rect.w), h: Math.round(rect.h),
      };
      opts.onConfirm?.(r, action);
      return;
    }
    // N1 录制模式整屏快捷确认：无选区（idle/空选区）Enter/双击 → 选区即整屏。
    // 拖拽中不触发（与既有 phase 门一致）；截图模式无 emptyConfirmRect → 不确认，
    // 行为与改动前逐字节一致。
    const fallback = opts.emptyConfirmRect;
    if (fallback && phase !== "dragging") {
      confirmingRef.current = true;
      opts.onConfirm?.(
        {
          x: Math.round(fallback.x), y: Math.round(fallback.y),
          w: Math.round(fallback.w), h: Math.round(fallback.h),
        },
        action,
      );
    }
  }, [phase, rect, opts]);

  // 失败解锁（0.1 hotfix）：confirm 置位的在途守卫原只有 cancel/reset 两个出口——
  // finish_capture IPC 失败时守卫仍为 true，本轮所有后续确认都被拒（用户必须 Esc 重选）。
  // 页面在 onConfirm 的 catch 路径调用本函数解锁，失败后无需 Esc 即可重试。成功路径
  // 无需调用（成功即 emit_capture_done 关窗）。与 cancel/reset 的差异：不动 phase/rect。
  const unlockConfirm = useCallback(() => {
    confirmingRef.current = false;
  }, []);

  // Task 6 click-to-select 直选：跳过拖拽直接以给定 rect 进入 selected（动作浮条随之
  // 出现）。不动 confirmingRef——在途守卫只由 confirm 链管理（controller 裁定 6）；
  // 清 start.current 纯属卫生：selectRect 后 phase 已是 selected，拖拽 move 分支天然
  // 短路，陈旧 start 无人再读（下一次 pointerdown 会整体覆盖）。
  const selectRect = useCallback((r: SelRect) => {
    start.current = null;
    // 与 confirm 出口同理（0.1 hotfix）：AX 帧坐标可为小数，入口即取整——渲染与
    // Enter 确认整链保持整数（同一 bug 的另一入口）。
    setRect({
      x: Math.round(r.x), y: Math.round(r.y),
      w: Math.round(r.w), h: Math.round(r.h),
    });
    setPhase("selected");
  }, []);

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
      // Task 4 方向键微调：必须先于 Escape/Enter 判断，且仅在 selected 态生效——
      // 拖拽中/空闲的方向键一律忽略（不 preventDefault，不改状态）。preventDefault
      // 阻止方向键的默认滚动/焦点移动；return 防止同一按键再落进 Escape/Enter 分支。
      if (phase === "selected" && e.key.startsWith("Arrow")) {
        e.preventDefault();
        const bounds = opts.bounds;
        if (bounds) {
          const step = e.shiftKey ? 10 : 1;
          const dx = e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0;
          const dy = e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0;
          // 先 nudge（夹取进 bounds），再 snap（最近单边 8px）；snap 的跨边吸附可能把
          // 选区推出 bounds 至多一个阈值，末尾再夹一次（位移 0 的 nudgeRect 即纯夹取）。
          setRect((r) => {
            if (!r) return r;
            const snapped = snapRect(
              nudgeRect(r, dx, dy, bounds),
              opts.candidatesRef?.current ?? [], 8,
            );
            return nudgeRect(snapped, 0, 0, bounds);
          });
        }
        return;
      }
      if (e.key === "Escape" && !opts.keysSuppressed) cancel();
      if (e.key === "Enter" && !opts.keysSuppressed) confirm();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cancel, confirm, phase, opts]);

  return { phase, rect, pointerHandlers, confirm, cancel, reset, selectRect, unlockConfirm };
}
