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

describe("recording bar format chip (N2)", () => {
  it("shows the GIF chip for gif sessions and drops it for mp4 sessions", async () => {
    render(<RecordingBarWindow />);
    await act(async () => {});
    // gif 会话：format=gif → 模式标签在盘（MP4 默认态不出标签）
    await act(async () => {
      getTickHandler.current?.({ payload: { session: 1, seconds: 2, paused: false, format: "gif" } });
    });
    expect(screen.getByText("GIF")).toBeTruthy();
    // mp4 会话（新 session）：标签消失（跨会话残留清理）
    await act(async () => {
      getTickHandler.current?.({ payload: { session: 2, seconds: 4, paused: false, format: "mp4" } });
    });
    expect(screen.queryByText("GIF")).toBeNull();
  });
});

describe("recording bar pip toggle (N3)", () => {
  it("invokes pip_update with optimistic on/off and reverts on camera failure", async () => {
    const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "pip_update") {
        calls.push({ cmd, args: args ?? {} });
        // 第三次调用（第二次开启）模拟相机权限拒绝（Rust Err 中文回流）。
        if (calls.length === 3) return Promise.reject("相机权限未授予：画中画保持关闭（录制不受影响）");
      }
      return Promise.resolve(null);
    });
    render(<RecordingBarWindow />);
    await act(async () => {});
    const btn = () => screen.getByRole("button", { name: "PiP" });
    // 默认关（aria-pressed=false）→ 点击 = 开启，乐观置位 + invoke 载荷 visible:true。
    expect(btn().getAttribute("aria-pressed")).toBe("false");
    await act(async () => { btn().click(); });
    expect(calls[0]).toEqual({ cmd: "pip_update", args: { visible: true } });
    expect(btn().getAttribute("aria-pressed")).toBe("true");
    // 再点击 = 关闭（visible:false）；此前开启失败场景见下。
    await act(async () => { btn().click(); });
    expect(calls[1]).toEqual({ cmd: "pip_update", args: { visible: false } });
    // 失败路径：开启被拒 → 回落 aria-pressed=false + 行内错误（Rust 文案）。
    await act(async () => { btn().click(); });
    expect(calls[2].args).toEqual({ visible: true });
    await act(async () => {}); // flush rejection
    expect(btn().getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByText(/相机权限未授予/)).toBeTruthy();
  });
});
