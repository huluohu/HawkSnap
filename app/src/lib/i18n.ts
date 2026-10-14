// i18n（D1）：轻量自研 key-value 双语层（不引 i18next——轻量原则，同 spec「自研
// 矢量层」逻辑）。扁平点分 key（panel.title / overlay.save / banner.hotkeyError），
// 词典为两份扁平 JSON 静态导入。解析顺序：显式语言 → "system" 时取 navigator.language
// 前缀 zh → zh-CN 否则 en-US；词典缺 key 回退 zh-CN，再缺回退 key 本身（console.warn
// dev 告警）。语言切换经模块级 listener 通知，React 侧用 useSyncExternalStore 订阅
// （见 App.tsx），面板随保存的设置即时重渲染，无需重载。
import zhCN from "../locales/zh-CN.json";
import enUS from "../locales/en-US.json";

export type Lang = "zh-CN" | "en-US";
// settings.language 的合法域（与 Rust settings.rs sanitize 白名单一致）。
export type LanguageSetting = "system" | Lang;

const BUILTIN_ZH: Record<string, string> = zhCN;
const BUILTIN_EN: Record<string, string> = enUS;

// 可替换（仅测试经 __setDictionariesForTests）：验证回退链。
const dicts: Record<Lang, Record<string, string>> = {
  "zh-CN": BUILTIN_ZH,
  "en-US": BUILTIN_EN,
};

/** 测试专用：替换词典（传 undefined 恢复内建）。生产代码不得调用。 */
export function __setDictionariesForTests(
  zh?: Record<string, string>,
  en?: Record<string, string>,
): void {
  dicts["zh-CN"] = zh ?? BUILTIN_ZH;
  dicts["en-US"] = en ?? BUILTIN_EN;
}

// —— 语言态（模块级单例）+ 订阅 ——

function systemLang(): Lang {
  return navigator.language.toLowerCase().startsWith("zh") ? "zh-CN" : "en-US";
}

let current: Lang = systemLang();
const listeners = new Set<() => void>();

/** 当前生效语言（渲染期读取；配合 subscribe 供 useSyncExternalStore 使用）。 */
export function getLanguage(): Lang {
  return current;
}

/** 切换语言：settings.language → "system" 时按 navigator 解析；同值幂等不通知。 */
export function setLanguage(setting: LanguageSetting): void {
  const next = setting === "system" ? systemLang() : setting;
  if (next === current) return;
  current = next;
  for (const fn of listeners) fn();
}

/** 订阅语言变化；返回取消订阅函数。 */
export function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

// —— 查询 ——

/**
 * 取文案：当前语言 → zh-CN 回退 → key 本身（缺位 warn）。vars 做 {name} 插值，
 * 缺参占位符原样保留。动态 key（模板字面量）约定仅限 hotkey.kind.*（横幅归因），
 * 由单测断言在盘；其余一律字面量 key（scripts/check-i18n.mjs 的 G6 门只扫字面量）。
 */
export function t(key: string, vars?: Record<string, string | number>): string {
  const raw = dicts[current][key] ?? dicts["zh-CN"][key];
  if (raw === undefined) {
    console.warn(`[i18n] missing key: ${key}`);
    return key;
  }
  if (!vars) return raw;
  return raw.replace(/\{(\w+)\}/g, (m, name) =>
    Object.prototype.hasOwnProperty.call(vars, name) ? String(vars[name]) : m,
  );
}
