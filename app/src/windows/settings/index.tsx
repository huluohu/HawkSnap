// ============================================================================
// HawkSnap 设置窗口（独立 webview，label "settings"，URL ?window=settings）
// ----------------------------------------------------------------------------
// 布局：左侧栏（搜索 + 应用资料 + 8 个分区导航）固定，右侧内容区可滚动；
//       顶部一排为「上一页 / 下一页 + 分区标题 + 右上角圆形保存钮」。
//
// 主题：appearance 来自 get_settings（system|light|dark）。写入
//       document.documentElement.dataset.theme —— 但**先读现有值**：若已有
//       "dark"/"light"（别的来源写过，例如主面板 App.tsx 也写同一个 dataset）
//       则让位不覆盖，避免两个窗口互相踩。system 时用
//       matchMedia("(prefers-color-scheme: dark)") 判定并订阅其变化。
//       主题同时镜像到根节点 data-theme（CSS 用 .hs-settings[data-theme="dark"]
//       兜底）与 body（铺底色）。窗口级标记 body[data-set-window="1"] 用于
//       清零 body 默认 margin。
//
// i18n：本窗口不碰 locales 词典（改词典会波及主面板），文案走 sections.tsx
//       里的本地 zh/en 表 + useT()（订阅 lib/i18n.ts 的语言态）。
//
// 命令契约：get_settings / set_settings（读写）、permission_status、
//       open_permission_settings、open_external、open_main_panel、
//       check_for_updates / install_update / restart_app —— 全部 catch，
//       失败给可读错误，绝不白屏。
// ============================================================================
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { setLanguage, type LanguageSetting } from "../../lib/i18n";
import {
  AboutSection,
  AnnotateSection,
  CaptureSection,
  GeneralSection,
  HistorySection,
  Icon,
  MenuBarSection,
  PermissionsSection,
  ShortcutsSection,
  comboFromEvent,
  useT,
  type CopyKey,
  type HotkeySlot,
  type IconName,
  type SectionProps,
  type SettingsForm,
} from "./sections";
// 真应用图标（光圈 + 鹰眼）——资料头用真图，不用 emoji 近似。
import appIcon from "../../assets/app-icon.png";
import "./Settings.css";

// —— 设置模型（与 Rust Settings 的 serde camelCase 对齐）——

interface Settings {
  hotkeyCapture: string;
  hotkeyPin: string;
  hotkeyRecord: string;
  outputFormat: string;
  jpegQuality: number;
  saveDir: string | null;
  launchAtLogin: boolean;
  appearance: string;
  language: string;
  updateEnabled: boolean;
}

const DEFAULTS: SettingsForm = {
  hotkeyCapture: "F1",
  hotkeyPin: "F2",
  hotkeyRecord: "F6",
  outputFormat: "png",
  jpegQuality: 90,
  saveDir: "",
  launchAtLogin: false,
  appearance: "system",
  language: "system",
  updateEnabled: true,
};

// saveDir 归一：null ↔ ""（空 = 默认 ~/Pictures/HawkSnap）。
const toForm = (s: Settings): SettingsForm => ({ ...s, saveDir: s.saveDir ?? "" });
const toPayload = (f: SettingsForm): Settings => ({
  ...f,
  saveDir: f.saveDir.trim() === "" ? null : f.saveDir.trim(),
});

// —— 分区表（id 与后端 open_main_panel 的 section 字符串一一对应）——

type SectionId =
  | "general"
  | "capture"
  | "annotate"
  | "history"
  | "shortcuts"
  | "permissions"
  | "menubar"
  | "about";

interface SectionDef {
  id: SectionId;
  labelKey: CopyKey;
  titleKey: CopyKey;
  icon: IconName;
  /** 侧栏图标色相（每分区一色，参照参考图的彩色圆角块——黑底统一色没有区分度） */
  hue: string;
  render: (props: SectionProps) => ReactNode;
}

