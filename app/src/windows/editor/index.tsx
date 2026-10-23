// 编辑器窗口（M2 Task M2 + M3）：?window=editor 路由。位图底 + 矢量层 + 撤销重做 +
// 全量工具（选择/矩形/椭圆/箭头/画笔/荧光笔/马赛克/序号/文本）+ 导出保存。
// 图片来源：Rust editor_open 建窗/复用后 emit hawk://editor-load { path, origin }（3 连发
// @150ms，覆盖层同款冷加载竞态——前端以收到的最后一条为准）；图片本体经 asset 协议
// convertFileSrc(path) 加载为 Image 元素（装载代数守卫），失败降级
// editor_image_dataurl（base64 dataURL，仅降级路径走 10MB 级 IPC）。
// 导出（F5 单路径）：exportToPng（位图 + 全形状经 drawShape 栅格化到离屏 canvas
// 自然尺寸 1:1）→ base64 → Rust editor_save 写回原路径（覆盖原图，⌘S 可触发），
// 成功闪「已保存」。origin（M3 来源契约，定义在 Rust editor.rs）：装载时随 editor-load
// 下发、保存时原样回传 editor_save 决定副作用——pin：回写贴图窗口图；capture：复制
// 剪贴板 + 存 save_dir 入库；null（历史重新编辑）：仅覆盖原文件。
import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type CSSProperties } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t, subscribe, getLanguage } from "../../lib/i18n";
import EditorCanvas, { type EditorStyle, type Tool } from "./EditorCanvas";
import { exportToPng } from "./export";
import { useEditorHistory } from "./history";
import { createLoadGuard } from "./loadGuard";
import { translate, withColor, withSize, withWidth, type Shape } from "./shapes";

// 样式记忆（计划接口）：color/width/size 持久化 localStorage，挂载时恢复。
const STYLE_KEY = "hawksnap.editor.style";

// 来源契约（M2 Task M3，Rust editor.rs EditorOrigin 的 TS 镜像，serde tag=kind camelCase）。
type EditorOrigin = { kind: "pin"; pinId: number } | { kind: "capture" };

const DEFAULT_STYLE: EditorStyle = { color: "#E5484D", width: 3, size: 24 };

// 预设色板（红/橙/黄/绿/蓝/紫/黑/白——截图标注的常用八色）
const PALETTE = ["#E5484D", "#F76B15", "#FFC53D", "#30A46C", "#0090FF", "#6E56CF", "#111111", "#FFFFFF"];

// 工具清单（G6 字面量 key 约定：文案在组件内以字面量 t() 取，本表只列 id 顺序）
const TOOLS: Tool[] = [
  "select", "rect", "ellipse", "arrow", "pen", "highlighter", "mosaic", "step", "text",
];

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

/** Blob → base64（dataURL 载体去前缀；分块无必要——FileReader 一次读入）。 */
function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const s = String(reader.result);
      const i = s.indexOf(",");
      resolve(i >= 0 ? s.slice(i + 1) : s);
    };
    reader.onerror = () => reject(new Error("PNG 数据读取失败"));
    reader.readAsDataURL(blob);
  });
}

