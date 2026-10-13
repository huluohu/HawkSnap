import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

function App() {
  const [hotkeyError, setHotkeyError] = useState<string | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    // Rust 侧热键注册失败时发出（spec §9：不允许静默失败）。非 Tauri 环境下降级为无提示。
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
      <h1>HawkSnap</h1>
      <p>主面板尚未实现（属于 Plan C）。按 F1 开始截图。</p>
    </main>
  );
}

export default App;
