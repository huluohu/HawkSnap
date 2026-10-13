#!/usr/bin/env bash
# scripts/bench-latency.sh — 手动跑 20 次 F1 截图后统计 P90（spec G3）
set -euo pipefail
echo "连按 20 次 F1（每次 Enter 确认），完成后回车继续..."
read -r
python3 - <<'EOF'
import csv, glob, statistics, sys, tempfile
# 与 Rust 侧 perf.rs 同目录：std::env::temp_dir()/hawksnap-perf（macOS 即 $TMPDIR，
# 与 tempfile.gettempdir() 一致，勿写死 /tmp）
perf_dir = tempfile.gettempdir() + "/hawksnap-perf"
rows = [list(csv.reader(open(f))) for f in glob.glob(perf_dir + "/*.csv")]
flat = [r for run in rows for r in run]
by = {}
for stage, ts in flat: by.setdefault(stage, []).append(int(ts))
# 长度校验（评审 Finding 2b）：Esc 取消的轮次缺 confirm/clipboard_done，游离/迟到事件会
# 多出行——按行 zip 配对会静默错位，宁可报错也不输出被污染的 P90。
n = len(by.get("hotkey", []))
if not (n > 0 and n == len(by.get("overlay_shown", [])) == len(by.get("clipboard_done", []))
        and len(by.get("confirm", [])) == n):
    sys.exit(f"数据不齐：清空 {perf_dir} 后重跑，bench 期间不要 Esc 取消")
def p90(xs): return sorted(xs)[int(len(xs)*0.9)]
h2o = [b-a for a,b in zip(by["hotkey"], by["overlay_shown"])]
# 确认→剪贴板从 confirm（finish_capture 首行，即 invoke 时刻）起算（评审 Finding 1）：
# 旧口径 overlay_shown→clipboard_done 混入人工框选时间，无法作为 G3 ≤100ms 的证据。
c2c = [b-a for a,b in zip(by["confirm"], by["clipboard_done"])]
print(f"热键→覆盖层 P90 = {p90(h2o)}ms (目标 ≤150)")
print(f"确认→剪贴板 P90 = {p90(c2c)}ms (目标 ≤100)")
EOF
