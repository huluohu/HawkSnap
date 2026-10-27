#!/usr/bin/env node
// scripts/scrollshot-eval/generate-pages.mjs — 生成 20 个长页面（HTML）。
//
// 用途：**人工验收层**（控制器在浏览器里打开这些页面、真滚动、真按 ⌘S）。
// 自动验收层（Rust 跑批器）用同一份 scenes.json 的场景参数合成像素内容，
// 两层共用场景定义 → 自动层的成功率对人工层有参考意义。
//
// 页面特性按 layout 分布（覆盖 F10 的场景多样性要求）：正文密集、图片块、
// 表格、代码、聊天、固定头部、粘性侧栏、深色模式、文档/图库/博客/维基/表单/
// 新闻/论坛/PDF 工具栏/电子表格/幻灯片/混合。
//
// 用法：node scripts/scrollshot-eval/generate-pages.mjs
import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const OUT_DIR = join(ROOT, "scripts", "scrollshot-eval", "pages");
const SCENES = JSON.parse(
  readFileSync(join(ROOT, "scripts", "scrollshot-eval", "scenes.json"), "utf8"),
);

/** 确定性伪随机（mulberry32）——页面内容可复现。 */
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const WORDS = [
  "scroll", "capture", "stitch", "window", "frame", "shift", "offset", "median",
  "profile", "correlation", "template", "patch", "overlay", "region", "pixels",
  "viewport", "column", "header", "sidebar", "footer", "margin", "baseline",
  "texture", "gradient", "alpha", "blend", "seam", "canvas", "output",
];

/** 一段正文（n 段，每段若干行）。 */
function paragraphs(n, rand, cls = "p") {
  let out = "";
  for (let i = 0; i < n; i++) {
    const lines = 3 + Math.floor(rand() * 5);
    out += `<div class="${cls}">`;
    for (let l = 0; l < lines; l++) {
      const words = 6 + Math.floor(rand() * 9);
      const text = Array.from({ length: words }, () => WORDS[Math.floor(rand() * WORDS.length)]).join(" ");
      out += `<p>${text}</p>`;
    }
    out += "</div>";
  }
  return out;
}

function images(n, rand) {
  let out = "";
  for (let i = 0; i < n; i++) {
    const hue = Math.floor(rand() * 360);
    const h = 120 + Math.floor(rand() * 160);
    out += `<figure style="background:linear-gradient(120deg,hsl(${hue} 70% 70%),hsl(${(hue + 60) % 360} 70% 45%));height:${h}px"></figure>`;
  }
  return out;
}

function table(rows, cols, rand) {
  let out = "<table>";
  for (let r = 0; r < rows; r++) {
    out += "<tr>";
    for (let c = 0; c < cols; c++) {
      out += `<td>${WORDS[Math.floor(rand() * WORDS.length)]}</td>`;
    }
    out += "</tr>";
  }
  return out + "</table>";
}

function code(lines, rand) {
  let out = '<pre class="code">';
  for (let i = 0; i < lines; i++) {
    const indent = " ".repeat(Math.floor(rand() * 5) * 2);
    const words = 2 + Math.floor(rand() * 6);
    const text = Array.from({ length: words }, () => WORDS[Math.floor(rand() * WORDS.length)]).join("_");
    out += `${indent}${text}(${Math.floor(rand() * 100)});\n`;
  }
  return out + "</pre>";
}

function chat(messages, rand) {
  let out = "";
  for (let i = 0; i < messages; i++) {
    const mine = rand() > 0.5;
    const lines = 1 + Math.floor(rand() * 3);
    let body = "";
    for (let l = 0; l < lines; l++) {
      body += `<p>${WORDS[Math.floor(rand() * WORDS.length)]} ${WORDS[Math.floor(rand() * WORDS.length)]}</p>`;
    }
    out += `<div class="bubble ${mine ? "mine" : "theirs"}">${body}</div>`;
  }
  return out;
}

