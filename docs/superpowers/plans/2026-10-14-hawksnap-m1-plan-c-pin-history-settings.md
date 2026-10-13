# HawkSnap M1-Plan C: 贴图 / 历史 / 主面板与设置 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 补齐 M1 剩余功能面（spec §11）：贴图 Pin（F2，原生窗口族）、历史简版（SQLite+缩略图+列表）、主面板与设置页（热键自定义/输出格式/保存目录/开机自启）、JPEG 输出，并完成 G2 整改（主窗口懒创建）与 overlay.rs 拆分（Plan B 最终评审裁定）。

**Architecture:** 贴图 Pin 走**原生 NSWindow+NSImageView**（objc2-app-kit，无 webview——8 张贴图若各带 webview 约需 240MB，直接击穿 G2 ≤80MB；纯图像窗口零 webview 成本，缩放/透明度/拖拽皆为窗口原生操作，Snipaste 同款架构）。主面板懒创建：启动无窗口（Accessory 常驻），托盘「主面板」按需创建/显示；热键注册失败时自动创建显示横幅（spec §9 可见反馈）。设置持久化用 JSON（app 数据目录），历史用 SQLite（rusqlite bundled）。

**Tech Stack:** 既有栈 + rusqlite（bundled SQLite）+ objc2-app-kit（已在树）+ tauri-plugin-autostart（开机自启）。

**Spec:** `docs/superpowers/specs/2026-10-13-hawksnap-design.md`（§3.2 贴图、§3.6 历史简版、§3.7 设置、§4 F2/F6 热键；验收对应 F3 部分、F12 的持久化部分——FTS 搜索归 M2 完整版）

## Global Constraints（继承 + 新增）

- 性能预算：常驻内存 ≤80MB（G2，本计划完成整改并实测归档）；安装包 ≤15MB（rusqlite bundled 约增 1-2MB，余量充足）；G3 已达标（105/6ms）不得回退——每个触及捕获/确认路径的任务完成后控制器复测。
- 隐私：无遥测无外联；tauri-plugin-autostart 仅写本地登录项。
- IPC 事件前缀 `hawk://`；TS strict；Rust edition 2021 + lints deny；每任务全绿 + conventional commit。
- UI 文案仍中文硬编码（i18n key 化归 Plan D，G6 门）；设置项「语言」占位渲染。
- macOS 专属 API 引用点与 target_os 门控同置；新 sync 命令禁止触碰窗口/CGEvent/AX（Plan B 线程模型纪律），涉及窗口的命令一律 `#[tauri::command(async)]` 或由前端触发的 async 命令。
- 平台差异：贴图/历史/设置的 Windows/Linux 适配归 Plan D CI 矩阵；本计划原生窗口代码 macos-gated。

---

### Task C0: overlay.rs 拆分（Plan B 继承项，先行）

**Files:**
- Create: `app/src-tauri/src/overlay/mod.rs`、`flow.rs`、`hover.rs`、`delay.rs`、`candidates.rs`、`hotkey.rs`
- Delete: `app/src-tauri/src/overlay.rs`（内容按职责迁入）

**Interfaces:**
- Produces: 纯机械拆分，零行为变更。`overlay::` 路径下所有 pub 项签名不变（lib.rs 的 `overlay::X` 引用经 mod.rs re-export 保持编译）。职责划分：`flow.rs`（start_capture_flow/hide_all_overlays/close_overlay 类/FLOW_SEQ/ANY_OVERLAY_VISIBLE）、`hover.rs`（T5/T6 的监听+resolve_hover/window_hit/display_frame_at）、`delay.rs`（T8 全部）、`candidates.rs`（RectLog/visible_window_rects）、`hotkey.rs`（register_hotkeys/HOTKEY_ERROR/hotkey_error/media key/register_hover_probe 的注册侧）、`mod.rs`（re-exports + 薄命令 cancel_capture/emit_capture_done）。

- [ ] **Step 1:** 按上述职责移动代码（`git mv` 不可用——内容移动，保持函数体逐字节不变）
- [ ] **Step 2:** `mod.rs` 写 `pub use` 清单；lib.rs 不变（`mod overlay;`）
- [ ] **Step 3:** Run: `cargo test --workspace` → 全绿（测试数与拆分前一致：app_lib 29）+ `npm run build` 不涉及
- [ ] **Step 4:** Commit: `refactor(overlay): split module by concern (flow/hover/delay/candidates/hotkey)`

