#!/usr/bin/env bash
# 真机自检（2026-11-20 真机验收战役）。
#
# **验什么**：HawkSnap 零窗口架构下启动只起托盘，「前端到底跑起来没有、真实素材
# 到底加载出来没有」在无人值守环境里无从观察；而 v1.2.1 新增的 CSP 与媒体白名单
# 正是「代码全绿但运行时空白/被拒」的高风险面。本脚本以 HAWKSNAP_SMOKE=1 启动
# （该模式自动造素材 + 跑白名单断言 + 打开编辑器与主面板 + 等前端回报），然后
# 读应用日志里的自检汇总行与**进程退出码**（0 = 全过，1 = 有断言失败）。
#
# 覆盖的断言（应用侧汇总行 `smoke summary:`）：
#   frontend=true     → CSP 放行 script-src + connect-src（前端跑起来且 IPC 通）
#   asset=Some(true)  → **asset 协议 img-src 运行时实证**（真实缩略图 onload）
#   editor=true       → 第二个窗口（编辑器）的 CSP + 白名单装载链路通
#   allow_ok=true     → 白名单：应用落盘根内的路径被放行
#   allow_blocked=true→ 白名单：应用落盘根外的路径被拒
#
# **在哪跑**：CI 三平台真机 runner（macOS/Windows/Linux 真机）；用户本机亦可。
#
# 用法：
#   bash scripts/smoke-launch.sh [二进制路径]
# 默认 target/debug/hawksnap（release 产物同理：target/release/hawksnap）。
set -uo pipefail

BIN="${1:-}"
if [ -z "$BIN" ]; then
  for cand in target/debug/hawksnap target/debug/hawksnap.exe \
              target/release/hawksnap target/release/hawksnap.exe; do
    if [ -x "$cand" ]; then BIN="$cand"; break; fi
  done
fi
if [ -z "$BIN" ]; then
  echo "[smoke] 找不到可执行文件（先构建：cargo build -p hawksnap --features tauri/custom-protocol）" >&2
  exit 2
fi

LOG="${SMOKE_LOG:-$(mktemp "${TMPDIR:-/tmp}/hawksnap-smoke.XXXXXX.log")}"
SHOT="${SMOKE_SHOT:-}"
echo "[smoke] 二进制: $BIN"
echo "[smoke] 日志:   $LOG"

# Linux：无 X 会话时 wry 无法建窗口 → 必须 Xvfb；托盘/appindicator 需要会话总线
# → dbus-run-session 兜一层（缺 dbus 时 setup 的 tray build 会失败 → 应用退出）。
if [ "${RUNNER_OS:-}" = "Linux" ] || [ "$(uname -s)" = "Linux" ]; then
  echo "[smoke] 启动（xvfb + dbus 会话）"
  HAWKSNAP_SMOKE=1 dbus-run-session -- xvfb-run -a "$BIN" >"$LOG" 2>&1 &
elif [ "${RUNNER_OS:-}" = "Windows" ]; then
  echo "[smoke] 启动（Windows）"
  HAWKSNAP_SMOKE=1 "$BIN" >"$LOG" 2>&1 &
else
  echo "[smoke] 启动（macOS）"
  HAWKSNAP_SMOKE=1 "$BIN" >"$LOG" 2>&1 &
fi
PID=$!

# 等自检完成（应用自检结束后自行退出；上限 90s 兜底）。
EXIT_CODE=""
SHOTTEN=0
for _ in $(seq 1 90); do
  # 取证时机：应用**还活着**时截（主面板是临时窗口，应用一退就没了）。
  if [ "$SHOTTEN" = "0" ] && [ -n "$SHOT" ] && grep -q "smoke summary:" "$LOG" 2>/dev/null; then
    SHOTTEN=1
    if [ "${RUNNER_OS:-}" = "macOS" ] || [ "$(uname -s)" = "Darwin" ]; then
      # 提到前台（Accessory 策略的进程在 runner 上可能被 Finder 盖住）。
      osascript -e 'tell application "System Events" to set frontmost of (first process whose name is "hawksnap") to true' >/dev/null 2>&1 || true
      sleep 1
      screencapture -x "$SHOT" >/dev/null 2>&1 && echo "[smoke] 截图: $SHOT" || \
        echo "[smoke] 截图失败（不计入判定）"
    fi
  fi
  if ! kill -0 "$PID" 2>/dev/null; then
    wait "$PID" 2>/dev/null
    EXIT_CODE=$?
    break
  fi
  sleep 1
done

if [ -z "$EXIT_CODE" ]; then
  echo "[smoke] 失败：应用未在时限内退出（自检卡住）" >&2
  tail -n 40 "$LOG" >&2
  # 收尾：别把进程留在 runner 上
  if [ "${RUNNER_OS:-}" = "Windows" ]; then
    taskkill //F //IM hawksnap.exe >/dev/null 2>&1 || true
  else
    pkill -f "hawksnap" >/dev/null 2>&1 || true
  fi
  exit 1
fi

SUMMARY="$(grep -m1 'smoke summary:' "$LOG" || true)"
echo "[smoke] 汇总: ${SUMMARY:-（无汇总行）}"
echo "[smoke] 握手: $(grep -m1 'frontend ready:' "$LOG" || echo '（无）')"
echo "[smoke] 资产: $(grep -m1 'asset probe:' "$LOG" || echo '（无）')"
echo "[smoke] 退出码: $EXIT_CODE"

if [ -n "$SHOT" ] && [ "$SHOTTEN" = "0" ]; then
  echo "[smoke] 未在应用存活期完成截图（不计入判定）"
fi

if [ "$EXIT_CODE" != "0" ]; then
  echo "[smoke] 失败：自检未全过（退出码 $EXIT_CODE）" >&2
  tail -n 40 "$LOG" >&2
  exit 1
fi
echo "[smoke] 通过：前端/资产/编辑器/白名单四项断言全过"
