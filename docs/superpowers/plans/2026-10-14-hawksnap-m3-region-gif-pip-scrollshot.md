# HawkSnap M3: 区域录制 / GIF / 画中画 / 按键显示 / 长截图 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 spec §11 M3 全量：区域/窗口录制（复用覆盖层选区）、GIF 导出（gifski）、摄像头画中画（录制中可调）、按键显示（keycast）、桌面长截图（滚动捕获+ORB 拼接）→ 发布 0.3.0。

**Architecture:** 区域录制 = F6 进入覆盖层**录制选区模式**（状态机扩展：capture-mode / record-mode 互斥，M2 终审的守卫模式复用）；GIF = gifski crate（帧序列→APNG/GIF，从录制管线旁路抽帧——录制时同步降采样攒帧）；画中画 = SCStream 第二路 camera 预览 + AVAssetWriter 画中画合成（层叠 layout：主画面全幅 + 圆角圆形相机层叠加）；keycast = CGEventTap 监听键盘（授权已含辅助功能）+ 悬浮条样式小窗；长截图 = 手动滚动模式（用户滚动，定时增量捕获同区域 → ORB 特征匹配 + 模板匹配找纵向偏移 → 拼接，spec §3.5 首发=手动模式）。

**Tech Stack:** 既有栈 + gifski（官方 crate，pkg-config 无需）、objc2-av-foundation（已在树）+ AVAssetWriter 多 input、CGEventTap（CoreGraphics extern，沿 ax.rs 模式）、orb-rs 或手写 ORB（裁定：**手写简化版特征匹配**——FAST 角点 + patch 相关性，避免重依赖；屏幕内容纹理丰富、纵向平移单一轴，简化版可满足 F10）。

**Spec:** `docs/superpowers/specs/2026-10-13-hawksnap-design.md`（§3.4 录屏进阶 P2 部分：GIF/画中画/keycast；§3.5 桌面长截图；§3.1 区域/窗口录制补全；验收 F8/F9/F10）

## Global Constraints（继承 + M3 新增）

- G1 ≤15MB（gifski 静态链 ~2MB，余量足）；G2 ≤80MB 不录屏时；F7 录屏 CPU ≤8%（画中画合成增量 ≤2%）；F8 GIF：800×600@15fps、10s 素材导出 ≤10s；F9 画中画录制中可调位置/大小且主画面无掉帧（抽帧检查）；F10 长截图 20 场景拼接成功率 ≥90%、输出 ≥20000px。
- i18n 双语全 key（G6）；Rust 错误文案中文（已裁定）；无遥测。
- M2 终审继承项**本计划内消化**（不另立批任务，逐任务携带）：①区域录制自带 REC/capture 互斥（M2 守卫模式）；②录制 kind 列 + prune 平权 + duration 列（schema user_version 3）；③per-pin rgba 上限（贴图 ≥4K 图拒绝/降采样）；④editor crop 工具（编辑器 10/10）；⑤ORT hash 缓存（逐调用 −200-400ms）；⑥record_start 并发败者不关胜者 bar；⑦托盘菜单动态重建；⑧perf CSV 门控（bench/G4 外不写）。
- 每任务全绿 + conventional commit；共享工作树纪律；G5 soak 运行中——**不 kill 进程、不碰 ~/Movies/HawkSnap/soak-*.csv**。

---

### Task N0: 继承项批处理（M2 终审清单）

**Files:**
- Modify: `history.rs`（shots 加 `duration` 列 + kind 语义修正：record_history 增 kind 参数，recording.rs 传 "recording"；prune 对所有 kind 平权；user_version → 3 迁移）
- Modify: `recording.rs`（record_start 并发败者路径移除 close_recording_bar）
- Modify: `lib.rs`（托盘菜单动态重建：settings.hotkey 变更 → 重建菜单，抽 `build_tray_menu(app, &settings) -> Result<Menu>` 复用 setup 与 set_settings 两处）
- Modify: `commands.rs`（perf mark 门控：非 bench/G4 环境不写 CSV——`perf::set_enabled(bool)`，bench_mode/g4 setup 时置 true，其余静默 no-op）
- Modify: `ocr.rs`（模型/ORT 文件 SHA256 缓存：OnceLock<(path, mtime, hash)>，mtime 变化才重算）
- Modify: `editor/`（crop 工具：选中态 crop 应用 = 裁位图 + 全形状平移裁剪 + 重置撤销栈；第 10 工具达成）
- Modify: `pins.rs`（pin_from_path / pin_latest 加尺寸上限：>4096px 边长降采样到 4096 后建 pin，注释记预算）

- [ ] Step 1: 逐项实现（history 迁移带 user_version 2→3 测试；crop 带 translate-shapes 测试）
- [ ] Step 2: 全绿（cargo 151+ / npm 120+）+ Commit: `refactor: m3 inherited items — kind column, tray rebuild, perf gate, hash cache, crop tool, pin cap`

---

### Task N1: 录制选区模式（区域/窗口录制）