---

### Task C1: 设置存储（JSON 持久化 + 默认值 + 热键字符串模型）

**Files:**
- Create: `app/src-tauri/src/settings.rs`
- Modify: `app/src-tauri/src/lib.rs`（`mod settings;`）

**Interfaces:**
- Produces: `Settings { hotkey_capture: String /*"F1"*/, hotkey_pin: String /*"F2"*/, hotkey_record: String /*"F6"*/, output_format: String /*"png"|"jpeg"*/, jpeg_quality: u8 /*1-100 默认 90*/, save_dir: Option<String> /*None=~/Pictures/HawkSnap*/, launch_at_login: bool, appearance: String /*"system"|"light"|"dark"*/ }`；`Settings::load(app) -> Settings`（无文件→默认）；`Settings::save(&self, app) -> Result<(), String>`；`settings_path(app) -> PathBuf`（`app_data_dir()/settings.json`）；`#[tauri::command] get_settings() -> Settings`、`#[tauri::command(async)] set_settings(s: Settings) -> Result<(), String>`（保存 + 重注册热键 + 自启开关应用，见 C7；本任务先只做保存+热键重注册 stub 注释）。
- 热键字符串模型：`tauri_plugin_global_shortcut::Shortcut::from_str`（如 "F1"、"Cmd+Shift+A"）——C7 依此注册。

- [ ] **Step 1: 失败测试**（settings.rs 内）：`defaults_are_sane`（默认值逐字段断言）；`roundtrip_preserves_fields`（save 到临时目录 → load 相等；路径注入：`save_to(path)/load_from(path)` 内部函数公开供测试）；`jpeg_quality_clamped`（save 时 >100 → 100，0 → 1）。
- [ ] **Step 2: RED** → 实现 → **GREEN**（cargo 29+3）。
- [ ] **Step 3:** lib.rs 注册两命令。
- [ ] **Step 4:** Commit: `feat(settings): json-backed settings store with typed hotkey model`

---

### Task C2: JPEG 输出（编码器 + 保存路径按设置）

**Files:**
- Modify: `crates/hawksnap-core/src/crop.rs`（`rgba_to_jpeg(rgba,w,h,quality) -> Result<Vec<u8>, CropError>`）
- Modify: `app/src-tauri/src/commands.rs`（finish_capture 保存分支按 `output_format` 选编码；扩展名 .png/.jpeg；`save_path_for` 生成对应扩展名）

**Interfaces:**
- Produces: `rgba_to_jpeg`（image crate JpegEncoder quality 1-100；测试：尺寸往返 + quality 边界 clamp）；保存契约：`savedTo` 路径扩展名随格式。

- [ ] **Step 1: 失败测试**（crop.rs）：`jpeg_roundtrips_dimensions`；`jpeg_quality_clamps_to_range`（0/255 → 1/100）。
- [ ] **Step 2: RED → 实现 → GREEN**（cargo core +2）。
- [ ] **Step 3:** commands.rs：`let ext = format; let bytes = match format.as_str() { "jpeg" => rgba_to_jpeg(&cropped, w, h, settings.jpeg_quality), _ => rgba_to_png(...) }`——settings 从 C1 `Settings::load(app)` 读取（每确认读一次小文件 ~µs，可接受；注释注明）。
- [ ] **Step 4:** Commit: `feat(save): jpeg output with quality from settings`

---

### Task C3: 历史存储（SQLite + 缩略图 + 保留策略）

**Files:**
- Modify: 根 `Cargo.toml`（workspace.dependencies 加 `rusqlite = { version = "0.37", features = ["bundled"] }`）+ `app/src-tauri/Cargo.toml`
- Create: `app/src-tauri/src/history.rs`
- Modify: `crates/hawksnap-core/src/crop.rs`（`rgba_thumbnail(rgba,w,h,max_edge) -> Result<(Vec<u8>,u32,u32),CropError>`——等比缩到 max_edge=320，Lanczos3）