const SECTIONS: SectionDef[] = [
  {
    id: "general",
    labelKey: "nav.general",
    titleKey: "general.title",
    icon: "sliders",
    hue: "#34c759",
    render: (p) => <GeneralSection {...p} />,
  },
  {
    id: "capture",
    labelKey: "nav.capture",
    titleKey: "capture.title",
    icon: "camera",
    hue: "#ff9f0a",
    render: (p) => <CaptureSection {...p} />,
  },
  {
    id: "annotate",
    labelKey: "nav.annotate",
    titleKey: "annotate.title",
    icon: "pen",
    hue: "#5e5ce6",
    render: (p) => <AnnotateSection {...p} />,
  },
  {
    id: "history",
    labelKey: "nav.history",
    titleKey: "history.title",
    icon: "clock",
    hue: "#0a84ff",
    render: (p) => <HistorySection {...p} />,
  },
  {
    id: "shortcuts",
    labelKey: "nav.shortcuts",
    titleKey: "shortcuts.title",
    icon: "keyboard",
    hue: "#8e8e93",
    render: (p) => <ShortcutsSection {...p} />,
  },
  {
    id: "permissions",
    labelKey: "nav.permissions",
    titleKey: "permissions.title",
    icon: "shield",
    hue: "#ff453a",
    render: (p) => <PermissionsSection {...p} />,
  },
  {
    id: "menubar",
    labelKey: "nav.menubar",
    titleKey: "menubar.title",
    icon: "menuBar",
    hue: "#40c8e0",
    render: (p) => <MenuBarSection {...p} />,
  },
  {
    id: "about",
    labelKey: "nav.about",
    titleKey: "about.title",
    icon: "infoCircle",
    hue: "#5ac8fa",
    render: (p) => <AboutSection {...p} />,
  },
];

