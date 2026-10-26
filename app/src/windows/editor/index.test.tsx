// 编辑器工具栏冒烟（M3 N0 第 10 工具）：裁剪按钮在盘、点击进入激活态。
// mock @tauri-apps/api（jsdom 无 Tauri 运行时）；位图装载路径不经由——工具栏与
// 位图无关（无图也渲染全部工具按钮，裁剪 apply 本身经 canvas 的像素语义由浏览器
// 人工验收单覆盖，与 F5 exportToPng 同一口径）。
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import EditorWindow from "./index";
import { setLanguage } from "../../lib/i18n";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
  convertFileSrc: (path: string) => `asset://test/${path}`,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ close: vi.fn(() => Promise.resolve()) }),
}));

beforeEach(() => {
  setLanguage("en-US");
});

afterEach(() => {
  cleanup();
});

describe("Editor toolbar crop tool (10/10)", () => {
  it("renders the crop button among the tools and activates it on click", () => {
    render(<EditorWindow />);
    const crop = screen.getByRole("button", { name: "Crop" });
    expect(crop.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(crop);
    expect(crop.getAttribute("aria-pressed"), "点击后进入裁剪工具选中态").toBe("true");
    // 工具全集 10 个（含裁剪）：G6 双语文案面就位即可数出全部按钮
    for (const label of ["Select", "Crop", "Rectangle", "Ellipse", "Arrow", "Pen", "Highlighter", "Mosaic", "Step", "Text"]) {
      expect(screen.getByRole("button", { name: label })).toBeTruthy();
    }
  });
});
