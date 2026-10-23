import { describe, expect, it } from "vitest";
import { renderHook } from "@testing-library/react";
import { act } from "react";
import {
  commitState,
  redoState,
  resetState,
  undoState,
  useEditorHistory,
  type EditorHistoryState,
} from "./history";

const a = [{ kind: "rect" as const, x: 0, y: 0, w: 1, h: 1, color: "#f00", width: 1 }];
const b = [{ kind: "text" as const, x: 1, y: 1, text: "b", color: "#00f", size: 12 }];
const c = [{ kind: "arrow" as const, x1: 0, y1: 0, x2: 2, y2: 2, color: "#00f", width: 1 }];

const init: EditorHistoryState = { past: [], present: a, future: [] };

describe("editor history pure core", () => {
  it("commit pushes present to past and clears future", () => {
    const withFuture = { ...init, future: [b] };
    const s = commitState(withFuture, c);
    expect(s.past).toEqual([a]);
    expect(s.present).toEqual(c);
    expect(s.future).toEqual([]); // redo after new commit clears future
  });

  it("commit/undo/redo cycles restore snapshots", () => {
    let s = commitState(init, b); // past=[a] present=b
    s = commitState(s, c); // past=[a,b] present=c
    s = undoState(s); // present=b future=[c]
    expect(s.present).toEqual(b);
    expect(s.past).toEqual([a]);
    expect(s.future).toEqual([c]);
    s = undoState(s); // present=a future=[b,c]
    expect(s.present).toEqual(a);
    s = redoState(s); // present=b
    expect(s.present).toEqual(b);
    s = redoState(s); // present=c future=[]
    expect(s.present).toEqual(c);
    expect(s.future).toEqual([]);
  });

  it("undo at the bottom is a no-op", () => {
    expect(undoState(init)).toEqual(init);
  });

  it("redo with empty future is a no-op", () => {
    expect(redoState(init)).toEqual(init);
  });

  it("reset clears both stacks", () => {
    let s = commitState(init, b);
    s = commitState(s, c);
    s = resetState(s, a);
    expect(s).toEqual({ past: [], present: a, future: [] });
  });

  it("has no step cap: 200 commits are undone by 200 undos", () => {
    let s = init;
    for (let i = 0; i < 200; i++) {
      s = commitState(s, [{ kind: "text", x: i, y: 0, text: `${i}`, color: "#000", size: 12 }]);
    }
    expect(s.past.length).toBe(200);
    expect(s.present[0]).toMatchObject({ text: "199" });
    // 第 k 次 undo 揭示第 199-k 个提交；第 200 次回到初始 present（a）
    for (let i = 199; i >= 1; i--) {
      s = undoState(s);
      expect(s.present[0]).toMatchObject({ text: `${i - 1}` });
    }
    s = undoState(s);
    expect(s.present).toEqual(a);
    expect(s.past.length).toBe(0);
    expect(undoState(s)).toEqual(s); // 再 undo 无事发生
  });

  it("actions return the same state object on no-op (functional-update friendly)", () => {
    expect(undoState(init)).toBe(init);
    expect(redoState(init)).toBe(init);
  });
});

describe("useEditorHistory hook", () => {
  it("wires state + actions with functional updates", () => {
    const { result } = renderHook(() => useEditorHistory(a));
    expect(result.current.present).toEqual(a);
    act(() => result.current.commit(b));
    expect(result.current.present).toEqual(b);
    act(() => result.current.commit(c));
    act(() => result.current.undo());
    expect(result.current.present).toEqual(b);
    expect(result.current.canUndo).toBe(true);
    expect(result.current.canRedo).toBe(true);
    act(() => result.current.redo());
    expect(result.current.present).toEqual(c);
    act(() => result.current.reset(a));
    expect(result.current.present).toEqual(a);
    expect(result.current.canUndo).toBe(false);
    expect(result.current.canRedo).toBe(false);
  });
});
