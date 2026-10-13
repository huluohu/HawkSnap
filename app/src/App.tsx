import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

function App() {
  const [hotkeyError, setHotkeyError] = useState<string | null>(null);
  const [captureError, setCaptureError] = useState<string | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    // pull 通道：setup 期的 hawk://hotkey-error emit 早于 listen 注册必丢，挂载即向后端
    // 拉取（后端读取即清除）。非 Tauri 环境下降级为无提示。
    invoke<string | null>("hotkey_error")
      .then((err) => {
        if (!disposed && err) setHotkeyError(err);
      })
      .catch(() => {});
    // push 通道：覆盖挂载后发生的注册失败（spec §9：不允许静默失败）。
    listen<string>("hawk://hotkey-error", (e) => setHotkeyError(e.payload))
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // 截图失败横幅：F1/托盘两条路径的 start_capture_flow 失败均 emit hawk://capture-error
  // （spec §9：不允许静默失败）。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<string>("hawk://capture-error", (e) => setCaptureError(e.payload))
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return (
    <main className="container">
      {hotkeyError && (
        <div
          role="alert"
          style={{
            margin: "12px",
            padding: "10px 14px",
            border: "1px solid #d9534f",
            borderRadius: 6,
            background: "#fdecea",
            color: "#a94442",
            fontSize: 14,
          }}
        >
          全局热键注册失败，F1 截图不可用：{hotkeyError}
        </div>
      )}
      {captureError && (
        <div
          role="alert"
          style={{
            margin: "12px",
            padding: "10px 14px",
            border: "1px solid #d9534f",
            borderRadius: 6,
            background: "#fdecea",
            color: "#a94442",
            fontSize: 14,
          }}
        >
          截图失败：{captureError}
        </div>
      )}
      <h1>HawkSnap</h1>
      <p>主面板尚未实现（属于 Plan C）。按 F1 开始截图。</p>
    </main>
  );
}

export default App;
