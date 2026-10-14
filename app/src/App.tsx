import { useEffect, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// —— 设置模型（C7）：与 Rust Settings（serde camelCase）逐字段对齐 ——
interface Settings {
  hotkeyCapture: string;
  hotkeyPin: string;
  hotkeyRecord: string;
  outputFormat: string; // "png" | "jpeg"（服务端 sanitize 兜底）
  jpegQuality: number; // 1-100（服务端 clamp）
  saveDir: string | null; // null/空 = 默认 ~/Pictures/HawkSnap
  launchAtLogin: boolean;
  appearance: string; // "system" | "light" | "dark"
}

// 表单态：saveDir 归一为 string（null ↔ "" 双向转换），便于受控输入。
type SettingsForm = Omit<Settings, "saveDir"> & { saveDir: string };

const DEFAULTS: SettingsForm = {
  hotkeyCapture: "F1",
  hotkeyPin: "F2",
  hotkeyRecord: "F6",
  outputFormat: "png",
  jpegQuality: 90,
  saveDir: "",
  launchAtLogin: false,
  appearance: "system",
};

const toForm = (s: Settings): SettingsForm => ({ ...s, saveDir: s.saveDir ?? "" });
const toPayload = (f: SettingsForm): Settings => ({
  ...f,
  saveDir: f.saveDir.trim() === "" ? null : f.saveDir.trim(),
});

type HotkeySlot = "hotkeyCapture" | "hotkeyPin" | "hotkeyRecord";

// 热键注册失败载荷（D0-⑤）：与后端 HotkeyError（serde camelCase）对齐——kind 供横幅
// 归因（截图/贴图/录屏），message 为人类可读文案（注册失败原因或媒体键降级提示）。
type HotkeyKind = "capture" | "pin" | "record";
interface HotkeyErrorPayload {
  kind: HotkeyKind;
  message: string;
}
const HOTKEY_KIND_LABEL: Record<HotkeyKind, string> = {
  capture: "截图",
  pin: "贴图",
  record: "录屏",
};

const HOTKEY_SLOTS: Array<{ key: HotkeySlot; label: string }> = [
  { key: "hotkeyCapture", label: "开始捕获" },
  { key: "hotkeyPin", label: "贴图" },
  { key: "hotkeyRecord", label: "录屏（占位）" },
];

// 纯修饰键按下（无主键）：不算组合，继续等待。
const MODIFIER_ONLY_KEYS = new Set([
  "Meta", "Shift", "Alt", "Control", "CapsLock", "Fn", "FnLock", "Hyper", "Symbol",
]);

// 组合键 → 插件词汇表字符串（global-hotkey parse_hotkey：修饰键 Cmd/Shift/Alt/Ctrl
// 在前 + 主键，"+" 连接；主键大小写不敏感，字母统一大写展示）。返回 null = 忽略
// （纯修饰键）；不认识的键不在此拦截——服务端 set_settings 解析失败即冲突 Err，
// 服务端是唯一权威。
function comboFromEvent(e: KeyboardEvent): string | null {
  if (MODIFIER_ONLY_KEYS.has(e.key)) return null;
  const mods: string[] = [];
  if (e.metaKey) mods.push("Cmd");
  if (e.shiftKey) mods.push("Shift");
  if (e.altKey) mods.push("Alt");
  if (e.ctrlKey) mods.push("Ctrl");
  // 数字键带修饰时 e.key 会变成符号（Shift+1 → "!"），按 e.code 取回数字本键；
  // 其余单字符统一大写（插件 parse_key 大小写不敏感，展示习惯大写字母）。
  let key: string;
  if (/^Digit\d$/.test(e.code)) key = e.code.slice(5);
  else if (e.key === " ") key = "Space";
  else if (e.key.length === 1) key = e.key.toUpperCase();
  else key = e.key;
  return [...mods, key].join("+");
}

function App() {
  const [hotkeyError, setHotkeyError] = useState<HotkeyErrorPayload | null>(null);
  const [captureError, setCaptureError] = useState<string | null>(null);
  const [comingSoon, setComingSoon] = useState<string | null>(null);
  const [form, setForm] = useState<SettingsForm>(DEFAULTS);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [capturing, setCapturing] = useState<HotkeySlot | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveOk, setSaveOk] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    // pull 通道：setup 期的 hawk://hotkey-error emit 早于 listen 注册必丢，挂载即向后端
    // 拉取（后端读取即清除）。非 Tauri 环境下降级为无提示。
    invoke<HotkeyErrorPayload | null>("hotkey_error")
      .then((err) => {
        if (!disposed && err) setHotkeyError(err);
      })
      .catch(() => {});
    // push 通道：覆盖挂载后发生的注册失败（spec §9：不允许静默失败）。
    listen<HotkeyErrorPayload>("hawk://hotkey-error", (e) => setHotkeyError(e.payload))
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

  // 截图失败横幅：F1/托盘两条路径的 start_capture_flow 失败均 emit hawk://capture-error
  // （spec §9：不允许静默失败）。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<string>("hawk://capture-error", (e) => setCaptureError(e.payload))
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

  // F6 占位（C7）：热键触发后端 emit「即将推出」，复用横幅展示，数秒后自动消隐。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<string>("hawk://coming-soon", (e) => {
      setComingSoon(e.payload);
      window.setTimeout(
        () => setComingSoon((cur) => (cur === e.payload ? null : cur)),
        4000,
      );
    })
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

  // 挂载即拉取设置填充表单（面板由 idle_reaper 销毁后重建时会重挂载 → 总能拿到最新值）。
  useEffect(() => {
    invoke<Settings>("get_settings")
      .then((s) => setForm(toForm(s)))
      .catch((e) => setSettingsError(String(e)));
  }, []);

  // 热键捕获模式：捕获态挂 window 捕获相 keydown——下一个组合键写入对应槽位；Esc 取消；
  // 纯修饰键忽略。capture 相 + stopPropagation 避免页面其他快捷逻辑收到该事件。
  useEffect(() => {
    if (!capturing) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setCapturing(null);
        return;
      }
      const combo = comboFromEvent(e);
      if (!combo) return;
      setForm((f) => ({ ...f, [capturing]: combo }));
      setCapturing(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [capturing]);

  // 面板入口（C6）：与 F1/F2 热键、托盘同链路（panel_trigger → start_capture_flow /
  // pin_latest），失败经 hawk://capture-error 横幅回流。热键名随设置回显（C7）。
  const trigger = (action: "capture" | "pin") => {
    invoke<void>("panel_trigger", { action }).catch((e) => setCaptureError(String(e)));
  };

  // 保存：客户端只做非空校验，冲突/解析权威在服务端——set_settings Err → 横幅 +
  // 重新拉取服务端设置回显（Err 不携带设置，且文件保持旧值）。
  const save = () => {
    if (
      form.hotkeyCapture.trim() === "" ||
      form.hotkeyPin.trim() === "" ||
      form.hotkeyRecord.trim() === ""
    ) {
      setSettingsError("热键不能为空");
      return;
    }
    setSaving(true);
    invoke<Settings>("set_settings", { s: toPayload(form) })
      .then((effective) => {
        setForm(toForm(effective)); // 回显服务端生效值（sanitize 后）
        setSettingsError(null);
        setSaveOk(true);
        window.setTimeout(() => setSaveOk(false), 1600);
      })
      .catch((e) => {
        setSettingsError(String(e));
        return invoke<Settings>("get_settings").then((s) => setForm(toForm(s)));
      })
      .finally(() => setSaving(false));
  };

  const setField = <K extends keyof SettingsForm>(key: K, value: SettingsForm[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  return (
    <main className="container">
      {hotkeyError && (
        <div role="alert" style={banner}>
          {HOTKEY_KIND_LABEL[hotkeyError.kind]} 热键注册失败：{hotkeyError.message}
        </div>
      )}
      {captureError && (
        <div role="alert" style={banner}>
          截图失败：{captureError}
        </div>
      )}
      {comingSoon && (
        <div role="status" style={{ ...banner, borderColor: "#f0ad4e", background: "#fcf8e3", color: "#8a6d3b" }}>
          {comingSoon}
        </div>
      )}
      {settingsError && (
        <div role="alert" style={banner}>
          设置保存失败：{settingsError}
        </div>
      )}
      <h1>HawkSnap</h1>
      <section style={{ display: "flex", gap: 12, justifyContent: "center", margin: "16px 0" }}>
        <button style={entryBtn} onClick={() => trigger("capture")}>截图 ({form.hotkeyCapture})</button>
        <button style={entryBtn} onClick={() => trigger("pin")}>贴图 ({form.hotkeyPin})</button>
        <button style={entryBtn} disabled title="历史功能将在后续版本提供">历史（即将开放）</button>
      </section>
      <section style={settingsSection}>
        <h2 style={{ fontSize: 16, margin: "0 0 12px" }}>设置</h2>

        <div style={fieldRow}>
          <label style={fieldLabel}>热键</label>
          <div style={{ display: "grid", gap: 6 }}>
            {HOTKEY_SLOTS.map(({ key, label }) => (
              <div key={key} style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ width: 90, color: "#666" }}>{label}</span>
                <button
                  type="button"
                  style={{ ...entryBtn, minWidth: 140, fontFamily: "monospace" }}
                  onClick={() => setCapturing(key)}
                >
                  {form[key]}
                </button>
                {capturing === key && (
                  <span style={{ color: "#396cd8" }}>按下组合键…（Esc 取消）</span>
                )}
              </div>
            ))}
          </div>
        </div>

        <div style={fieldRow}>
          <label style={fieldLabel}>输出格式</label>
          <div style={{ display: "flex", alignItems: "center", gap: 16 }}>
            <label>
              <input
                type="radio"
                name="output-format"
                checked={form.outputFormat === "png"}
                onChange={() => setField("outputFormat", "png")}
              />{" "}
              PNG
            </label>
            <label>
              <input
                type="radio"
                name="output-format"
                checked={form.outputFormat === "jpeg"}
                onChange={() => setField("outputFormat", "jpeg")}
              />{" "}
              JPEG
            </label>
            {form.outputFormat === "jpeg" && (
              <label style={{ display: "flex", alignItems: "center", gap: 8 }}>
                质量 {form.jpegQuality}
                <input
                  type="range"
                  min={1}
                  max={100}
                  value={form.jpegQuality}
                  onChange={(e) => setField("jpegQuality", Number(e.target.value))}
                />
              </label>
            )}
          </div>
        </div>

        <div style={fieldRow}>
          <label style={fieldLabel} htmlFor="save-dir">保存目录</label>
          <div style={{ display: "flex", gap: 8 }}>
            <input
              id="save-dir"
              type="text"
              style={{ ...entryBtn, flex: 1, cursor: "text" }}
              placeholder="默认：~/Pictures/HawkSnap"
              value={form.saveDir}
              onChange={(e) => setField("saveDir", e.target.value)}
            />
            <button
              type="button"
              style={entryBtn}
              onClick={() => setField("saveDir", "")}
              title="清空 = 使用默认目录"
            >
              恢复默认
            </button>
          </div>
        </div>

        <div style={fieldRow}>
          <label style={fieldLabel} htmlFor="launch-at-login">开机自启</label>
          <label style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <input
              id="launch-at-login"
              type="checkbox"
              checked={form.launchAtLogin}
              onChange={(e) => setField("launchAtLogin", e.target.checked)}
            />
            登录时自动启动 HawkSnap
          </label>
        </div>

        <div style={fieldRow}>
          <span style={fieldLabel}>外观</span>
          <div>
            {(["system", "light", "dark"] as const).map((mode) => (
              <label key={mode} style={{ marginRight: 16 }}>
                <input
                  type="radio"
                  name="appearance"
                  checked={form.appearance === mode}
                  onChange={() => setField("appearance", mode)}
                />{" "}
                {mode === "system" ? "跟随系统" : mode === "light" ? "浅色" : "深色"}
              </label>
            ))}
          </div>
        </div>

        <div style={fieldRow}>
          <label style={fieldLabel} htmlFor="language">语言</label>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <select id="language" style={{ ...entryBtn }} disabled>
              <option>中文</option>
            </select>
            <span style={planDBadge}>Plan D</span>
          </div>
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 12, marginTop: 16 }}>
          <button type="button" style={{ ...entryBtn, padding: "8px 32px" }} onClick={save} disabled={saving}>
            {saving ? "保存中…" : "保存"}
          </button>
          {saveOk && <span style={{ color: "#3c763d" }}>已保存</span>}
        </div>
      </section>
    </main>
  );
}

const banner: CSSProperties = {
  margin: "12px",
  padding: "10px 14px",
  border: "1px solid #d9534f",
  borderRadius: 6,
  background: "#fdecea",
  color: "#a94442",
  fontSize: 14,
};

const entryBtn: CSSProperties = {
  padding: "10px 20px",
  fontSize: 15,
  borderRadius: 6,
  border: "1px solid #ccc",
  background: "#fff",
  cursor: "pointer",
};

const settingsSection: CSSProperties = {
  margin: "24px auto 40px",
  maxWidth: 560,
  textAlign: "left",
  fontSize: 14,
};

const fieldRow: CSSProperties = {
  display: "grid",
  gridTemplateColumns: "90px 1fr",
  alignItems: "start",
  gap: 8,
  marginBottom: 14,
};

const fieldLabel: CSSProperties = { color: "#666", paddingTop: 6 };

const planDBadge: CSSProperties = {
  fontSize: 11,
  padding: "1px 6px",
  border: "1px solid #bbb",
  borderRadius: 8,
  color: "#888",
};

export default App;
