#!/usr/bin/env bash
# scripts/bench-synth.sh [N] — 合成 N 轮 F1→拖拽→双击基准（免人工按键），跑完自动统计 P90
# 前置自检：HawkSnap 须已运行；$TMPDIR/hawksnap-perf 须不存在或为空（脚本不代删，防误删历史数据）。
set -euo pipefail
N="${1:-20}"
DIR="$(cd "$(dirname "$0")" && pwd)"

pgrep -f "MacOS/hawksnap" >/dev/null || { echo "HawkSnap 未运行"; exit 1; }
PERF_DIR="${TMPDIR%/}/hawksnap-perf"
if [ -d "$PERF_DIR" ] && [ -n "$(ls -A "$PERF_DIR")" ]; then
  echo "先清空 $PERF_DIR"
  exit 1
fi

swift "$DIR/bench_synth.swift" "$N"
sleep 5
echo | "$DIR/bench-latency.sh"
