// 编辑器窗口（M2 Task M2 + M3）：?window=editor 路由。位图底 + 矢量层 + 撤销重做 +
// 全量工具（选择/裁剪/矩形/椭圆/箭头/画笔/荧光笔/马赛克/序号/文本，M3 N0 起 10/10）+
// 导出保存。
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
import { formatError } from "../../lib/errors";
import EditorCanvas, { ZOOM_MAX, ZOOM_MIN, type EditorStyle, type Tool } from "./EditorCanvas";
import { exportToPng, ExportError } from "./export";
import { useEditorHistory } from "./history";
import { createLoadGuard } from "./loadGuard";
import { applyCrop, translate, withColor, withSize, withWidth, type BBox, type MosaicMode, type Shape } from "./shapes";

// 样式记忆（计划接口）：color/width/size 持久化 localStorage，挂载时恢复。
const STYLE_KEY = "hawksnap.editor.style";

// 来源契约（M2 Task M3，Rust editor.rs EditorOrigin 的 TS 镜像，serde tag=kind camelCase）。
type EditorOrigin = { kind: "pin"; pinId: number } | { kind: "capture" };

// OCR（M4）：Rust OcrLine 的 TS 镜像（box: [{x,y}×4] 原图坐标 TL→TR→BR→BL）。
type OcrPoint = { x: number; y: number };
type OcrLine = { box: OcrPoint[]; text: string; score: number };
// hawk://ocr-model-progress 载荷（Rust ocr.rs OcrModelProgress，camelCase）。
type OcrProgress = { phase: "download" | "verify"; downloaded: number; total: number };

const DEFAULT_STYLE: EditorStyle = { color: "#E5484D", width: 3, size: 24 };

// 预设色板（红/橙/黄/绿/蓝/紫/黑/白——截图标注的常用八色）
const PALETTE = ["#E5484D", "#F76B15", "#FFC53D", "#30A46C", "#0090FF", "#6E56CF", "#111111", "#FFFFFF"];

// 工具清单（G6 字面量 key 约定：文案在组件内以字面量 t() 取，本表只列 id 顺序）
// M3 N0：+crop（第 10 工具，编辑器 10/10 达成）。
const TOOLS: Tool[] = [
  "select", "crop", "rect", "ellipse", "arrow", "pen", "highlighter", "mosaic", "step", "text",
];

// 单键工具切换（v1.2）：与主流标注工具一致的字母位；文本输入聚焦时不接管。
const TOOL_KEYS: Record<string, Tool> = {
  v: "select", c: "crop", r: "rect", e: "ellipse", a: "arrow",
  p: "pen", h: "highlighter", m: "mosaic", n: "step", t: "text",
};

function loadStyle(): EditorStyle {
  try {
    const raw = localStorage.getItem(STYLE_KEY);
    if (!raw) return DEFAULT_STYLE;
    const s = JSON.parse(raw) as Partial<EditorStyle>;
    return {
      color: typeof s.color === "string" ? s.color : DEFAULT_STYLE.color,
      width: typeof s.width === "number" ? s.width : DEFAULT_STYLE.width,
      size: typeof s.size === "number" ? s.size : DEFAULT_STYLE.size,
      // 遮蔽模式（v1.2）：脏值不炸窗口，非法值回退默认像素化
      mosaicMode: s.mosaicMode === "blur" ? "blur"
        : s.mosaicMode === "pixelate" ? "pixelate"
        : undefined,
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
    // v1.2 i18n（G6）：此前硬编码中文漏出到 saveError/ocrError 的 {message} 槽，
    // en-US 用户看到外语环境里的中文串——错误文案统一走词典。
    reader.onerror = () => reject(new Error(t("editor.pngReadFailed")));
    reader.readAsDataURL(blob);
  });
}

