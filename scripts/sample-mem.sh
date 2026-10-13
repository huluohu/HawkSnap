#!/usr/bin/env bash
# scripts/sample-mem.sh — G2: 空载常驻内存采样（spec §5 ≤80MB）
# 采样 5 次、间隔 10s，RSS/1024 换算 MB；读数时应用应处于空载（未截图）状态。
set -euo pipefail
# pgrep 无匹配返回 1，配合 pipefail 会静默退出——|| true 保住下面的友好报错。
PID=$(pgrep -f "MacOS/hawksnap" | head -1 || true)
[ -z "$PID" ] && { echo "HawkSnap 未运行"; exit 1; }
echo "采样 5 次（间隔 10s，空载）..."
for i in 1 2 3 4 5; do
  sleep 10
  ps -o rss= -p "$PID" | awk '{printf "sample %d: %.1f MB\n", '"$i"', $1/1024}'
done
