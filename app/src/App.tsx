import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  t,
  setLanguage,
  getLanguage,
  subscribe,
  type LanguageSetting,
} from "./lib/i18n";
import { formatShotDate, shotKind, canReedit, type ShotRow } from "./lib/history";
// 真应用图标（光圈 + 鹰眼，512×512）——品牌位一律用它，不用 emoji 近似。
import appIcon from "./assets/app-icon.png";
import "./App.css";

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
  language: string; // "system" | "zh-CN" | "en-US"（服务端 sanitize 兜底，D1）
  updateEnabled: boolean; // 自动更新开关（D3；服务端不 sanitize，bool 无脏值）
}

// —— 自动更新（D3）—— 与后端 updater.rs 对齐 ——
interface UpdateInfo {
  version: string;
  notes: string | null;
  currentVersion: string;
}

// 启动静默检查去重：面板会被 idle_reaper 销毁重建（webview 重挂载），每次重挂载
// 都打一次更新接口是惊扰——本模块级标记保证每次应用运行（webview 生命周期）只自动
// 检查一次；用户点「检查更新」不受此限（显式动作）。
let autoCheckFired = false;

// 热键注册失败载荷（D0-⑤）：与后端 HotkeyError（serde camelCase）对齐——kind 供横幅
// 归因，message 为人类可读文案（注册失败原因）。kind 的展示名走
// t(`hotkey.kind.${kind}`)（唯一约定的动态 key 族，单测断言双语在盘——检查器扫不到
// 模板字面量，scripts/check-i18n.mjs 对此类调用报 warn 不 fail）。
type HotkeyKind = "capture" | "pin" | "record" | "scrollshot";

// 历史类型筛选（kind 值与 Rust KIND_FILTERS 白名单一致）。
type KindFilter = "all" | "region" | "recording" | "scrollshot";

// pin_list 载荷（Rust PinInfo serde camelCase）。
interface PinInfo { id: number; wPx: number; hPx: number }
interface HotkeyErrorPayload {
  kind: HotkeyKind;
  message: string;
}


