use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingCtx { pub app: String, pub date: String, pub time: String }

pub fn expand(template: &str, ctx: &NamingCtx, mut exists: impl FnMut(&str) -> bool) -> String {
    let base = template
        .replace("{app}", &ctx.app)
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
}
