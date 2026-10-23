// 编辑器撤销/重做（M2 Task M1）：纯核心（可测）+ hook 包装。
// 裁定：撤销栈**无步数限制**（内存换正确性——形状对象轻量，会话级生命周期）。
// 结构 { past, present, future }：commit 把 present 压入 past 并清空 future（新
// 分支作废重做链）；undo/redo 在栈底/栈顶为 no-op 且返回**同一状态对象**（React
// 函数式更新下 Object.is 短路，不多触发渲染）。
import { useCallback, useState } from "react";
import type { Shape } from "./shapes";

export interface EditorHistoryState {
  past: Shape[][];
  present: Shape[];
  future: Shape[][];
}

/** 提交一份新形状数组：present → past，future 清空。 */
export function commitState(s: EditorHistoryState, shapes: Shape[]): EditorHistoryState {
  return { past: [...s.past, s.present], present: shapes, future: [] };
}

/** 撤销：栈底为 no-op（返回原对象）。 */
export function undoState(s: EditorHistoryState): EditorHistoryState {
  if (s.past.length === 0) return s;
  return {
    past: s.past.slice(0, -1),
    present: s.past[s.past.length - 1],
    future: [s.present, ...s.future],
  };
}

/** 重做：栈顶为 no-op（返回原对象）。 */
export function redoState(s: EditorHistoryState): EditorHistoryState {
  if (s.future.length === 0) return s;
  return {
    past: [...s.past, s.present],
    present: s.future[0],
    future: s.future.slice(1),
  };
}

/** 重置（换图/初始化）：双栈清空，present 置为给定形状（通常为空数组）。 */
export function resetState(_s: EditorHistoryState, shapes: Shape[]): EditorHistoryState {
  return { past: [], present: shapes, future: [] };
}

/** 编辑器历史 hook：state + 动作（全部函数式更新）+ canUndo/canRedo 供按钮禁用。 */
export function useEditorHistory(initial: Shape[] = []) {
  const [state, setState] = useState<EditorHistoryState>({
    past: [],
    present: initial,
    future: [],
  });
  const commit = useCallback((shapes: Shape[]) => setState((s) => commitState(s, shapes)), []);
  const undo = useCallback(() => setState(undoState), []);
  const redo = useCallback(() => setState(redoState), []);
  const reset = useCallback((shapes: Shape[]) => setState((s) => resetState(s, shapes)), []);
  return {
    ...state,
    canUndo: state.past.length > 0,
    canRedo: state.future.length > 0,
    commit,
    undo,
    redo,
    reset,
  };
}
