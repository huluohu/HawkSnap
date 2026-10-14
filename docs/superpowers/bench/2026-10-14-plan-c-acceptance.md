# Plan C 验收数据（跨平台：macOS + Linux）

- 日期：2026-10-14
- 版本：v0.1.0-alpha.3（eb8d6ad + README 9855f51）
- 方法：macOS = `scripts/bench-synth.sh 20`（CGEvent HID 注入）；Linux = xdotool 注入（Xvfb :99 1920×1080，openbox，无 Retina scale=1.0），两者均为「F1 → 拖拽 → 复制」× 20 + `scripts/bench-latency.sh`

## G3 延迟（spec §6.1）

| 指标 | 目标 P90 | macOS（aarch64） | Linux（x86_64 / Xvfb-X11） |
|---|---|---|---|
| 热键→覆盖层 | ≤150ms | 105ms ✅（基线 459ms） | **73ms** ✅ |
| 确认→剪贴板 | ≤100ms | 6ms ✅（基线 141ms） | **1ms** ✅ |

macOS 分段（中位）：抓屏 62 / 建窗 0（常驻复用）/ 帧加载 30。
Linux 20/20 轮完整（守卫零告警），全链路（F1→抓屏→覆盖层→选区→确认→剪贴板）在 Xvfb 上实证工作。

## G1 安装包

| 平台 | 产物 | 体积 |
|---|---|---|
| macOS | dmg | 4.21 MiB ✅（≤15MB） |
| Linux | .deb | 4.67 MiB ✅（≤15MB；AppImage 81.5MB 为未压缩格式，不计门槛） |

## G2 常驻内存（新启动空载口径）

| 平台 | 目标 | 实测 |
|---|---|---|
| macOS（零窗口启动） | ≤80MB | **76.6MB** ✅（Plan B 为 100.6MB，主面板懒创建整改生效） |
| Linux（Xvfb） | ≤80MB | **54.7MB** ✅ |

## 跨平台 bring-up 记录（Ubuntu 24.04 x86_64，SSH 测试机）

- 首轮 `cargo check --workspace` **零错误**（cfg 门控 + stub 全部在岗）；单测全绿（app 35 / core 24 / platform 2，较 macOS 少的为 macos 门控内用例）
- 唯一修复：`examples/pin_smoke.rs` 补非 macOS 门控（examples 随 cargo test 构建）——commit `fix(platform): gate pin_smoke example body for non-macos compile`
- 全链路 E2E（Xvfb）：F1 热键 → 冻结帧覆盖层（选区/动作浮条渲染正确）→ 复制/保存 → PNG + 缩略图 + SQLite 历史行 ✓
- 已知 Linux 差异（stub，归 Plan D）：贴图 Pin、AX 悬停吸附、媒体键（Linux F2 原生可用无需媒体键路径）；Wayland 走 portal（spec §6.3）

## 备注

- macOS G2 口径说明：76.6MB 为零窗口启动（C6 主面板懒创建后）；覆盖层使用期间上涨、5 分钟空闲回收（Plan B 机制）
- Linux 基准脚本未入库（xdotool 版），如需固化可从本档案的注入序列重建：`xdotool key F1 → mousemove/mousedown/mousemove/mouseup → 计算「复制」按钮坐标 click`
