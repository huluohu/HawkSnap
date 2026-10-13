#!/usr/bin/env bash
# scripts/check-size.sh — 安装包体积断言（G1: ≤15MB）
set -euo pipefail

# Cargo workspace 共享根 target/：用 cargo metadata 权威解析目标目录；
# brief 中的 app/src-tauri/target 仅作独立构建时的兜底路径。
# || true：pipefail 下无匹配时 ls 非零会导致赋值失败、脚本静默退出，必须吞掉。
TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | python3 -c "import sys,json;print(json.load(sys.stdin)['target_directory'])")
DMG=$(ls "$TARGET_DIR"/release/bundle/dmg/*.dmg 2>/dev/null | head -1 || true)
if [ -z "$DMG" ]; then
  DMG=$(ls app/src-tauri/target/release/bundle/dmg/*.dmg 2>/dev/null | head -1 || true)
fi
[ -z "$DMG" ] && { echo "未找到 dmg：请先在 app/ 目录运行 npm run tauri build"; exit 1; }
SIZE_MB=$(du -m "$DMG" | cut -f1)
echo "dmg = ${SIZE_MB}MB ($DMG)"
[ "$SIZE_MB" -le 15 ] || { echo "超过 15MB 预算"; exit 1; }
