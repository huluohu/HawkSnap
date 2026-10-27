// N5 长截图选区模式渲染冒烟：payload.intent = "scrollshot" 时提示横幅在盘、
// 动作浮条缺席、确认路由到 scrollshot_start（req camelCase 契约）。
// mock @tauri-apps/api（jsdom 无 Tauri 运行时）；canvas 2d 上下文为 null → 冻结帧
// 绘制路径短路，本测试只覆盖横幅/浮条/确认路由的渲染决策面。
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import OverlayWindow from "./index";
import { setLanguage } from "../../lib/i18n";

type Payload = Record<string, unknown>;

const { handlers, invokeMock } = vi.hoisted(() => ({
  handlers: new Map<string, (e: { payload: Payload | Payload[] | null }) => void>(),
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  convertFileSrc: (path: string) => `asset://test/${path}`,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (e: { payload: Payload | Payload[] | null }) => void) => {
    handlers.set(event, handler);
    return Promise.resolve(() => handlers.delete(event));
  }),
  emit: vi.fn(() => Promise.resolve()),
}));

const display = (intent: string, flow: number) => ({
  displayId: 2, x: 0, y: 0, width: 1440, height: 900, scale: 2,
  frameUrl: "hawk-grab:///tmp/a.raw", flow, intent,
});

beforeEach(() => {
  setLanguage("en-US");
  handlers.clear();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_settings") return Promise.resolve({ language: "en-US" });
    if (cmd === "snap_candidates") return Promise.resolve([]);
    return Promise.resolve(null);
  });
  window.__LABEL__ = "overlay-2";
});

afterEach(() => {
  cleanup();
  delete window.__LABEL__;
});

async function pushPayload(intent: string, flow: number) {
  await act(async () => {
    handlers.get("hawk://overlay-ready")?.({ payload: [display(intent, flow)] });
  });
}

describe("overlay scrollshot mode (N5)", () => {
  it("shows the scrollshot hint and no action bar after a drag", async () => {
    render(<OverlayWindow />);
    await pushPayload("scrollshot", 1);
    expect(screen.getByText("Drag to select the area for the long screenshot")).toBeTruthy();
    const canvas = document.querySelector("canvas")!;
    fireEvent.pointerDown(canvas, { clientX: 100, clientY: 100 });
    fireEvent.pointerMove(canvas, { clientX: 300, clientY: 250 });
    fireEvent.pointerUp(canvas, { clientX: 300, clientY: 250 });
    // 动作浮条（复制/保存/标注）必须缺席——长截图只有「选区 → 滚动 → 完成」一条路
    expect(screen.queryByRole("button", { name: "Copy" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
  });

  it("routes confirm to scrollshot_start with the selected rect", async () => {
    render(<OverlayWindow />);
    await pushPayload("scrollshot", 1);
    const canvas = document.querySelector("canvas")!;
    fireEvent.pointerDown(canvas, { clientX: 100, clientY: 100 });
    fireEvent.pointerMove(canvas, { clientX: 300, clientY: 250 });
    fireEvent.pointerUp(canvas, { clientX: 300, clientY: 250 });
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    });
    expect(invokeMock).toHaveBeenCalledWith("scrollshot_start", {
      req: { displayId: 2, rect: { x: 100, y: 100, w: 200, h: 150 } },
    });
  });

  it("does not treat Enter-without-selection as full screen (scrollshot needs an explicit region)", async () => {
    render(<OverlayWindow />);
    await pushPayload("scrollshot", 1);
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    });
    expect(invokeMock).not.toHaveBeenCalledWith("scrollshot_start", expect.anything());
  });
});
