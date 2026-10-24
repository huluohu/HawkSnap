import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t } from "../../lib/i18n";

// M6 录屏悬浮条（280×56 深色实心、常驻复用——与 countdown 同款 show/hide 模式）：
// 计时 mm:ss + 暂停/恢复 + 停止。时间语义单源在 Rust（hawk://record-tick 每秒广播
// {session, seconds, paused}），本页纯渲染——webview 后台节流不影响计时正确性。mount 时先经
// record_status 拉取补显（push 早于 listen 必丢，countdown 同一问题形态）。
// mm:ss（tabular-nums 由 time 样式保证）
const mmss = (s: number) =>
  `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;

export default function RecordingBarWindow() {
  const [seconds, setSeconds] = useState<number | null>(null);
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 上一 tick 的会话号（ref 而非 state：listen 回调闭包不随渲染更新，state 会取到
  // 初值 null 导致每 tick 都判「新会话」而误清当会话错误）。
  const lastSessionRef = useRef<number | null>(null);

  // pull 补显 + push 订阅（窗口常驻复用：监听与页面同寿命）
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void invoke<{ seconds: number; paused: boolean } | null>("record_status")
      .then((r) => {
        if (disposed || !r) return;
        setSeconds(r.seconds);
        setPaused(r.paused);
      })
      .catch(() => {});
    listen<{ session: number; seconds: number; paused: boolean }>("hawk://record-tick", (e) => {
      // 新会话（session 由 Rust 每次 record_start 单调取号）：清除上一会话的行内
      // 错误残留（M6 遗留 minor——窗口常驻复用，组件状态跨会话存活）。
      if (lastSessionRef.current !== e.payload.session) {
        lastSessionRef.current = e.payload.session;
        setError(null);
      }
      setSeconds(e.payload.seconds);
      setPaused(e.payload.paused);
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const togglePause = () => {
    const cmd = paused ? "record_resume" : "record_pause";
    void invoke(cmd).catch((e) => setError(String(e)));
  };

  // 停止（评审修复：不静默）——record_stop 失败（磁盘满 / MP4 收尾失败 / 收尾超时）
  // 此前被吞掉而悬浮条已隐藏，主停止入口零反馈。与 togglePause 同款行内 setError：
  // Rust 侧仍持会话（stop 已从 REC 取出、半成品保留在盘），错误文案即 Err 串。
  const stop = () => {
    void invoke<string>("record_stop").catch((e) => setError(String(e)));
  };

  const bar: CSSProperties = {
    position: "fixed", inset: 0, display: "flex", alignItems: "center",
    gap: 8, padding: "0 12px", boxSizing: "border-box", background: "#0D1522",
    fontFamily: "system-ui, sans-serif", userSelect: "none", cursor: "default",
  };
  const dot: CSSProperties = {
    width: 10, height: 10, borderRadius: "50%",
    background: paused ? "#8a8f98" : "#E5484D",
    animation: paused ? "none" : "hawksnap-rec-pulse 1.2s ease-in-out infinite",
    flexShrink: 0,
  };
  const time: CSSProperties = {
    fontSize: 20, lineHeight: 1, color: "#F5F7FA", fontWeight: 600,
    fontVariantNumeric: "tabular-nums", flex: 1, minWidth: 0,
  };
  const btn: CSSProperties = {
    fontSize: 12, padding: "5px 10px", borderRadius: 6, border: "1px solid #2A3648",
    background: "#16202E", color: "#E8ECF1", cursor: "pointer", flexShrink: 0,
  };
  const btnDanger: CSSProperties = { ...btn, background: "#3A1620", borderColor: "#5B2430" };

  return (
    <div style={bar} role="status" aria-label={t("record.barLabel")}>
      <style>{`@keyframes hawksnap-rec-pulse { 50% { opacity: 0.35; } }`}</style>
      <span style={dot} aria-hidden="true" />
      <span style={time}>
        {paused ? <span style={{ color: "#FFB020", fontSize: 12, marginRight: 6 }}>{t("record.paused")}</span> : null}
        {seconds === null ? "00:00" : mmss(seconds)}
      </span>
      {error && <span style={{ fontSize: 10, color: "#E5484D", maxWidth: 80, overflow: "hidden" }}>{error}</span>}
      <button style={btn} onClick={togglePause}>
        {paused ? t("record.resume") : t("record.pause")}
      </button>
      <button style={btnDanger} onClick={stop}>{t("record.stop")}</button>
    </div>
  );
}
