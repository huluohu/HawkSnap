#!/usr/bin/env node
// scripts/check-i18n.mjs — G6 i18n key 覆盖率门（纯 node，无依赖）。
//
// 语义（plan D2）：
//   ① 扫描 app/src/**/*.{ts,tsx} 中 t("...") / t('...') 字面量调用 → key 集合 A；
//   ② 读 app/src/locales/zh-CN.json 与 en-US.json（扁平词典）→ 集合 Z / E；
//   ③ A⊆Z 且 A⊆E 且 Z==E（双向无孤儿）→ exit 0；否则打印缺失/孤儿清单 → exit 1。
//
// 动态 key 约定：`t(variable)` / t(`template`) 扫不到——约定禁止新增动态 key，
// 检查器对 t( 后非引号字面量的代码行报 warn（不 fail）。现存唯一的豁免族是
// t(`hotkey.kind.${kind}`)（横幅归因），其三个 key 双语在盘由 app/src/lib/i18n.test.ts
// 的单测断言兜底（检查器之外的补网）。
// 注释行（//、/*、* 开头）与 t 的函数定义不参与扫描——注释里的 t("...") 示例不是文案面。
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SRC = join(ROOT, "app", "src");
const LOCALE_FILES = {
  "zh-CN": join(SRC, "locales", "zh-CN.json"),
  "en-US": join(SRC, "locales", "en-US.json"),
};

// 收集 .ts/.tsx 源文件（跳过 .test.ts——测试里的 t("...") 属测试自身构造，不是 UI
// 文案面；词典导入与断言不在 G6 语义内）。
function walk(dir, out = []) {
  for (const name of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, name.name);
    if (name.isDirectory()) walk(p, out);
    else if (/\.(ts|tsx)$/.test(name.name) && !/\.test\.(ts|tsx)$/.test(name.name)) {
      out.push(p);
    }
  }
  return out;
}

