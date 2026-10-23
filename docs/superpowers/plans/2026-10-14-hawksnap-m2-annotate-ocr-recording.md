# HawkSnap M2: 标注编辑器 / OCR / 取色标尺 / 基础录屏 / 历史完整版 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 spec §11 M2 全量：标注编辑器（自研矢量层，10 类工具+撤销重做）、OCR（RapidOCR 本地推理）、取色器+像素标尺、基础录屏（MP4 硬编+音频+点击高亮）、历史完整版（FTS5 搜索+保留策略 UI+重新编辑/贴图/打开目录）→ 发布 0.2.0。

**Architecture:** 编辑器 = Canvas 位图底 + 自研矢量标注层（React state 管理形状数组，命中测试自研，导出栅格化——spec ADR 2，拒绝 Fabric.js）；OCR = RapidOCR ONNX 模型（onnxruntime-node 不引入——改为 **Rust 侧 ort crate** 推理，模型按需下载校验 SHA256，首次使用引导）；录屏 = macOS ScreenCaptureKit 经 **screencapturekit-rs**（或 objc2 手写 SCStream——按锁定绑定可用性裁定），AudioUnit 采集混音，VideoToolbox 硬编（objc2 绑定），MP4 分段写盘；历史 UI = 主面板新增历史分区（ShotRow 契约已就绪）。

**Tech Stack:** 既有栈 + ort（ONNX Runtime）+ objc2-screen-capture-kit / objc2-core-media / objc2-audio-toolkit（按实际可用绑定裁定）。

**Spec:** `docs/superpowers/specs/2026-10-13-hawksnap-design.md`（§3.3 编辑器+OCR、§3.4 录屏基础、§3.6 历史完整、§3.1 取色器/标尺；验收 F5/F6/F7/F12 + M1 遗留 G5 启测）

## Global Constraints（继承 + M2 新增）

- 包体 ≤15MB **不含 OCR 模型**（按需下载 ~12MB，下载器校验 SHA256 + 断点续传）；常驻内存 ≤80MB **不录屏时**（录屏运行时预算另计：F7 CPU ≤8%）。
- 隐私：OCR 模型下载是唯一新外联（spec 明示可配置）；无遥测。
- 编辑器导出必须与预览像素一致（F5）——导出走「矢量层重绘到离屏 canvas」单一路径，禁止预览/导出两套绘制代码。
- 录屏（G5 启测：交付后运行 72h 稳定性脚本，崩溃=0、内存增长 <10%——测试由控制器在专用时段执行）。
- 撤销/重做无步数限制（会话内）——内存换正确性，形状对象轻量。
- i18n：全部新 UI 文案 key 化双语（G6）；Rust 错误文案仍中文（已裁定）。
- 依赖下载类（ONNX Runtime 二进制）经 crates.io/官方 release URL + SHA256 校验；不引入 git 依赖。
- 每任务全绿 + conventional commit；共享工作树纪律（提交前查 status/log）。
- **范围裁定**：编辑器入口 = 贴图双击 + 历史重新编辑 + 截图确认后「标注」按钮启用；录屏 = F6 启停 + 悬浮条（计时/暂停恢复/停止），点击高亮涟漪。

---

### Task M0: 历史完整版（FTS5 + 搜索 UI + 操作）

**Files:**
- Modify: `app/src-tauri/src/history.rs`（FTS5 虚表 + 触发器同步 + `search(query, limit)` + `list` 支持 offset 分页 + open_in_folder/delete 级联）
- Modify: `app/src-tauri/src/panel.rs`（`open_path(path)` 命令——`open`/`explorer`/`xdg-open` 按平台）
- Modify: `app/src/App.tsx`（历史分区 UI：搜索框+缩略图网格+右键/按钮操作[重新编辑/贴图/打开目录/删除]）
- Modify: `crates/hawksnap-platform-mac/src/pin.rs`→复用（贴图 from history path：读 PNG→rgba→PinWindow）

**Interfaces:**
- Produces: `history_search(query) -> Vec<ShotRow>`（FTS5 MATCH，OCR 文本列——本任务先索引文件名+应用名，OCR 文本列在 M2-OCR 任务补）；`open_path(path)`；`pin_from_path(path) -> Result<u64>`。FTS5 查询 ≤200ms/500 条（F12）。
- 搜索输入防抖 250ms；删除带确认。