**Interfaces:**
- Produces: `history::init(app) -> Result<(),String>`（建表 `shots(id INTEGER PRIMARY KEY, created_at TEXT, kind TEXT DEFAULT 'region', path TEXT, thumb_path TEXT, w INTEGER, h INTEGER, app TEXT)`，DB 路径 `app_data_dir()/history.db`）；`history::record(path,&thumb,w,h,app) -> Result<i64,String>`；`history::list(limit=200) -> Vec<ShotRow>`（新→旧）；`history::delete(id) -> Result<(),String>`（删行+文件+缩略图文件）；`history::prune(max=500)`（超限删最旧行及文件，record 后调用）；命令 `history_list() -> Vec<ShotRow>`、`#[tauri::command(async)] history_delete(id)`。
- `ShotRow { id, created_at, path, thumb_path, w, h, app }`（serde camelCase: createdAt/thumbPath）。

- [ ] **Step 1: 失败测试**（history.rs，临时目录 DB）：`init_is_idempotent`（两次 init 不炸）；`record_list_roundtrip`（record 两条 → list 顺序与字段）；`delete_removes_row_and_files`（真实临时文件，删后不存在）；`prune_keeps_newest_max`（插 6 条 max=5 → 剩 5 条且最旧被删）。
- [ ] **Step 2: RED** → `rgba_thumbnail` 失败测试（等比缩放断言 + 非整数比）→ 实现 → **GREEN**。
- [ ] **Step 3:** lib.rs：`mod history;` + setup `history::init(app.handle())?` + 两命令注册。
- [ ] **Step 4:** Commit: `feat(history): sqlite store with thumbnails and retention prune`

---

### Task C4: 捕获 → 历史集成

**Files:**
- Modify: `app/src-tauri/src/commands.rs`（finish_capture 保存成功后 `history::record`）

**Interfaces:**
- Produces: 保存即入库（path/thumb/w=physical.w/h=physical.h/app="HawkSnap"——frontmost 检测归后续，注释更新 Plan B 遗留的过时说法）；**clipboard-only 不入库**（无文件；注释说明，完整版再议临时文件方案）。

- [ ] **Step 1:** 实现（record 失败仅 eprintln 不影响保存成功——历史是旁路）。
- [ ] **Step 2:** 测试：现有 finish 逻辑无单测覆盖（AppHandle 依赖）；控制器 E2E 验证（保存后查 DB 行数）。单元不可测处注明。
- [ ] **Step 3:** Commit: `feat(history): record saved captures into history`

---

### Task C5: 贴图 Pin（原生窗口族，无 webview）

**Files:**
- Create: `crates/hawksnap-platform-mac/src/pin.rs`（NSWindow+NSImageView 封装）
- Modify: `crates/hawksnap-platform-mac/src/lib.rs`（`pub mod pin;`）
- Modify: `app/src-tauri/src/overlay.rs`→`flow.rs`（C0 后：F2 热键注册进 `hotkey.rs`，触发 `pin_latest`）
- Create: `app/src-tauri/src/pins.rs`（Pin 管理：id↔窗口映射、来源图像解析、关闭全部）

**Interfaces:**
- Produces: `pin::PinWindow::create(rgba: Arc<Vec<u8>>, w_px, h_px, at: (f64,f64), scale: f64) -> Result<PinWindow, String>`——NSWindow（borderless、always_on_top、不透明背景、`setMovable` via mouseDownCanMove、初始 logical 尺寸 = w_px/scale × 0.5 起始显示比）、NSImageView setImage（CGImage from raw RGBA）、窗口 `acceptsFirstResponder` false；`set_opacity(f64)`（20%-100% clamp）；`zoom_by(factor)`（10%-3200% clamp，以窗口中心缩放）；`close()`。objc2 用法对照 Task 5/13 既有模式；**macos-gated + 非 macos stub**（返回 Err("unsupported platform")）。
- `pins.rs`：`#[tauri::command(async)] pin_latest(app) -> Result<(),String>`——图像来源顺序：CAPTURES 最后一轮的 rgba（未释放时）→ arboard `get_image()`（剪贴板图像）→ Err("剪贴板没有图像且无最近截图")；位置 = 主显示器中心偏移 (n*24, n*24)；每 pin 递增 id。
- 命令：`#[tauri::command(async)] pin_set_opacity(id, opacity)`、`pin_zoom(id, factor)`、`#[tauri::command(async)] pin_close(id)`、`pin_close_all()`；托盘菜单加「关闭全部贴图」（有 pin 时）。
- F2 热键（settings `hotkey_pin`）注册于 hotkey.rs（C7 前先按默认 "F2" 硬编码注册，C7 切到 settings）。
- **交互**：拖拽移动（NSWindow isMovableByWindowBackground）、滚轮缩放（NSImageView 子类重写 scrollWheel → zoom_by，objc2 declare_class——对照 tao 的 NSWindow 子类声明模式）、Ctrl+滚轮透明度、双击 → 暂无编辑器（M2）→ 轻微闪烁反馈 + eprintln、Esc 关闭聚焦贴图（keyDown 监听，declare_class 一并做；仅当该 pin 是 key window）。
- 缩放/透明度范围即 F3 验收（10%-3200% / 20%-100%）。

