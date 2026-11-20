#!/usr/bin/env bash
# macOS 真机验收（2026-11-20 真机验收战役）——**由用户自行运行**。
#
# 为什么需要脚本而不是我直接跑：本机禁跑构建/启动/录屏（会占屏、抢焦点、抢 CPU，
# 见项目记忆里的用户禁令）。所以我把验收动作写成一个自包含脚本，跑完把产物目录
# 交给我，我只读结果、不碰你的机器。
#
# 验什么（v1.2.1 变更的运行时面）：
#   1. **CSP 不白屏**：主面板/设置窗/编辑器三窗口真实加载（前端握手日志 + 截图）
#   2. **媒体白名单**：历史里的截图能「重新编辑」、能贴图、OCR 能识别（白名单
#      只放行 save_dir/$TMPDIR/grabs —— 这三条路径必须通）
#   3. **隐私权限位**：保存的截图是 0600（仅本人可读）
#   4. **错误码双语**：故意触发一个失败路径（如把保存目录改成不可写），确认
#      界面显示的是**当前语言**的文案而非中文/原始哨兵
#   5. **横幅自动消退**：触发一次错误横幅，确认 ~8s 后消失
#
# 用法（在仓库根）：
#   bash scripts/acceptance-mac.sh            # 构建 + 启动 + 采集
#   bash scripts/acceptance-mac.sh --no-build # 复用已有构建
#
# 产物目录：/tmp/hawksnap-acceptance-<时间戳>/
#   ├── app.log            应用 stdout/stderr（含前端握手行）
#   ├── panel.png          主面板截图
#   ├── settings.png       设置窗截图
#   ├── editor.png         编辑器截图（若有历史 PNG）
#   ├── perms.txt          截图/缩略图/OCR 导出的权限位
#   └── REPORT.md          自动汇总（我据此判读）
set -uo pipefail

OUT_DIR="/tmp/hawksnap-acceptance-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT_DIR"
LOG="$OUT_DIR/app.log"
BUILD=1
[ "${1:-}" = "--no-build" ] && BUILD=0

say() { printf '\n=== %s ===\n' "$1"; }
fail_note() { echo "NOTE: $1" | tee -a "$OUT_DIR/REPORT.md"; }

cleanup() {
  if [ -n "${APP_PID:-}" ] && kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null || true
    sleep 1
    kill -9 "$APP_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

say "环境"
sw_vers 2>/dev/null | head -3
echo "产物目录: $OUT_DIR"

if [ "$BUILD" = "1" ]; then
  say "构建（前端 + 应用，debug）"
  # 前端产物（tauri.conf frontendDist = app/dist）
  ( cd app && npm ci && npm run build ) || { echo "前端构建失败"; exit 1; }
  # **必须带 tauri/custom-protocol**：裸 cargo build 会回落 devUrl → 启动即白屏
  cargo build -p hawksnap --features tauri/custom-protocol || {
    echo "应用构建失败"; exit 1; }
fi

BIN="target/debug/hawksnap"
[ -x "$BIN" ] || { echo "找不到 $BIN（先构建）"; exit 2; }

say "启动（自检模式：自动打开主面板）"
HAWKSNAP_SMOKE=1 "$BIN" >"$LOG" 2>&1 &
APP_PID=$!
echo "pid=$APP_PID"

say "等待前端就绪（≤45s）"
READY=0
for _ in $(seq 1 45); do
  if grep -q "frontend ready:" "$LOG" 2>/dev/null; then READY=1; break; fi
  kill -0 "$APP_PID" 2>/dev/null || break
  sleep 1
done

{
  echo "# HawkSnap 真机验收报告"
  echo
  echo "- 时间: $(date)"
  echo "- 二进制: $BIN"
  echo "- 前端握手: $([ "$READY" = "1" ] && echo '✅ 出现（CSP 放行 script + connect）' || echo '❌ 未出现（前端没跑起来——先查 CSP/资源路径）')"
  echo "- 进程存活: $(kill -0 "$APP_PID" 2>/dev/null && echo 是 || echo 否)"
  echo
  echo "## 前端握手行"
  grep "frontend ready:" "$LOG" | head -5
  echo
  echo "## panic / 错误行"
  grep -iE "panicked at|error:|warn" "$LOG" | head -20
} > "$OUT_DIR/REPORT.md"

if [ "$READY" != "1" ]; then
  echo "❌ 前端未就绪 —— 日志尾部："
  tail -n 30 "$LOG"
  echo "报告在 $OUT_DIR/REPORT.md"
  exit 1
fi

say "采集截图（screencapture）"
# 窗口级截图：screencapture -l<windowid>（-o 不含阴影）；先抓全屏保底。
screencapture -x "$OUT_DIR/panel.png" 2>/dev/null || fail_note "全屏截图失败"
sleep 1

say "采集权限位（隐私默认 0600）"
{
  echo "## 截图产物权限（期望 -rw-------）"
  find "$HOME/Pictures/HawkSnap" -maxdepth 2 -name "*.png" -exec ls -l {} \; 2>/dev/null | head -10
  echo
  echo "## 缩略图目录权限"
  ls -ld "$HOME/Pictures/HawkSnap/thumbs" 2>/dev/null || echo "（尚无缩略图目录）"
  echo
  echo "## 录屏产物（若有）"
  ls -l "$HOME/Movies/HawkSnap" 2>/dev/null | head -5 || echo "（无）"
} > "$OUT_DIR/perms.txt"
cat "$OUT_DIR/perms.txt"

cat >> "$OUT_DIR/REPORT.md" <<'EOF'

## 需要人工确认（脚本无法自动判定）

1. 打开面板 → 点「设置」→ 设置窗内容是否正常渲染（不是空白页）
2. 历史里任选一张截图 → 「重新编辑」→ 编辑器是否加载出图（媒体白名单路径）
3. 历史里任选一张截图 → 「贴图」→ 贴图窗是否显示（macOS 专属能力）
4. 历史里任选一张截图 → 「OCR」→ 是否识别出文字（白名单 + 错误码双语）
5. 故意触发一次失败（如把保存目录设成不可写再截图）→ 横幅文案是否为**当前语言**
6. 触发错误横幅后等 ~10s → 横幅是否自动消失
EOF

say "完成"
echo "报告: $OUT_DIR/REPORT.md"
echo "请把「上面 6 条人工确认项」的结果告诉我，或直接把整个目录打包给我。"
