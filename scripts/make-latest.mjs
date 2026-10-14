#!/usr/bin/env node
// scripts/make-latest.mjs — 组装 Tauri updater 静态源 latest.json（plan D6，G7 更新通道）。
//
// 用法：node scripts/make-latest.mjs --tag v0.1.0 --repo OWNER/REPO --dist <dir> --out <dir>
//
// 在 --dist（publish job 里为三平台 release artifact 合并下载目录）中为每个
// updater 平台键各找一份更新载荷 + 同名 .sig：
//   darwin-aarch64  <- *.app.tar.gz     （tauri build --bundles app 的 updater 归档）
//   linux-x86_64    <- *.AppImage       （v2 updater 直接分发 AppImage 本体）
//   windows-x86_64  <- *-setup.exe      （v2 updater 直接分发 NSIS 安装器本体）
// 语义：任一平台缺失载荷/签名、或多于一份 → exit 1（宁可失败也不出脏 feed；
// .sig 缺失即「未签名产物不得发布」——与 workflow 的 secrets 硬失败同一防线）。
// signature 字段 = .sig 全文（minisign 两行格式，untrusted comment + base64），
// 仅去首尾空白；url 指向该 tag 的 Release 下载地址。
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";

const PAYLOADS = [
  { key: "darwin-aarch64", ends: ".app.tar.gz" },
  { key: "linux-x86_64", ends: ".AppImage" },
  { key: "windows-x86_64", ends: "-setup.exe" },
];

function walk(dir, out = []) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

const argv = process.argv.slice(2);
const arg = (name) => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
};
const tag = arg("--tag");
const repo = arg("--repo");
const dist = arg("--dist");
const outDir = arg("--out");
for (const [flag, v] of [["--tag", tag], ["--repo", repo], ["--dist", dist], ["--out", outDir]]) {
  if (!v) {
    console.error(`make-latest: 缺少参数 ${flag}（用法：--tag v0.1.0 --repo OWNER/REPO --dist <dir> --out <dir>）`);
    process.exit(2);
  }
}
if (!/^v\d/.test(tag)) {
  console.error(`make-latest: tag 应形如 v0.1.0，实得：${tag}`);
  process.exit(2);
}

const files = walk(dist);
const platforms = {};
for (const { key, ends } of PAYLOADS) {
  const hits = files.filter((f) => f.endsWith(ends));
  if (hits.length !== 1) {
    console.error(
      `make-latest: ${key} 期望恰好 1 份 *${ends}，实得 ${hits.length}：\n  ${hits.map((f) => relative(dist, f)).join("\n  ") || "(无)"}`,
    );
    process.exit(1);
  }
  const payload = hits[0];
  const sigPath = `${payload}.sig`;
  let signature;
  try {
    signature = readFileSync(sigPath, "utf8").trim();
  } catch {
    console.error(`make-latest: 缺签名文件 ${relative(dist, sigPath)}——未签名产物不得发布`);
    process.exit(1);
  }
  if (!signature) {
    console.error(`make-latest: 空签名 ${relative(dist, sigPath)}`);
    process.exit(1);
  }
  platforms[key] = {
    signature,
    url: `https://github.com/${repo}/releases/download/${tag}/${payload.split("/").pop()}`,
  };
  console.log(`make-latest: ${key} <- ${relative(dist, payload)}`);
}

const latest = {
  version: tag.slice(1),
  notes: `HawkSnap ${tag} — see README for install steps and known limitations.`,
  pub_date: new Date().toISOString(),
  platforms,
};
writeFileSync(join(outDir, "latest.json"), `${JSON.stringify(latest, null, 2)}\n`);
console.log(
  `make-latest: wrote ${join(outDir, "latest.json")} (version ${latest.version}, ${Object.keys(platforms).length} platforms)`,
);