- [ ] **Step 1: 失败测试**（纯逻辑）：`pins.rs` 的 `next_pin_offset(n) -> (f64,f64)`（级联偏移测试）；`clamp_zoom(cur, factor) -> f64`（0.1-3.2 边界）；`clamp_opacity`（0.2-1.0）。platform-mac 的 NSWindow 封装走 ignored 集成测试：`#[ignore] fn real_pin_window_creates_and_closes()`（创建 100×100 红RGBA pin → 200ms → close；需窗口环境，控制器不跑、留授权机）。
- [ ] **Step 2: RED → 实现 → GREEN**。
- [ ] **Step 3:** 控制器 E2E：F2 → 贴图窗口出现（观测：窗口列表出现无边框窗口 + RSS 增量 <10MB/pin——无 webview 的实证）。
- [ ] **Step 4:** Commit: `feat(pins): native image pin windows with zoom/opacity/drag (f2)`

---

### Task C6: 主面板懒创建 + G2 整改 + 托盘入口

**Files:**
- Modify: `app/src-tauri/tauri.conf.json`（`app.windows` 数组清空——启动无窗口）
- Modify: `app/src-tauri/src/panel.rs`（新建：`ensure_main_panel(app) -> Result<WebviewWindow,String>`——label "main"，按原 conf 几何创建，存在则 show+focus；空路由 `/`=App.tsx）
- Modify: `app/src-tauri/src/lib.rs`（托盘加「主面板」项 → 线程 spawn ensure；热键注册失败路径：除横幅通道外 spawn ensure_main_panel 让横幅可见——spec §9 收口；`settings.rs`/`history.rs` init 顺序不变）
- Modify: `app/src/App.tsx`（面板骨架：标题 HawkSnap、三个分区入口按钮「截图 (F1)」「贴图 (F2)」「历史」+ 设置区占位 C7 填充 + 既有横幅保留）
- Modify: `app/src-tauri/capabilities/default.json`（windows 加 `main`——已含，确认即可）
- Modify: `app/src-tauri/src/idle_reaper.rs`（book 扩展：主面板隐藏 10 分钟销毁——面板 webview 也在 G2 账单上；`ensure_main_panel` show 时从 book 移除——复用 expire_labels，加入 "main" label）

**Interfaces:**
- Produces: 启动零窗口（托盘+热键常驻，spec G2 口径）；`hawk://panel-closed` 不需要——reaper 按 is_visible 巡检即可（hide 由红绿灯叉触发：现有 prevent_exit 只拦 ExitRequested，窗口 close 仍走 Destroyed——**注意**：用户点叉会 DESTROY 主窗口（tauri 默认 close=destroy）→ ensure_main_panel 可重建 ✓；确认 prevent_exit 逻辑在零窗口+面板销毁后不误触发——ExitRequested code=None 由「最后一个窗口销毁」引发，本就 prevent ✓）。

- [ ] **Step 1:** 实现上述；`npm run build` 前端骨架先行。
- [ ] **Step 2:** 验证：cargo 全绿 + build 绿 + 控制器 E2E（启动后窗口列表无 hawksnap 窗口；托盘点主面板 → 窗口出现；RSS：启动空载 ≤80MB 实测——**这是 G2 达标点**）。
- [ ] **Step 3:** Commit: `feat(panel): lazy main panel with tray entry and g2 remediation`

---

### Task C7: 设置页（热键自定义 + 冲突处理 + 格式/目录/自启/外观）

