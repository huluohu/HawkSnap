// N1 录制选区模式渲染冒烟：payload.intent = "record" 时提示横幅在盘、动作浮条缺席
// （拖出选区也不出现），intent = "capture" 时恢复截图原貌。mock @tauri-apps/api
// （jsdom 无 Tauri 运行时）；canvas 2d 上下文在 jsdom 为 null → 冻结帧绘制路径短路，
// 本测试只覆盖横幅/浮条的渲染决策面（绘制语义由几何/选区既有测试 + 人工验收覆盖）。
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import OverlayWindow from "./index";
import { setLanguage } from "../../lib/i18n";

type Payload = Record<string, unknown>;

// vi.mock 工厂被提升到文件顶部、先于模块级 const 执行 → 工厂引用的状态必须经
// vi.hoisted 创建（否则 TDZ 引用错误，listen 注册静默失败、载荷永远到不了组件）。
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

describe("overlay record mode (N1)", () => {
  it("record payload shows the record hint banner and no action bar after a drag", async () => {
    render(<OverlayWindow />);
    await pushPayload("record", 1);
    // 提示横幅（en-US 文案在盘）
    expect(screen.getByText("Drag to select the area to record, Enter for full screen")).toBeTruthy();
    // 拖出选区：phase → selected，但动作浮条（复制/保存/标注）必须缺席
    const canvas = document.querySelector("canvas")!;
    fireEvent.pointerDown(canvas, { clientX: 100, clientY: 100 });
    fireEvent.pointerMove(canvas, { clientX: 300, clientY: 250 });
    fireEvent.pointerUp(canvas, { clientX: 300, clientY: 250 });
    expect(screen.queryByRole("button", { name: "Copy" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Annotate" })).toBeNull();
  });

  it("record payload routes confirm to record_start_region (full-display shortcut on Enter with no rect)", async () => {
    render(<OverlayWindow />);
    await pushPayload("record", 1);
    // 无选区 Enter：emptyConfirmRect = 整屏 → record_start_region（req camelCase 契约；
    // format 默认 mp4——N2 分段开关的初始态）
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    });
    expect(invokeMock).toHaveBeenCalledWith("record_start_region", {
      req: { displayId: 2, rect: { x: 0, y: 0, w: 1440, h: 900 }, format: "mp4" },
    });
  });

  it("record format toggle (N2): segmented control defaults to MP4 and passes gif through invoke", async () => {
    render(<OverlayWindow />);
    await pushPayload("record", 1);
    // 分段开关在盘（提示横幅右侧），默认 MP4 高亮（aria-pressed）
    const mp4Btn = screen.getByRole("button", { name: "MP4" });
    const gifBtn = screen.getByRole("button", { name: "GIF" });
    expect(mp4Btn.getAttribute("aria-pressed")).toBe("true");
    expect(gifBtn.getAttribute("aria-pressed")).toBe("false");
    // 切到 GIF → 确认载荷 format=gif（state 流经 invoke 的契约面）
    await act(async () => {
      fireEvent.click(gifBtn);
    });
    expect(gifBtn.getAttribute("aria-pressed")).toBe("true");
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    });
    expect(invokeMock).toHaveBeenLastCalledWith("record_start_region", {
      req: { displayId: 2, rect: { x: 0, y: 0, w: 1440, h: 900 }, format: "gif" },
    });
  });

  it("capture payload keeps the original behavior: no hint, action bar after a drag", async () => {
    render(<OverlayWindow />);
    await pushPayload("record", 1);
    await pushPayload("capture", 2); // 新一轮（flow 推进）：覆盖层切换回截图模式
    expect(screen.queryByText("Drag to select the area to record, Enter for full screen")).toBeNull();
    const canvas = document.querySelector("canvas")!;
    fireEvent.pointerDown(canvas, { clientX: 100, clientY: 100 });
    fireEvent.pointerMove(canvas, { clientX: 300, clientY: 250 });
    fireEvent.pointerUp(canvas, { clientX: 300, clientY: 250 });
    // 截图模式：动作浮条照常出现
    expect(screen.getByRole("button", { name: "Copy" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Save" })).toBeTruthy();
  });
});
