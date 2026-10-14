# HawkSnap M1-Plan D: i18n / 自动更新 / 权限引导 / CI 矩阵 → 发布 0.1 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 M1 出口（spec §11）：i18n 双语（zh-CN/en-US + key 覆盖率 CI）、自动更新（updater + Ed25519 + 伪签名测试）、macOS 权限引导（含修复 Plan B 裁定的静默壁纸缺陷）、热键单一真源与冲突候选、三平台 CI 矩阵、G1–G8 全量验收 → 发布 v0.1.0。

**Architecture:** i18n 用轻量自研 key-value 层（`t(key)` + 两份扁平 JSON 词典 + 跟随系统/覆盖，不引 i18next——轻量原则与 spec「自研矢量层」同逻辑）；更新走 tauri-plugin-updater 官方插件 + GitHub Releases 静态 latest.json 通道；权限引导以 CGPreflight 系预检 + 深链系统设置为核心；CI 用 GitHub Actions 三平台矩阵（build + test + i18n 检查 + 体积断言）。

**Tech Stack:** 既有栈 + tauri-plugin-updater 2.x + GitHub Actions。不自研更新签名（插件内置 Ed25519）。

**Spec:** `docs/superpowers/specs/2026-10-13-hawksnap-design.md`（§3.7 i18n/更新、§4 冲突处理、§9 错误处理、§6.1 G1-G8、§6.3 平台矩阵）

## Global Constraints

- 隐私不变：更新检查是唯一新增外联（spec §5 明示可配置，默认开启可关闭——接 settings）。
- Rust edition 2021 + lints deny；TS strict；每任务全绿 + conventional commit；UI 文案一律 key（G6）。
- 新增文案 key 双语同时落盘（漏一边 = CI 红，G6 门）。
- Windows 范围裁定：CI 编译+单测+打包绿为 D 出口；实机 E2E 矩阵延后（无实机），Release 说明标注 Windows beta。
- G5（72h 录屏）在 0.1 阶段 N/A（录屏为 M2 交付物）——验收矩阵标注，非遗漏。
- 热键冲突「列出占用应用名」macOS 无公开 API——降级为「候选键自动探测建议」（spec §9 期望的部分实现，裁定记录）。
- 继承 Plan C 终审：热键单一真源、F6 静默修复、bench interleaving 守卫、常量下沉、xdotool 入库、横幅归因。

---

### Task D0: 继承项批处理（Plan C 终审清单）

**Files:**
- Modify: `app/src-tauri/src/overlay/hotkey.rs`、`overlay/flow.rs`、`overlay/mod.rs`、`app/src-tauri/src/bench_mode.rs`、`app/src/App.tsx`、`crates/hawksnap-core/src/geometry.rs`（常量下沉）、`crates/hawksnap-platform-mac/src/pin.rs`、`scripts/`（xdotool 入库）
- Create: `scripts/bench-xdotool.sh`

**Interfaces:**
- Produces: `hotkey.rs` 内 `pub fn capture_label(s: &Settings) -> &str` 类辅助不存在——改为：所有用户可见热键名一律从 `Settings::load` 读取（媒体键路径 hint 文案、F6 coming-soon 文案、面板按钮 label、横幅）；`geometry.rs` 新增 `pub const PIN_ZOOM_MIN: f64 = 0.1; PIN_ZOOM_MAX: f64 = 3.2; PIN_OPACITY_MIN: f64 = 0.2; PIN_OPACITY_MAX: f64 = 1.0;`（pin.rs 与 pins.rs 改引 core 常量）；`start_capture_flow` 首行 bench 守卫：`if crate::bench_mode::is_active() && 非bench触发 { return Ok(()) }`——实现为 bench_mode 暴露 `pub fn guard_external_trigger() -> bool`（bench 进行中返回 false），hotkey/tray/panel 三处触发点调用，bench 内部触发绕过。

- [ ] **Step 1:** 逐项落地：①媒体键 hint + F6 文案 + 面板按钮读 settings；②F6 处理器触发前 ensure_main_panel（coming-soon 横幅可见）；③bench guard 三处；④常量下沉 core（两处 clamp 改引）；⑤横幅归因细化（App.tsx hotkey-error 文案区分 capture/pin/record 来源——payload 加 kind 字段，emit 方带）；⑥`scripts/bench-xdotool.sh` 依姊妹档案的注入序列固化（xdotool key F1 → mousemove/down/move/up → click 复制按钮坐标），头注释注明需 Xvfb + xdotool。
- [ ] **Step 2:** `cargo test --workspace` 全绿（app_lib 46+ 增量）；`npm test`、`npm run build` 全绿。
- [ ] **Step 3:** Commit: `refactor: plan-d inherited items — hotkey single source, f6 visibility, bench guard, shared clamps, xdotool script`