function App() {
  // 订阅 i18n 语言态：语言切换触发本组件重渲染，全部文案即时换新（不重载）。
  useSyncExternalStore(subscribe, getLanguage);
  const [hotkeyError, setHotkeyError] = useState<HotkeyErrorPayload | null>(null);
  const [captureError, setCaptureError] = useState<string | null>(null);
  const [recordError, setRecordError] = useState<string | null>(null);
  // —— 设置已迁到独立窗口（M3 重做，Snapzy 式侧栏+卡片）——主面板只保留三件事：
  // ① 热键文案回显（入口按钮上的「截图 (F1)」）② 外观主题跟随 ③ 更新可用性横幅。
  const [hotkeys, setHotkeys] = useState({ capture: "F1", pin: "F2", record: "F6" });
  const [appearance, setAppearance] = useState("system");
  // 发现新版本 → 横幅提示「去设置里更新」（安装流程在设置窗口，主面板不重复实现）。
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  // —— 浏览器插件分区（M4 P1）—— Native Messaging 桥是否就绪（后端监听状态）+
  // 浏览器截图入库后的提示横幅。
  const [nativeReady, setNativeReady] = useState<boolean | null>(null);
  const [browserCapture, setBrowserCapture] = useState<string | null>(null);
  // —— 历史分区（M0）—— 懒加载：挂载即 history_list(50)（面板打开时填充）；输入
  // 经 250ms 防抖切换到 history_search（计划接口）。确认删除是瓦片内两步按钮
  // （[删除]→[确认删除]，避免原生 dialog 管线）。
  const [shots, setShots] = useState<ShotRow[]>([]);
  const [historyQuery, setHistoryQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<number | null>(null);
  // —— 类型筛选（v1.2，§3.6「按类型」）：chips 只作用于列表/搜索的 kind 条件；
  // "all" 不携带 kind 字段（载荷与旧版一致，兼容测试断言）。
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  // —— 贴图管理（v1.2 最小版，§3.2）：贴图清单 + 单张关闭。清单 10s 轮询 + 贴图
  // 动作后即时刷新（贴图也可经 F2/托盘增减——轮询兜住非本面板路径）。
  const [pins, setPins] = useState<PinInfo[]>([]);

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

  // 录屏失败横幅（M6）：F6 toggle / 悬浮条操作的录屏错误经 hawk://record-error 回流
  // （panel::show_record_error 懒拉主面板后 emit——与 capture-error 同款不静默收尾）。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<string>("hawk://record-error", (e) => setRecordError(e.payload))
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

  // 浏览器插件分区（M4 P1）：Native Messaging 桥状态 + 浏览器截图回流横幅。
  // 状态轮询（5s）——监听是后台线程启停的，命令读的是进程内标志位。
  useEffect(() => {
    let disposed = false;
    const poll = () => {
      invoke<boolean>("native_messaging_status")
        .then((ok) => {
          if (!disposed) setNativeReady(ok);
        })
        .catch(() => {
          if (!disposed) setNativeReady(false);
        });
    };
    poll();
    const timer = window.setInterval(poll, 5000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, []);

  // 浏览器截图入库（后端落盘 + 入库后推事件）→ 横幅告知用户「收到了」。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<{ path: string; width: number; height: number }>(
      "hawk://browser-capture",
      (e) => {
        if (disposed) return;
        setBrowserCapture(e.payload.path);
      },
    )
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

  // 挂载即拉取设置（面板由 idle_reaper 销毁后重建时会重挂载 → 总能拿到最新值）。
  // 主面板只消费三样：语言（对齐本窗口与覆盖层窗口）、外观（主题）、热键（入口回显）。
  // 设置的编辑面在独立设置窗口——此处只读，不持有表单态。
  useEffect(() => {
    invoke<Settings>("get_settings")
      .then((s) => {
        setLanguage(s.language as LanguageSetting);
        setAppearance(s.appearance);
        setHotkeys({
          capture: s.hotkeyCapture,
          pin: s.hotkeyPin,
          record: s.hotkeyRecord,
        });
      })
      .catch(() => {
        /* 设置读失败不打扰主面板：入口按钮回显默认热键即可 */
      });
  }, []);

  // —— 外观主题接线（表现层，本次 UI 重设计新增的唯一逻辑）——
  // appearance（system|light|dark）→ documentElement.dataset.theme（light/dark）；
  // "跟随系统"经 prefers-color-scheme 解析并订阅其变化实时切换。App.css 全部颜色走
  // CSS 变量，[data-theme="dark"] 整组覆盖。面板 body 打标 hs-panel-body——本样式表
  // 经 main.tsx 打进所有窗口的产物，body 级规则借此只作用主面板。
  useEffect(() => {
    const mq =
      typeof window.matchMedia === "function"
        ? window.matchMedia("(prefers-color-scheme: dark)")
        : null;
    const apply = () => {
      const dark =
        appearance === "dark" || (appearance === "system" && !!mq?.matches);
      document.documentElement.dataset.theme = dark ? "dark" : "light";
    };
    apply();
    document.body.classList.add("hs-panel-body");
    mq?.addEventListener("change", apply);
    return () => mq?.removeEventListener("change", apply);
  }, [appearance]);

  // —— 贴图清单轮询（v1.2）：10s 兜底轮询（F2/托盘路径不在本面板内），贴图动作
  // 后另有即时刷新（refreshPins）。非 macOS 后端恒空 → 卡片显示空态（能力缺席
  // 显式声明，不假报状态）。
  const refreshPins = useCallback(() => {
    invoke<PinInfo[]>("pin_list")
      .then((p) => setPins(Array.isArray(p) ? p : []))
      .catch(() => setPins([]));
  }, []);

  useEffect(() => {
    refreshPins();
    const timer = window.setInterval(refreshPins, 10000);
    return () => window.clearInterval(timer);
  }, [refreshPins]);

  // —— 历史分区（M0）—— 搜索输入 250ms 防抖（计划接口）；空查询走 history_list，
  // 非空走 history_search。加载失败入 historyError（分区内横幅，不打扰设置区）。
  // v1.2：kind 筛选随载荷下发（"all" 不携带，保持旧载荷形态）。
  const refreshShots = useCallback((query: string, kind: KindFilter) => {
    const kindArg = kind === "all" ? {} : { kind };
    const req =
      query.trim() === ""
        ? invoke<ShotRow[]>("history_list", { limit: 50, offset: 0, ...kindArg })
        : invoke<ShotRow[]>("history_search", { query, limit: 50, ...kindArg });
    req
      .then((rows) => {
        setShots(rows);
        setHistoryError(null);
      })
      .catch((e) => setHistoryError(String(e)));
  }, []);

  useEffect(() => {
    const h = window.setTimeout(() => setDebouncedQuery(historyQuery), 250);
    return () => window.clearTimeout(h);
  }, [historyQuery]);

  useEffect(() => {
    refreshShots(debouncedQuery, kindFilter);
  }, [debouncedQuery, kindFilter, refreshShots]);

  const pinShot = (path: string) => {
    invoke<number>("pin_from_path", { path })
      .then(() => refreshPins())
      .catch((e) => setHistoryError(String(e)));
  };
  const openShot = (path: string) => {
    invoke<void>("open_path", { path }).catch((e) => setHistoryError(String(e)));
  };
  const removeShot = (id: number) => {
    invoke<void>("history_delete", { id })
      .then(() => {
        setConfirmDeleteId(null);
        refreshShots(debouncedQuery, kindFilter);
      })
      .catch((e) => setHistoryError(String(e)));
  };
  // M2 Task M3：重新编辑 → editor_open（origin=null：保存只覆盖原文件，无剪贴板/
  // 回写/重复入库副作用——契约在 Rust editor.rs「来源契约」）。
  const reeditShot = (path: string) => {
    invoke<void>("editor_open", { path, origin: null }).catch((e) =>
      setHistoryError(String(e)),
    );
  };

  // 启动静默检查（评审 Fix 1：让 update_enabled 真正门控自动检查）。manual=false 时
  // 门控全在命令侧：开关关闭 → Ok(None)；占位公钥 → 同样 Ok(None)——两者都渲染为无。
  // 只有真的发现新版本才出横幅（安装/重启流程在设置窗口，主面板只做「去设置」引导）。
  useEffect(() => {
    if (autoCheckFired) return;
    autoCheckFired = true;
    invoke<UpdateInfo | null>("check_for_updates", { manual: false })
      .then((info) => {
        if (info) setUpdateInfo(info);
      })
      .catch(() => {});
  }, []);

  // 面板入口（C6）：与 F1/F2 热键、托盘同链路（panel_trigger → start_capture_flow /
  // pin_latest），失败经 hawk://capture-error 横幅回流。热键名随设置回显（C7）。
  const trigger = (action: "capture" | "pin") => {
    invoke<void>("panel_trigger", { action }).catch((e) => setCaptureError(String(e)));
  };

  // 打开设置窗口（M3 重做）：主面板只做入口，编辑面在独立窗口（侧栏分区 + 卡片）。
  const openSettings = () => {
    invoke<void>("open_settings").catch((e) => setCaptureError(String(e)));
  };

  return (
    <main className="hs-root">
      <header className="hs-topbar">
        <h1 className="hs-brand">
          <img className="hs-brand-icon" src={appIcon} alt="" aria-hidden="true" />
          {t("brand.name")}
        </h1>
        <div className="hs-topbar-actions">
          <button className="hs-btn hs-btn-primary" onClick={() => trigger("capture")}>
            {t("panel.capture", { key: hotkeys.capture })}
          </button>
          <button className="hs-btn hs-btn-primary" onClick={() => trigger("pin")}>
            {t("panel.pin", { key: hotkeys.pin })}
          </button>
          {/* 设置入口：独立窗口（M3 重做，Snapzy 式侧栏+卡片）。齿轮按钮而非文字——
              主面板是「历史/快速访问」面，设置是低频配置面，两者分工。 */}
          <button
            type="button"
            className="hs-btn hs-btn-icon"
            onClick={openSettings}
            title={t("panel.openSettings")}
            aria-label={t("panel.openSettings")}
          >
            <span aria-hidden="true">⚙</span>
          </button>
        </div>
      </header>

      {/* 通知条区域：五路错误（热键/截图/录屏/设置/更新）红色系，浏览器截图入库
          为状态告知绿色系；允许多条堆叠，视觉安静（浅色底 + 细描边）。 */}
      <div className="hs-banners">
        {hotkeyError && (
          <div role="alert" className="hs-banner hs-banner-error">
            {t("banner.hotkeyError", {
              kind: t(`hotkey.kind.${hotkeyError.kind}`),
              message: hotkeyError.message,
            })}
          </div>
        )}
        {captureError && (
          <div role="alert" className="hs-banner hs-banner-error">
            {t("banner.captureError", { message: captureError })}
          </div>
        )}
        {recordError && (
          <div role="alert" className="hs-banner hs-banner-error">
            {t("banner.recordError", { message: recordError })}
          </div>
        )}
        {/* 发现新版本 → 引导去设置窗口安装（安装/重启流程在设置窗口统一实现，
            主面板不重复一套，避免两处状态机打架）。 */}
        {updateInfo && (
          <div role="status" className="hs-banner hs-banner-info">
            <span>{t("banner.updateAvailable", { version: updateInfo.version })}</span>
            <button
              type="button"
              className="hs-btn hs-btn-secondary hs-btn-sm"
              onClick={openSettings}
            >
              {t("banner.updateGoSettings")}
            </button>
          </div>
        )}
        {browserCapture && (
          <div role="status" className="hs-banner hs-banner-info">
            {t("banner.browserCapture", { path: browserCapture })}
          </div>
        )}
      </div>

      <div className="hs-content">
        {/* —— 历史分区（M0）：替代 C6 的禁用「历史」入口行 —— */}
        <section className="hs-card">
          <h2 className="hs-section-title">{t("history.title")}</h2>
          <input
            type="search"
            className="hs-search"
            placeholder={t("history.searchPlaceholder")}
            aria-label={t("history.searchPlaceholder")}
            value={historyQuery}
            onChange={(e) => setHistoryQuery(e.target.value)}
          />
          {/* 类型筛选 chips（v1.2，§3.6「按类型」）：作用于列表与搜索的 kind 条件；
              键面全部字面量 t()（G6 约定：动态 key 仅 hotkey.kind.* 豁免）。 */}
          <div className="hs-chip-row" role="group" aria-label={t("history.filter")}>
            {([
              { id: "all", label: t("history.kind.all") },
              { id: "region", label: t("history.kind.region") },
              { id: "recording", label: t("history.kind.recording") },
              { id: "scrollshot", label: t("history.kind.scrollshot") },
            ] as const).map(({ id, label }) => (
              <button
                key={id}
                type="button"
                aria-pressed={kindFilter === id}
                className={`hs-chip-btn${kindFilter === id ? " is-active" : ""}`}
                onClick={() => setKindFilter(id)}
              >
                {label}
              </button>
            ))}
          </div>
          {historyError && (
            <div role="alert" className="hs-banner hs-banner-error hs-banner-sm">
              {t("history.actionError", { message: historyError })}
            </div>
          )}
          {shots.length === 0 ? (
            <div className="hs-empty">
              <span className="hs-empty-icon" aria-hidden="true">🖼️</span>
              <p className="hs-empty-text">
                {debouncedQuery.trim() === "" ? t("history.empty") : t("history.emptySearch")}
              </p>
            </div>
          ) : (
            <div className="hs-grid">
              {shots.map((shot) => {
                const title = `${formatShotDate(shot.createdAt, getLanguage())} · ${shot.app}`;
                return (
                  <div key={shot.id} className="hs-tile" title={title}>
                    {shot.thumbPath ? (
                      <img
                        src={convertFileSrc(shot.thumbPath)}
                        alt={title}
                        loading="lazy"
                        className="hs-thumb"
                      />
                    ) : shotKind(shot) === "recording" ? (
                      // 录屏条目（M6）：无缩略图 → 🎬 占位（贴图/重新编辑对 MP4 无意义，
                      // 下方动作区隐藏；打开目录/删除照常）。按 kind 判定（不再靠 app
                      // 列复用——kind 已是契约字段）。
                      <div className="hs-thumb hs-thumb-record" aria-hidden="true">🎬</div>
                    ) : (
                      <div className="hs-thumb hs-thumb-blank" aria-hidden="true" />
                    )}
                    <div className="hs-tile-meta">{title}</div>
                    <div className="hs-tile-actions">
                      <button className="hs-tile-btn" onClick={() => pinShot(shot.path)}>
                        {t("history.pin")}
                      </button>
                      <button className="hs-tile-btn" onClick={() => openShot(shot.path)}>
                        {t("history.openFolder")}
                      </button>
                      {confirmDeleteId === shot.id ? (
                        // 两步删除第二步：确认态红底白字（逻辑同旧实现，仅换皮）
                        <button
                          className="hs-tile-btn hs-tile-btn-danger"
                          onClick={() => removeShot(shot.id)}
                        >
                          {t("history.confirmDelete")}
                        </button>
                      ) : (
                        <button
                          className="hs-tile-btn"
                          onClick={() => setConfirmDeleteId(shot.id)}
                        >
                          {t("history.delete")}
                        </button>
                      )}
                      {/* M2 Task M3：重新编辑启用——editor_open（origin=null）；录屏条目
                          不提供（MP4 非编辑器输入，M6 裁定）；**长截图条目同样不提供**
                          （终审 wave I7：长图可达 32MP，编辑器画布/导出路径按巨幅位图
                          走会出内存尖峰与导出超时）。按 kind 门控。 */}
                      {canReedit(shot) && (
                        <button className="hs-tile-btn" onClick={() => reeditShot(shot.path)}>
                          {t("history.reedit")}
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </section>

        {/* —— 贴图管理（v1.2 最小版，§3.2「贴图管理器」）：清单 + 单张关闭 + 空态
            引导（按贴图热键把最近截图钉上桌面）。非 macOS 恒空态——贴图窗口是
            macOS 原生 NSWindow，能力缺席显式声明。 —— */}
        <section className="hs-card">
          <h2 className="hs-section-title">{t("pins.title")}</h2>
          {pins.length === 0 ? (
            <p className="hs-empty-text">{t("pins.empty", { key: hotkeys.pin })}</p>
          ) : (
            <ul className="hs-pin-list">
              {pins.map((p) => (
                <li key={p.id} className="hs-pin-row">
                  <span className="hs-pin-meta">
                    {t("pins.item", { id: p.id, w: p.wPx, h: p.hPx })}
                  </span>
                  <button
                    type="button"
                    className="hs-tile-btn"
                    onClick={() =>
                      invoke<void>("pin_close", { id: p.id })
                        .then(() => refreshPins())
                        .catch((e) => setHistoryError(String(e)))
                    }
                  >
                    {t("pins.close")}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        {/* —— 浏览器插件分区（M4 P1：Native Messaging 桥）——
            只展示状态与扩展 ID 占位（注册动作在宿主侧：scripts/install-native-host.sh）。
            状态来自 native_messaging_status（后端监听是否在跑）。 */}
        <section className="hs-card">
          <h2 className="hs-section-title">{t("plugin.title")}</h2>
          <div className="hs-field">
            <span className="hs-field-label">{t("plugin.statusLabel")}</span>
            <div className="hs-field-control">
              <span className="hs-status-line">
                <span
                  aria-hidden="true"
                  className={
                    nativeReady === null
                      ? "hs-dot hs-dot-idle"
                      : nativeReady
                        ? "hs-dot hs-dot-ready"
                        : "hs-dot hs-dot-stopped"
                  }
                />
                {nativeReady === null
                  ? t("plugin.status.unknown")
                  : nativeReady
                    ? t("plugin.status.ready")
                    : t("plugin.status.stopped")}
              </span>
            </div>
          </div>
          <div className="hs-field">
            <span className="hs-field-label">{t("plugin.extIdLabel")}</span>
            <div className="hs-field-control">
              <code className="hs-code">{t("plugin.extIdPlaceholder")}</code>
            </div>
          </div>
          <p className="hs-field-hint">{t("plugin.installHint")}</p>
        </section>
      </div>
    </main>
  );
}

export default App;
