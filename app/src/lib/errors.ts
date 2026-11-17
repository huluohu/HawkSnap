// 错误码映射（v1.2，§3.7 G6）：Rust 侧用户可见错误串已错误码化——
// `<hserr:code>|k=v;k=v` 哨兵（app/src-tauri/src/errcode.rs），本文件解析后映射到
// 词典 `err.*` 键（zh/en 双语）并做 {k} 插值；**未知码原样透传**（向前兼容：映射
// 漏配时用户看到原始串而非空串/码本身，与 updater 哨兵先例同一机制）。
import { t } from "./i18n";

const SENTINEL = /^<hserr:([a-z0-9_]+)>(?:\|((?:[^|<>\n]|:(?!hserr:))*))?$/;

/**
 * 把后端错误串渲染为当前语言的文案：哨兵码 → t("err.<code>", vars)；
 * 非哨兵串（含历史消息/第三方原始串）原样返回。词典缺键时 t() 回退 key 本身，
 * 与原串不同 → 返回原串（宁 show 原文不 show 码）。
 */
export function formatError(raw: string): string {
  const m = SENTINEL.exec(raw);
  if (!m) return raw;
  const key = `err.${m[1]}`;
  const vars: Record<string, string> = {};
  if (m[2]) {
    for (const pair of m[2].split(";")) {
      const i = pair.indexOf("=");
      if (i > 0) vars[pair.slice(0, i)] = pair.slice(i + 1);
    }
  }
  const out = t(key, vars);
  return out === key ? raw : out;
}
