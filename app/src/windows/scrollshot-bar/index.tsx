import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t } from "../../lib/i18n";

// N5 长截图提示条（360×64 浮窗，点击穿透——Rust 侧 set_ignore_cursor_events：
// 用户要滚动被拍的内容，鼠标必须能穿过去；完成手势是 ⌘S，由 Rust 侧注册全局
// 热键兜住，浮窗不抢键盘焦点）。
//
// 帧进度语义单源在 Rust：hawk://scrollshot-tick {session, frames} 每抓一帧广播
// 一次；本页 mount 时经 scrollshot_status 拉一次补显（push 早于 listen 必丢，
// 与 recording-bar / countdown 同款问题形态）。
export default function ScrollshotBarWindow() {
  const [frames, setFrames] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  // 上一 tick 的会话号（ref 而非 state：listen 回调闭包不随渲染更新）。
  const lastSessionRef = useRef<number | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    // pull 补显 + push 订阅（窗口常驻复用：监听与页面同寿命）。
    void invoke<{ session: number; frames: number } | null>("scrollshot_status")
      .then((s) => {
        if (disposed || !s) return;
        lastSessionRef.current = s.session;
        setFrames(s.frames);
      })
      .catch(() => {});
    // listen 的 promise 拒绝 = 该窗口标签没有对应 capability（ACL 按标签授权）
    // ——必须 catch，否则变成 unhandled rejection（白屏 + 控制台噪音）。
    listen<{ session: number; frames: number }>("hawk://scrollshot-tick", (e) => {
      if (lastSessionRef.current !== e.payload.session) {
        lastSessionRef.current = e.payload.session;
        setError(null); // 新会话 → 清上一会话的残留错误
      }
      setFrames(e.payload.frames);
    })
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      })
      .catch((e) => {
        console.error(
          "[scrollshot-bar] listen(hawk://scrollshot-tick) 失败（窗口 capability 缺失？）",
          e,
        );
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const bar: CSSProperties = {
    position: "fixed", inset: 0, display: "flex", alignItems: "center",
    gap: 10, padding: "0 14px", boxSizing: "border-box", background: "#0D1522",
    fontFamily: "system-ui, sans-serif", userSelect: "none", cursor: "default",
  };
  const dot: CSSProperties = {
    width: 10, height: 10, borderRadius: "50%", background: "#FFB020", flexShrink: 0,
    animation: "hawksnap-ss-pulse 1.2s ease-in-out infinite",
  };
  const hint: CSSProperties = {
    fontSize: 13, lineHeight: 1, color: "#F5F7FA", flex: 1, minWidth: 0,
    whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis",
  };
  const count: CSSProperties = {
    fontSize: 12, lineHeight: 1, color: "#8A8F98", flexShrink: 0,
    fontVariantNumeric: "tabular-nums",
  };
  const err: CSSProperties = {
    fontSize: 10, color: "#E5484D", maxWidth: 120, overflow: "hidden",
    whiteSpace: "nowrap", flexShrink: 1,
  };

  return (
    <div style={bar} role="status" aria-label={t("scrollshot.bar.hint")}>
      <style>{`@keyframes hawksnap-ss-pulse { 50% { opacity: 0.35; } }`}</style>
      <span style={dot} aria-hidden="true" />
      <span style={hint}>{t("scrollshot.bar.hint")}</span>
      {error && <span style={err}>{error}</span>}
      <span style={count}>{t("scrollshot.bar.frames", { n: frames ?? 0 })}</span>
    </div>
  );
}
