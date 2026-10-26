// N1 区域录制标题冒烟：record-tick 载荷带 regionW/regionH（camelCase 契约，Rust 侧
// record_tick_serializes_region_dims_camel_case 对偶）→ 条上出现「区域 W×H」；全屏
// tick（字段缺席）→ 标题消失（跨会话残留清理）。mock @tauri-apps/api（jsdom 无 Tauri）。
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import RecordingBarWindow from "./index";
import { setLanguage } from "../../lib/i18n";

type TickHandler = (e: { payload: Record<string, unknown> }) => void;

// vi.mock 工厂提升先于模块级 const → 状态经 vi.hoisted 创建（同 recordMode.test 注）。
const { getTickHandler, invokeMock } = vi.hoisted(() => ({
  getTickHandler: { current: null as TickHandler | null },
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((_event: string, handler: TickHandler) => {
    getTickHandler.current = handler;
    return Promise.resolve(() => { getTickHandler.current = null; });
  }),
}));

beforeEach(() => {
  setLanguage("en-US");
  getTickHandler.current = null;
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "record_status") return Promise.resolve(null);
    return Promise.resolve(null);
  });
});

afterEach(cleanup);

describe("recording bar region title (N1)", () => {
  it("shows 区域 W×H when the tick carries region dims and hides it for full-screen ticks", async () => {
    render(<RecordingBarWindow />);
    await act(async () => {}); // flush mount effects（pull + listen 注册）
    // 区域会话：regionW/regionH 在场 → 标题出现（en-US 文案）
    await act(async () => {
      getTickHandler.current?.({ payload: { session: 1, seconds: 3, paused: false, regionW: 1600, regionH: 1200 } });
    });
    expect(screen.getByText("Region 1600×1200")).toBeTruthy();
    // 全屏会话（新 session）：字段缺席 → 标题消失（复用条无残留）
    await act(async () => {
      getTickHandler.current?.({ payload: { session: 2, seconds: 1, paused: false } });
    });
    expect(screen.queryByText("Region 1600×1200")).toBeNull();
  });
});
