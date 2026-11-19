//! 错误码化（v1.2，§3.7 G6 i18n）：用户可见的 Err 串不再内嵌中文——改为哨兵码
//! `<hserr:code>`（可选参数 `|k=v;k=v`），前端 `lib/errors.ts` 解析后映射到词典
//! `err.*` 键（zh/en 双语），未知码**原样透传**（协议向前兼容：旧前端收到新码、
//! 映射漏配时用户看到的是原始串而不是空串/码本身——与 updater.rs 的
//! `updater_not_configured` 哨兵先例同一机制，此处泛化）。
//!
//! 参数值经 [`sanitize_kv`] 约束（`;`/`=`/换行是协议分隔符，值内出现会破坏解析）；
//! 自由文本（OS 错误详情等）以 `detail` 单参数传入，供词典 `{detail}` 插值。
//!
//! 不进此机制的错误串：native_protocol / native_messaging / hawksnap-host
//! （插件↔宿主协议面，错误直接显示在插件自己的中文 UI 里，随插件 i18n 一并处理）；
//! 测试夹具串。

/// 哨兵码前缀（解析锚点）：尖括号形态在既有错误串里不存在，且不会被 serde/路径
/// 处理破坏。
pub const TAG: &str = "<hserr:";

/// 无参错误码：`<hserr:code>`。
pub fn coded(code: &str) -> String {
    format!("{TAG}{code}>")
}

/// 带参错误码：`<hserr:code>|k=v;k=v`。值经 [`sanitize_kv`] 清洗。
pub fn coded_kv(code: &str, kvs: &[(&str, &str)]) -> String {
    if kvs.is_empty() {
        return coded(code);
    }
    let body = kvs
        .iter()
        .map(|(k, v)| format!("{k}={}", sanitize_kv(v)))
        .collect::<Vec<_>>()
        .join(";");
    format!("{TAG}{code}>|{body}")
}

/// 参数值清洗：分隔符（`;`/`=`/`|`）与尖括号、换行替换为空格（诊断信息折行可
/// 接受，解析不可破）。`|`/`<`/`>` 是哨兵语法的结构字符——前端 SENTINEL 正则的
/// kv 值域为 `[^\|<>\n]`，值含它们时**整个正则失配**，用户会看到 `<hserr:…>` 原文
/// 而非文案（2026-11-20 前端审计：两侧值域不对称，本函数以对齐前端值域收口）。
fn sanitize_kv(v: &str) -> String {
    v.replace([';', '=', '\n', '\r', '|', '<', '>'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coded_formats_sentinel() {
        assert_eq!(coded("path_empty"), "<hserr:path_empty>");
    }

    #[test]
    fn coded_kv_joins_pairs_and_sanitizes_values() {
        let s = coded_kv("disk_low", &[("free", "0.8"), ("detail", "a;b=c\nd")]);
        assert_eq!(s, "<hserr:disk_low>|free=0.8;detail=a b c d");
    }

    #[test]
    fn sanitize_kv_strips_sentinel_structural_chars() {
        // detail 含 | < > 时不再破坏哨兵解析（前端正则值域 [^|<>\n]）。
        let s = coded_kv("x", &[("detail", "a|b<c>d")]);
        assert_eq!(s, "<hserr:x>|detail=a b c d");
    }

    #[test]
    fn coded_kv_without_params_is_bare_code() {
        assert_eq!(coded_kv("path_empty", &[]), "<hserr:path_empty>");
    }
}
