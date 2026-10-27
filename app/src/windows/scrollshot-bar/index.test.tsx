// N5 长截图提示条：hawk://scrollshot-tick 事件 → 帧数渲染；mount 时经
// scrollshot_status 拉一次补显（push 早于 listen 必丢）。
// 另覆盖 listen 被 ACL 拒绝的分支：必须 catch（不留 unhandled rejection）且不崩。
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ScrollshotBarWindow from "./index";
import { setLanguage } from "../../lib/i18n";

type Handler = (e: { payload: { session: number; frames: number } }) => void;

const { getHandler, listenMock, invokeMock, errorSpy } = vi.hoisted(() => ({
  getHandler: { current: null as Handler | null },
  listenMock: vi.fn(),
  invokeMock: vi.fn(),
  errorSpy: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

beforeEach(() => {
  setLanguage("en-US");
  getHandler.current = null;
  listenMock.mockReset();
  invokeMock.mockReset();
  errorSpy.mockReset();
  vi.spyOn(console, "error").mockImplementation(errorSpy);
  listenMock.mockImplementation((_event: string, handler: Handler) => {
    getHandler.current = handler;
    return Promise.resolve(() => {
      getHandler.current = null;
    });
  });
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "scrollshot_status") {
      return Promise.resolve({ session: 1, frames: 3, active: true });
    }
    return Promise.resolve(null);
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("scrollshot bar window (N5)", () => {
  it("pulls the initial frame count and updates it on every tick", async () => {
    render(<ScrollshotBarWindow />);
    await act(async () => {}); // flush mount effects（pull + listen 注册）
    // pull 补显：3 帧
    expect(screen.getByText("3 frames")).toBeTruthy();
    await act(async () => {
      getHandler.current?.({ payload: { session: 1, frames: 9 } });
    });
    expect(screen.getByText("9 frames")).toBeTruthy();
  });

  it("renders the scroll hint", async () => {
    render(<ScrollshotBarWindow />);
    await act(async () => {});
    expect(screen.getByText("Scroll the page… press ⌘S to finish")).toBeTruthy();
  });

  it("catches a rejected listen (missing window capability) without an unhandled rejection", async () => {
    listenMock.mockImplementation(() => Promise.reject(new Error("not allowed by ACL")));
    render(<ScrollshotBarWindow />);
    await act(async () => {});
    expect(errorSpy).toHaveBeenCalledTimes(1);
    expect(String(errorSpy.mock.calls[0][0])).toContain("listen");
  });
});
