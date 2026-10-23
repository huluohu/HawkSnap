#!/usr/bin/env bash
# G5 启测（M2 Task M6 交付，控制器执行）：72h 录屏稳定性脚本。
#
# 设计：单进程跑满全程——驱动 hawksnap-platform-mac 的 soak_segment_records_and_samples
# ignored 测试（一次录制会话贯穿整个时段，内存增长在同进程内累积，才能测出「增长 <10%」），
# 测试内部每小时自采样 RSS/CPU 追加到 CSV（HAWKSNAP_SOAK_CSV 指定落点）。
#
# 用法：
#   scripts/soak-record.sh                     # 默认 72h（HAWKSNAP_SOAK_HOURS=72）
#   HAWKSNAP_SOAK_HOURS=0.002 scripts/soak-record.sh   # 短模式自测（~10min）
#
# 环境变量：
#   HAWKSNAP_SOAK_HOURS      录制总时长（小时），默认 72
#   HAWKSNAP_SOAK_CSV        采样 CSV 路径，默认 ~/Movies/HawkSnap/soak-<ts>.csv
#   HAWKSNAP_SOAK_SAMPLE_SECS  采样间隔秒，默认 3600（短模式自动收紧为总时长/8）
#
# 判定（G5 验收）：测试进程退出码 0（崩溃=0）+ CSV 内存首尾对比（<10% 增长）。
# 本脚本对 CSV 做首尾 RSS 对比并打印结论；增长 ≥10% 时退出码 1。
set -euo pipefail

cd "$(dirname "$0")/.."

HOURS="${HAWKSNAP_SOAK_HOURS:-72}"
SECS=$(python3 -c "print(int(float('${HOURS}') * 3600))")
TS=$(date +%Y%m%d-%H%M%S)
OUT_DIR="$HOME/Movies/HawkSnap"
mkdir -p "$OUT_DIR"
CSV="${HAWKSNAP_SOAK_CSV:-$OUT_DIR/soak-$TS.csv}"

# 短模式：采样间隔收紧（保证 ≥2 行），并在 CSV 首行写表头。
SAMPLE_SECS="${HAWKSNAP_SOAK_SAMPLE_SECS:-$(( SECS / 8 ))}"
if [ "$SAMPLE_SECS" -lt 1 ]; then SAMPLE_SECS=1; fi

echo "[soak] duration=${SECS}s (${HOURS}h) sample-every=${SAMPLE_SECS}s csv=$CSV"
echo "time,pid,rss_kb,cpu_percent" > "$CSV"

# 前置：编译（record 测试依赖屏幕录制授权——首次运行 TCC 可能弹窗，需人工授权）。
export HAWKSNAP_RECORD_SECS="$SECS"
export HAWKSNAP_SOAK_CSV="$CSV"
export HAWKSNAP_SOAK_SAMPLE_SECS="$SAMPLE_SECS"

# 测试进程（唯一被测对象）在子 shell 里跑，便于采样其 RSS/CPU。
cargo test -p hawksnap-platform-mac --lib soak_segment_records_and_samples -- \
  --ignored --nocapture &
TEST_PID=$!

# 长时段启动看护（短模式跳过——测试可能先于本检查退出，wait 会拿到真实退出码）：
# 启动后 15s 采第一行（等 SCStream 就绪）；此后由测试自采样，本循环仅作看护心跳。
if [ "$SECS" -gt 120 ]; then
  sleep 15
  if ! kill -0 "$TEST_PID" 2>/dev/null; then
    echo "[soak] FAIL：测试进程在启动期退出（看 scripts 输出定位）" >&2
    exit 1
  fi
fi

wait "$TEST_PID"
STATUS=$?
if [ "$STATUS" -ne 0 ]; then
  echo "[soak] FAIL：录制测试退出码 $STATUS（崩溃≠0，G5 不通过）" >&2
  exit "$STATUS"
fi

# 内存增长判定：CSV 首行（就绪后首个样本）vs 末行 RSS。
python3 - "$CSV" << 'PYEOF'
import csv, sys

rows = list(csv.DictReader(open(sys.argv[1])))
if len(rows) < 2:
    print(f"[soak] 样本不足（{len(rows)} 行），跳过增长判定（时段过短）")
    sys.exit(0)
first, last = int(rows[0]["rss_kb"]), int(rows[-1]["rss_kb"])
growth = (last - first) / first * 100 if first else 0.0
peak = max(int(r["rss_kb"]) for r in rows)
print(f"[soak] rss {first}KB → {last}KB（增长 {growth:+.1f}%，峰值 {peak}KB，{len(rows)} 样本）")
if growth > 10.0:
    print("[soak] FAIL：内存增长 ≥10%（G5 不通过）")
    sys.exit(1)
print("[soak] PASS：进程存活全程 + 内存增长 <10%")
PYEOF
