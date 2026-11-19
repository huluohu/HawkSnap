use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingCtx { pub app: String, pub date: String, pub time: String }

/// 文件名分量清洗（2026-11-20 审计修复）：`{app}` 来自前台应用名——macOS 的
/// localizedName 由第三方开发者任意控制，可含 `/`（路径分隔符 → join 出不存在
/// 的子目录 → 保存整体失败）、前导 `.`（截图变隐藏文件）、控制字符等。清洗
/// 只作用于**文件名**；历史「来源」列仍存原文（分列，见 commands.rs）。
fn sanitize_component(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            c if (c as u32) < 0x20 || c as u32 == 0x7f => '-',
            c => c,
        })
        .collect();
    // 去首尾空白 + 剥前导点（防隐藏文件/相对路径）与尾点/尾空格（Windows 文件名
    // 禁止以它们结尾）；限长 64 字符（chars 计，保多字节）。
    let mut out: String = mapped
        .trim()
        .trim_start_matches('.')
        .chars()
        .take(64)
        .collect();
    out = out.trim_end_matches(['.', ' ']).to_string();
    // Windows 保留设备名（含带扩展名的形态，如 "AUX.png" 也被系统拒绝）——模板
    // 恰为「{app}」单独成 stem 时兜底。
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| out.eq_ignore_ascii_case(r)) {
        out = format!("{out}-");
    }
    if out.is_empty() { out = "HawkSnap".into(); }
    out
}

pub fn expand(template: &str, ctx: &NamingCtx, mut exists: impl FnMut(&str) -> bool) -> String {
    let base = template
        .replace("{app}", &sanitize_component(&ctx.app))
        .replace("{date}", &ctx.date)
        .replace("{time}", &ctx.time)
        .replace("{n}", "001");
    if !exists(&base) { return base; }
    for i in 2..10_000 {
        let candidate = format!("{base} {i}");
        if !exists(&candidate) { return candidate; }
    }
    format!("{base} {}", chrono_free_suffix())
}

fn chrono_free_suffix() -> u64 { std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0) }

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> NamingCtx {
        NamingCtx { app: "Safari".into(), date: "2026-10-13".into(), time: "153000".into() }
    }

    #[test]
    fn expands_all_variables() {
        let out = expand("{app} {date} {time}", &ctx(), |_| false);
        assert_eq!(out, "Safari 2026-10-13 153000");
    }

    #[test]
    fn appends_counter_on_collision() {
        let mut calls = 0;
        let out = expand("shot", &ctx(), |name| {
            calls += 1;
            name == "shot" || name == "shot 2" // 已存在 shot 与 shot 2
        });
        assert_eq!(out, "shot 3");
        assert_eq!(calls, 3);
    }

    #[test]
    fn zero_pads_sequence_number() {
        let out = expand("{n}", &ctx(), |_| false);
        assert_eq!(out, "001"); // 三位序号，{n} 为当日第几张由调用方传入计数——M1 简化为 001 起步
    }

    // —— sanitize_component（2026-11-20 审计修复）：{app} 不可信（localizedName
    // 由第三方应用控制），文件名分量必须清洗；历史来源列存原文不受影响。——

    #[test]
    fn sanitizes_path_separators_and_control_chars() {
        let c = NamingCtx {
            app: "a/b\\c:d\u{7}e".into(),
            date: "2026-11-20".into(),
            time: "120000".into(),
        };
        let out = expand("{app}", &c, |_| false);
        assert_eq!(out, "a-b-c-d-e");
    }

    #[test]
    fn strips_leading_dots_and_trims() {
        let c = NamingCtx { app: "  .hidden  ".into(), ..ctx() };
        assert_eq!(expand("{app}", &c, |_| false), "hidden");
        // Windows 禁止文件名以点/空格结尾。
        let c2 = NamingCtx { app: "Inc. ".into(), ..ctx() };
        assert_eq!(expand("{app}", &c2, |_| false), "Inc");
    }

    #[test]
    fn truncates_long_names_by_chars() {
        let c = NamingCtx { app: "é".repeat(80), ..ctx() };
        let out = expand("{app}", &c, |_| false);
        assert_eq!(out.chars().count(), 64);
    }

    #[test]
    fn empty_app_falls_back_to_brand() {
        let c = NamingCtx { app: "   ".into(), ..ctx() };
        assert_eq!(expand("{app}", &c, |_| false), "HawkSnap");
    }

    #[test]
    fn windows_reserved_stem_gets_suffix() {
        let c = NamingCtx { app: "aux".into(), ..ctx() };
        assert_eq!(expand("{app}", &c, |_| false), "aux-");
    }
}
