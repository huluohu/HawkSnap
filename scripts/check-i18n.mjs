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
        `${rel}:${i + 1}  t(${m[1]}…  — 动态 key，检查器盲区（约定：仅 hotkey.kind.* 豁免，单测兜底）`,
      );
    }
  });
}

// 读词典（扁平 key 集合；解析失败即红）。
const dicts = {};
for (const [lang, path] of Object.entries(LOCALE_FILES)) {
  try {
    dicts[lang] = Object.keys(JSON.parse(readFileSync(path, "utf8")));
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
if (failed) process.exit(1);
console.log("[i18n] OK：A⊆Z、A⊆E 且 Z==E");
