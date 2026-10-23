// 装载代数守卫（评审修复 2，纯逻辑可测）：editor-load 三连发 @150ms 让旧事件的
// Image 探测迟到成为常态（A 的补发可落在 B 的事件之后）。每个 load 事件分配单调
// 递增代数；探测完成回调里仅当代数仍为最新才应用 imageSrc/reset/loadError——
// 与覆盖层 FLOW_SEQ 代数守卫同一模式（flow.rs）。
export interface LoadGuard {
  /** 分配新代数（每个 load 事件调用一次），返回本次事件代数。 */
  begin(): number;
  /** 探测完成回调内判断：该事件代数是否仍为最新（false = 迟到旧事件，自弃）。 */
  isCurrent(gen: number): boolean;
}

export function createLoadGuard(): LoadGuard {
  let gen = 0;
  return {
    begin: () => ++gen,
    isCurrent: (g) => g === gen,
  };
}