/** 按 layout 生成正文 HTML（不含 <head>/CSS —— CSS 在 buildPage 里统一给）。 */
function bodyFor(layout, rand) {
  switch (layout) {
    case "text":
      return paragraphs(160, rand);
    case "text-dense":
      return `<div class="dense">${paragraphs(240, rand, "p")}</div>`;
    case "images":
      return paragraphs(40, rand) + images(40, rand) + paragraphs(40, rand);
    case "tables":
      return paragraphs(20, rand) + table(40, 6, rand) + paragraphs(20, rand) + table(30, 4, rand);
    case "code":
      return paragraphs(20, rand) + code(120, rand) + paragraphs(20, rand) + code(80, rand);
    case "chat":
      return chat(220, rand);
    case "fixed-header":
      // 固定头部由 CSS 的 position:sticky 表达（滚动时不动 → 考验「顶栏切除」）
      return `<div class="sticky-header">固定头部 · HawkSnap 文档中心</div>` + paragraphs(150, rand);
    case "sticky-sidebar":
      return `<div class="sticky-sidebar">${paragraphs(30, rand, "side")}</div>` + paragraphs(150, rand);
    case "dark":
      return paragraphs(160, rand);
    case "doc":
      return `<h1>产品设计说明</h1>` + paragraphs(30, rand) + `<h2>背景</h2>` + paragraphs(40, rand) +
        `<h2>方案</h2>` + paragraphs(50, rand) + table(20, 4, rand);
    case "gallery":
      return `<div class="grid">${images(160, rand)}</div>`;
    case "blog":
      let blog = "";
      for (let i = 0; i < 40; i++) {
        blog += `<article><h3>第 ${i + 1} 篇</h3>${images(1, rand)}${paragraphs(1, rand)}</article>`;
      }
      return blog;
    case "wiki":
      return `<div class="toc">目录</div>` + `<h1>条目</h1>` + paragraphs(60, rand) + table(24, 3, rand) +
        `<h2>历史</h2>` + paragraphs(60, rand);
    case "form":
      let form = "";
      for (let i = 0; i < 60; i++) {
        form += `<label>字段 ${i + 1}<input value="${WORDS[Math.floor(rand() * WORDS.length)]}" /></label>`;
      }
      return form + paragraphs(60, rand);
    case "news":
      return `<h1>今日头条</h1>` + paragraphs(50, rand) + images(6, rand) + paragraphs(80, rand);
    case "forum":
      let forum = "";
      for (let i = 0; i < 45; i++) {
        forum += `<div class="post"><h4>楼主 #${i + 1}</h4>${paragraphs(2, rand)}</div>`;
      }
      return forum;
    case "pdf-toolbar":
      return `<div class="sticky-header">PDF 阅读器 · 第 1 / 60 页</div>` + paragraphs(170, rand);
    case "spreadsheet":
      let sheet = "";
      for (let b = 0; b < 6; b++) sheet += table(46, 8, rand);
      return `<div class="sheet">${sheet}</div>`;
    case "slides":
      let slides = "";
      for (let i = 0; i < 30; i++) {
        slides += `<section class="slide"><h2>${WORDS[Math.floor(rand() * WORDS.length)]}</h2>${paragraphs(2, rand)}</section>`;
      }
      return slides;
    case "mixed":
    default:
      return (
        paragraphs(40, rand) + images(20, rand) + paragraphs(40, rand) +
        table(30, 5, rand) + code(60, rand) + paragraphs(40, rand)
      );
  }
}

