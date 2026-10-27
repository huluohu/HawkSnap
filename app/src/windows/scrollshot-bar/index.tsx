import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t } from "../../lib/i18n";

// N5 长截图提示条（360×64 浮窗）。
//
// **可交互**（I4）：条上有「完成」按钮 → 用户始终有鼠标出口（⌘S 完成热键注册
// 失败时它是唯一出口）。Rust 侧不再设点击穿透——代价是条覆盖区域不再鼠标
// 穿透，故条被刻意定位在避开选区的上下边缘。
//
// 帧进度语义单源在 Rust：hawk://scrollshot-tick {session, frames, hotkey?} 每
// 抓一帧广播一次；本页 mount 时经 scrollshot_status 拉一次补显（push 早于
// listen 必丢，与 recording-bar / countdown 同款问题形态）。
export default function ScrollshotBarWindow() {
  const [frames, setFrames] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  // ⌘S 完成热键是否可用（false → 「完成」按钮是唯一出口，条上明示）。
  const [hotkeyOk, setHotkeyOk] = useState(true);
  // 上一 tick 的会话号（ref 而非 state：listen 回调闭包不随渲染更新）。
  const lastSessionRef = useRef<number | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    // pull 补显 + push 订阅（窗口常驻复用：监听与页面同寿命）。
    void invoke<{ session: number; frames: number; hotkey?: boolean } | null>(
      "scrollshot_status",
    )
      .then((s) => {
        if (disposed || !s) return;
        lastSessionRef.current = s.session;
        setFrames(s.frames);
        // 热键状态**必须**走 pull 通道：会话启动时那次 push（带 hotkey）几乎
        // 必然早于本页 listen 就绪 → 必丢（push 早于 listen 是本仓老问题形态）。
        if (typeof s.hotkey === "boolean") setHotkeyOk(s.hotkey);
      })
      .catch(() => {});
    // listen 的 promise 拒绝 = 该窗口标签没有对应 capability（ACL 按标签授权）
    // ——必须 catch，否则变成 unhandled rejection（白屏 + 控制台噪音）。
    listen<{ session: number; frames: number; hotkey?: boolean }>(
      "hawk://scrollshot-tick",
      (e) => {
        if (lastSessionRef.current !== e.payload.session) {
          lastSessionRef.current = e.payload.session;
          setError(null); // 新会话 → 清上一会话的残留错误
        }
        setFrames(e.payload.frames);
        if (typeof e.payload.hotkey === "boolean") setHotkeyOk(e.payload.hotkey);
      },
    )
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

  // 完成：invoke scrollshot_stop（Rust 侧拼接 + 落盘 + 入库）。失败 → 行内错误
  // （条保持可见，用户可重试或用 ⌘S）。
  const finish = () => {
    void invoke<string>("scrollshot_stop").catch((e) => setError(String(e)));
  };

  const bar: CSSProperties = {
    position: "fixed", inset: 0, display: "flex", alignItems: "center",
    gap: 10, padding: "0 12px", boxSizing: "border-box", background: "#0D1522",
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
    fontSize: 10, color: "#E5484D", maxWidth: 110, overflow: "hidden",
    whiteSpace: "nowrap", flexShrink: 1,
  };
  const btn: CSSProperties = {
    fontSize: 12, padding: "5px 10px", borderRadius: 6, border: "1px solid #2A3648",
    background: "#16202E", color: "#E8ECF1", cursor: "pointer", flexShrink: 0,
  };
  const btnDanger: CSSProperties = { ...btn, background: "#3A1620", borderColor: "#5B2430" };

  // ⌘S 不可用 → 提示语改口（否则用户会一直等一个永远不来的热键）。
  const hintText = hotkeyOk
    ? t("scrollshot.bar.hint")
    : t("scrollshot.bar.hintNoHotkey");

  return (
    <div style={bar} role="status" aria-label={hintText}>
      <style>{`@keyframes hawksnap-ss-pulse { 50% { opacity: 0.35; } }`}</style>
      <span style={dot} aria-hidden="true" />
      <span style={hint}>{hintText}</span>
      {error && <span style={err}>{error}</span>}
      <span style={count}>{t("scrollshot.bar.frames", { n: frames ?? 0 })}</span>
      {/* 完成按钮：⌘S 不可用时它是唯一出口 → 高亮提示 */}
      <button
        style={hotkeyOk ? btn : btnDanger}
        onClick={finish}
        title={hotkeyOk ? t("scrollshot.bar.finish") : t("scrollshot.bar.finishOnly")}
      >
        {t("scrollshot.bar.finish")}
      </button>
    </div>
  );
}
