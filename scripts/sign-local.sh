#!/usr/bin/env bash
# 用稳定的自签身份重签本地构建产物——TCC 授权（屏幕录制/辅助功能）随证书锚定跨构建持久，
# 不再出现「每次构建后权限失效」。身份生成 SOP 见 ~/.tauri/codesign/ 与 README 已知限制。
# 用法：构建后执行 scripts/sign-local.sh；换机需重新导入 sign3.p12 并 add-trusted-cert。
set -euo pipefail
IDENTITY="HawkSnapCodesign"
APP="target/release/bundle/macos/HawkSnap.app"
cd "$(dirname "$0")/.."
[ -d "$APP" ] || { echo "未找到 $APP，先 npm run tauri build"; exit 1; }
codesign --force --deep --sign "$IDENTITY" "$APP"
codesign -d -r- "$APP" 2>&1 | grep -o 'certificate leaf = H"[0-9a-f]*"' | head -1
echo "重签完成：TCC 授权随证书锚定，跨构建有效"