/** 页面骨架 + CSS（各 layout 的差异体现在 bodyFor 里）。 */
function buildPage(scene) {
  const rand = rng(scene.seed);
  const dark = scene.layout === "dark";
  const body = bodyFor(scene.layout, rand);
  return `<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8" />
<title>${scene.id} · HawkSnap 长截图 eval</title>
<style>
  :root { color-scheme: ${dark ? "dark" : "light"}; }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 0 0 4000px;
    font: ${dark ? "#d8dee9" : "#1d2430"} ${dark ? "16" : "15"}px/1.7 system-ui, sans-serif;
    background: ${dark ? "#14171c" : "#ffffff"};
  }
  .wrap { max-width: 860px; margin: 0 auto; padding: 0 24px; }
  p { margin: 0 0 10px; }
  h1 { font-size: 30px; margin: 28px 0 16px; }
  h2 { font-size: 22px; margin: 24px 0 12px; }
  h3 { font-size: 18px; margin: 20px 0 10px; }
  figure { margin: 18px 0; border-radius: 6px; }
  table { border-collapse: collapse; width: 100%; margin: 18px 0; }
  td { border: 1px solid ${dark ? "#333a45" : "#d8dee6"}; padding: 6px 8px; font-size: 13px; }
  pre.code { background: ${dark ? "#1f242c" : "#f4f6f9"}; padding: 14px; border-radius: 6px;
             font: 13px/1.6 ui-monospace, Menlo, monospace; overflow: visible; }
  .dense p { margin: 0 0 4px; font-size: 13px; line-height: 1.35; }
  .sticky-header { position: sticky; top: 0; z-index: 5; background: ${dark ? "#1f242c" : "#eef2f7"};
                   border-bottom: 1px solid ${dark ? "#333a45" : "#d8dee6"}; padding: 14px 24px; font-weight: 600; }
  .sticky-sidebar { position: sticky; top: 0; float: left; width: 220px; height: 100vh;
                     margin-right: 24px; padding: 16px; background: ${dark ? "#1b1f26" : "#f4f6f9"};
                     border-radius: 0 8px 8px 0; }
  .grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 14px; }
  .grid figure { height: 150px; margin: 0; }
  article { margin: 0 0 26px; }
  .bubble { max-width: 62%; margin: 10px 0; padding: 10px 14px; border-radius: 12px;
            background: ${dark ? "#242a33" : "#eef2f7"}; }
  .bubble.mine { margin-left: auto; background: ${dark ? "#204a3c" : "#d8f0e2"}; }
  .post { border: 1px solid ${dark ? "#2b313a" : "#e2e8f0"}; border-radius: 8px; padding: 14px; margin: 0 0 18px; }
  label { display: block; margin: 10px 0; }
  input { display: block; width: 320px; padding: 6px 8px; margin-top: 4px;
          border: 1px solid ${dark ? "#333a45" : "#c8d0dc"}; border-radius: 4px; background: transparent; color: inherit; }
  .sheet { overflow: visible; }
  .slide { min-height: 620px; border-bottom: 1px dashed ${dark ? "#333a45" : "#d8dee6"}; padding-bottom: 24px; }
  .toc { position: sticky; top: 0; float: right; width: 200px; padding: 12px;
         background: ${dark ? "#1b1f26" : "#f4f6f9"}; border-radius: 8px; }
</style>
</head>
<body>
<div class="wrap">${body}</div>
</body>
</html>
`;
}

mkdirSync(OUT_DIR, { recursive: true });
const index = [];
for (const scene of SCENES.scenes) {
  const file = join(OUT_DIR, `${scene.id}.html`);
  writeFileSync(file, buildPage(scene));
  index.push(`${scene.id}.html`);
}
writeFileSync(join(OUT_DIR, "index.html"),
  `<!doctype html><meta charset="utf-8"><title>长截图 eval 页面</title>` +
  `<h1>HawkSnap 长截图 eval 页面（20 个）</h1><ul>` +
  SCENES.scenes.map((s) => `<li><a href="${s.id}.html">${s.id}</a> — ${s.layout}</li>`).join("") +
  `</ul>`);
console.log(`[scrollshot-eval] 已生成 ${index.length} 个页面 → ${OUT_DIR}`);