export default function SettingsWindow() {
  const tx = useT();

  const [active, setActive] = useState<SectionId>("general");
  const [query, setQuery] = useState("");
  const [form, setForm] = useState<SettingsForm>(DEFAULTS);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [capturing, setCapturing] = useState<HotkeySlot | null>(null);
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [version, setVersion] = useState("");
  // 主题是否已由本窗口接管（首帧让位给已有值，之后的变更才由本窗口写）。
  const themeOwnedRef = useRef(false);

  // —— 窗口级标记：body[data-set-window="1"]（CSS 里据此清零 margin）——
  useEffect(() => {
    document.body.dataset.setWindow = "1";
    return () => {
      delete document.body.dataset.setWindow;
    };
  }, []);

  // —— 拉取设置填充表单（窗口重建时会重新挂载 → 总能拿到最新值）——
  useEffect(() => {
    let disposed = false;
    invoke<Settings>("get_settings")
      .then((s) => {
        if (disposed) return;
        setForm(toForm(s));
        setLanguage(s.language as LanguageSetting);
        setLoaded(true);
      })
      .catch((e) => {
        if (disposed) return;
        setError(tx("error.generic", { message: String(e) }));
        setLoaded(true);
      });
    return () => {
      disposed = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 仅挂载时拉一次
  }, []);

  // —— 应用版本号（侧栏资料头）——
  useEffect(() => {
    let disposed = false;
    getVersion()
      .then((v) => {
        if (!disposed) setVersion(v);
      })
      .catch(() => {});
    return () => {
      disposed = true;
    };
  }, []);

  // —— 外观主题接线 ——
  useEffect(() => {
    if (!loaded) return;
    const root = document.documentElement;
    const mq =
      typeof window.matchMedia === "function"
        ? window.matchMedia("(prefers-color-scheme: dark)")
        : null;
    const compute = (): "light" | "dark" => {
      if (form.appearance === "dark") return "dark";
      if (form.appearance === "light") return "light";
      return mq?.matches ? "dark" : "light";
    };
    const write = (next: "light" | "dark") => {
      root.dataset.theme = next;
      document.body.dataset.theme = next;
      setTheme(next);
    };
    if (!themeOwnedRef.current) {
      // 首帧：已有值就让位（不覆盖别人写好的 theme），否则按 appearance 落定。
      themeOwnedRef.current = true;
      const existing = root.dataset.theme;
      if (existing === "dark" || existing === "light") setTheme(existing);
      else write(compute());
    } else {
      write(compute());
    }
    const onSchemeChange = () => {
      if (form.appearance === "dark" || form.appearance === "light") return;
      write(compute());
    };
    mq?.addEventListener("change", onSchemeChange);
    return () => mq?.removeEventListener("change", onSchemeChange);
  }, [form.appearance, loaded]);

  const setField = useCallback(
    <K extends keyof SettingsForm>(key: K, value: SettingsForm[K]) => {
      setForm((f) => ({ ...f, [key]: value }));
    },
    [],
  );

  const notifyError = useCallback((message: string) => {
    setError(message);
  }, []);

  // —— 热键捕获：捕获态挂 window 捕获相 keydown，下一个组合键写入对应槽位；
  //    Esc 取消；纯修饰键忽略。——
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
      setField(capturing, combo);
      setCapturing(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [capturing, setField]);

  // —— 保存：服务端 sanitize 后的设置回显（回填即权威值）——
  const save = () => {
    setError(null);
    setSaving(true);
    invoke<Settings>("set_settings", { s: toPayload(form) })
      .then((effective) => {
        setForm(toForm(effective));
        setLanguage(effective.language as LanguageSetting);
        setSaved(true);
        window.setTimeout(() => setSaved(false), 1600);
      })
      .catch((e) => setError(tx("error.save", { message: String(e) })))
      .finally(() => setSaving(false));
  };

  const activeIndex = SECTIONS.findIndex((s) => s.id === active);
  const activeSection = SECTIONS[activeIndex] ?? SECTIONS[0];

  const goPrev = () => {
    if (activeIndex > 0) setActive(SECTIONS[activeIndex - 1].id);
  };
  const goNext = () => {
    if (activeIndex < SECTIONS.length - 1) setActive(SECTIONS[activeIndex + 1].id);
  };

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (q === "") return SECTIONS;
    return SECTIONS.filter((s) => tx(s.labelKey).toLowerCase().includes(q));
  }, [query, tx]);

  return (
    <div className="hs-settings" data-theme={theme}>
      <aside className="hs-sidebar">
        <div className="hs-search">
          <span className="hs-search-icon">
            <Icon name="search" />
          </span>
          <input
            type="search"
            value={query}
            placeholder={tx("nav.searchPlaceholder")}
            aria-label={tx("nav.searchPlaceholder")}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>

        <div className="hs-profile">
          <img className="hs-profile-logo" src={appIcon} alt="" aria-hidden="true" />
          <div className="hs-profile-meta">
            <div className="hs-profile-name">{tx("app.name")}</div>
            <div className="hs-profile-version">{version ? `v${version}` : ""}</div>
          </div>
          <span className="hs-dot" title={tx("app.running")} aria-label={tx("app.running")} />
        </div>

        <nav className="hs-nav" aria-label={tx("app.name")}>
          {visible.map((s) => (
            <button
              key={s.id}
              type="button"
              className={`hs-nav-item${s.id === active ? " is-active" : ""}`}
              aria-current={s.id === active ? "page" : undefined}
              onClick={() => setActive(s.id)}
            >
              <span className="hs-nav-icon" data-sec={s.id} style={{ ["--nav-hue" as string]: s.hue }}>
                <Icon name={s.icon} />
              </span>
              <span className="hs-nav-label">{tx(s.labelKey)}</span>
            </button>
          ))}
          {visible.length === 0 ? <p className="hs-nav-empty">{tx("nav.empty")}</p> : null}
        </nav>
      </aside>

      <main className="hs-main">
        <header className="hs-head">
          <div className="hs-head-nav">
            <button
              type="button"
              className="hs-round-btn"
              disabled={activeIndex <= 0}
              onClick={goPrev}
              title={tx("action.prev")}
              aria-label={tx("action.prev")}
            >
              <Icon name="chevronLeft" />
            </button>
            <button
              type="button"
              className="hs-round-btn"
              disabled={activeIndex >= SECTIONS.length - 1}
              onClick={goNext}
              title={tx("action.next")}
              aria-label={tx("action.next")}
            >
              <Icon name="chevronRight" />
            </button>
          </div>
          <h1 className="hs-head-title">{tx(activeSection.titleKey)}</h1>
          <div className="hs-head-actions">
            {saved ? (
              <span className="hs-saved" role="status">
                {tx("action.saved")}
              </span>
            ) : null}
            <button
              type="button"
              className="hs-round-btn hs-round-btn-primary"
              onClick={save}
              disabled={saving || !loaded}
              title={saving ? tx("action.saving") : tx("action.saveTip")}
              aria-label={saving ? tx("action.saving") : tx("action.save")}
            >
              <Icon name="save" />
            </button>
          </div>
        </header>

        <div className="hs-scroll">
          {error ? (
            <div className="hs-error-bar" role="alert">
              {error}
            </div>
          ) : null}
          {activeSection.render({ form, setField, capturing, setCapturing, notifyError })}
        </div>
      </main>
    </div>
  );
}
