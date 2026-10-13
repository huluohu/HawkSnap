import { useEffect, useState, type CSSProperties } from "react";
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

  // 面板入口（C6）：与 F1/F2 热键、托盘同链路（panel_trigger → start_capture_flow /
  // pin_latest），失败经 hawk://capture-error 横幅回流。热键名硬编码，C7 切到设置。
  const trigger = (action: "capture" | "pin") => {
    invoke<void>("panel_trigger", { action }).catch((e) => setCaptureError(String(e)));
  };

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
      <section style={{ display: "flex", gap: 12, justifyContent: "center", margin: "16px 0" }}>
        <button style={entryBtn} onClick={() => trigger("capture")}>截图 (F1)</button>
        <button style={entryBtn} onClick={() => trigger("pin")}>贴图 (F2)</button>
        <button style={entryBtn} disabled title="历史功能将在后续版本提供">历史（即将开放）</button>
      </section>
      <section style={{ marginTop: 24, fontSize: 14, color: "#888" }}>
        <h2 style={{ fontSize: 16 }}>设置</h2>
        <p>设置项将在后续版本提供（热键自定义 / 输出格式 / 保存目录 / 开机自启）。</p>
      </section>
    </main>
  );
}

const entryBtn: CSSProperties = {
  padding: "10px 20px",
  fontSize: 15,
  borderRadius: 6,
  border: "1px solid #ccc",
  background: "#fff",
  cursor: "pointer",
};

export default App;
