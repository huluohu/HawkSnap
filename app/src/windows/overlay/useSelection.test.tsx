import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useSelection } from "./useSelection";

// vitest globals 未开启 → RTL 的自动 cleanup 未注册；keydown 监听挂在 window 上，
// 不清理会跨用例泄漏（上一例 selected 态的监听会 preventDefault 下一例 idle 态的方向键）。
afterEach(cleanup);

describe("useSelection", () => {
  it("drag normalizes rect and tracks phases", () => {
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600 }));
    expect(result.current.phase).toBe("idle");
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 300, clientY: 200 } as PointerEvent));
    expect(result.current.phase).toBe("dragging");
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 100, clientY: 50 } as PointerEvent));
    expect(result.current.rect).toEqual({ x: 100, y: 50, w: 200, h: 150 });
    act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
    expect(result.current.phase).toBe("selected");
  });

  it("Enter confirms only when a rect exists", () => {
    const onConfirm = vi.fn();
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600, onConfirm }));
    act(() => result.current.confirm());
    expect(onConfirm).not.toHaveBeenCalled(); // 空选区不可确认
  });

  it("Esc cancels and resets", () => {
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600 }));
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 10, clientY: 10 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 60, clientY: 40 } as PointerEvent));
    act(() => result.current.cancel());
    expect(result.current.phase).toBe("idle");
    expect(result.current.rect).toBeNull();
  });

  it("a fresh pointer-down after a selection keeps the rect confirmable", () => {
    // 回归：down 不得清空选区，否则 dblclick（down#2→up#2 后触发）时 rect 已为 null，
    // confirm 永远拒绝——双击确认成死代码
    const onConfirm = vi.fn();
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600, onConfirm }));
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 100, clientY: 100 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 200, clientY: 200 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 500, clientY: 500 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
    expect(result.current.rect).toEqual({ x: 100, y: 100, w: 100, h: 100 });
    act(() => result.current.confirm()); // dblclick 到来时的 confirm 路径（缺省动作 = 复制）
    expect(onConfirm).toHaveBeenCalledWith({ x: 100, y: 100, w: 100, h: 100 }, "copy");
  });

  it("reset returns to idle and clears in-flight guard", () => {
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600 }));
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 10, clientY: 10 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 60, clientY: 40 } as PointerEvent));
    act(() => result.current.cancel()); // cancel 内部也会 reset
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 5, clientY: 5 } as PointerEvent));
    act(() => result.current.reset());
    expect(result.current.phase).toBe("idle");
    expect(result.current.rect).toBeNull();
    // reset 后 confirm 可以再次触发（in-flight 守卫被清）
    expect(() => result.current.confirm()).not.toThrow();
  });

  it("reset clears the in-flight confirm guard so the next flow can confirm again", () => {
    // 加强版（评审 Finding 2）：onConfirm 用永不 resolve 的 spy 模拟在途 finish 链——
    // 若 delete 掉 reset() 里的 confirmingRef.current = false，第二次 confirm 会被
    // 在途守卫拒绝（calledTimes 停在 1），本用例即失败，真正钉死「reset 清守卫」。
    const onConfirm = vi.fn(() => new Promise(() => {}));
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600, onConfirm }));
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 10, clientY: 10 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 60, clientY: 40 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
    act(() => result.current.confirm());
    expect(onConfirm).toHaveBeenCalledTimes(1); // 守卫已置位，链在途
    act(() => result.current.reset());
    expect(result.current.phase).toBe("idle");
    expect(result.current.rect).toBeNull();
    // 新一轮：完整拖选后 confirm 必须能再次触发（in-flight 守卫被清）
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 5, clientY: 5 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 50, clientY: 40 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
    act(() => result.current.confirm());
    expect(onConfirm).toHaveBeenCalledTimes(2);
  });

  it("confirm is re-entrancy guarded while a confirm is in flight", () => {
    // 回归：phase 在异步 finish 链期间保持 selected，Enter 自动重复不得再次触发 onConfirm
    const onConfirm = vi.fn();
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600, onConfirm }));
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 10, clientY: 10 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 50, clientY: 50 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
    act(() => {
      result.current.confirm();
      result.current.confirm(); // 在途（链未结束）的第二次确认
    });
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("confirm forwards the requested action", () => {
    const onConfirm = vi.fn();
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600, onConfirm }));
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 10, clientY: 10 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 60, clientY: 40 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
    act(() => result.current.confirm("save"));
    expect(onConfirm).toHaveBeenCalledWith({ x: 10, y: 10, w: 50, h: 30 }, "save");
  });

  // —— Task 4：方向键微调 + 磁吸 ——

  const arrowKey = (key: string, shift = false) => {
    act(() => window.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey: shift, cancelable: true })));
  };
  const dragTo = (result: { current: { pointerHandlers: {
    onPointerDown: (e: PointerEvent) => void; onPointerMove: (e: PointerEvent) => void;
    onPointerUp: (e: PointerEvent) => void } } }, x: number, y: number) => {
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: x, clientY: y } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: x + 50, clientY: y + 40 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
  };

  it("arrow keys nudge the selected rect by 1 (10 with Shift) and clamp to bounds", () => {
    const candidatesRef = { current: [] as { x: number; y: number; w: number; h: number }[] };
    const { result } = renderHook(() => useSelection({
      width: 1000, height: 600,
      bounds: { x: 0, y: 0, w: 1000, h: 600 }, candidatesRef,
    }));
    dragTo(result, 100, 100);
    expect(result.current.rect).toEqual({ x: 100, y: 100, w: 50, h: 40 });
    arrowKey("ArrowLeft");
    expect(result.current.rect).toEqual({ x: 99, y: 100, w: 50, h: 40 });
    arrowKey("ArrowUp", true); // Shift = 10px
    expect(result.current.rect).toEqual({ x: 99, y: 90, w: 50, h: 40 });
    // 夹取：推到左上角之外也不出 bounds
    for (let i = 0; i < 200; i++) arrowKey("ArrowLeft", true);
    expect(result.current.rect).toEqual({ x: 0, y: 90, w: 50, h: 40 });
  });

  it("arrow nudge snaps to the nearest candidate edge (8px)", () => {
    const candidatesRef = {
      current: [{ x: 150, y: 0, w: 300, h: 600 }], // 右边缘 450
    };
    const { result } = renderHook(() => useSelection({
      width: 1000, height: 600,
      bounds: { x: 0, y: 0, w: 1000, h: 600 }, candidatesRef,
    }));
    dragTo(result, 100, 100); // 选区 x=100, 右边缘 150 → 距候选左边缘 150 恰好 0？否：右 150 vs 候选左 150 → d=0 排除
    arrowKey("ArrowLeft"); // 右边缘 149 vs 候选左 150 → d=1 → 吸附 x=100（右=150）
    expect(result.current.rect).toEqual({ x: 100, y: 100, w: 50, h: 40 });
    arrowKey("ArrowLeft"); // 已对齐（d=0 排除），不再扰动
    expect(result.current.rect).toEqual({ x: 100, y: 100, w: 50, h: 40 });
    // Shift 右移 10：右边缘 160 vs 150 → d=10 超阈值，不吸；左边缘 100 vs 候选右 450 远
    arrowKey("ArrowRight", true);
    expect(result.current.rect).toEqual({ x: 110, y: 100, w: 50, h: 40 });
  });

  it("arrows are ignored outside the selected phase and never block Escape/Enter", () => {
    const onConfirm = vi.fn();
    const { result } = renderHook(() => useSelection({
      width: 1000, height: 600,
      bounds: { x: 0, y: 0, w: 1000, h: 600 },
      onConfirm,
    }));
    // idle 态：方向键不产生 rect、不 preventDefault（事件保持可取消能力）
    const idle = new KeyboardEvent("keydown", { key: "ArrowLeft", cancelable: true });
    act(() => window.dispatchEvent(idle));
    expect(idle.defaultPrevented).toBe(false);
    expect(result.current.rect).toBeNull();
    // selected 态方向键被消费（preventDefault），随后 Escape 仍正常取消（分支顺序）
    dragTo(result, 100, 100);
    const sel = new KeyboardEvent("keydown", { key: "ArrowLeft", cancelable: true });
    act(() => window.dispatchEvent(sel));
    expect(sel.defaultPrevented).toBe(true);
    expect(result.current.rect).toEqual({ x: 99, y: 100, w: 50, h: 40 });
    arrowKey("Escape");
    expect(result.current.phase).toBe("idle");
    expect(result.current.rect).toBeNull();
  });
});