---

### Task D1: i18n 框架 + 全量 key 化

**Files:**
- Create: `app/src/lib/i18n.ts`、`app/src/locales/zh-CN.json`、`app/src/locales/en-US.json`
- Modify: `app/src/App.tsx`、`app/src/windows/overlay/index.tsx`、`app/src/windows/countdown/index.tsx`（全部用户可见文案 → `t(key)`）
- Modify: `app/src-tauri/src/settings.rs`（Settings 增 `language: String`，默认 "system"；枚举 system/zh-CN/en-US）

**Interfaces:**
- Produces: `t(key: string, vars?: Record<string, string|number>): string`——扁平 key 点分层级（`panel.title`、`overlay.save`、`banner.hotkeyError` 等）；初始化：`settings.language` → "system" 时取 `navigator.language` 前缀匹配 zh → zh-CN 否则 en-US；词典缺失 key 回退 zh-CN 再回退 key 本身（dev 告警 console.warn）；`set_language(l)` 写 settings 并热切换（React context 或简单模块级 listener——保持 ~100 行内）。
- Rust 侧文案（热键 hint、coming-soon、错误串）暂不 key 化——G6 范围明确为前端 UI 文案（CI 只查前端）；README 注明 Rust 文案 i18n 归后续。

- [ ] **Step 1: 失败测试**（i18n.test.ts）：`t_returns_zh_for_known_key`；`t_falls_back_to_zh_then_key`（en 词典缺 key 时）；`t_interpolates_vars`（`{n}` 插值）；`language_resolution`（system + navigator zh → zh-CN；en-US → en-US；显式覆盖优先）。
- [ ] **Step 2: RED → 实现 → GREEN**；词典初版覆盖现有全部文案（盘点 App.tsx/overlay/countdown 的中文字面量，全量迁移；overlay 的尺寸标签 `{w}×{h}` 无需 key）。
- [ ] **Step 3:** `cargo test --workspace`（settings +1 字段）、`npm test`、`npm run build` 全绿。
- [ ] **Step 4:** Commit: `feat(i18n): lightweight bilingual i18n with full ui key coverage`

---

### Task D2: i18n key 覆盖率 CI（G6）

**Files:**
- Create: `scripts/check-i18n.mjs`、`.github/workflows/ci.yml`（首个 workflow，含后续任务的 job 骨架）

**Interfaces:**
- Produces: `node scripts/check-i18n` 退出码语义：①扫描 `app/src/**/*.{ts,tsx}` 中 `t("...")` 字面量调用 → key 集合 A；②读两份词典 → 集合 Z/E；③A⊆Z 且 A⊆E 且 Z==E（双向无孤儿）→ 0，否则打印缺失/孤儿清单 exit 1。CI workflow：`on: [push, pull_request]`；jobs：`rust-test`（macos-14 + ubuntu-24.04 + windows-2022：cargo test --workspace、clippy -D warnings）、`frontend`（npm ci/test/build）、`i18n`（check-i18n）、`size`（macos 构建后 check-size.sh；此 job 先 allow-failure 占位，D6 转正式）。

- [ ] **Step 1:** check-i18n.mjs（纯 node，无依赖；用正则抽 `t("key")` 与 `t('key')`；动态 key `t(variable)` 扫不到——约定禁止动态 key，检查器对 `t(` 非字面量调用行报 warn 不 fail，注释说明）。
- [ ] **Step 2:** 本地跑通（对 D1 后的树应 exit 0；故意删一个 key 验证 exit 1）。
- [ ] **Step 3:** ci.yml 落盘（rust-test 三平台先行——这一步同时就是 Windows/Linux CI 编译验证的落地，Windows job 若因平台差异红，修复 cfg 至绿——**这是 D 阶段 Windows bring-up 的主战场**，预计 cfg 门控补漏；如遇 Windows-only 编译错误，最小修复并记录）。
- [ ] **Step 4:** Commit: `feat(ci): i18n coverage gate and three-platform matrix`

