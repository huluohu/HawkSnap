import React, { Suspense, lazy } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
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

// 前端就绪握手（2026-11-20 真机验收）：挂载后立即 invoke 一次 → 后端落一行
// `[hawksnap] frontend ready: <window>`。它是「CSP 放行了 script + connect-src
// 放行了 IPC」的运行时证据（CSP 写错时页面会加载但空白，日志里**没有**这行）。
// 失败（invoke 被拦/后端不可用）只记 console——握手不该影响任何功能。
//
// 回执 ack.smoke = 后端处于自检模式（HAWKSNAP_SMOKE=1）→ 此时**额外**做一次
// 资产探测：等历史瓦片里的缩略图出现并真的 onload → 回报（证明 asset 协议的
// img-src 在真实 webview 上放行）。**非自检模式不做任何额外事**（生产零开销）。
void invoke<{ smoke: boolean }>("frontend_ready", { window: route ?? "main" })
  .then((ack) => {
    // 只有主面板做资产探测（它渲染历史缩略图 → asset: URL）；编辑器/设置窗
    // 没有缩略图，探测它们只会误报 fail。
    if (ack?.smoke && (route ?? "main") === "main") probeAssetProtocol(route ?? "main");
  })
  .catch(() => {
    // 握手失败不阻断渲染；仅提示（真机排障线索）。
    console.warn("[hawksnap] frontend_ready 握手未成功");
  });

/** 自检模式：探测 asset 协议加载（历史缩略图 → asset: URL → img-src）。 */
function probeAssetProtocol(windowLabel: string) {
  const report = (result: string) => {
    void invoke("frontend_ready", { window: windowLabel, asset: result }).catch(() => {});
  };
  const settle = (img: HTMLImageElement) => {
    // load 事件不重放：已完成且有自然宽 → 立即判 ok。
    if (img.complete && img.naturalWidth > 0) {
      report(`ok:${img.naturalWidth}x${img.naturalHeight}`);
      return;
    }
    img.addEventListener("load", () => report(`ok:${img.naturalWidth}x${img.naturalHeight}`), {
      once: true,
    });
    img.addEventListener("error", () => report("fail"), { once: true });
  };
  const tryBind = () => {
    // 选择器：缩略图的 class 在 **img 自身**（App.tsx: className="hs-thumb"），
    // 不是父容器 —— 2026-11-20 首轮 CI 自检实测：写成 `.hs-thumb img` 永远匹配
    // 不到（报 fail:no-thumb）。两种形态都收，避免将来结构调整再踩。
    const img = document.querySelector<HTMLImageElement>(
      "img.hs-thumb, .hs-thumb img",
    );
    if (!img) return false;
    settle(img);
    return true;
  };
  if (tryBind()) return;
  // 历史是懒加载的：等 DOM 里出现缩略图（MutationObserver，无需轮询）。
  const obs = new MutationObserver(() => {
    if (tryBind()) obs.disconnect();
  });
  obs.observe(document.body, { childList: true, subtree: true });
  // 兜底：15s 后若仍未见缩略图（历史为空等），回报 fail（自检据此判红）。
  window.setTimeout(() => {
    obs.disconnect();
    if (!document.querySelector("img.hs-thumb, .hs-thumb img")) report("fail:no-thumb");
  }, 15000);
}

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