export default function EditorWindow() {
  // i18n 语言态订阅（覆盖层/面板同款；编辑器窗口与主面板共享设置语言）
  useSyncExternalStore(subscribe, getLanguage);
  // 动作回调是稳定 useCallback——解构持有，避免 effect 随渲染重挂（hook 返回对象
  // 本身每帧新建，不可作为依赖）。
  const { present, canUndo, canRedo, commit, undo, redo, reset, restore, past, future } = useEditorHistory([]);
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
  // OCR 流状态（M4）：运行中 / 模型下载进度 / 错误 / 结果浮层（null = 收起）。
  const [ocrRunning, setOcrRunning] = useState(false);
  const [ocrProgress, setOcrProgress] = useState<OcrProgress | null>(null);
  const [ocrError, setOcrError] = useState<string | null>(null);
  const [ocrLines, setOcrLines] = useState<OcrLine[] | null>(null);
  const [ocrFlash, setOcrFlash] = useState<"copied" | "exported" | null>(null);
  const ocrFlashTimer = useRef<number | undefined>(undefined);
  // 装载代数守卫（评审修复 2）：useRef 惰性持有（每渲染新建的备用实例被丢弃）
  const loadGuardRef = useRef(createLoadGuard());
  // —— 画布缩放（v1.2）：null = 适应窗口（默认）；Ctrl/⌘+滚轮与工具栏按钮调节 ——
  const [zoom, setZoom] = useState<number | null>(null);
  // —— OCR 结果高亮（v1.2 文本框浮层联动，§3.3）：行 hover 时该行的外接框 ——
  const [ocrHighlight, setOcrHighlight] = useState<BBox | null>(null);
  // OCR 导出落点（v1.2）：flash 显示具体路径（此前只闪「已导出」，落点用户不可见）
  const [ocrExportPath, setOcrExportPath] = useState<string | null>(null);
  // —— 自动 OCR（v1.2 动作浮条「OCR」）：装载载荷 runOcr=true → 位图就绪自动识别 ——
  const [autoOcr, setAutoOcr] = useState(false);
  // 会话内只自动跑一次（补发事件不会反复触发；applyImage 换图时复位）
  const autoOcrFiredRef = useRef(false);
  // —— Capture 来源首次保存的落盘副本（v1.2 保存去重）：首次 ⌘S 得到 save_dir
  // 副本路径后，后续保存切到该路径 + origin=null——重复 ⌘S 只覆盖同一文件，
  // 不再每次新建时间戳副本、重复入库（审计 P2 修复）——
  const [savedCapturePath, setSavedCapturePath] = useState<string | null>(null);
  // —— 裁剪撤销（v1.2，§3.3「撤销无步数限制」的裁剪补全）：裁剪前的完整会话快照
  // （位图 + 形状 + 双栈）。形状撤销栈耗尽后 ⌘Z 逐级恢复裁剪（多层裁剪多层回退）；
  // 裁剪本身不进重做（单向恢复）。count 是触发重渲染的镜像（undo 按钮可用态含它）。
  const cropUndoRef = useRef<Array<{
    img: HTMLImageElement; shapes: Shape[]; past: Shape[][]; future: Shape[][];
  }>>([]);
  const [cropUndoCount, setCropUndoCount] = useState(0);

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

  // OCR 进度通道（push）：运行期收 hawk://ocr-model-progress；监听常驻（挂载期注册
  // 一次），进度载荷仅 OCR 运行期间有意义。
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<OcrProgress>("hawk://ocr-model-progress", (e) => {
      if (!disposed) setOcrProgress(e.payload);
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // OCR 闪样（已复制/已导出）定时器清理
  useEffect(() => () => window.clearTimeout(ocrFlashTimer.current), []);

  // 剪贴板：navigator.clipboard 优先（用户手势内调用），失败降级 execCommand。
  const copyText = useCallback(async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }
  }, []);

  // 打开 OCR 模型目录（§9 手动下载兜底，v1.2）：目录路径来自 ocr_models_dir，
  // 打开复用主面板同款 open_path（访达/资源管理器）。
  const openModelsDir = useCallback(async () => {
    try {
      const dir = await invoke<string>("ocr_models_dir");
      await invoke("open_path", { path: dir });
    } catch (e) {
      setOcrError(String(e));
    }
  }, []);

  // 图片装载：asset 协议优先，onerror 降级 dataURL（editor_image_dataurl）。
  // 代数守卫（评审修复 2）：×3 补发使旧事件的 Image 探测必然迟到（A 的补发可落
  // 在 B 的事件后）——每个事件分配新代数，探测完成时仅最新代可应用 image/path/
  // reset/loadError，迟到旧探测自弃；未守卫时最终显示的是「最后完成 onload 的图」
  // 而非「最后请求的 path」。装载成功 = 新会话：撤销栈清空 + 取消选中（旧图的
  // 选中下标对新图无意义）+ 裁剪撤销栈/保存副本/自动 OCR/OCR 高亮全部复位。
  const applyImage = useCallback((img: HTMLImageElement, path: string, org: EditorOrigin | null, runOcr: boolean) => {
    reset([]); // 换图生效 = 清空撤销栈（新会话）
    setSelected(null);
    setLoadError(null);
    setImage(img);
    setImagePath(path);
    setOrigin(org);
    setAutoOcr(runOcr);
    autoOcrFiredRef.current = false; // 新会话重新允许自动识别
    setSavedCapturePath(null);
    cropUndoRef.current = [];
    setCropUndoCount(0);
    setOcrHighlight(null);
    setOcrExportPath(null);
    setOcrLines(null);
  }, [reset]);

  const loadFromPath = useCallback((path: string, org: EditorOrigin | null, runOcr: boolean) => {
    const myGen = loadGuardRef.current.begin();
    const url = convertFileSrc(path);
    const probe = new Image();
    probe.onload = () => {
      if (!loadGuardRef.current.isCurrent(myGen)) return;
      applyImage(probe, path, org, runOcr);
    };
    probe.onerror = () => {
      if (!loadGuardRef.current.isCurrent(myGen)) return;
      // 降级：仅当 asset 加载失败才走 base64 IPC（上下文裁定）
      invoke<string>("editor_image_dataurl", { path })
        .then((dataUrl) => {
          if (!loadGuardRef.current.isCurrent(myGen)) return;
          const img = new Image();
          img.onload = () => {
            if (loadGuardRef.current.isCurrent(myGen)) applyImage(img, path, org, runOcr);
          };
          img.onerror = () => {
            if (loadGuardRef.current.isCurrent(myGen)) setLoadError(t("editor.decodeFailed"));
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
    // runOcr（v1.2）：动作浮条「OCR」入口（annotate_crop runOcr=true）——装载后
    // 自动跑一次识别（见 autoOcr effect）。
    listen<{ path: string; origin: EditorOrigin | null; runOcr?: boolean }>("hawk://editor-load", (e) => {
      if (!disposed) loadFromPath(e.payload.path, e.payload.origin ?? null, e.payload.runOcr === true);
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
    // 选中形状无 color 字段（mosaic）时不入历史；**值相同也不入**（v1.2：同色
    // 重复点击不再产生 no-op 空档——withColor 值未变时返回原对象引用可判等）
    if (selected != null && present[selected]) {
      const replaced = withColor(present[selected], c);
      if (replaced !== present[selected]) {
        commit(present.map((s, i) => (i === selected ? replaced : s)));
      }
    }
  }, [commit, present, selected]);

  // 遮蔽模式切换（v1.2，§3.3 像素化/模糊）：记忆进样式（持久化）；选中的遮蔽块
  // 同步换模式（一档历史；值相同则跳过——与 pickColor 同款 no-op 防线）。
  const pickMosaicMode = useCallback((m: MosaicMode) => {
    setStyle((s) => ({ ...s, mosaicMode: m }));
    if (selected != null && present[selected]?.kind === "mosaic") {
      const s0 = present[selected];
      if (s0.mode !== m) {
        commit(present.map((s, i) => (i === selected && s.kind === "mosaic" ? { ...s, mode: m } : s)));
      }
    }
  }, [commit, present, selected]);

  const commitWidthRestyle = useCallback(() => {
    setRestyle(false);
    // width 只对描边类形状有意义；值相同不入历史（v1.2 no-op 空档防线）
    if (selected != null && present[selected]) {
      const replaced = withWidth(present[selected], styleRef.current.width);
      if (replaced !== present[selected]) {
        commit(present.map((s, i) => (i === selected ? replaced : s)));
      }
    }
  }, [commit, present, selected]);

  const commitSizeRestyle = useCallback(() => {
    setRestyle(false);
    if (selected != null && present[selected]?.kind === "text") {
      const replaced = withSize(present[selected], styleRef.current.size);
      if (replaced !== present[selected]) {
        commit(present.map((s, i) => (i === selected ? replaced : s)));
      }
    }
  }, [commit, present, selected]);

  // —— 裁剪工具（M3 N0 第 10 工具，前端单路径）：把位图裁到选框 → 形状平移/丢弃
  // （applyCrop 纯函数）→ 工具复位 select（防误二次裁）。v1.2（审计修复，§3.3
  // 「撤销无步数限制」）：裁剪前的完整会话（位图 + 形状 + 撤销双栈）压入
  // cropUndoRef——形状撤销栈耗尽后 ⌘Z 继续逐级撤销裁剪（多层裁剪多层回退）；
  // 裁剪不再是一个无法回头的「新会话」。重做不跨裁剪（单向恢复，⌘⇧Z 不恢复裁剪
  // ——重做链在裁剪时已随快照保存，恢复裁剪后原重做链不适用新位图）。
  // Rust 侧零改动：imagePath/origin 不变，⌘S 时 editor_save 写回的就是
  // 裁后尺寸的 PNG（前端发什么写什么）；Pin 回写按新尺寸 set_image，Capture 来源走
  // record_history 同链。同步换图（setImage）使画布/导出的 internal dims 自然更新
  // （naturalWidth/Height 读自新 Image）。 ——

  const applyCropImage = useCallback(
    (rect: BBox) => {
      if (!image) return;
      // 选框钳到图内（拖拽可越出画布边缘；toCanvas 是浮点坐标）
      const cx = Math.max(0, Math.min(rect.x, image.naturalWidth));
      const cy = Math.max(0, Math.min(rect.y, image.naturalHeight));
      const cw = Math.min(rect.w, image.naturalWidth - cx);
      const ch = Math.min(rect.h, image.naturalHeight - cy);
      if (cw < 1 || ch < 1) return;
      const canvas = document.createElement("canvas");
      canvas.width = Math.round(cw);
      canvas.height = Math.round(ch);
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.drawImage(image, cx, cy, cw, ch, 0, 0, canvas.width, canvas.height);
      const url = canvas.toDataURL("image/png");
      const img = new Image();
      img.onload = () => {
        // 裁剪撤销快照：裁剪**生效前**的完整会话（含撤销/重做栈——恢复裁剪时原样
        // 写回，裁前的标注编辑历史一个不少）。
        cropUndoRef.current = [
          ...cropUndoRef.current,
          { img: image, shapes: present, past, future },
        ];
        setCropUndoCount(cropUndoRef.current.length);
        setImage(img);
        reset(applyCrop(present, { x: cx, y: cy, w: cw, h: ch }));
        setSelected(null);
        setTool("select");
      };
      img.src = url;
    },
    [image, present, past, future, reset],
  );

  // 撤销一次裁剪（⌘Z 在形状栈耗尽后的延续）：恢复快照的位图 + 形状 + 双栈，
  // 弹出该快照。多层裁剪逐级回退；无快照时 no-op。
  const undoCrop = useCallback(() => {
    const snap = cropUndoRef.current[cropUndoRef.current.length - 1];
    if (!snap) return false;
    cropUndoRef.current = cropUndoRef.current.slice(0, -1);
    setCropUndoCount(cropUndoRef.current.length);
    setImage(snap.img);
    restore({ past: snap.past, present: snap.shapes, future: snap.future });
    setSelected(null);
    return true;
  }, [restore]);

  // —— 导出保存（F5：经 exportToPng 单路径栅格化 → editor_save 写回 + origin
  // 副作用（M3 契约：pin 回写贴图 / capture 剪贴板+入库 / null 无））——
  // v1.2 保存去重（审计修复）：Capture 来源首次 ⌘S 落盘 save_dir 副本后记住该
  // 路径（editor_save 现返回最终落盘路径），后续 ⌘S 切到「覆盖该副本 + origin=null」
  // ——重复保存不再每次新建时间戳文件、重复入库。

  const save = useCallback(async () => {
    if (!image || !imagePath || saving) return;
    const target = savedCapturePath ?? imagePath;
    setSaving(true);
    setSaveError(null);
    try {
      const blob = await exportToPng(image, present, image.naturalWidth, image.naturalHeight);
      const pngBase64 = await blobToBase64(blob);
      const savedTo = await invoke<string>("editor_save", {
        path: target,
        pngBase64,
        origin: savedCapturePath ? null : origin,
      });
      if (origin?.kind === "capture" && !savedCapturePath && savedTo) {
        setSavedCapturePath(savedTo);
      }
      setSavedFlash(true);
      window.clearTimeout(flashTimer.current);
      flashTimer.current = window.setTimeout(() => setSavedFlash(false), 2000);
    } catch (e) {
      // ExportError.dictKey → 词典键就地翻译（错误串不再内联中文，见 export.ts）；
      // 其余（invoke 回来的 hserr 哨兵等）走原样/formatError 链。
      setSaveError(e instanceof ExportError ? t(e.dictKey) : String(e));
    } finally {
      setSaving(false);
    }
  }, [image, imagePath, present, saving, origin, savedCapturePath]);

  // —— OCR（M4）：识别 → 结果浮层（单块复制/全部复制/导出 txt/关闭）。首次调用
  // 触发模型下载（Rust 侧经 hawk://ocr-model-progress 推进度，此处转内联横幅）。
  // v1.2「所见即所识」（审计修复）：识别**当前画布**（裁剪后位图 + 全部标注形状，
  // exportToPng 栅格化）而非磁盘原图——裁剪后结果不再错位，马赛克遮蔽的敏感文字
  // 不会被结果面板明文暴露。box 坐标 = 画布自然坐标（画布内 1:1 高亮联动）。

  const runOcr = useCallback(async () => {
    if (!image || ocrRunning) return;
    setOcrRunning(true);
    setOcrError(null);
    setOcrProgress(null);
    setOcrFlash(null);
    setOcrHighlight(null);
    try {
      const blob = await exportToPng(image, present, image.naturalWidth, image.naturalHeight);
      const pngBase64 = await blobToBase64(blob);
      const lines = await invoke<OcrLine[]>("ocr_recognize_png", { pngBase64 });
      setOcrLines(lines);
    } catch (e) {
      setOcrError(e instanceof ExportError ? t(e.dictKey) : String(e));
      setOcrLines(null);
    } finally {
      setOcrRunning(false);
      setOcrProgress(null);
    }
  }, [image, present, ocrRunning]);

  // 自动 OCR（v1.2）：动作浮条「OCR」入口的 runOcr 载荷 → 位图就绪即跑一次。
  useEffect(() => {
    if (!autoOcr || autoOcrFiredRef.current) return;
    if (!image || ocrRunning) return;
    autoOcrFiredRef.current = true;
    setAutoOcr(false);
    void runOcr();
  }, [autoOcr, image, ocrRunning, runOcr]);

  /** OCR 行 box（4 点）→ 外接框（高亮联动用）。 */
  const lineToBox = (line: OcrLine): BBox => {
    const xs = line.box.map((p) => p.x);
    const ys = line.box.map((p) => p.y);
    const x = Math.min(...xs);
    const y = Math.min(...ys);
    return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
  };

  const ocrAllText = ocrLines?.map((l) => l.text).join("\n") ?? "";

  const copyOcrLine = useCallback(
    (line: OcrLine) => {
      void copyText(line.text);
      setOcrFlash("copied");
      window.clearTimeout(ocrFlashTimer.current);
      ocrFlashTimer.current = window.setTimeout(() => setOcrFlash(null), 2000);
    },
    [copyText],
  );

  const copyOcrAll = useCallback(() => {
    void copyText(ocrAllText);
    setOcrFlash("copied");
    window.clearTimeout(ocrFlashTimer.current);
    ocrFlashTimer.current = window.setTimeout(() => setOcrFlash(null), 2000);
  }, [copyText, ocrAllText]);

  const exportOcrTxt = useCallback(async () => {
    if (!imagePath) return;
    try {
      const out = await invoke<string>("ocr_export_txt", { path: imagePath, text: ocrAllText });
      // v1.2：flash 显示具体落点（此前只闪「已导出」——capture/pin 来源的落盘
      // 位置在 $TMPDIR，用户根本找不到导出的 txt）。
      setOcrExportPath(out);
      setOcrFlash("exported");
      window.clearTimeout(ocrFlashTimer.current);
      ocrFlashTimer.current = window.setTimeout(() => setOcrFlash(null), 4000);
    } catch (e) {
      setOcrError(String(e));
    }
  }, [imagePath, ocrAllText]);

  // 快捷键：⌘Z/⌘⇧Z（撤销重做）、Delete/Backspace（删除选中）、Esc（分层取消：
  // 收 OCR 浮层 → 取消选中；进行中的裁剪框由 EditorCanvas 的先注册监听优先吞掉）、
  // ⌘S（保存）、单键工具切换（v1.2：V/R/E/A/P/H/M/N/T/C——与主流标注器一致；
  // 文本输入聚焦时不接管）。文本输入聚焦时不接管（内联输入的字符编辑优先；其自身的
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
          // 撤销分层（v1.2）：形状栈耗尽后 ⌘Z 继续逐级撤销裁剪
          if (canUndo) undo();
          else undoCrop();
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
        return;
      }
      if (e.key === "Escape") {
        // Esc 分层消费（v1.2 审计修复）：裁剪框（EditorCanvas 吞掉）→ OCR 浮层 →
        // 选中态。窗体不随 Esc 关闭（编辑中的成果不因误按丢失——关闭走「关闭」按钮）。
        if (ocrLines !== null) {
          setOcrLines(null);
          setOcrHighlight(null);
        } else {
          setSelected(null);
        }
        return;
      }
      // 单键工具切换（v1.2）：无修饰键的单字母，命中工具表即切（不抢占系统键）
      if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
        const tool = TOOL_KEYS[e.key.toLowerCase()];
        if (tool) setTool(tool);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo, save, selected, deleteSelected, undoCrop, ocrLines, canUndo]);

  // 工具文案（G6 字面量 key：全部 t() 调用都是字面量，检查器可扫；随语言态重渲染换新）
  const toolLabels: Record<Tool, string> = {
    select: t("editor.tool.select"),
    crop: t("editor.tool.crop"),
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
    <div className="hs-editor" style={{ display: "flex", flexDirection: "column", height: "100vh", background: "#1f1f1f" }}>
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
            data-swatch=""
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
        {(tool === "mosaic" || (selected != null && present[selected]?.kind === "mosaic")) && (
          // 遮蔽模式分段开关（v1.2，§3.3「马赛克（像素化/模糊）」）：记忆进样式 +
          // 选中的遮蔽块同步换模式（pickMosaicMode）
          <div role="group" aria-label={t("editor.tool.mosaic")} style={{ display: "flex", border: "1px solid #444", borderRadius: 4, overflow: "hidden" }}>
            {([
              { id: "pixelate" as const, label: t("editor.mosaic.pixelate") },
              { id: "blur" as const, label: t("editor.mosaic.blur") },
            ]).map(({ id, label }) => (
              <button key={id} type="button" aria-pressed={(style.mosaicMode ?? "pixelate") === id}
                onClick={() => pickMosaicMode(id)}
                style={{ padding: "3px 8px", fontSize: 12, lineHeight: "16px", cursor: "pointer",
                  border: "none",
                  background: (style.mosaicMode ?? "pixelate") === id ? "#223047" : "transparent",
                  color: (style.mosaicMode ?? "pixelate") === id ? "#fff" : "#ccc" }}>
                {label}
              </button>
            ))}
          </div>
        )}
        <span style={sep} />
        {/* 画布缩放（v1.2）：− / 百分比（点击 = 100%）/ + / 适应窗口；Ctrl/⌘+滚轮同效。
            大图（高 DPI 截图/竖长图）上做细部标注、核对遮蔽覆盖的必备。 */}
        <button type="button" style={btn} title={t("editor.zoom.out")}
          onClick={() => setZoom((z) => Math.max(ZOOM_MIN, (z ?? 1) * 0.8))}>−</button>
        <button type="button" style={btn} title={t("editor.zoom.reset")}
          onClick={() => setZoom(1)}>{zoom == null ? t("editor.zoom.fit") : `${Math.round(zoom * 100)}%`}</button>
        <button type="button" style={btn} title={t("editor.zoom.in")}
          onClick={() => setZoom((z) => Math.min(ZOOM_MAX, (z ?? 1) * 1.25))}>+</button>
        <button type="button" style={{ ...btn, ...(zoom == null ? btnActive : {}) }}
          title={t("editor.zoom.fit")}
          onClick={() => setZoom(null)}>{t("editor.zoom.fit")}</button>
        <span style={sep} />
        <button type="button" style={btn} disabled={!canUndo && cropUndoCount === 0}
          title={t("editor.undo")} onClick={() => (canUndo ? undo() : undoCrop())}>
          {t("editor.undo")}
        </button>
        <button type="button" style={btn} disabled={!canRedo} title={t("editor.redo")} onClick={redo}>
          {t("editor.redo")}
        </button>
        <span style={{ flex: 1 }} />
        <button
          type="button"
          style={{ ...btn, ...(ocrRunning ? btnActive : {}) }}
          disabled={!image || !imagePath || ocrRunning}
          title={t("editor.ocr")}
          onClick={() => void runOcr()}
        >
          {ocrRunning ? t("editor.ocr.running") : t("editor.ocr")}
        </button>
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
          {t("editor.loadError", { message: formatError(loadError) })}
        </div>
      )}
      {saveError && (
        <div role="alert" style={errorBanner}>
          {t("editor.saveFailed", { message: formatError(saveError) })}
        </div>
      )}
      {ocrError && (
        <div role="alert" style={{ ...errorBanner, display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ flex: 1 }}>{t("editor.ocr.failed", { message: formatError(ocrError) })}</span>
          {/* §9「手动下载入口」（v1.2）：模型下载失败/校验失败时可打开模型目录手动
              放置文件后重试（ensure_models 幂等——SHA256 对得上就不重下）。 */}
          <button type="button" style={btn} onClick={() => void openModelsDir()}>
            {t("editor.ocr.modelsDir")}
          </button>
        </div>
      )}
      {ocrRunning && ocrProgress && (
        <div style={progressBanner} role="status">
          {ocrProgress.phase === "verify"
            ? t("editor.ocr.verify")
            : t("editor.ocr.download", {
                percent:
                  ocrProgress.total > 0
                    ? Math.min(100, Math.round((ocrProgress.downloaded * 100) / ocrProgress.total))
                    : 0,
              })}
        </div>
      )}
      <div style={canvasArea}>
        <EditorCanvas
          image={image}
          shapes={present}
          tool={tool}
          style={style}
          selected={selected}
          previewStyle={restyle && selected != null ? style : null}
          zoom={zoom}
          onZoomChange={setZoom}
          ocrHighlight={ocrHighlight}
          onSelect={setSelected}
          onCommit={commitShape}
          onMoveShape={moveShape}
          onReplaceText={replaceText}
          onCrop={applyCropImage}
        />
        {ocrLines !== null && (
          <div style={ocrPanel} role="complementary" aria-label={t("editor.ocr.title")}>
            <div style={ocrPanelHead}>
              <span style={{ fontWeight: 600 }}>{t("editor.ocr.title")}</span>
              <button
                type="button"
                style={btn}
                aria-label={t("editor.ocr.close")}
                title={t("editor.ocr.close")}
                onClick={() => {
                  setOcrLines(null);
                  setOcrHighlight(null);
                }}
              >
                <svg viewBox="0 0 24 24" width="12" height="12" fill="none"
                  stroke="currentColor" strokeWidth={1.8} strokeLinecap="round"
                  aria-hidden="true">
                  <path d="M6 6l12 12M18 6 6 18" />
                </svg>
              </button>
            </div>
            {ocrLines.length === 0 ? (
              <div style={ocrEmpty}>{t("editor.ocr.empty")}</div>
            ) : (
              <>
                <div style={ocrLineList}>
                  {ocrLines.map((line, i) => (
                    <button
                      key={i}
                      type="button"
                      style={ocrLineBtn}
                      title={t("editor.ocr.copyLine")}
                      onClick={() => copyOcrLine(line)}
                      // 行 hover ↔ 画布内该行外接框高亮（v1.2 文本框浮层联动，§3.3）
                      onMouseEnter={() => setOcrHighlight(lineToBox(line))}
                      onMouseLeave={() => setOcrHighlight(null)}
                      onFocus={() => setOcrHighlight(lineToBox(line))}
                      onBlur={() => setOcrHighlight(null)}
                    >
                      <span style={ocrScore}>{Math.round(line.score * 100)}%</span>
                      <span style={ocrTextSpan}>{line.text}</span>
                    </button>
                  ))}
                </div>
                <div style={ocrPanelFoot}>
                  {ocrFlash && (
                    <span style={ocrFlashText}
                      title={ocrFlash === "exported" ? (ocrExportPath ?? "") : undefined}>
                      {ocrFlash === "copied"
                        ? t("editor.ocr.copied")
                        : t("editor.ocr.exportedTo", { path: ocrExportPath ?? "" })}
                    </span>
                  )}
                  <span style={{ flex: 1 }} />
                  <button type="button" style={btn} title={t("editor.ocr.copyAll")} onClick={copyOcrAll}>
                    {t("editor.ocr.copyAll")}
                  </button>
                  <button type="button" style={btn} title={t("editor.ocr.export")} onClick={() => void exportOcrTxt()}>
                    {t("editor.ocr.export")}
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </div>
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
  // 深色窗体上的错误横幅用深底红字（v1.2：旧浅底红字浮在暗色 UI 上极违和）
  border: "1px solid #5B2430",
  borderRadius: 6,
  background: "#2a1518",
  color: "#E5484D",
  fontSize: 13,
};

const progressBanner: CSSProperties = {
  margin: "8px 8px 0",
  padding: "8px 12px",
  border: "1px solid #4aa3ff",
  borderRadius: 6,
  background: "#1a2a40",
  color: "#9ecbff",
  fontSize: 13,
};

const canvasArea: CSSProperties = {
  position: "relative",
  flex: 1,
  minHeight: 0,
  display: "flex",
};

const ocrPanel: CSSProperties = {
  position: "absolute",
  top: 12,
  right: 12,
  width: 300,
  maxHeight: "calc(100% - 24px)",
  display: "flex",
  flexDirection: "column",
  background: "rgba(17, 19, 24, 0.96)",
  border: "1px solid #3a3f4a",
  borderRadius: 8,
  boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
  zIndex: 10,
};

const ocrPanelHead: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: "8px 12px",
  borderBottom: "1px solid #333",
  color: "#ddd",
  fontSize: 13,
};

const ocrPanelFoot: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "8px 12px",
  borderTop: "1px solid #333",
};

const ocrLineList: CSSProperties = {
  overflowY: "auto",
  flex: 1,
  padding: 6,
  display: "flex",
  flexDirection: "column",
  gap: 4,
};

const ocrLineBtn: CSSProperties = {
  display: "flex",
  alignItems: "baseline",
  gap: 6,
  width: "100%",
  textAlign: "left",
  padding: "6px 8px",
  fontSize: 12,
  lineHeight: 1.5,
  borderRadius: 4,
  border: "1px solid #2a2e36",
  background: "#1c1f26",
  color: "#ddd",
  cursor: "pointer",
  wordBreak: "break-all",
};

const ocrScore: CSSProperties = {
  color: "#7fbf7f",
  fontSize: 10,
  flexShrink: 0,
};

const ocrTextSpan: CSSProperties = {
  flex: 1,
};

const ocrEmpty: CSSProperties = {
  padding: "16px 12px",
  color: "#888",
  fontSize: 13,
  textAlign: "center",
};

const ocrFlashText: CSSProperties = {
  color: "#8ff0c0",
  fontSize: 12,
  // 导出落点路径可长（$TMPDIR 深处）——面板宽 300px 内截断，完整路径在 title
  maxWidth: 170,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
