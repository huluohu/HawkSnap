import React, { Suspense, lazy } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// 覆盖层窗口由 Rust 侧 WebviewUrl::App("overlay?window=overlay")、倒计时浮窗由
// WebviewUrl::App("countdown?window=countdown")、编辑器窗口由
// WebviewUrl::App("editor?window=editor")、录屏悬浮条由
// WebviewUrl::App("recording-bar?window=recording-bar")、按键显示小窗由
// WebviewUrl::App("keycast?window=keycast")、长截图提示条由
// WebviewUrl::App("scrollshot-bar?window=scrollshot-bar")、设置窗口由
// WebviewUrl::App("?window=settings") 打开，这里按 query 区分渲染
// 内容：?window=overlay → 冻结帧覆盖层，?window=countdown → 延时倒计时浮窗，
// ?window=editor → 标注编辑器，?window=recording-bar → 录屏悬浮条，
// ?window=keycast → 按键显示小窗，?window=scrollshot-bar → 长截图提示条，
// ?window=settings → 独立设置窗口（侧栏分区 + 卡片内容），否则默认主窗口。
//
// FE-P2-7（2026-11-20 前端审计）：此前全部窗口组件静态 import——每个窗口（keycast
// 倒计时这类小浮窗）都要解析执行全量 bundle，覆盖层冷加载（Rust 实测 ~267ms 口径）
// 随窗口数放大。React.lazy 按路由分包：共享 lib（i18n/errors/history）留公共 chunk，
// 各窗口只载自己的 chunk；主面板（App）保持静态（托盘首开最常见路径）。Suspense
// fallback = null：本地 chunk 加载 <10ms，空白帧不可感。
const OverlayWindow = lazy(() => import("./windows/overlay"));
const CountdownWindow = lazy(() => import("./windows/countdown"));
const EditorWindow = lazy(() => import("./windows/editor"));
const RecordingBarWindow = lazy(() => import("./windows/recording-bar"));
const KeycastWindow = lazy(() => import("./windows/keycast"));
const ScrollshotBarWindow = lazy(() => import("./windows/scrollshot-bar"));
const SettingsWindow = lazy(() => import("./windows/settings"));

const route = new URLSearchParams(window.location.search).get("window");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Suspense fallback={null}>
      {route === "overlay"
        ? <OverlayWindow />
        : route === "countdown" ? <CountdownWindow />
        : route === "editor" ? <EditorWindow />
        : route === "recording-bar" ? <RecordingBarWindow />
        : route === "keycast" ? <KeycastWindow />
        : route === "scrollshot-bar" ? <ScrollshotBarWindow />
        : route === "settings" ? <SettingsWindow />
        : <App />}
    </Suspense>
  </React.StrictMode>,
);
