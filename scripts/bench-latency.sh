#!/usr/bin/env bash
# scripts/bench-latency.sh — 手动跑 20 次 F1 截图后统计 P90（spec G3）
set -euo pipefail
echo "连按 20 次 F1（每次 Enter 确认），完成后回车继续..."
read -r
python3 - <<'EOF'
import csv, glob, statistics, tempfile
# 与 Rust 侧 perf.rs 同目录：std::env::temp_dir()/hawksnap-perf（macOS 即 $TMPDIR，
# 与 tempfile.gettempdir() 一致，勿写死 /tmp）
rows = [list(csv.reader(open(f))) for f in glob.glob(tempfile.gettempdir() + "/hawksnap-perf/*.csv")]
flat = [r for run in rows for r in run]
by = {}
for stage, ts in flat: by.setdefault(stage, []).append(int(ts))
def p90(xs): return sorted(xs)[int(len(xs)*0.9)]
h2o = [b-a for a,b in zip(by["hotkey"], by["overlay_shown"])]
c2c = [b-a for a,b in zip(by["overlay_shown"], by["clipboard_done"])]
print(f"热键→覆盖层 P90 = {p90(h2o)}ms (目标 ≤150)")
print(f"确认→剪贴板 P90 = {p90(c2c)}ms (目标 ≤100)")
EOF
