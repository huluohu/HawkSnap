// App 历史分区（M0）冒烟测试：mock @tauri-apps/api（jsdom 无 Tauri 运行时）——
// 挂载即 history_list 懒加载、缩略图经 convertFileSrc、重新编辑禁用（Ruling 1）、
// 空态文案、250ms 防抖后切 history_search。
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { setLanguage } from "./lib/i18n";
import type { ShotRow } from "./lib/history";

// —— invoke 按命令分派；convertFileSrc 确定性映射 ——
const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  convertFileSrc: (path: string) => `asset://test/${path}`,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

const SETTINGS = {
  hotkeyCapture: "F1",
  hotkeyPin: "F2",
  hotkeyRecord: "F6",
  outputFormat: "png",
  jpegQuality: 90,
  saveDir: null,
  launchAtLogin: false,
  appearance: "system",
  language: "en-US",
  updateEnabled: true,
};

const SHOTS: ShotRow[] = [
  {
    id: 2,
    createdAt: "2026-10-23T10:30:00+08:00",
    path: "/pics/HawkSnap/Safari 2026-10-23 103000.png",
    thumbPath: "/pics/HawkSnap/thumbs/Safari 2026-10-23 103000.png",
    w: 800,
    h: 600,
    app: "Safari",
  },
  {
    id: 1,
    createdAt: "2026-10-22T09:00:00+08:00",
    path: "/pics/HawkSnap/Finder 2026-10-22 090000.png",
    thumbPath: "", // 无缩略图 → 占位块分支
    w: 100,
    h: 80,
    app: "Finder",
  },
];

function stubInvoke() {
  invokeMock.mockImplementation((cmd: string, _args?: Record<string, unknown>) => {
    switch (cmd) {
      case "hotkey_error":
      case "check_for_updates":
        return Promise.resolve(null);
      case "get_settings":
        return Promise.resolve(SETTINGS);
      case "history_list":
        return Promise.resolve(SHOTS);
      case "history_search":
        return Promise.resolve(SHOTS.filter((s) => s.app === "Safari"));
      default:
        return Promise.resolve(null);
    }
  });
}

beforeEach(() => {
  setLanguage("en-US");
  stubInvoke();
});

afterEach(() => {
  cleanup();
  invokeMock.mockReset();
});

describe("App history section", () => {
  it("lazy-loads history_list on mount and renders the grid via convertFileSrc", async () => {
    render(<App />);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("history_list", { limit: 50, offset: 0 }),
    );
    // 两块瓦片：有缩略图的 img 走 asset 协议；无缩略图的走占位分支（无 img）
    const img = await screen.findByRole("img");
    expect(img.getAttribute("src")).toBe(
      "asset://test//pics/HawkSnap/thumbs/Safari 2026-10-23 103000.png",
    );
    expect(img.getAttribute("alt")).toContain("Safari");
  });

  it("renders per-tile actions with re-edit disabled (Ruling 1)", async () => {
    render(<App />);
    await screen.findByRole("img");
    const reedit = screen.getAllByTitle("Editor coming soon");
    expect(reedit.length).toBe(2);
    for (const b of reedit) {
      expect(b.tagName).toBe("BUTTON");
      expect((b as HTMLButtonElement).disabled).toBe(true);
    }
    // 其余操作可用：每瓦片 贴图 / 打开目录 / 删除 各一枚
    expect(screen.getAllByRole("button", { name: "Pin" }).length).toBe(2);
    expect(screen.getAllByRole("button", { name: "Reveal" }).length).toBe(2);
    expect(screen.getAllByRole("button", { name: "Delete" }).length).toBe(2);
  });

  it("two-step delete confirms then invokes history_delete and refreshes", async () => {
    render(<App />);
    await screen.findByRole("img");
    // 第一步：删除 → 第二步：确认删除（同瓦片内替换）
    fireEvent.click(screen.getAllByRole("button", { name: "Delete" })[0]);
    const confirm = screen.getAllByRole("button", { name: "Confirm delete" })[0];
    fireEvent.click(confirm);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("history_delete", { id: 2 }));
    // 删除成功后按当前查询刷新（空查询 → history_list）
    await waitFor(() =>
      expect(invokeMock).toHaveBeenLastCalledWith("history_list", { limit: 50, offset: 0 }),
    );
  });

  it("debounces search input 250ms into history_search; empty falls back to history_list", async () => {
    vi.useFakeTimers();
    try {
      render(<App />);
      // 挂载即懒加载（不走防抖）
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(invokeMock).toHaveBeenCalledWith("history_list", { limit: 50, offset: 0 });
      invokeMock.mockClear();

      const input = screen.getByPlaceholderText("Search shots (file name / app)");
      fireEvent.change(input, { target: { value: "safari" } });
      // 防抖窗口内不触发
      await act(async () => {
        await vi.advanceTimersByTimeAsync(100);
      });
      expect(invokeMock).not.toHaveBeenCalledWith("history_search", expect.anything());
      await act(async () => {
        await vi.advanceTimersByTimeAsync(200);
      });
      expect(invokeMock).toHaveBeenCalledWith("history_search", { query: "safari", limit: 50 });
      // 搜索结果只有 Safari 一条（invoke 分派过滤）。伪定时器下不能用 findAll*（其
      // 等待依赖定时器会挂死）——advanceTimersByTimeAsync 已冲刷微任务，同步查询即可。
      expect(screen.getAllByRole("img")).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows keyed empty states for no-history and no-search-hit", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "get_settings"
        ? Promise.resolve(SETTINGS)
        : cmd === "history_list" || cmd === "history_search"
          ? Promise.resolve([])
          : Promise.resolve(null),
    );
    render(<App />);
    expect(await screen.findByText("No screenshots yet")).toBeTruthy();
    // 输入查询 → 防抖后空命中态
    fireEvent.change(screen.getByPlaceholderText("Search shots (file name / app)"), {
      target: { value: "zzz" },
    });
    expect(await screen.findByText("No matching shots")).toBeTruthy();
  });
});
