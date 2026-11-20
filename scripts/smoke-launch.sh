#!/usr/bin/env bash
# 真机启动冒烟（2026-11-20 真机验收战役）。
#
# **验什么**：HawkSnap 零窗口架构下启动只起托盘，主面板要用户点托盘才建——于是
# 「应用能不能真的把前端跑起来」这条链路在无人值守环境里无从观察。本脚本：
#   1. 以 HAWKSNAP_SMOKE=1 启动（该变量让应用自打开主面板）；
#   2. 等前端握手日志 `[hawksnap] frontend ready: main` 出现——**这一行同时证明**：
#      CSP 放行了 script-src（bundle 执行了）且 connect-src 放行了 IPC（invoke
#      往返成功）；窗口 webview 真实加载成功；
#   3. 判定进程仍存活（没崩）且日志无 panic；
#   4. 收尾（杀进程）。
#
# **在哪跑**：CI 三平台真机 runner（macOS/Windows/Linux 真机）；用户本机亦可
# （本机跑构建/启动会占屏——**由用户自行决定何时跑**，我不代跑）。
#
# 用法：
#   bash scripts/smoke-launch.sh [二进制路径]
# 默认 target/debug/hawksnap（release 产物同理：target/release/hawksnap）。
set -uo pipefail

BIN="${1:-}"
if [ -z "$BIN" ]; then
  if [ -x "target/debug/hawksnap" ]; then
    BIN="target/debug/hawksnap"
  elif [ -x "target/debug/hawksnap.exe" ]; then
    BIN="target/debug/hawksnap.exe"
  elif [ -x "target/release/hawksnap" ]; then
    BIN="target/release/hawksnap"
  elif [ -x "target/release/hawksnap.exe" ]; then
    BIN="target/release/hawksnap.exe"
  else
    echo "[smoke] 找不到可执行文件（先构建：cargo build -p hawksnap --features tauri/custom-protocol）" >&2
    exit 2
  fi
fi

LOG="$(mktemp "${TMPDIR:-/tmp}/hawksnap-smoke.XXXXXX.log")"
echo "[smoke] 二进制: $BIN"
echo "[smoke] 日志:   $LOG"

cleanup() {
  if [ "${RUNNER_OS:-}" = "Windows" ]; then
    taskkill //F //IM hawksnap.exe >/dev/null 2>&1 || true
  else
    pkill -f "hawksnap" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

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

# 等握手（最多 45s：webview 冷启动 + 面板建窗 + 前端挂载）。
READY=""
for _ in $(seq 1 45); do
  if grep -q "frontend ready:" "$LOG" 2>/dev/null; then
    READY="yes"
    break
  fi
  if ! kill -0 "$PID" 2>/dev/null; then
    break
  fi
  sleep 1
done

FAILED=""
if [ -z "$READY" ]; then
  FAILED="前端未就绪（无 frontend ready 日志）"
fi
if ! kill -0 "$PID" 2>/dev/null; then
  FAILED="${FAILED:+$FAILED; }进程已退出"
fi
if grep -q "panicked at" "$LOG" 2>/dev/null; then
  FAILED="${FAILED:+$FAILED; }日志含 panic"
fi

if [ -n "$FAILED" ]; then
  echo "[smoke] 失败：$FAILED" >&2
  echo "[smoke] ---- 日志尾部 ----" >&2
  tail -n 40 "$LOG" >&2
  exit 1
fi

echo "[smoke] 通过：前端已加载并完成 IPC 握手，进程存活，无 panic"
echo "[smoke] 握手行: $(grep -m1 'frontend ready:' "$LOG")"