**Files:**
- Modify: `overlay.rs`（覆盖层状态机：`CAPTURE_MODE`/`RECORD_MODE` 互斥枚举 CaptureIntent；F6 → record 模式启动覆盖层（选区 UI 复用，提示文案不同）；确认 → `record_start_region(rect)` 而非 finish_capture；Esc 退出；跨显示语义=只录选区所在显示器）
- Modify: `recording.rs`/`record.rs`（`ScreenRecorder::start_region(display_id, rect_px)`：SCContentFilter `initWithDisplay_excludingApplications_exceptingWindows` + rect 裁剪参数——SCStream 捕获全屏然后裁？NO：SCStreamConfiguration 支持 `sourceRect`/`destinationRect`——用 sourceRect 裁剪，帧尺寸=选区尺寸，CPU 随面积下降）
- Modify: 悬浮条（区域录制同款 bar；标题显示选区尺寸）

**Interfaces:**
- Produces: F6 完整语义 = 全屏（无选区直接录）→ 现状保留？裁定：**F6 统一进入选区模式**（Snipaste 语义一致），Esc=取消、Enter/双击=全屏快捷确认（选区=全屏）、拖选=区域；`record_start_region` 与 `record_start`（全屏，保留给 API 兼容）并存。

- [ ] Step 1: 状态机互斥失败测试（capture 模式下 F6 → 拒绝横幅【M2 已有】，record 模式下 F1 → 拒绝【M2 已有】——补 record 模式枚举纯测试）→ record.rs sourceRect 参数化测试（构造断言）。
- [ ] Step 2: 实现（overlay 模式 + record.rs region 参数 + bar 尺寸显示）。
- [ ] Step 3: 真录制 #[ignore]：区域 800×600 录 3s → 帧尺寸断言（mvhd + 帧宽高）→ CPU 采样（面积减半 → CPU 应 <3%）。
- [ ] Step 4: 全绿 + Commit: `feat(record): region selection mode with source-rect capture`

---

### Task N2: 录制帧抽取 → GIF（gifski）

**Files:**
- Create: `crates/hawksnap-gif/` 或 app 内 `gif/` 模块（裁定：app 内模块，gifski 直接依赖）
- Modify: `recording.rs`/`record.rs`（录制会话可选 `gif: bool`——**旁路帧抽取**：SCStream 视频回调处按 15fps 抽帧降采样（最长边 800）→ RGBA 帧队列（有界 32MB，超出丢最旧）→ 停止后异步 gifski 编码）；命令 `record_start(opts)` 增 format: "mp4"|"gif"；停止时 gif 路径 → gifski::new + add_rgba_frame × N → write 到 `~/Movies/HawkSnap/<ts>.gif`
- Modify: 悬浮条（录制模式选择：MP4/GIF 二选一切换按钮）

**Interfaces:**
- Produces: F8 验收 = 800×600@15fps、10s 素材导出 ≤10s（编码计时断言在 #[ignore] 测试）。

- [ ] Step 1: 帧抽取队列纯测试（有界丢弃语义）→ 实现；gifski 编码 #[ignore]（合成 150 帧 → 计时 ≤10s + 输出 GIF 头校验）。
- [ ] Step 2: record pipeline 接线（mp4/gif 双路互斥）；悬浮条 UI + i18n。
- [ ] Step 3: 全绿 + Commit: `feat(gif): optional gif capture lane via gifski with bounded frame queue`

---

### Task N3: 摄像头画中画（PiP）

**Files:**
- Modify: `record.rs`（AVCaptureSession 相机采集（objc2-av-foundation AVCaptureDeviceDiscoverySession + AVCaptureVideoDataOutput）→ CVPixelBuffer 回调 → **PiP 合成层**：主画面帧 + 相机帧（圆形裁剪 + 描边）合成到 adaptor 输入——合成在 CPU 侧（draw 到 CGBitmapContext）或 GPU；M3 裁定 CPU 合成（帧率 30fps 下 CoreGraphics 绘制圆角贴片 ~2-4ms，预算内）；PiP 状态 IPC：`hawk://pip-state` {x,y,size,visible}）
- Modify: `recording.rs`（PiP 默认右下角 240px；命令 `pip_update(x,y,size,visible)`——录制中经 MUTEX 更新共享 PiPState，下一帧生效=「可调」）；悬浮条 PiP 开关
- Modify: Info.plist NSCameraUsageDescription（tauri.conf infoPlist 路径——Plan D 坑典条目：该键是路径不是 map；建 desktop/Info.plist 合并）；TCC 相机权限预检 + 拒绝时优雅降级（无 PiP 继续录）

**Interfaces:**
- Produces: F9 = 录制中调位置/大小（下一帧生效 ≤33ms）+ 主画面无掉帧（抽帧：录制 10s，统计视频帧时间戳间隔连续性）。

- [ ] Step 1: PiPState 纯测试（默认位/边界 clamp/不可见）→ 相机采集（#[ignore]：无相机 CI skip；本机 FaceTime HD 实测帧回调计数）。
- [ ] Step 2: 合成 + pip_update 命令 + 悬浮条开关 + 权限降级。
- [ ] Step 3: 真录制 #[ignore]：PiP on 录 10s → 抽帧检查 PiP 可见 + 帧间隔连续（F9 抽帧检查脚本化：mvhd/逐帧 pts dump）。
- [ ] Step 4: 全绿 + Commit: `feat(record): camera picture-in-picture with live position control`

