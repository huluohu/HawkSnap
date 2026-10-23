import { describe, expect, it } from "vitest";
import { createLoadGuard } from "./loadGuard";

describe("createLoadGuard", () => {
  it("assigns monotonically increasing generations per load event", () => {
    const g = createLoadGuard();
    expect(g.begin()).toBe(1);
    expect(g.begin()).toBe(2);
    expect(g.begin()).toBe(3);
  });

  it("isCurrent holds only for the latest generation (stale probes self-discard)", () => {
    // 场景对齐评审：开 A（gen1）→ 快速开 B（gen2）→ A 的探测迟到时 gen1 已非最新
    const g = createLoadGuard();
    const a = g.begin();
    expect(g.isCurrent(a)).toBe(true);
    const b = g.begin();
    expect(g.isCurrent(a)).toBe(false); // A 迟到 → 自弃
    expect(g.isCurrent(b)).toBe(true); // B 最新 → 应用
  });

  it("independent guards do not share state", () => {
    const g1 = createLoadGuard();
    const g2 = createLoadGuard();
    expect(g1.begin()).toBe(1);
    expect(g2.begin()).toBe(1); // 各自从 1 起算
  });
});