---

### Task D3: 自动更新（updater + Ed25519 + 伪签名测试，G7）

**Files:**
- Modify: `app/src-tauri/Cargo.toml`（tauri-plugin-updater = "2"）、`lib.rs`（插件注册）、`capabilities/default.json`（updater 权限：default + activate）
- Modify: `app/src-tauri/tauri.conf.json`（`plugins.updater.endpoints: ["https://github.com/guxiaolong/HawkSnap/releases/latest/download/latest.json"]`、`pubkey`——密钥对生成是控制器手工步骤，占位 `REPLACE_AT_RELEASE`，D8 填真钥）
- Modify: `settings.rs`（`update_enabled: bool` 默认 true）、`App.tsx`（设置区「自动更新」开关 + 「检查更新」按钮 + 更新日志对话框占位——下载进度文本即可）
- Create: `app/src-tauri/src/updater.rs`（`#[tauri::command(async)] check_for_updates(app, manual: bool)`——update_enabled=false 且 !manual → Ok(None)；否则插件 check → 有更新返回版本号+notes+下载进度事件 `hawk://update-progress`；`install_update(app)`——下载+安装+relaunch）
- Test: G7 伪签名——`app/src-tauri/tests/update_signature.rs` 集成测试：本地起一个 hyper/tiny_http 静态服务指向 `tests/fixtures/`（伪 latest.json + 伪签名更新包），endpoint 指向它 → check 返回错误且**不安装**（断言错误含 signature 字样 + 进程仍运行旧版本语义）。插件签名校验在插件内部——测试目标降级为「错误被 check_for_updates 捕获为 Err 且无 install 调用」。

- [ ] **Step 1:** 依赖 + 插件 + 命令骨架 + settings 开关 + App.tsx UI（key 化文案）。
- [ ] **Step 2:** 伪签名集成测试（tiny_http dev-dependency；fixtures 提交；CI 也跑——updater check 对坏签名必须 Err）。
- [ ] **Step 3:** `cargo test --workspace` 全绿；`npm test/build` 全绿（新文案 key 化，i18n 检查本地过）。
- [ ] **Step 4:** Commit: `feat(updater): tauri updater with signed channel, settings toggle, fake-signature test`

---

### Task D4: macOS 权限引导（修静默壁纸缺陷）

**Files:**
- Create: `crates/hawksnap-platform-mac/src/permissions.rs`（`screen_recording_prechecked() -> bool`——CGPreflightScreenCaptureAccess extern，模式同 ax.rs；`open_screen_recording_settings() -> Result<(),String>`——`open "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"` 经 CoreServices or `std::process::Command::new("open")`）
- Modify: `crates/hawksnap-platform-mac/src/lib.rs`（`pub mod permissions;`）
- Modify: `overlay/flow.rs`（start_capture_flow 抓屏前预检：未授权 → Err("屏幕录制权限未授予…已打开系统设置") + 打开设置面板——**修 Plan B 裁定的静默壁纸缺陷**：未授权时不再返回壁纸帧，而是引导+失败）；`overlay/hotkey.rs`（媒体键路径同预检）
- Modify: `App.tsx`（捕获失败横幅已存在——文案 key 化即可；授权返回后的「自动重试」简化为提示用户再按 F1，注释注明全自动重试归后续）
- Test: permissions.rs 单测——`screen_recording_prechecked` 在本机返回 bool 不 panic（真值依赖授权态）；开放设置函数不测（副作用）。ignored 集成：未授权语义无法脚本化——留人工。

- [ ] **Step 1:** 失败测试（预检函数存在且不 panic）→ 实现 → GREEN。
- [ ] **Step 2:** flow.rs 预检接线（bench 模式绕过预检！`HIDE_WINDOWS` 时跳过——bench 环境无授权上下文干扰）。
- [ ] **Step 3:** `cargo test --workspace` 全绿。控制器人工验证：撤权 → F1 → 系统设置被打开 + 横幅出现。
- [ ] **Step 4:** Commit: `feat(permissions): screen-recording preflight with settings deep-link (fixes silent wallpaper)`

---

### Task D5: G4 仪器（冷启动→托盘 ≤800ms；主面板 ≤300ms）

