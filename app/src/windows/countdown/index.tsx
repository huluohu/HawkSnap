import { useEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t, setLanguage, getLanguage, subscribe, type LanguageSetting } from "../../lib/i18n";

// 取消：非活动期调用返回 Err（托盘「取消倒计时」恒可用带来的良性竞态，裁定 5），
// 吞掉。不依赖组件状态，提为模块级函数，事件监听无需随渲染重挂。
const cancel = () => { void invoke("cancel_delayed_capture").catch(() => {}); };

// 倒计时浮窗（Plan B Task 8）：Rust 侧 200×120 实心深色小窗、常驻复用（show/hide，
// 页面与监听跨越多轮存活）。显示 hawk://countdown 的剩余整秒数；点击任意处 / Esc 取消。
// v1.2（审计修复）：Rust 侧不再 set_focus 抢焦点——倒计时 3 秒内打断用户正在输入的
// 应用是不必要的惊扰；取消出口有三条（点击本窗 / Esc 当窗口聚焦时 / 托盘「取消倒计时」），
// 底部提示行让「点击可取消」从隐藏知识变成可见 affordance。
export default function CountdownWindow() {
  // 语言态订阅 + 挂载拉取设置（覆盖层同款：常驻复用窗口的语言在每轮 show 前无法
  // 由 Rust 推送，挂载拉一次 + 语言变更经设置保存后 set_settings 重建流程自然生效）。
  useSyncExternalStore(subscribe, getLanguage);
  const [remaining, setRemaining] = useState<number | null>(null);

  useEffect(() => {
    let disposed = false;
    void invoke<{ language: string }>("get_settings")
      .then((s) => { if (!disposed) setLanguage(s.language as LanguageSetting); })
      .catch(() => {});
    return () => { disposed = true; };
  }, []);

  // push 通道：整数秒变化逐秒推送。mount 时先 pull 一次（countdown_remaining）：窗口
  // 首建的 webview 冷加载期早于首条 emit 必丢（与 overlay-ready 三连发 / HOTKEY_ERROR
  // 拉取通道同一问题形态），拉取补显；非活动期为 null，照常等事件。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void invoke<number | null>("countdown_remaining")
      .then((r) => { if (!disposed && r !== null) setRemaining(r); })
      .catch(() => {});
    listen<{ remaining: number }>("hawk://countdown", (e) => {
      setRemaining(e.payload.remaining);
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // Esc 取消（窗口常驻复用：监听与页面同寿命，无需随轮重挂）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div onClick={cancel} role="timer" aria-live="off" aria-label={t("countdown.aria", { secs: remaining ?? 0 })} style={{
      position: "fixed", inset: 0, cursor: "pointer",
      display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
      gap: 4, background: "#0D1522",
    }}>
      <span style={{
        fontSize: 56, lineHeight: 1, color: "#FFB020", fontWeight: 600,
        fontVariantNumeric: "tabular-nums",
      }}>{remaining ?? ""}</span>
      <span style={{ fontSize: 11, lineHeight: 1.4, color: "#8FA3BD" }}>{t("countdown.hint")}</span>
    </div>
  );
}