// 字面量调用：t("key") / t('key')（含换行空白）。\b 保证不误伤 split( / setTimeout( 等
// （词字符紧邻的 t 无边界）；键内不出现引号/反斜杠——key 词汇表不含此类字符。
const LITERAL_T = /\bt\(\s*(['"])([^'"\\]+)\1/g;
// 非字面量调用：t( 后第一个非空白不是引号（变量、模板串）→ warn。(?<!function ) 排除
// t 本身的函数定义行。
const NON_LITERAL_T = /(?<!function )\bt\(\s*([^'"\s)])/g;

const files = walk(SRC);
const used = new Map(); // key → [file:line, ...]
const dynamicCalls = [];

for (const file of files) {
  const rel = file.slice(ROOT.length + 1);
  const lines = readFileSync(file, "utf8").split("\n");
  lines.forEach((line, i) => {
    const trimmed = line.trimStart();
    if (trimmed.startsWith("//") || trimmed.startsWith("/*") || trimmed.startsWith("*")) return;
    for (const m of line.matchAll(LITERAL_T)) {
      const key = m[2];
      if (!used.has(key)) used.set(key, []);
      used.get(key).push(`${rel}:${i + 1}`);
    }
    for (const m of line.matchAll(NON_LITERAL_T)) {
      dynamicCalls.push(
        `${rel}:${i + 1}  t(${m[1]}…  — 动态 key，检查器盲区（约定豁免族：hotkey.kind.* / lib/errors.ts 的 err.* / editor/export.ts 的 ExportError.dictKey——三族由单测与词典成对性兜底）`,
      );
    }
  });
}

// 读词典（扁平 key 集合；解析失败即红）。en 值语言守卫需要完整对象，一并保留。
const dicts = {};
const dictObjs = {};
for (const [lang, path] of Object.entries(LOCALE_FILES)) {
  try {
    const parsed = JSON.parse(readFileSync(path, "utf8"));
    dictObjs[lang] = parsed;
    dicts[lang] = Object.keys(parsed);
  } catch (e) {
    console.error(`[i18n] 词典不可读/非法 JSON: ${path}\n  ${e.message}`);
    process.exit(1);
  }
}
const [Z, E] = [new Set(dicts["zh-CN"]), new Set(dicts["en-US"])];

const missingInZh = [...used.keys()].filter((k) => !Z.has(k));
const missingInEn = [...used.keys()].filter((k) => !E.has(k));
const orphans = [
  ...[...Z].filter((k) => !E.has(k)).map((k) => `${k}  (仅 zh-CN)`),
  ...[...E].filter((k) => !Z.has(k)).map((k) => `${k}  (仅 en-US)`),
];

console.log(`[i18n] 扫描 ${files.length} 个源文件：字面量 key ${used.size} 个；zh=${Z.size} en=${E.size}`);
for (const line of dynamicCalls) console.warn(`[i18n] WARN 动态调用 ${line}`);

let failed = false;
if (missingInZh.length) {
  failed = true;
  console.error(`[i18n] 缺失于 zh-CN.json（A⊄Z）:\n  ${missingInZh.join("\n  ")}`);
}
if (missingInEn.length) {
  failed = true;
  console.error(`[i18n] 缺失于 en-US.json（A⊄E）:\n  ${missingInEn.join("\n  ")}`);
}
if (orphans.length) {
  failed = true;
  console.error(`[i18n] 词典孤儿（Z≠E，双向不一致）:\n  ${orphans.join("\n  ")}`);
}

// —— en 值语言守卫（2026-11-20 审计修复）——键集合门抓不到「值漏翻」：en-US.json
// 曾出现值仍是中文的键（saveDirPlaceholder），外语用户在 UI 直接看到中文。
// en 值含 CJK 统一表意字符即红（U+3400-9FFF + 兼容表意；⌘/—/全角标点不在列，
// 不误伤）。豁免：语言选项键（*.language.zh）的值**有意**用原生文字（英文界面
// 里语言名也显示「简体中文」，i18n 惯例）。设置 COPY en 表同门（见下方 COPY 门）。
const CJK = /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/;
const NATIVE_NAME_KEY = /\.(language|lang)\.[a-z-]+$/;
const cjkEn = Object.entries(dictObjs["en-US"])
  .filter(([, v]) => typeof v === "string" && CJK.test(v))
  .map(([k]) => k)
  .filter((k) => !NATIVE_NAME_KEY.test(k));
if (cjkEn.length) {
  failed = true;
  console.error(`[i18n] en-US.json 值残留中文（漏翻）:\n  ${cjkEn.join("\n  ")}`);
}

// —— 设置窗口 COPY 表门（v1.2）——
// sections.tsx 的本地 zh/en 文案表（tx( 取词)游离在词典门禁之外：\bt\( 正则不匹配
// tx(——此前 ~200 对文案零校验，漏键/不对齐只有运行时可见。这里补三道断言：
//   ① zh/en 键集完全相等（主词典 Z==E 的镜像语义）；
//   ② tx("...") 字面量引用的每个 key 都在表内（tx( 全量扫，无动态豁免——sections
//      现状无模板字面量取词）；
//   ③ COPY 键不得与主词典同名（同键双源 = 未来必漂移，出现即红）。
function extractObject(source, decl) {
  const at = source.indexOf(decl);
  if (at < 0) throw new Error(`check-i18n: 未找到 ${decl}`);
  const open = source.indexOf("{", at);
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}") {
      depth--;
      if (depth === 0) return source.slice(open, i + 1);
    }
  }
  throw new Error(`check-i18n: ${decl} 花括号不配对`);
}
function copyKeys(body) {
  return new Set([...body.matchAll(/"([^"]+)"\s*:/g)].map((m) => m[1]));
}
try {
  const settingsSrc = readFileSync(join(SRC, "windows", "settings", "sections.tsx"), "utf8");
  const settingsIndexSrc = readFileSync(join(SRC, "windows", "settings", "index.tsx"), "utf8");
  const zhBody = extractObject(settingsSrc, "const zh = {");
  const enBody = extractObject(settingsSrc, "const en: Record<CopyKey, string> = {");
  const zhKeys = copyKeys(zhBody);
  const enKeys = copyKeys(enBody);
  const txUsed = new Map();
  for (const [file, src] of [["sections.tsx", settingsSrc], ["settings/index.tsx", settingsIndexSrc]]) {
    src.split("\n").forEach((line, i) => {
      const trimmed = line.trimStart();
      if (trimmed.startsWith("//") || trimmed.startsWith("/*") || trimmed.startsWith("*")) return;
      for (const m of line.matchAll(/\btx\(\s*(['"])([^'"\\]+)\1/g)) {
        if (!txUsed.has(m[2])) txUsed.set(m[2], []);
        txUsed.get(m[2]).push(`${file}:${i + 1}`);
      }
    });
  }
  const copyMissingZh = [...txUsed.keys()].filter((k) => !zhKeys.has(k));
  const copyMissingEn = [...txUsed.keys()].filter((k) => !enKeys.has(k));
  const copyOrphanZh = [...zhKeys].filter((k) => !enKeys.has(k)).map((k) => `${k}  (仅 zh)`);
  const copyOrphanEn = [...enKeys].filter((k) => !zhKeys.has(k)).map((k) => `${k}  (仅 en)`);
  const colliding = [...zhKeys].filter((k) => Z.has(k) || E.has(k));
  console.log(`[i18n] 设置 COPY 表：zh=${zhKeys.size} en=${enKeys.size}，tx 字面量 ${txUsed.size} 个`);
  let copyFailed = false;
  if (copyMissingZh.length || copyMissingEn.length) {
    copyFailed = true;
    console.error(`[i18n] tx 引用缺失于 COPY 表:\n  zh缺: ${copyMissingZh.join(", ")}\n  en缺: ${copyMissingEn.join(", ")}`);
  }
  if (copyOrphanZh.length || copyOrphanEn.length) {
    copyFailed = true;
    console.error(`[i18n] COPY 表 zh/en 不对齐:\n  ${[...copyOrphanZh, ...copyOrphanEn].join("\n  ")}`);
  }
  if (colliding.length) {
    copyFailed = true;
    console.error(`[i18n] COPY 键与主词典同名（双源必漂移）: ${colliding.join(", ")}`);
  }
  // COPY en 表值语言守卫（与主词典同一纪律；语言选项原生名豁免）。
  const cjkCopy = [...enBody.matchAll(/"([^"]+)"\s*:\s*"([^"]*)"/g)]
    .filter(([, , v]) => CJK.test(v))
    .map(([, k]) => k)
    .filter((k) => !NATIVE_NAME_KEY.test(k));
  if (cjkCopy.length) {
    copyFailed = true;
    console.error(`[i18n] 设置 COPY en 表值残留中文（漏翻）: ${cjkCopy.join(", ")}`);
  }
  if (copyFailed) process.exit(1);
} catch (e) {
  console.error(`[i18n] 设置 COPY 表门异常: ${e.message}`);
  process.exit(1);
}

if (failed) process.exit(1);
console.log("[i18n] OK：A⊆Z、A⊆E 且 Z==E");