**Files:**
- Modify: `app/src-tauri/src/perf.rs`（`mark` 复用；新增阶段 `tray_ready`（托盘 build 完成后）、`panel_shown`（ensure_main_panel 返回后））
- Create: `scripts/bench-g4.sh`：启动二进制（HAWKSNAP_BENCH 同款 env 门控思路：`HAWKSNAP_G4=1` 时 setup 完成托盘后自动 spawn 主面板再自动退出）→ 从 perf CSV 取 `process_start`（新增：main 首行 mark，用 std::time since UNIX_EPOCH——冷启动 t0 用 CSV 内首行即可）→ 输出 cold-start = tray_ready - 首mark、panel = panel_shown - panel_requested（新 mark `panel_requested`）×5 次取中位/P90。
- Test: 无新单测（集成仪器）；bash -n。

- [ ] **Step 1:** marks + env 门控自动开面板逻辑（模式同 bench_mode，独立 `HAWKSNAP_G4`）。
- [ ] **Step 2:** 脚本 + 本机实测 5 次 → 输出两行（冷启动 ≤800ms 判定；主面板 ≤300ms 判定）。
- [ ] **Step 3:** Commit: `feat(perf): g4 cold-start and panel-open instrumentation`

---

### Task D6: CI 矩阵正式化 + G1 断言（收口 D2 的占位）

**Files:**
- Modify: `.github/workflows/ci.yml`（size job 转正式：macos build → check-size.sh exit 0；release job：tag 推送时 `tauri build` 三平台 → 上传产物 + `latest.json` 生成（updater 格式，版本/平台/签名——签名需 D8 的私钥，workflow 从 secrets 取））

**Interfaces:**
- Produces: push → 三平台测试矩阵绿；tag v* → 三平台产物 + updater latest.json（GitHub Releases）。

- [ ] **Step 1:** workflow 补全（windows job 的 tauri build 首次真跑——预计暴露 Windows 打包问题：图标/WebView2 bootstrap；逐个最小修复，问题清单进报告）。
- [ ] **Step 2:** 本地 `act` 不可用则依赖推送后 Actions 页观察（控制器步骤）。
- [ ] **Step 3:** Commit: `feat(ci): size gate formalized and release pipeline with updater feed`

---

### Task D7: G1-G8 全量验收归档 → 发布 0.1

**Files:**
- Create: `docs/superpowers/bench/2026-10-14-plan-d-g1-g8.md`（验收矩阵：G1 体积三平台、G2 内存 macOS+Linux、G3 bench 模式、G4 新仪器、G5 N/A（录屏 M2）、G6 CI 绿、G7 伪签名测试绿、G8 单测全绿 + tarpaulin/llvm-cov 覆盖率（capture/crop/naming ≥70% 目标——实际数如实记录））
- Modify: `Cargo.toml` 三处版本 → **0.1.0**、README 进度段、`git tag v0.1.0`
- 控制器步骤：推送 tag → Actions release 流 → GitHub Release 草稿（标题 0.1.0，说明 = README 摘录 + 已知限制（Windows beta、G5 N/A、Rust 文案未 i18n））

- [ ] **Step 1:** 全套验收采集（bench 模式 G3、sample-mem、bench-g4、CI 全绿截图/日志引用、覆盖率跑分）→ 归档。
- [ ] **Step 2:** 版本 0.1.0 + tag + 推送。
- [ ] **Step 3:** Commit: `chore: release 0.1.0`

---

## 自审记录（writing-plans self-review）

1. **Spec 覆盖**：§3.7 i18n→D1/D2（Rust 文案除外——注明）；更新→D3/D6/D8；§4 冲突→D0（候选键探测未含——降级裁定：macOS 无法列占用应用，自动候选探测与 spec 原文差距在 README 已知限制中注明，实现成本高收益低，**留 M2 评估**）；§9 权限引导→D4；§6.1 G1-G8→D7（G5 N/A 裁定）；§6.3 矩阵→D6（Windows beta 裁定）。**缺口声明**：更新日志展示做下载进度级简版（完整 changelog 渲染归 M2 打磨）。
2. **无占位符**：pubkey 占位 `REPLACE_AT_RELEASE` 是流程性占位（密钥是控制器手工产物，D8 填入），非实现 TBD；updater 插件 API 按锁定版本适配（沿 C7 autostart 先例）。
3. **类型一致性**：`language` 字段 D1 定义、CI 检查器读同一词典文件；`hawk://update-progress` 事件 D3 单处定义。