- [ ] Step 1: 失败测试（history.rs：FTS 建表幂等、search 命中/未命中、中文分词默认 unicode61——记录分词局限）；实现；lib.rs 注册。
- [ ] Step 2: App.tsx 历史分区（懒加载：面板打开时 history_list(50)）；操作接线。
- [ ] Step 3: 全绿 + Commit: `feat(history): fts5 search, grid ui, open/re-pin/delete actions`

### Task M1: 编辑器骨架（位图底 + 矢量层 + 撤销重做）

**Files:**
- Create: `app/src/windows/editor/`（index.tsx / shapes.ts / history.ts / EditorCanvas.tsx）+ `?window=editor` 路由 + Rust `editor_open(png_path)` 命令（读图→dataUrl 传前端→建窗 900×640）
- Interfaces: `Shape` 判别联合（rect/ellipse/arrow/pen/highlighter/text/mosaic/step/delete-region/crop 起步 6 类实做：rect/ellipse/arrow/pen/highlighter/text）；`useEditorHistory` 撤销栈（无步数限制）+ redo；工具栏（颜色/粗细/字号记忆——localStorage）。

- [ ] Step 1: shapes.ts 纯函数（创建/包围盒/命中测试/边界变换）+ 失败测试；实现。
- [ ] Step 2: 编辑器骨架：位图底绘制 + 矢量层实时预览（pointer 事件→shape draft→commit）+ 工具栏 + 撤销重做（⌘Z/⌘⇧Z）。
- [ ] Step 3: 全绿 + Commit: `feat(editor): bitmap+vector editor skeleton with unlimited undo`

### Task M2: 编辑器工具全集 + 导出一致性

