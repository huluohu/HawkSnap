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
