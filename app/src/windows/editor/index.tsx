// 编辑器窗口（M2 Task M1）：?window=editor 路由。位图底 + 矢量层 + 撤销重做骨架。
// 图片来源：Rust editor_open 建窗/复用后 emit hawk://editor-load { path }（3 连发
// @150ms，覆盖层同款冷加载竞态——前端以收到的最后一条为准）；图片本体经 asset 协议
// convertFileSrc(path) 加载，失败时降级 editor_image_dataurl（base64 dataURL，仅
// 降级路径走 10MB 级 IPC）。保存按钮为 M2 占位（导出接线归 M2）；「标注」入口 /
// 历史重新编辑按钮的启用归 M3（Ruling 1）。
import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type CSSProperties } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t, subscribe, getLanguage } from "../../lib/i18n";
import EditorCanvas, { type EditorStyle, type Tool } from "./EditorCanvas";
import { useEditorHistory } from "./history";
import { createLoadGuard } from "./loadGuard";
import type { Shape } from "./shapes";

// 样式记忆（计划接口）：color/width/size 持久化 localStorage，挂载时恢复。
const STYLE_KEY = "hawksnap.editor.style";

const DEFAULT_STYLE: EditorStyle = { color: "#E5484D", width: 3, size: 24 };

// 预设色板（红/橙/黄/绿/蓝/紫/黑/白——截图标注的常用八色）
const PALETTE = ["#E5484D", "#F76B15", "#FFC53D", "#30A46C", "#0090FF", "#6E56CF", "#111111", "#FFFFFF"];

// 工具清单（G6 字面量 key 约定：文案在组件内以字面量 t() 取，本表只列 id 顺序）
const TOOLS: Tool[] = ["rect", "ellipse", "arrow", "pen", "highlighter", "text"];

function loadStyle(): EditorStyle {
  try {
    const raw = localStorage.getItem(STYLE_KEY);
    if (!raw) return DEFAULT_STYLE;
    const s = JSON.parse(raw) as Partial<EditorStyle>;
    return {
      color: typeof s.color === "string" ? s.color : DEFAULT_STYLE.color,
      width: typeof s.width === "number" ? s.width : DEFAULT_STYLE.width,
      size: typeof s.size === "number" ? s.size : DEFAULT_STYLE.size,
    };
  } catch {
    return DEFAULT_STYLE; // 脏值不炸窗口
  }
}