export default function EditorWindow() {
  // i18n 语言态订阅（覆盖层/面板同款；编辑器窗口与主面板共享设置语言）
  useSyncExternalStore(subscribe, getLanguage);
  // 动作回调是稳定 useCallback——解构持有，避免 effect 随渲染重挂（hook 返回对象
  // 本身每帧新建，不可作为依赖）。
  const { present, canUndo, canRedo, commit, undo, redo, reset } = useEditorHistory([]);
  const [tool, setTool] = useState<Tool>("rect");
  const [style, setStyle] = useState<EditorStyle>(DEFAULT_STYLE);
  // 位图元素 + 来源路径 + 来源（导出保存副作用用）。element/path/origin 同生同灭
  // （apply 成对设置——M3：origin 随 editor-load 下发，保存时回传 editor_save）。
  const [image, setImage] = useState<HTMLImageElement | null>(null);
  const [imagePath, setImagePath] = useState<string | null>(null);
  const [origin, setOrigin] = useState<EditorOrigin | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // 选择态：选中形状下标（删除/移动/样式改的目标；形状数组变化越界即取消）
  const [selected, setSelected] = useState<number | null>(null);
  // 滑杆样式改的实时预览门（拖动期间选中形状按未提交样式绘制，pointerup/keyup 一次提交）
  const [restyle, setRestyle] = useState(false);
  // 保存流状态
  const [saving, setSaving] = useState(false);
  const [savedFlash, setSavedFlash] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const flashTimer = useRef<number | undefined>(undefined);
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

  // 最新 style 镜像（restyle 提交回调读取，规避闭包过期）
  const styleRef = useRef(style);
  styleRef.current = style;

  // 选中下标越界守卫（撤销/删除后 shapes 变短）：自动取消选中
  useEffect(() => {
    if (selected != null && (selected < 0 || selected >= present.length)) setSelected(null);
  }, [present, selected]);

  // 已保存闪样的定时器清理
  useEffect(() => () => window.clearTimeout(flashTimer.current), []);

  // 图片装载：asset 协议优先，onerror 降级 dataURL（editor_image_dataurl）。
  // 代数守卫（评审修复 2）：×3 补发使旧事件的 Image 探测必然迟到（A 的补发可落
  // 在 B 的事件后）——每个事件分配新代数，探测完成时仅最新代可应用 image/path/
  // reset/loadError，迟到旧探测自弃；未守卫时最终显示的是「最后完成 onload 的图」
  // 而非「最后请求的 path」。装载成功 = 新会话：撤销栈清空 + 取消选中（旧图的
  // 选中下标对新图无意义）。
  const applyImage = useCallback((img: HTMLImageElement, path: string, org: EditorOrigin | null) => {
    reset([]); // 换图生效 = 清空撤销栈（新会话）
    setSelected(null);
    setLoadError(null);
    setImage(img);
    setImagePath(path);
    setOrigin(org);
  }, [reset]);

  const loadFromPath = useCallback((path: string, org: EditorOrigin | null) => {
    const myGen = loadGuardRef.current.begin();
    const url = convertFileSrc(path);
    const probe = new Image();
    probe.onload = () => {
      if (!loadGuardRef.current.isCurrent(myGen)) return;
      applyImage(probe, path, org);
    };
    probe.onerror = () => {
      if (!loadGuardRef.current.isCurrent(myGen)) return;
      // 降级：仅当 asset 加载失败才走 base64 IPC（上下文裁定）
      invoke<string>("editor_image_dataurl", { path })
        .then((dataUrl) => {
          if (!loadGuardRef.current.isCurrent(myGen)) return;
          const img = new Image();
          img.onload = () => {
            if (loadGuardRef.current.isCurrent(myGen)) applyImage(img, path, org);
          };
          img.onerror = () => {
            if (loadGuardRef.current.isCurrent(myGen)) setLoadError("decode failed");
          };
          img.src = dataUrl;
        })
        .catch((e) => {
          if (loadGuardRef.current.isCurrent(myGen)) setLoadError(String(e));
        });
    };
    probe.src = url;
  }, [applyImage]);

  // push 通道：hawk://editor-load（3 连发 @150ms），后到的 path 覆盖先到的（单编辑器
  // 模型：编辑中再开新图 = 替换当前图 + 清空撤销栈——新图探测完成应用时 reset，
  // 迟到旧事件经代数守卫自弃，不触发 reset）。origin（M3）随载荷下发，与 path 同生
  // 同灭（applyImage 成对设置）。
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<{ path: string; origin: EditorOrigin | null }>("hawk://editor-load", (e) => {
      if (!disposed) loadFromPath(e.payload.path, e.payload.origin ?? null);
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [loadFromPath]);

  // —— 形状提交动作（全部一档历史）——

  const commitShape = useCallback((s: Shape) => commit([...present, s]), [commit, present]);

  const moveShape = useCallback((index: number, dx: number, dy: number) => {
    commit(present.map((s, i) => (i === index ? translate(s, dx, dy) : s)));
  }, [commit, present]);

  const replaceText = useCallback((index: number, text: string) => {
    commit(present.map((s, i) => (i === index && s.kind === "text" ? { ...s, text } : s)));
  }, [commit, present]);

  const deleteSelected = useCallback(() => {
    if (selected == null) return;
    commit(present.filter((_, i) => i !== selected));
    setSelected(null);
  }, [commit, present, selected]);

  // —— 样式控制（选中态 = 替换拷贝 + 一档历史；未选中 = 只改工具栏记忆）——
  // 色板是离散点击：每次点击一档。滑杆（width/size）拖动 onChange 高频：拖动期间只
  // 更新工具栏 + 经 previewStyle 实时预览，pointerup/keyup 一次性提交（一档）。

  const pickColor = useCallback((c: string) => {
    setStyle((s) => ({ ...s, color: c }));
    // 选中形状无 color 字段（mosaic）时不入历史（替换拷贝为原样 = 空档）
    if (selected != null && present[selected] && "color" in present[selected]) {
      commit(present.map((s, i) => (i === selected ? withColor(s, c) : s)));
    }
  }, [commit, present, selected]);

  const commitWidthRestyle = useCallback(() => {
    setRestyle(false);
    // 同上：width 只对描边类形状有意义，其余不入历史
    if (selected != null && present[selected] && "width" in present[selected]) {
      const w = styleRef.current.width;
      commit(present.map((s, i) => (i === selected ? withWidth(s, w) : s)));
    }
  }, [commit, present, selected]);

  const commitSizeRestyle = useCallback(() => {
    setRestyle(false);
    if (selected != null && present[selected]?.kind === "text") {
      const z = styleRef.current.size;
      commit(present.map((s, i) => (i === selected ? withSize(s, z) : s)));
    }
  }, [commit, present, selected]);

  // —— 导出保存（F5：经 exportToPng 单路径栅格化 → editor_save 写回原路径 + origin
  // 副作用（M3 契约：pin 回写贴图 / capture 剪贴板+入库 / null 无））——

  const save = useCallback(async () => {
    if (!image || !imagePath || saving) return;
    setSaving(true);
    setSaveError(null);
    try {
      const blob = await exportToPng(image, present, image.naturalWidth, image.naturalHeight);
      const pngBase64 = await blobToBase64(blob);
      await invoke("editor_save", { path: imagePath, pngBase64, origin });
      setSavedFlash(true);
      window.clearTimeout(flashTimer.current);
      flashTimer.current = window.setTimeout(() => setSavedFlash(false), 2000);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  }, [image, imagePath, present, saving, origin]);

  // 快捷键：⌘Z/⌘⇧Z（撤销重做）、Delete/Backspace（删除选中）、Esc（取消选中）、
  // ⌘S（保存）。文本输入聚焦时不接管（内联输入的字符编辑优先；其自身的
  // stopPropagation 亦兜底）。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
      const mac = /Mac|iP(hone|ad|od)/.test(navigator.platform);
      const mod = mac ? e.metaKey : e.ctrlKey;
      if (mod) {
        const key = e.key.toLowerCase();
        if (key === "z" && !e.shiftKey) {
          e.preventDefault();
          undo();
        } else if ((key === "z" && e.shiftKey) || key === "y") {
          e.preventDefault();
          redo();
        } else if (key === "s") {
          e.preventDefault();
          void save();
        }
        return;
      }
      if (e.key === "Delete" || e.key === "Backspace") {
        if (selected != null) {
          e.preventDefault();
          deleteSelected();
        }
      } else if (e.key === "Escape") {
        setSelected(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo, save, selected, deleteSelected]);

  // 工具文案（G6 字面量 key：全部 t() 调用都是字面量，检查器可扫；随语言态重渲染换新）
  const toolLabels: Record<Tool, string> = {
    select: t("editor.tool.select"),
    rect: t("editor.tool.rect"),
    ellipse: t("editor.tool.ellipse"),
    arrow: t("editor.tool.arrow"),
    pen: t("editor.tool.pen"),
    highlighter: t("editor.tool.highlighter"),
    mosaic: t("editor.tool.mosaic"),
    step: t("editor.tool.step"),
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
            onClick={() => pickColor(c)}
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
            onPointerDown={() => setRestyle(true)}
            onPointerUp={commitWidthRestyle}
            onKeyUp={commitWidthRestyle}
            onBlur={commitWidthRestyle}
            onChange={(e) => setStyle((s) => ({ ...s, width: Number(e.target.value) }))}
          />
        </label>
        {(tool === "text" || (selected != null && present[selected]?.kind === "text")) && (
          <label style={{ display: "flex", alignItems: "center", gap: 4, color: "#ccc", fontSize: 12 }}>
            {t("editor.fontSize")}
            <input
              type="range" min={12} max={64} step={2}
              value={style.size}
              onPointerDown={() => setRestyle(true)}
              onPointerUp={commitSizeRestyle}
              onKeyUp={commitSizeRestyle}
              onBlur={commitSizeRestyle}
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
        <button
          type="button"
          style={{ ...btn, ...(savedFlash ? btnSaved : {}) }}
          disabled={!image || !imagePath || saving}
          title={t("editor.saveTip")}
          onClick={() => void save()}
        >
          {savedFlash ? t("editor.saved") : t("editor.save")}
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
      {saveError && (
        <div role="alert" style={errorBanner}>
          {t("editor.saveFailed", { message: saveError })}
        </div>
      )}
      <EditorCanvas
        image={image}
        shapes={present}
        tool={tool}
        style={style}
        selected={selected}
        previewStyle={restyle && selected != null ? style : null}
        onSelect={setSelected}
        onCommit={commitShape}
        onMoveShape={moveShape}
        onReplaceText={replaceText}
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

const btnSaved: CSSProperties = {
  borderColor: "#30A46C",
  background: "#1d3a2b",
  color: "#8ff0c0",
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