---

### Task N4: 按键显示（keycast）

**Files:**
- Create: `app/src-tauri/src/keycast.rs`（CGEventTap listen-only keyboard tap（session 模式，辅助功能授权——已有门槛；未授权 → 优雅禁用）→ 按键序列化（修饰符+键名）→ 事件 `hawk://keycast` {combo, ts}；节流 100ms 合并）
- Create: `app/src/windows/keycast/`（小悬浮窗 320×88：大字显示最近组合键，1.5s 淡出——CSS 动画；仅录屏进行中显示【F9 联动：keycast 可选开关在悬浮条】）
- Modify: recording.rs（keycast 开关 → keycast tap 启停；bar 按钮三态）

**Interfaces:**
- Produces: 按键显示跟随录屏开关；tap 泄漏守卫（stop 后 tap 失效——CGEventTap 用完 disable）。

- [ ] Step 1: 键名映射纯测试（keycode→显示名，修饰符顺序 Cmd⇧⌥⌃）→ tap 实现（#[ignore] 真实按键无法脚本注入——SE 注入测试按键探针计数）。
- [ ] Step 2: keycast 窗口 + 悬浮条开关 + i18n。
- [ ] Step 3: 全绿 + Commit: `feat(keycast): keyboard display overlay during recording`

---

### Task N5: 长截图（滚动捕获 + 拼接）

**Files:**
- Create: `app/src-tauri/src/scrollshot.rs`（会话：start(region) → 定时抓取该区域（SCStream 短会话或 CGWindowListCreateImage 定时——裁定：SCScrollWindowImage 不可用，用 **CGWindowListCreateImage 定时 500ms** 简单可靠）→ 帧队列；stop(region_frames) → 拼接）
- Create: `app/src/windows/scrollshot/`（长截图模式 UI：覆盖层选区 → 顶部固定提示条「滚动页面…完成按 ⌘S」→ 帧进度 N → 完成）
- Create: `app/src-tauri/src/stitch.rs`（拼接：相邻帧纵向重叠匹配——FAST 角点（手写，阈值响应）+ 32×32 patch 相关性匹配取纵向偏移中位数（多列采样鲁棒）→ 偏移序列 → 拼接画布（取首帧宽，累计高度）→ 重叠区渐变融合（8px 线性 alpha））
- Modify: history（kind="scrollshot" 入库）

**Interfaces:**
- Produces: F10 验收 = 20 标准滚动场景（网页/文档/聊天）拼接成功率 ≥90%（≥18/20）、无重影、输出 ≥20000px。**验收脚本**：`scripts/scrollshot-eval.sh`（准备 20 个测试页面——本地 HTML 长页 ×20 种布局，CXPredict 不可用→人工滚动采样脚本记录，控制器执行）。
- 诚实边界：固定头部/侧栏（spec 的 ORB+模板匹配处理项）→ 简化版处理：多列匹配天然容忍固定头部（头部区域无新特征，偏移由内容区主导）——记录局限，复杂场景失败率在 eval 中量化。

- [ ] Step 1: stitch.rs 纯测试（合成平移图对：偏移恢复精度 ±2px；纯色图无特征 → 拒绝拼接错误）→ FAST+匹配实现。
- [ ] Step 2: 采集会话 + UI + 拼接输出（PNG 落 save_dir + history）。
- [ ] Step 3: eval 脚本 + 人工 20 场景首跑（控制器；记录成功率）。
- [ ] Step 4: 全绿 + Commit: `feat(scrollshot): rolling capture with fast-based stitching`

---

### Task N6: M3 收尾（验收 + 0.3.0）

- `scripts/` 验收采集（F8 GIF 计时、F9 抽帧、F10 eval 成功率、G1-G4 复测、F7 复测含 PiP 增量）；README（M3 清单+已知限制更新：长截图简化匹配局限、keycast 需辅助功能）；版本 0.3.0 四处；归档 docs/superpowers/bench/2026-10-14-m3-acceptance.md；tag v0.3.0。
- Commit: `chore: m3 closeout and release 0.3.0`

## 自审记录

1. **Spec 覆盖**：§3.4 进阶 P2 → N2(GIF)/N3(画中画)/N4(keycast)；§3.5 桌面 → N5（浏览器插件仍 M4）；§3.1 区域/窗口录制 → N1（**窗口录制**未含——裁定：区域录制覆盖 80% 场景，窗口录制依赖 CGWindowID 级 filter，列 M4 评估，README 注明）；F8/F9/F10 → N2/N3/N5。**G5 继承**：soak 已含 M2 录屏；M3 录制改动后 soak 脚本重跑一轮 24h 短版。
2. **占位符检查**：N5 FAST/匹配为手写简化（预授权，测试契约完整）；N3 CPU 合成预授权；eval 20 场景为控制器人工步骤。
3. **类型一致**：CaptureIntent/EditorOrigin 复用既有模式；PiPState/KeycastEvent 单处定义；ShotRow 增 duration 字段向后兼容（serde default）。