export default function EditorWindow() {
  // i18n 语言态订阅（覆盖层/面板同款；编辑器窗口与主面板共享设置语言）
  useSyncExternalStore(subscribe, getLanguage);
  // 动作回调是稳定 useCallback——解构持有，避免 effect 随渲染重挂（hook 返回对象
  // 本身每帧新建，不可作为依赖）。
  const { present, canUndo, canRedo, commit, undo, redo, reset } = useEditorHistory([]);
  const [tool, setTool] = useState<Tool>("rect");
  const [style, setStyle] = useState<EditorStyle>(DEFAULT_STYLE);
  const [imageSrc, setImageSrc] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // 装载代数守卫（评审修复 2）：useRef 惰性持有（每渲染新建的备用实例被丢弃）
  const loadGuardRef = useRef(createLoadGuard());

  // 挂载恢复样式记忆
  useEffect(() => {
    setStyle(loadStyle());
  }, []);

  // 样式变化即持久化（挂载首帧也会写一次默认值——幂等无害）
  useEffect(() => {
    try {
      localStorage.setItem(STYLE_KEY, JSON.stringify(style));
    } catch {
      // 存储满/禁用：样式记忆静默失效，编辑器其余功能不受影响
    }
  }, [style]);

  // 图片装载：asset 协议优先，onerror 降级 dataURL（editor_image_dataurl）。
  // 代数守卫（评审修复 2）：×3 补发使旧事件的 Image 探测必然迟到（A 的补发可落
  // 在 B 的事件后）——每个事件分配新代数，探测完成时仅最新代可应用 imageSrc/reset/
  // loadError，迟到旧探测自弃；未守卫时最终显示的是「最后完成 onload 的图」而非
  // 「最后请求的 path」（开 A 后快速开 B 可能显示 A 而撤销栈已按 B 重置）。
  const loadFromPath = useCallback((path: string) => {
    const myGen = loadGuardRef.current.begin();
    const url = convertFileSrc(path);
    const probe = new Image();
    const apply = (src: string) => {
      reset([]); // 换图生效 = 清空撤销栈（新会话；迟到旧事件不会误清新会话的绘制）
      setLoadError(null);
      setImageSrc(src);
    };
    probe.onload = () => {
      if (!loadGuardRef.current.isCurrent(myGen)) return;
      apply(url);
    };
    probe.onerror = () => {
      if (!loadGuardRef.current.isCurrent(myGen)) return;
      // 降级：仅当 asset 加载失败才走 base64 IPC（上下文裁定）
      invoke<string>("editor_image_dataurl", { path })
        .then((dataUrl) => {
          if (loadGuardRef.current.isCurrent(myGen)) apply(dataUrl);
        })
        .catch((e) => {
          if (loadGuardRef.current.isCurrent(myGen)) setLoadError(String(e));
        });
    };
    probe.src = url;
  }, [reset]);

  // push 通道：hawk://editor-load（3 连发 @150ms），后到的 path 覆盖先到的（单编辑器
  // 模型：编辑中再开新图 = 替换当前图 + 清空撤销栈——新图探测完成应用时 reset，
  // 迟到旧事件经代数守卫自弃，不触发 reset）。
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<{ path: string }>("hawk://editor-load", (e) => {
      if (!disposed) loadFromPath(e.payload.path);
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [loadFromPath]);

  // ⌘Z / ⌘⇧Z（mac metaKey，其余 ctrlKey——navigator.platform 判定）。文本输入聚焦时
  // 不接管（内联输入的字符编辑优先）。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
      const mac = /Mac|iP(hone|ad|od)/.test(navigator.platform);
      const mod = mac ? e.metaKey : e.ctrlKey;
      if (!mod) return;
      const key = e.key.toLowerCase();
      if (key === "z" && !e.shiftKey) {
        e.preventDefault();
        undo();
      } else if ((key === "z" && e.shiftKey) || key === "y") {
        e.preventDefault();
        redo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo]);

  const commitShape = useCallback((s: Shape) => commit([...present, s]), [commit, present]);

  // 工具文案（G6 字面量 key：全部 t() 调用都是字面量，检查器可扫；随语言态重渲染换新）
  const toolLabels: Record<Tool, string> = {
    rect: t("editor.tool.rect"),
    ellipse: t("editor.tool.ellipse"),
    arrow: t("editor.tool.arrow"),
    pen: t("editor.tool.pen"),
    highlighter: t("editor.tool.highlighter"),
    text: t("editor.tool.text"),
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "#1f1f1f" }}>
      {/* 工具栏（图标用文字标签起步——计划口径；全部文案 t() 双语） */}
      <div style={toolbar} role="toolbar" aria-label={t("editor.toolbar")}>
        {TOOLS.map((id) => (
          <button
            key={id}
            type="button"
            style={{ ...btn, ...(tool === id ? btnActive : {}) }}
            aria-pressed={tool === id}
            title={toolLabels[id]}
            onClick={() => setTool(id)}
          >
            {toolLabels[id]}
          </button>
        ))}
        <span style={sep} />
        {PALETTE.map((c) => (
          <button
            key={c}
            type="button"
            aria-label={`${t("editor.color")} ${c}`}
            title={`${t("editor.color")} ${c}`}
            onClick={() => setStyle((s) => ({ ...s, color: c }))}
            style={{
              ...swatch,
              background: c,
              outline: style.color === c ? "2px solid #4aa3ff" : "none",
            }}
          />
        ))}
        <span style={sep} />
        <label style={{ display: "flex", alignItems: "center", gap: 4, color: "#ccc", fontSize: 12 }}>
          {t("editor.strokeWidth")}
          <input
            type="range" min={1} max={16} step={1}
            value={style.width}
            onChange={(e) => setStyle((s) => ({ ...s, width: Number(e.target.value) }))}
          />
        </label>
        {tool === "text" && (
          <label style={{ display: "flex", alignItems: "center", gap: 4, color: "#ccc", fontSize: 12 }}>
            {t("editor.fontSize")}
            <input
              type="range" min={12} max={64} step={2}
              value={style.size}
              onChange={(e) => setStyle((s) => ({ ...s, size: Number(e.target.value) }))}
            />
          </label>
        )}
        <span style={sep} />
        <button type="button" style={btn} disabled={!canUndo} title={t("editor.undo")} onClick={undo}>
          {t("editor.undo")}
        </button>
        <button type="button" style={btn} disabled={!canRedo} title={t("editor.redo")} onClick={redo}>
          {t("editor.redo")}
        </button>
        <span style={{ flex: 1 }} />
        {/* 保存：M1 占位（导出接线归 M2，计划 M2 接口） */}
        <button type="button" style={btn} disabled title={t("editor.saveTip")}>
          {t("editor.save")}
        </button>
        <button type="button" style={btn} title={t("editor.close")} onClick={() => void getCurrentWindow().close()}>
          {t("editor.close")}
        </button>
      </div>
      {loadError && (
        <div role="alert" style={errorBanner}>
          {t("editor.loadError", { message: loadError })}
        </div>
      )}
      <EditorCanvas
        imageUrl={imageSrc}
        shapes={present}
        tool={tool}
        style={style}
        onCommit={commitShape}
      />
    </div>
  );
}

// —— 样式（窗口内联，独立于主面板 App.css） ——

const toolbar: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "6px 10px",
  background: "#111318",
  borderBottom: "1px solid #333",
  flexWrap: "wrap",
};

const btn: CSSProperties = {
  padding: "4px 10px",
  fontSize: 13,
  borderRadius: 4,
  border: "1px solid #444",
  background: "#1c1f26",
  color: "#ddd",
  cursor: "pointer",
};

const btnActive: CSSProperties = {
  borderColor: "#4aa3ff",
  background: "#223047",
  color: "#fff",
};

const swatch: CSSProperties = {
  width: 20,
  height: 20,
  borderRadius: 4,
  border: "1px solid #555",
  cursor: "pointer",
  padding: 0,
};

const sep: CSSProperties = { width: 1, height: 20, background: "#333", margin: "0 2px" };

const errorBanner: CSSProperties = {
  margin: 8,
  padding: "8px 12px",
  border: "1px solid #d9534f",
  borderRadius: 6,
  background: "#fdecea",
  color: "#a94442",
  fontSize: 13,
};