**Files:**
- Modify: `app/src/App.tsx`（设置分区：热键捕获控件（点击→按下组合键→显示）、格式单选 png/jpeg + 质量滑杆、保存目录（文本框+默认按钮）、自启开关、外观三选、语言下拉占位（禁用+「Plan D」角标）、保存按钮 → `set_settings`）
- Modify: `app/src-tauri/src/settings.rs`（set_settings 完整应用：热键重注册=unregister_all + 按 settings 注册 F1/F2（F6 占位注册→触发「即将推出」横幅）；冲突：新热键注册失败 → Err 返回旧值前端回显，设置不落盘）；`apply_launch_at_login`（tauri-plugin-autostart：Manager enable/disable）
- Modify: `app/src-tauri/src/hotkey.rs`（`register_all_from_settings(app, &Settings)`——注册失败语义见上；F1 失败走 HOTKEY_ERROR 通道不变）
- Modify: 根/`app/src-tauri/Cargo.toml`（tauri-plugin-autostart = "2"）+ lib.rs `.plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))` + capabilities `autostart:allow-enable/allow-disable/allow-status`
- Modify: `crates/hawksnap-platform-mac`/前端无需改

**Interfaces:**
- Produces: 热键冲突 UX = spec §9 最小集（失败提示 + 不生效；候选键建议归后续）；F6 = 注册 + 触发时 emit `hawk://coming-soon`（"录屏功能将在后续版本提供"）→ App.tsx 横幅复用。
- 保存流程前端：调 `set_settings` → Err 显示 + 本地回滚显示值。

- [ ] **Step 1: 失败测试**（settings.rs）：`apply_hotkey_failure_keeps_file`（set_settings 传注册不了的键——用不可能组合如 "Cmd+Q"？Q 会被系统占……用注入 trait 而非真注册：`register_fn` 可注入 → 测试注入失败闭包 → 文件内容不变、返回 Err）。
- [ ] **Step 2: RED → 实现 → GREEN**（cargo +1-2）。
- [ ] **Step 3:** 控制器 E2E：改 F1 为 Ctrl+Alt+1 → 旧 F1 失效新键生效；设 jpeg → 保存出 .jpeg 文件；自启开关 → `osascript -e 'tell application "System Events" to get the name of every login item'` 含 HawkSnap。
- [ ] **Step 4:** Commit: `feat(settings): settings panel with hotkey capture, conflict handling, autostart`

---

### Task C8: F6 占位 + 收尾（README / 基准复测 / G2 归档 / tag）

**Files:**
- Modify: `README.md`（进度：贴图/历史/设置/主面板/JPEG；G2 达标数据；设置项说明）
- Modify: `app/src-tauri/tauri.conf.json` + `app/src-tauri/Cargo.toml` + 根 `Cargo.toml`（0.1.0-alpha.3）
- Create: `docs/superpowers/bench/2026-10-14-plan-c-acceptance.md`（控制器填数）

- [ ] **Step 1:** F6 触发 `hawk://coming-soon` 已在 C7；README 更新；版本三处对齐 alpha.3。
- [ ] **Step 2:** 控制器执行：`npm run tauri build` → `./scripts/bench-synth.sh 20`（G3 不回退）→ G2 采样（**≤80MB 达标判定**）→ `./scripts/check-size.sh`（rusqlite bundled 后体积复判 ≤15MB）→ 归档 bench 文档 → `cargo test --workspace && npm test && npm run build` → `git add -A && git commit -m "chore: plan-c closeout" && git tag v0.1.0-alpha.3`。
- [ ] **Step 3:** Commit: `chore: plan-c closeout readme and acceptance archive`

---

## 自审记录（writing-plans self-review）

1. **Spec 覆盖**：§3.2 贴图→C5（管理器 UI 未含——spec 将其列于贴图条目内，M1 验收 F3 只需多贴图+缩放+透明度，管理器 UI 归 M2 随编辑器；已在此注明为有意裁剪）。§3.6 简版→C3/C4（FTS/搜索/重新编辑归 M2 完整版）。§3.7 设置→C1/C7（更新开关归 D——更新器本体在 D；语言占位）。§4 F2→C5、F6→C7 占位。G2→C6。JPEG→C2。
2. **无占位符**：C5 的 NSImageView scrollWheel/keyDown declare_class 与 C7 的 autostart 插件 API 允许实现者按锁定版本适配（对照 T5/T13 既有 objc2 模式），测试契约已给全。
3. **类型一致性**：`Settings` 字段名 C1↔C7 一致；`ShotRow` camelCase；`pin_latest`/`pin_*` 命令名前后一致；`expire_labels` 扩展 "main" label 复用既有纯函数。
