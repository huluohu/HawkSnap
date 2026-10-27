#!/usr/bin/env bash
# scripts/scrollshot-eval/run-eval.sh — 长截图拼接 eval 跑批（F10 自动验收层）。
#
# 用法：
#   scripts/scrollshot-eval/run-eval.sh            # 跑 20 场景（自动层）
#   scripts/scrollshot-eval/run-eval.sh --pages    # 顺带生成 20 个 HTML 长页面（人工层）
#
# 自动层做什么：
#   1. 按 scenes.json 的 20 个场景合成「长页面」像素内容（正文/图片/表格/代码/
#      聊天/固定头部/粘性侧栏/深色…）；
#   2. 复刻产品侧的**自适应采集间隔**（500ms 基准 / 200ms 加密）与滚动速度模型，
#      切出一串视口帧（= 用户滚动时真实会拍到的帧序列）；
#   3. 走**生产同一条**拼接路径（hawksnap_core::stitch::stitch）；
#   4. 判据：无重影（对齐率 ≥0.95）+ 输出 ≥20000px + 无内容缺口（漂移 ≤1%）。
#
# 人工层（控制器执行）：用 pages/*.html 在浏览器里真滚动、真按 ⌘S。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

SCENES="scripts/scrollshot-eval/scenes.json"
RUNNER="scrollshot_eval"

echo "[scrollshot-eval] ① 生成 20 个长页面（人工层素材）"
node scripts/scrollshot-eval/generate-pages.mjs

echo "[scrollshot-eval] ② 构建跑批器（release —— 匹配是重计算，debug 会慢一个量级）"
cargo build --release -p hawksnap-core --example "$RUNNER"

echo "[scrollshot-eval] ③ 跑 20 场景"
set +e
"./target/release/examples/$RUNNER" "$SCENES"
rc=$?
set -e

if [ $rc -ne 0 ]; then
  echo "[scrollshot-eval] ✗ 成功率未达 90% 门槛（退出码 $rc）" >&2
  exit $rc
fi
echo "[scrollshot-eval] ✓ 成功率达标（≥90%）"
echo "[scrollshot-eval] 人工层：浏览器打开 scripts/scrollshot-eval/pages/index.html → 选区 → 滚动 → ⌘S"
