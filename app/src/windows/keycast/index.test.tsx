// N4 keycast 小窗：hawk://keycast 事件 → 大字显示组合键。
// 另覆盖评审修复的 reject 分支：listen 的 promise 被拒（窗口 capability 缺失）
// 时必须被 catch（不得留 unhandled rejection），且页面不崩。
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import KeycastWindow from "./index";

type Handler = (e: { payload: { combo: string } }) => void;

const { getHandler, listenMock, errorSpy } = vi.hoisted(() => ({
  getHandler: { current: null as Handler | null },
  listenMock: vi.fn(),
  errorSpy: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

beforeEach(() => {
  getHandler.current = null;
  listenMock.mockReset();
  errorSpy.mockReset();
  vi.spyOn(console, "error").mockImplementation(errorSpy);
  listenMock.mockImplementation((_event: string, handler: Handler) => {
    getHandler.current = handler;
    return Promise.resolve(() => {
      getHandler.current = null;
    });
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("keycast window (N4)", () => {
  it("renders the combo from hawk://keycast and restarts the fade on each event", async () => {
    render(<KeycastWindow />);
    await act(async () => {}); // flush mount effects（listen 注册）
    expect(screen.queryByText("⌘⇧A")).toBeNull();
    await act(async () => {
      getHandler.current?.({ payload: { combo: "⌘⇧A" } });
    });
    expect(screen.getByText("⌘⇧A")).toBeTruthy();
    // 第二次事件 → 重建节点（key 变化）→ 动画重放。
    await act(async () => {
      getHandler.current?.({ payload: { combo: "A" } });
    });
    expect(screen.getByText("A")).toBeTruthy();
    expect(screen.queryByText("⌘⇧A")).toBeNull();
  });

  it("catches a rejected listen (missing window capability) without an unhandled rejection", async () => {
    listenMock.mockImplementation(() => Promise.reject(new Error("not allowed by ACL")));
    render(<KeycastWindow />);
    await act(async () => {}); // flush rejection
    // 错误被记录一次，页面不崩、不残留可见内容。
    expect(errorSpy).toHaveBeenCalledTimes(1);
    expect(String(errorSpy.mock.calls[0][0])).toContain("listen");
    expect(screen.getByTestId("keycast-root")).toBeTruthy();
  });
});
