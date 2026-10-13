import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import OverlayWindow from "./windows/overlay";

// 覆盖层窗口由 Rust 侧 WebviewUrl::App("overlay?window=overlay") 打开，
// 这里按 query 区分渲染内容：?window=overlay → 冻结帧覆盖层，否则默认主窗口占位页。
const route = new URLSearchParams(window.location.search).get("window");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {route === "overlay" ? <OverlayWindow /> : <App />}
  </React.StrictMode>,
);
