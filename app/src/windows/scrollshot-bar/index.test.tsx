// N5 长截图提示条：hawk://scrollshot-tick 事件 → 帧数渲染；mount 时经
// scrollshot_status 拉一次补显（push 早于 listen 必丢）。
// 另覆盖 listen 被 ACL 拒绝的分支：必须 catch（不留 unhandled rejection）且不崩。
//
// **失败必须看得见**（I4 残留，评审必修）：Rust 侧 `scrollshot_stop` 改成
// **先拼接、成功后才收尾**（关条 + 注销热键 + 消费会话）→ 拼接失败时提示条
// **仍然活着**。本文件锁定前端这一侧的可观察行为：失败 → 行内错误可见 +
// 条仍在（完成按钮还在）+ 多出「放弃」出口；Rust 直发的失败事件同样落到
// 行内错误槽（不依赖 invoke 回流）。
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ScrollshotBarWindow from "./index";
import { setLanguage } from "../../lib/i18n";

type Handler = (e: { payload: any }) => void;

const { getHandler, getErrorHandler, listenMock, invokeMock, errorSpy } = vi.hoisted(() => ({
  getHandler: { current: null as Handler | null },
  getErrorHandler: { current: null as Handler | null },
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
  getErrorHandler.current = null;
  listenMock.mockReset();
  invokeMock.mockReset();
  errorSpy.mockReset();
  vi.spyOn(console, "error").mockImplementation(errorSpy);
  listenMock.mockImplementation((event: string, handler: Handler) => {
    if (event === "hawk://scrollshot-error") {
      getErrorHandler.current = handler;
    } else {
      getHandler.current = handler;
    }
    return Promise.resolve(() => {
      if (event === "hawk://scrollshot-error") {
        getErrorHandler.current = null;
      } else {
        getHandler.current = null;
      }
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

  it("keeps the bar alive and shows the error inline when finishing fails (I4 residual)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "scrollshot_status") {
        return Promise.resolve({ session: 1, frames: 4, active: true });
      }
      if (cmd === "scrollshot_stop") return Promise.reject(new Error("拼接失败"));
      return Promise.resolve(null);
    });
    render(<ScrollshotBarWindow />);
    await act(async () => {});
    // 完成按钮在盘（鼠标出口——⌘S 不可用时是唯一出口）
    const btn = screen.getByRole("button", { name: "Finish" });
    await act(async () => {
      fireEvent.click(btn);
    });
    expect(invokeMock).toHaveBeenCalledWith("scrollshot_stop");
    // 失败 → 行内错误可见（不再是死代码的错误槽）
    expect(screen.getByText(/拼接失败/)).toBeTruthy();
    // **条仍在**：Rust 侧失败不关条 → 前端不得自行卸载任何东西。
    // 「完成」按钮仍在（可重试），且多出「放弃」出口（用户有出路）。
    const retryBtn = screen.getByRole("button", { name: "Finish" }) as HTMLButtonElement;
    expect(retryBtn).toBeTruthy();
    // 失败后按钮**恢复可点**（收尾在飞行中的禁用只持续到失败反馈为止）。
    expect(retryBtn.disabled).toBe(false);
    const abortBtn = screen.getByRole("button", { name: "Discard" });
    expect(abortBtn).toBeTruthy();
    // 帧数仍在更新（条是活的，不是死壳）。
    await act(async () => {
      getHandler.current?.({ payload: { session: 1, frames: 11 } });
    });
    expect(screen.getByText("11 frames")).toBeTruthy();
  });

  it("disables the Finish button while a stop is in flight (concurrent-finish guard)", async () => {
    let resolveStop: ((v: string) => void) | null = null;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "scrollshot_status") {
        return Promise.resolve({ session: 1, frames: 4, active: true });
      }
      if (cmd === "scrollshot_stop") {
        // 模拟 Rust 侧秒级拼接：停在 pending 上，直到我们手动 resolve。
        return new Promise<string>((res) => {
          resolveStop = res;
        });
      }
      return Promise.resolve(null);
    });
    render(<ScrollshotBarWindow />);
    await act(async () => {});
    const btn = screen.getByRole("button", { name: "Finish" }) as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
    await act(async () => {
      fireEvent.click(btn);
    });
    // 收尾在飞行中 → 按钮禁用（连点不会打出第二个收尾请求）。
    expect((screen.getByRole("button", { name: "Finish" }) as HTMLButtonElement).disabled).toBe(true);
    await act(async () => {
      resolveStop?.("/tmp/out.png");
    });
    // 成功后 Rust 关条 → 前端不再需要恢复按钮状态（条已消失）。
    expect(invokeMock).toHaveBeenCalledTimes(2); // scrollshot_status + scrollshot_stop
  });

  it("ellipsizes long error text instead of hard-cutting it", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "scrollshot_status") {
        return Promise.resolve({ session: 1, frames: 4, active: true });
      }
      return Promise.resolve(null);
    });
    render(<ScrollshotBarWindow />);
    await act(async () => {});
    await act(async () => {
      getErrorHandler.current?.({
        payload: {
          session: 1,
          message: "未能识别滚动位移：画面内容没有明显变化，或内容重复导致匹配歧义",
        },
      });
    });
    const errEl = screen.getByText(/未能识别滚动位移/);
    // nowrap + overflow:hidden 必须配 textOverflow:ellipsis，否则文案被硬切。
    expect((errEl as HTMLElement).style.textOverflow).toBe("ellipsis");
  });

  it("renders the Rust-emitted failure event inline (does not rely on the invoke round trip)", async () => {
    render(<ScrollshotBarWindow />);
    await act(async () => {});
    expect(screen.queryByText(/未能识别滚动位移/)).toBeNull();
    await act(async () => {
      getErrorHandler.current?.({
        payload: { session: 1, message: "未能识别滚动位移：画面内容没有明显变化" },
      });
    });
    expect(screen.getByText(/未能识别滚动位移/)).toBeTruthy();
    // 条仍在 + 「放弃」出口出现。
    expect(screen.getByRole("button", { name: "Finish" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Discard" })).toBeTruthy();
  });

  it("discards the session through scrollshot_abort", async () => {
    render(<ScrollshotBarWindow />);
    await act(async () => {});
    await act(async () => {
      getErrorHandler.current?.({
        payload: { session: 1, message: "未能识别滚动位移" },
      });
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    });
    expect(invokeMock).toHaveBeenCalledWith("scrollshot_abort");
  });

  it("switches the hint when the ⌘S hotkey is unavailable (pull channel, I4)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "scrollshot_status") {
        // ⌘S 注册失败 → Rust 侧 hotkey=false → 提示条必须改口（否则用户等一个
        // 永远不会响的按键）。
        return Promise.resolve({ session: 1, frames: 2, active: true, hotkey: false });
      }
      return Promise.resolve(null);
    });
    render(<ScrollshotBarWindow />);
    await act(async () => {});
    expect(screen.getByText("Scroll the page… click Finish to finish")).toBeTruthy();
    expect(
      screen.queryByText("Scroll the page… press ⌘S to finish"),
    ).toBeNull();
    // 完成按钮仍在（唯一出口）。
    expect(screen.getByRole("button", { name: "Finish" })).toBeTruthy();
  });

  it("catches a rejected listen (missing window capability) without an unhandled rejection", async () => {
    listenMock.mockImplementation(() => Promise.reject(new Error("not allowed by ACL")));
    render(<ScrollshotBarWindow />);
    await act(async () => {});
    expect(errorSpy).toHaveBeenCalledTimes(2);
    expect(String(errorSpy.mock.calls[0][0])).toContain("listen");
  });
});
