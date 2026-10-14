#!/usr/bin/env bash
# scripts/bench-xdotool.sh [N] — Linux/X11 合成 N 轮「F1 → 拖拽 → 复制」基准（免人工按键），
# 跑完接 bench-latency.sh 统计 P90（spec G3）。macOS 版同类脚本是 bench-synth.sh（CGEvent）。
#
# 注入序列固化自 Plan C Linux 验收（docs/superpowers/bench/2026-10-14-plan-c-acceptance.md
# 备注）：xdotool key F1 → mousemove/mousedown/mousemove/mouseup → 计算「复制」按钮坐标 click。
#
# 前置：
#   - Xvfb + xdotool（X11；Wayland 无 xdotool 注入路径）；例：
#       Xvfb :99 -screen 0 1920x1080x24 &  DISPLAY=:99 openbox &
#       DISPLAY=:99 HAWKSNAP_... hawknap 应用已在同一 DISPLAY 运行
#   - 截图热键为默认 F1（脚本按 F1 注入；改过设置请自行调整）
#   - $TMPDIR/hawksnap-perf 不存在或为空（bench-latency.sh 会拒绝脏数据）
# 环境：DISPLAY（默认 :99）、VIEW_W/VIEW_H（默认取 xdotool getdisplaygeometry）、
#       X1/Y1/X2/Y2（拖拽对角，默认 400,300 → 700,500 = 300×200 选区）。
# 注意：本脚本依赖 X11 xdotool，macOS 不可运行（Darwin 直接退出，走 bench-synth.sh）。
set -euo pipefail

if [ "$(uname -s)" = "Darwin" ]; then
  echo "本脚本面向 Linux/X11（xdotool 注入）；macOS 请用 scripts/bench-synth.sh" >&2
  exit 1
fi
command -v xdotool >/dev/null 2>&1 || { echo "缺 xdotool（apt install xdotool）" >&2; exit 1; }

N="${1:-20}"
export DISPLAY="${DISPLAY:-:99}"

DIR="$(cd "$(dirname "$0")" && pwd)"
pgrep -x hawksnap >/dev/null 2>&1 || echo "警告：未见 hawksnap 进程（仍继续尝试）" >&2

# 视口与拖拽对角（逻辑坐标 = X11 坐标，Xvfb 无 Retina 缩放 scale=1.0）
if [ -z "${VIEW_W:-}" ] || [ -z "${VIEW_H:-}" ]; then
  read -r VIEW_W VIEW_H <<< "$(xdotool getdisplaygeometry)"
fi
X1="${X1:-400}" Y1="${Y1:-300}"   # 拖拽起点
X2="${X2:-700}" Y2="${Y2:-500}"   # 拖拽终点（选区 300×200，与 bench_mode 同尺寸）

# 「复制」按钮坐标 = 选区确认浮条内第一枚按钮的中心。浮条定位与前端
# app/src/windows/overlay/actionBar.ts 的 barPosition 同式（BAR_W=252 BAR_H=40
# MARGIN=12；改前端样式须同步此处）：默认置于选区下方 12px、右对齐选区、夹在视口内；
# 下方放不下翻到选区上方。按钮内容盒 ≈ 50×24（padding 4/12 + 13px 字号），中心约在
# 浮条左上角内 (35, 20)。
BAR_W=252 BAR_H=40 MARGIN=12
below_y=$((Y2 + MARGIN))
if [ $((below_y + BAR_H)) -gt "$VIEW_H" ]; then
  top=$((Y1 - BAR_H - MARGIN))
  [ "$top" -lt "$MARGIN" ] && top=$MARGIN
else
  top=$below_y
fi
left1=$((X2 - BAR_W)); [ "$left1" -lt "$MARGIN" ] && left1=$MARGIN
left2=$((VIEW_W - BAR_W - MARGIN)); [ "$left2" -lt "$MARGIN" ] && left2=$MARGIN
if [ "$left1" -lt "$left2" ]; then left=$left1; else left=$left2; fi
CLICK_X=$((left + 35))
CLICK_Y=$((top + 20))

echo "bench-xdotool：${N} 轮 @ ${DISPLAY}（${VIEW_W}x${VIEW_H}），拖拽 (${X1},${Y1})→(${X2},${Y2})，复制按钮 @ (${CLICK_X},${CLICK_Y})"
for _ in $(seq 1 "$N"); do
  xdotool key F1                       # 截图热键 → 全屏冻结帧覆盖层
  sleep 0.5                            # 等覆盖层上屏（冻结帧绘制）
  xdotool mousemove "$X1" "$Y1" mousedown 1 mousemove "$X2" "$Y2" mouseup 1
  sleep 0.2                            # 等选区确认浮条渲染
  xdotool mousemove "$CLICK_X" "$CLICK_Y" click 1   # 复制 → confirm → 覆盖层隐藏
  sleep 0.6                            # 等确认→剪贴板落定、覆盖层退场
done
sleep 5                                  # 末轮收尾，再进统计（与 bench-synth.sh 同节奏）
echo | "$DIR/bench-latency.sh"
