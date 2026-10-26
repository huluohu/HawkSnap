import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import OverlayWindow from "./windows/overlay";
import CountdownWindow from "./windows/countdown";
import EditorWindow from "./windows/editor";
import RecordingBarWindow from "./windows/recording-bar";
import KeycastWindow from "./windows/keycast";

// 覆盖层窗口由 Rust 侧 WebviewUrl::App("overlay?window=overlay")、倒计时浮窗由
// WebviewUrl::App("countdown?window=countdown")、编辑器窗口由
// WebviewUrl::App("editor?window=editor")、录屏悬浮条由
// WebviewUrl::App("recording-bar?window=recording-bar")、按键显示小窗由
// WebviewUrl::App("keycast?window=keycast") 打开，这里按 query 区分渲染
// 内容：?window=overlay → 冻结帧覆盖层，?window=countdown → 延时倒计时浮窗，
// ?window=editor → 标注编辑器，?window=recording-bar → 录屏悬浮条，
// ?window=keycast → 按键显示小窗，否则默认主窗口。
const route = new URLSearchParams(window.location.search).get("window");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {route === "overlay"
      ? <OverlayWindow />
      : route === "countdown" ? <CountdownWindow />
      : route === "editor" ? <EditorWindow />
      : route === "recording-bar" ? <RecordingBarWindow />
      : route === "keycast" ? <KeycastWindow />
      : <App />}
  </React.StrictMode>,
);
