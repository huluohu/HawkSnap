# HawkSnap

**像鹰一样准，一按就截。**

HawkSnap 是一款免费、纯本地的轻量级截屏工具，基于 Tauri 2 + Rust + React 构建——无账号、无云端、无遥测。

## 当前进度

M1 Plan A 已完成——F1 截图闭环：

- 常驻托盘菜单「截图 (F1)」或全局热键 F1 触发截图；
- 在冻结帧覆盖层上拖拽框选（Enter / 双击确认，Esc 取消）；
- 确认后图像进入剪贴板，并保存到 `~/Pictures/HawkSnap`。

配套：延迟打点与压测脚本 `scripts/bench-latency.sh` 已就位，性能数据（G3）待手动运行归档；安装包体积预算 ≤15MB，由 `scripts/check-size.sh` 断言。

## 后续计划

见 [docs/superpowers/plans/](docs/superpowers/plans/)（M1 剩余 Plan B / C / D 在前一计划验收归档后另立文档）。

## 开发环境

- macOS 12.3+
- Node.js 20+
- Rust stable

## 本地开发

```bash
cd app
npm install
npm run tauri dev
```

构建安装包并检查体积（产物在 Cargo workspace 根 `target/release/bundle/` 下）：

```bash
cd app && npm run tauri build
./scripts/check-size.sh
```

## 屏幕录制权限

首次截图时 macOS 会请求「屏幕录制」权限，需允许才能捕获屏幕内容；若未弹窗或截图失败，前往 系统设置 → 隐私与安全性 → 屏幕录制，为 HawkSnap（`tauri dev` 模式下为承载它的终端）开启权限，然后重启应用重试。

### F1 按键模式（媒体键兼容）

macOS 默认把 F1 当作亮度降低键（「将 F1、F2 等键用作标准功能键」未开启），此时按下 F1 不会发出普通按键事件。HawkSnap 会自动兼容这种模式，但需要二选一授权：

1. 在 系统设置 → 键盘 → 键盘快捷键 → 功能键 中开启「将 F1、F2 等键用作标准功能键」（F1 变回普通 F 键，走全局热键路径）；或
2. 保持默认（F1 = 亮度键），在 系统设置 → 隐私与安全性 → 辅助功能 中添加 HawkSnap（`tauri dev` 模式下为承载它的终端），HawkSnap 通过媒体键监听识别亮度键按下并触发截图（推荐，开箱即用，fn+F1 组合同样有效）。

两者都不满足时，应用启动会给出「提示：」横幅引导（不会静默失效）。

已知问题：macOS 14+ 上若未授予屏幕录制权限，抓屏可能静默返回仅壁纸画面而非报错（权限引导将在后续版本加入）。
