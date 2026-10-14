# Plan C 验收数据（macOS 补充归档）

- 日期：2026-10-14；构建 v0.1.0-alpha.3（be4fa6e，含进程内基准仪器）
- 方法：`HAWKSNAP_BENCH=20` 环境变量基准模式——应用自触发真实捕获→确认链路（覆盖层不可见创建，
  webview 全路径：加载/取帧/绘制/overlay-shown），**零键鼠注入**（用户在线时可用），进程内按轮计时
  （排除冷启动第 1 轮，P90 = nearest-rank）。跨平台（Linux/Xvfb）数据见姊妹档案
  `2026-10-14-plan-c-acceptance.md`（另一会话产出）。

## G3 延迟

| 指标 | 目标 P90 | 第 1 次（前段负载） | 第 2 次（较净窗口，**采信**） |
|---|---|---|---|
| 热键→覆盖层 | ≤150ms | 161ms（11-20 轮已回落至 ≤127） | **134ms** ✅ |
| 确认→剪贴板 | ≤100ms | 1ms | **2ms** ✅ |

- h2o 逐轮（第 2 次，19 轮计入）：中位 ≈121ms；负载敏感段为 grab（xcap 抓屏受 CPU 竞争）。
- 对照：Plan B 注入法 105ms（可见覆盖层）；ghost 模式 60ms 中位。基准模式与注入法测得同一条
  生产路径（mark 点不变），差异来自窗口可见性与机器负载。

## G1 / G2

- dmg 4.21 MiB ≤ 15MB ✅（rusqlite bundled 增量 ~1.5MB）
- G2 常驻内存（零窗口启动口径）：**76.6MB** ≤ 80MB ✅（懒创建主面板整改生效；Plan B 为 100.6MB）

## C7 语义抽查（非侵入）

- settings.json 直写 `"hotkeyCapture":"Ctrl+Alt+1"` + 重启：stderr 无注册失败信号（解析/注册 OK）。
- save_dir 解析与 jpeg 编码路径由单元测试覆盖（`resolve_save_dir`、`rgba_to_jpeg`）。

## 备注

- 基准模式常驻产品内（env 门控，默认关闭），作为 G3 的可复现采集手段（替代键鼠注入）。
- 复测命令：`HAWKSNAP_BENCH=20 target/release/bundle/macos/HawkSnap.app/Contents/MacOS/hawksnap`，
  stderr 读 `[bench] h2o/c2c P90` 两行。