**Files:**
- Modify: editor/*（补齐 mosaic[像素化方块]/step[序号气泡]/text 编辑态/选中态[移动/删除/样式改]）；`export_image`（离屏 canvas 重绘全形状→PNG，**与预览共用同一 drawShape 函数**）
- Interfaces: `drawShape(ctx, shape)` 为预览与导出唯一路径（F5 约束）；导出按钮 → 写回原路径（或另存对话框——裁定：覆盖原图 + ⌘S 提示）。

- [ ] Step 1: drawShape 抽取 + 导出往返测试（canvas mock 下几何断言 + 真实浏览器人工验收）；工具实现。
- [ ] Step 2: 选中/移动/删除/样式修改；马赛克 = 底图像素化块。
- [ ] Step 3: 全绿 + Commit: `feat(editor): full tool set with single-path rasterized export`

### Task M3: 贴图双击进编辑器 + 截图标注入口

**Files:**
- Modify: pin.rs（双击 → editor_open(临时 PNG)——贴图当前帧落盘）；overlay/index.tsx（「标注」按钮启用 → finish(copy=false,save=false)+直接把裁剪 rgba 传编辑器——裁定：标注中截图先临时存盘再编辑，确认后覆盖剪贴板）

**Interfaces:** 编辑器保存后：来源=贴图 → 更新贴图窗口图；来源=截图 → 自动复制到剪贴板。

- [ ] Step 1: 两入口接线 + i18n + 全绿。
- [ ] Step 2: Commit: `feat(editor): wire pin double-click and capture annotate entry`

### Task M4: OCR（RapidOCR + 模型下载器）

**Files:**
- Create: `crates/hawksnap-ocr/`（ort 推理：det+cls+rec 三模型流水线；模型管理：`models_dir` 检查 + SHA256 校验 + 断点下载器；文本框后处理[det 多边形→框，rec CTC 解码]）
- Modify: commands.rs（`ocr_recognize(png_path) -> Vec<OcrLine{box,text,score}>`）；编辑器工具栏 OCR 按钮 → 结果浮层（单块复制/全部复制/导出 txt——spec §3.3）
- Modify: history.rs（shots 表加 ocr_text 列 + FTS 索引——M0 建列，此任务填充）

**Interfaces:**
- Produces: 模型 URL（rapidocr 官方 release，ppocrv4 det/cls/rec，~12MB 总）+ SHA256 常量（实现者从官方 release 校验后写入，报告注明来源）；首次 OCR → 下载进度事件 → 就绪。
- 1080p 区域识别 ≤1.5s（F6）——ort CPU ep 起步。

- [ ] Step 1: 模型下载器（SHA256 + 续传）失败测试 → 实现。
- [ ] Step 2: 推理流水线（fixture 小图 + 小模型单测——CI 无模型则 ignore+本地跑）。
- [ ] Step 3: 编辑器接线 + 结果浮层 + 历史列填充；全绿。
- [ ] Step 4: Commit: `feat(ocr): rapidocr pipeline with model downloader and result overlay`

### Task M5: 取色器 + 像素标尺

**Files:**
- Modify: overlay/index.tsx（按 C 拾色：从冻结帧读像素 → HEX/RGB 浮签+复制；标尺=选区边缘实时尺寸[已有]增强为边缘像素刻度线）
- Interfaces: 冻结帧 ImageData 已在前端（物理像素）——直接索引，零 IPC。

- [ ] Step 1: 取色 + 浮签（i18n）+ 刻度线增强；Step 2: 全绿；Commit: `feat(overlay): color picker with hex/rgb copy and pixel ruler`

### Task M6: 基础录屏（macOS SCStream + VT 硬编 + 音频混音）

**Files:**
- Create: `crates/hawksnap-platform-mac/src/record.rs`（SCStream 区域捕获 → CVPixelBuffer → VideoToolbox H264 硬编 → MP4 分段；SCStreamDelegate 回调线程纪律；麦克风+系统声[SCStream 自带系统声]独立音量混音——AVAudioEngine 或 SCStream audio 通道）
- Create: `app/src-tauri/src/recording.rs`（状态机 idle/recording/paused；命令 record_start(rect, opts)/record_pause/record_resume/record_stop→MP4 路径；悬浮条窗口 label `recording-bar`[非透明，常驻模式]）
- Modify: overlay 复用选区（F6 → 覆盖层选区模式 → 确认后 start）；`?window=recording-bar` 前端（计时/暂停恢复/停止/音量双滑杆）；G5 启动：`scripts/soak-record.sh`（72h 计划脚本，控制器执行）

**Interfaces:**
- 录屏文件默认 `~/Movies/HawkSnap/<date>-<time>.mp4`；历史入库（kind='recording'）。
- **绑定风险前置裁定**：objc2-screen-capture-kit 若不可用 → objc2 手写 SCStream extern（沿 ax.rs 模式）；若 AudioUnit 绑定过深 → 首版仅系统声（SCStream audio sample handler），麦克风列 M2.1。实现者按实际可用性裁定并在报告说明，不阻塞主线。

- [ ] Step 1: record.rs 捕获→硬编→mp4（ignored 集成测试：录 3s→文件可解码[ffprobe 或 AVAsset 断言]）。
- [ ] Step 2: 状态机 + 悬浮条 + F6 选区入口；暂停恢复无丢帧（帧计数断言）。
- [ ] Step 3: 性能抽测：1080p30 CPU ≤8%（`ps` 采样脚本）；全绿。
- [ ] Step 4: Commit: `feat(record): screencapturekit region recording with hardware encode`

### Task M7: M2 收尾（G5 启动 + 0.2.0）

- soak-record.sh 交付（控制器 72h 执行）；README/版本 0.2.0；bench 归档（F5 导出一致性人工验收单、F6 OCR 计时、F7 CPU 采样）；`git tag v0.2.0`（同 v0.1.0 前置：repo+secrets）。
- Commit: `chore: m2 closeout and release 0.2.0`

## 自审记录

1. **Spec 覆盖**：§3.3→M1-M4（取色器标尺在 §3.3 内→M5）；§3.4 基础→M6（GIF/画中画/keycast 归 M3）；§3.6→M0+M4 历史列；F5 导出一致→M2 单路径约束；F6 OCR 计时→M4；F7 录屏指标→M6；F12 FTS→M0；G5→M7 启测。**缺口**：样式记忆颜色/粗细→M1（localStorage）；「更多设置」浮条项→归 Plan C 已有设置页入口，不再重复。
2. **占位符检查**：M4 模型 SHA256 常量「实现者从官方 release 校验后写入」是流程性说明（值取决于下载时点），测试契约完整；M6 绑定选型为预授权裁定。
3. **类型一致**：ShotRow 既有契约复用；OcrLine 单处定义；Shape 联合在 M1 定型 M2 扩展。
