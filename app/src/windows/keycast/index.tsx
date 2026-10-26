import { useEffect, useState, type CSSProperties } from "react";
import { listen } from "@tauri-apps/api/event";

// N4 按键显示小窗（320×88 透明窗体、点击穿透——Rust 侧 set_ignore_cursor_events）：
// 显示最近一次键盘组合键，1.5s CSS 淡出后隐形（fill forwards），窗体本身始终在屏
// 但零可见像素 = 零内容时的「无窗体验」。每次 hawk://keycast {combo, ts} 事件以
// 递增 key 重建胶囊节点 → CSS 动画自然重放（无需 class 开关/reflow 技巧）。
// Rust 侧 ComboThrottle 已做 100ms 同组合合并（长按自动重复只显示一次）。
export default function KeycastWindow() {
  const [combo, setCombo] = useState<string | null>(null);
  const [seq, setSeq] = useState(0);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<{ combo: string }>("hawk://keycast", (e) => {
      setCombo(e.payload.combo);
      setSeq((s) => s + 1); // key 变化 → 重建节点 → 淡出动画从头重放
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const wrap: CSSProperties = {
    position: "fixed", inset: 0, display: "flex", alignItems: "center",
    justifyContent: "center", pointerEvents: "none", userSelect: "none",
  };
  const pill: CSSProperties = {
    background: "rgba(13, 21, 34, 0.88)",
    border: "1px solid rgba(255, 255, 255, 0.14)",
    borderRadius: 14, padding: "12px 26px", boxSizing: "border-box",
    color: "#F5F7FA", fontFamily: "system-ui, sans-serif", fontSize: 30,
    fontWeight: 600, lineHeight: 1.2, letterSpacing: 1, whiteSpace: "nowrap",
    maxWidth: 304, overflow: "hidden", textOverflow: "ellipsis",
    textShadow: "0 1px 2px rgba(0, 0, 0, 0.4)",
    animation: "hawksnap-keycast-fade 1.5s ease-out forwards",
  };

  return (
    <div style={wrap} data-testid="keycast-root">
      <style>{`@keyframes hawksnap-keycast-fade { 0%, 60% { opacity: 1; } 100% { opacity: 0; } }`}</style>
      {combo !== null && (
        <div key={seq} style={pill} aria-hidden="true">{combo}</div>
      )}
    </div>
  );
}
