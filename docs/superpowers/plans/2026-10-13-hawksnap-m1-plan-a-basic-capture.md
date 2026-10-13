# HawkSnap M1-Plan A: 项目骨架 + 基础截图闭环 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从零搭建 HawkSnap 仓库与 Tauri 2 + React 18 应用骨架，打通最小截图闭环：按 F1 → 捕获冻结帧 → 全屏透明覆盖层框选 → 裁剪 → 复制到剪贴板/保存文件 → 托盘常驻。

**Architecture:** Rust workspace 分层——`hawksnap-core`（纯逻辑：几何/命名/裁剪/捕获 trait，全平台无依赖）+ `hawksnap-platform-mac`（xcap 实现抓屏）+ `app/src-tauri`（Tauri 命令、热键、覆盖层窗口编排）。前端 React 覆盖层用 Canvas 显示冻结帧并承载框选交互。本计划只覆盖主显示器；多显示器与智能吸附在 Plan B。

**Tech Stack:** Tauri 2、Rust（workspace）、React 18 + TypeScript(strict) + Vite、zustand、Vitest + React Testing Library、`xcap`（抓屏）、`arboard`（剪贴板）、`tauri-plugin-global-shortcut`（热键）。

**Spec:** `docs/superpowers/specs/2026-10-13-hawksnap-design.md`（本计划实现其 §3.1 基础截图与 §3.7 设置的最小子集、§7 架构；验收对应 G1/G2/G3 部分、F1 部分达到「主显示器可用」）

## Global Constraints

- 性能预算（spec §5）：热键→覆盖层 ≤150ms（P90）；确认选区→剪贴板 ≤100ms（P90）；常驻内存 ≤80MB；安装包 ≤15MB。
- 隐私（spec §1）：无遥测、无账号、无外联；本计划不引入任何网络请求。
- 命名（spec §7.1）：crate 名 `hawksnap-core`、`hawksnap-platform-mac`；IPC 事件前缀 `hawk://`。
- 默认热键 F1 = 区域截图（spec §4），注册失败必须给用户可见反馈，不允许静默失败。
- TypeScript `strict: true`；Rust edition 2021、`#![deny(unsafe_op_in_unsafe_fn)]`；每个任务以 `cargo test` / `npm test` 全绿 + git commit 结束。
- 依赖版本以本计划锁定为准；若 crate 小版本 API 漂移，允许最小适配，但不得改变各任务 Tests 中断言的行为契约。
- 开发/验证平台：macOS（本仓库当前开发机 darwin arm64）；Windows/Linux 编译验证延后到 CI（Plan D）。
- git 提交信息用 conventional commits（`feat:`/`test:`/`chore:`/`refactor:`）。

---

### Task 1: 仓库初始化与 Tauri/React 脚手架

**Files:**
- Create: `/Users/guxiaolong/workspace/HawkSnap/.gitignore`
- Create: `/Users/guxiaolong/workspace/HawkSnap/Cargo.toml`（workspace 根）
- Create: `app/`（`npm create tauri-app` 脚手架产物）
- Create: `crates/hawksnap-core/Cargo.toml`、`crates/hawksnap-core/src/lib.rs`

**Interfaces:**
- Produces: 仓库布局 `app/`（Tauri 应用）+ `crates/hawksnap-core`（后续所有纯逻辑的家）；`hawksnap-core` 对外暴露空 crate（Task 2 起填充）。

- [ ] **Step 1: 脚手架**

```bash
cd /Users/guxiaolong/workspace/HawkSnap
npm create tauri-app@latest app -- --template react-ts --manager npm --identifier com.hawksnap.app --yes
# 若 CLI 仍交互提问：project name=hawksnap，其余默认。
cd app && npm install
```

- [ ] **Step 2: 根 workspace Cargo.toml**

```toml
# /Users/guxiaolong/workspace/HawkSnap/Cargo.toml
[workspace]
resolver = "2"
members = ["app/src-tauri", "crates/*"]

[workspace.package]
edition = "2021"
version = "0.1.0-alpha.1"

[workspace.dependencies]
hawksnap-core = { path = "crates/hawksnap-core" }
image = "0.25"
thiserror = "1"
serde = { version = "1", features = ["derive"] }
```

- [ ] **Step 3: hawksnap-core 骨架**

```toml
# crates/hawksnap-core/Cargo.toml
[package]
name = "hawksnap-core"
version.workspace = true
edition.workspace = true

[dependencies]
image.workspace = true
thiserror.workspace = true
serde.workspace = true
```

```rust
// crates/hawksnap-core/src/lib.rs
pub mod geometry;

#[cfg(test)]
mod smoke_tests {
    #[test]
    fn workspace_builds() { assert_eq!(1 + 1, 2); }
}
```

```rust
// crates/hawksnap-core/src/geometry.rs
//! 占位模块，Task 2 填充。
```

- [ ] **Step 4: .gitignore 与前端测试依赖**

`.gitignore`（仓库根）：

```
node_modules/
dist/
target/
.DS_Store
*.png.tmp
.superpowers/
app/src-tauri/gen/schemas/
```

```bash
cd /Users/guxiaolong/workspace/HawkSnap/app
npm install -D vitest @testing-library/react @testing-library/dom jsdom
```

`app/package.json` 增加：

```json
"scripts": {
  "dev": "vite",
  "build": "tsc && vite build",
  "test": "vitest run",
  "tauri": "tauri"
}
```

- [ ] **Step 5: git 初始化并提交**

```bash
cd /Users/guxiaolong/workspace/HawkSnap
git init
git add -A
git commit -m "chore: scaffold tauri2+react workspace with hawksnap-core crate"
```

- [ ] **Step 6: 验证**

Run: `cargo test --workspace` → Expected: PASS（smoke_tests）。
Run: `cd app && npm test` → Expected: PASS, no tests found（vitest 退出码 0 需在 `vitest.config.ts` 设 `passWithNoTests: true`）。
Run: `cd app && npm run build` → Expected: 构建成功。

---

### Task 2: 选区几何（Rect / Scale / 微调 / 磁吸）

**Files:**
- Modify: `crates/hawksnap-core/src/geometry.rs`
- Test: `crates/hawksnap-core/src/geometry.rs`（`#[cfg(test)]` 内联）

**Interfaces:**
- Produces: `Rect{x:i32,y:i32,w:u32,h:u32}`、`Scale(f64)`、`Rect::from_points`、`Rect::clamp_to`、`Rect::nudge`、`Rect::snap_to`、`Scale::to_physical`、`Scale::to_logical`——Plan B 的吸附高亮与本计划的覆盖层都依赖这些签名。

- [ ] **Step 1: 写失败测试**

```rust
// geometry.rs 末尾
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_points_normalizes_reverse_drag() {
        // 从右下往左上拖拽也要得到正的宽高
        let r = Rect::from_points(300, 200, 100, 50);
        assert_eq!((r.x, r.y, r.w, r.h), (100, 50, 200, 150));
    }

    #[test]
    fn clamp_keeps_rect_inside_bounds() {
        let r = Rect { x: -20, y: -10, w: 100, h: 50 };
        let b = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        let c = r.clamp_to(b);
        assert_eq!((c.x, c.y), (0, 0));
        assert!(c.x + c.w as i32 <= 1920 && c.y + c.h as i32 <= 1080);
    }

    #[test]
    fn nudge_moves_origin_only() {
        let r = Rect { x: 10, y: 10, w: 100, h: 100 };
        let n = r.nudge(5, -3);
        assert_eq!((n.x, n.y, n.w, n.h), (15, 7, 100, 100));
    }

    #[test]
    fn snap_pulls_edge_within_threshold() {
        let sel = Rect { x: 103, y: 10, w: 50, h: 50 };
        let target = Rect { x: 0, y: 0, w: 1920, h: 1080 };
        // 左边缘距 target 左边缘 103px，仅当阈值 ≥103 时吸附
        let s = sel.snap_to(&[target], 8);
        assert_eq!(s.x, 103);
        let s2 = sel.snap_to(&[target], 120);
        assert_eq!(s2.x, 0);
    }

    #[test]
    fn scale_roundtrip_physical_logical() {
        let s = Scale(2.0); // Retina
        assert_eq!(s.to_physical(100), 200);
        assert_eq!(s.to_logical(200), 100);
        let s125 = Scale(1.25);
        assert_eq!(s125.to_physical(100), 125);
        assert_eq!(s125.to_logical(125), 100);
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p hawksnap-core`
Expected: FAIL（`from_points` 等未定义）。

- [ ] **Step 3: 实现**

```rust
// geometry.rs（替换占位内容）
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale(pub f64);

impl Rect {
    pub fn from_points(ax: i32, ay: i32, bx: i32, by: i32) -> Self {
        let (x0, x1) = if ax <= bx { (ax, bx) } else { (bx, ax) };
        let (y0, y1) = if ay <= by { (ay, by) } else { (by, ay) };
        Rect { x: x0, y: y0, w: (x1 - x0) as u32, h: (y1 - y0) as u32 }
    }

    pub fn right(&self) -> i32 { self.x + self.w as i32 }
    pub fn bottom(&self) -> i32 { self.y + self.h as i32 }

    pub fn clamp_to(self, bounds: Rect) -> Rect {
        let x = self.x.clamp(bounds.x, bounds.right() - 1);
        let y = self.y.clamp(bounds.y, bounds.bottom() - 1);
        let w = (self.w as i32).clamp(1, bounds.right() - x) as u32;
        let h = (self.h as i32).clamp(1, bounds.bottom() - y) as u32;
        Rect { x, y, w, h }
    }

    pub fn nudge(self, dx: i32, dy: i32) -> Rect { Rect { x: self.x + dx, y: self.y + dy, ..self } }

    /// 任一边缘与某候选边缘距离 < threshold 时吸附到该候选。
    pub fn snap_to(self, candidates: &[Rect], threshold: i32) -> Rect {
        let mut r = self;
        for c in candidates {
            let pairs = [(r.x, c.x), (r.right(), c.right()), (r.y, c.y), (r.bottom(), c.bottom())];
            let deltas = pairs.map(|(a, b)| b - a);
            if deltas.iter().any(|d| d.abs() < threshold) {
                if (c.x - r.x).abs() < threshold { r.x = c.x; }
                if (c.right() - r.right()).abs() < threshold { r.x = c.right() - r.w as i32; }
                if (c.y - r.y).abs() < threshold { r.y = c.y; }
                if (c.bottom() - r.bottom()).abs() < threshold { r.y = c.bottom() - r.h as i32; }
            }
        }
        r
    }
}

impl Scale {
    pub fn to_physical(self, v: i32) -> i32 { (v as f64 * self.0).round() as i32 }
    pub fn to_logical(self, v: i32) -> i32 { (v as f64 / self.0).round() as i32 }
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p hawksnap-core`
Expected: PASS（6 个测试）。

- [ ] **Step 5: 提交**

```bash
git add crates/hawksnap-core
git commit -m "feat(core): selection geometry with clamp/nudge/snap and dpi scale"
```

---

### Task 3: 文件命名模板引擎

**Files:**
- Create: `crates/hawksnap-core/src/naming.rs`
- Modify: `crates/hawksnap-core/src/lib.rs`（`pub mod naming;`）

**Interfaces:**
- Produces: `NamingCtx{app,date,time}`、`expand(template,&ctx,exists)->String`（spec §3.1 变量 `{app} {date} {time} {n}`，重名自动追加序号）。

- [ ] **Step 1: 写失败测试**

```rust
// naming.rs
#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> NamingCtx {
        NamingCtx { app: "Safari".into(), date: "2026-10-13".into(), time: "153000".into() }
    }

    #[test]
    fn expands_all_variables() {
        let out = expand("{app} {date} {time}", &ctx(), |_| false);
        assert_eq!(out, "Safari 2026-10-13 153000");
    }

    #[test]
    fn appends_counter_on_collision() {
        let mut calls = 0;
        let out = expand("shot", &ctx(), |name| {
            calls += 1;
            name == "shot" || name == "shot 2" // 已存在 shot 与 shot 2
        });
        assert_eq!(out, "shot 3");
        assert_eq!(calls, 3);
    }

    #[test]
    fn zero_pads_sequence_number() {
        let out = expand("{n}", &ctx(), |_| false);
        assert_eq!(out, "001"); // 三位序号，{n} 为当日第几张由调用方传入计数——M1 简化为 001 起步
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p hawksnap-core naming`
Expected: FAIL（模块为空）。

- [ ] **Step 3: 实现**

```rust
// naming.rs（置于 tests 模块之前）
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingCtx { pub app: String, pub date: String, pub time: String }

pub fn expand(template: &str, ctx: &NamingCtx, exists: impl Fn(&str) -> bool) -> String {
    let base = template
        .replace("{app}", &ctx.app)
        .replace("{date}", &ctx.date)
        .replace("{time}", &ctx.time)
        .replace("{n}", "001");
    if !exists(&base) { return base; }
    for i in 2..10_000 {
        let candidate = format!("{base} {i}");
        if !exists(&candidate) { return candidate; }
    }
    format!("{base} {}", chrono_free_suffix())
}

fn chrono_free_suffix() -> u64 { std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0) }
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p hawksnap-core`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/hawksnap-core
git commit -m "feat(core): filename template engine with collision counter"
```

---

### Task 4: 捕获抽象与内存桩（ScreenGrabber trait）

**Files:**
- Create: `crates/hawksnap-core/src/capture.rs`
- Modify: `crates/hawksnap-core/src/lib.rs`（`pub mod capture;`）

**Interfaces:**
- Produces: `ScreenFrame{display_id,x,y,width,height,scale,png:Vec<u8>}`、`trait ScreenGrabber{fn grab_all(&mut self)->Result<Vec<ScreenFrame>,CaptureError>}`、`CaptureError`、测试桩 `InMemGrabber`。Task 5 的 mac 实现与 Task 6 的命令层都依赖此 trait。

- [ ] **Step 1: 写失败测试**

```rust
// capture.rs
#[cfg(test)]
mod tests {
    use super::*;

    fn frame_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::new(w, h);
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[test]
    fn in_mem_grabber_returns_frames_in_order() {
        let g = InMemGrabber::new(vec![
            ScreenFrame { display_id: 1, x: 0, y: 0, width: 100, height: 50, scale: 2.0, png: frame_png(200, 100) },
        ]);
        let frames = g.grab_all().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!((frames[0].width, frames[0].height), (100, 50)); // 逻辑尺寸
        assert_eq!(frames[0].png.len() > 8, true); // 是 PNG 字节
    }

    #[test]
    fn grab_error_is_typed() {
        let g = InMemGrabber::new(vec![]);
        assert!(matches!(g.grab_all(), Err(CaptureError::NoDisplays)));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p hawksnap-core capture`
Expected: FAIL。

- [ ] **Step 3: 实现**

```rust
// capture.rs（置于 tests 之前）
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ScreenFrame {
    pub display_id: u64,
    pub x: i32, pub y: i32,          // 逻辑坐标（多屏虚拟桌面）
    pub width: u32, pub height: u32, // 逻辑尺寸
    pub scale: f64,                  // DPI 缩放（Retina=2.0）
    pub png: Vec<u8>,                // 物理像素 PNG
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("no display found")]
    NoDisplays,
    #[error("capture failed: {0}")]
    Backend(String),
    #[error("permission denied for screen capture")]
    Permission,
}

pub trait ScreenGrabber {
    fn grab_all(&mut self) -> Result<Vec<ScreenFrame>, CaptureError>;
}

/// 测试桩：预置帧直接返回。
pub struct InMemGrabber { frames: Vec<ScreenFrame> }

impl InMemGrabber {
    pub fn new(frames: Vec<ScreenFrame>) -> Self { InMemGrabber { frames } }
}

impl ScreenGrabber for InMemGrabber {
    fn grab_all(&mut self) -> Result<Vec<ScreenFrame>, CaptureError> {
        if self.frames.is_empty() { Err(CaptureError::NoDisplays) } else { Ok(self.frames.clone()) }
    }
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p hawksnap-core`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/hawksnap-core
git commit -m "feat(core): ScreenGrabber trait with typed errors and in-memory stub"
```

---

### Task 5: macOS 抓屏实现（xcap）

**Files:**
- Create: `crates/hawksnap-platform-mac/Cargo.toml`、`crates/hawksnap-platform-mac/src/lib.rs`
- Modify: 根 `Cargo.toml`（`workspace.dependencies` 增加 `hawksnap-platform-mac`、`xcap`）

**Interfaces:**
- Consumes: `hawksnap_core::capture::{ScreenGrabber, ScreenFrame, CaptureError}`
- Produces: `XcapGrabber::new()`——Task 7 的热键流程用 `Box<dyn ScreenGrabber>` 持有它。

- [ ] **Step 1: crate 骨架与依赖**

```toml
# crates/hawksnap-platform-mac/Cargo.toml
[package]
name = "hawksnap-platform-mac"
version.workspace = true
edition.workspace = true

[dependencies]
hawksnap-core.workspace = true
xcap = "0.4"
image.workspace = true
```

根 Cargo.toml `[workspace.dependencies]` 追加：

```toml
hawksnap-platform-mac = { path = "crates/hawksnap-platform-mac" }
xcap = "0.4"
```

- [ ] **Step 2: 写失败测试（含需权限的集成测试，默认 ignore）**

```rust
// lib.rs
#[cfg(test)]
mod tests {
    use hawksnap_core::capture::ScreenGrabber;

    #[test]
    #[ignore = "需要 macOS 屏幕录制权限；授权后 cargo test -- --ignored 运行"]
    fn real_grab_produces_decodable_png() {
        let mut g = hawksnap_platform_mac::XcapGrabber::new();
        let frames = g.grab_all().unwrap();
        assert!(!frames.is_empty());
        let f = &frames[0];
        let img = image::load_from_memory(&f.png).unwrap();
        assert_eq!(img.width(), (f.width as f64 * f.scale).round() as u32);
    }
}
```

- [ ] **Step 3: 实现**

```rust
// lib.rs（置于 tests 之前）
use hawksnap_core::capture::{CaptureError, ScreenFrame, ScreenGrabber};

pub struct XcapGrabber;

impl XcapGrabber {
    pub fn new() -> Self { XcapGrabber }
}

impl Default for XcapGrabber { fn default() -> Self { Self::new() } }

impl ScreenGrabber for XcapGrabber {
    fn grab_all(&mut self) -> Result<Vec<ScreenFrame>, CaptureError> {
        let monitors = xcap::Monitor::all().map_err(|e| CaptureError::Backend(e.to_string()))?;
        if monitors.is_empty() { return Err(CaptureError::NoDisplays); }
        let mut out = Vec::with_capacity(monitors.len());
        for m in monitors {
            let img = m.capture_image().map_err(|e| {
                if e.to_string().to_lowercase().contains("permission") {
                    CaptureError::Permission
                } else { CaptureError::Backend(e.to_string()) }
            })?;
            let mut png = Vec::new();
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|e| CaptureError::Backend(e.to_string()))?;
            out.push(ScreenFrame {
                display_id: m.id(),
                x: m.x(), y: m.y(),
                width: m.width(), height: m.height(),
                scale: m.scale_factor(),
                png,
            });
        }
        Ok(out)
    }
}
```

- [ ] **Step 4: 编译验证 + 手动跑权限测试**

Run: `cargo test -p hawksnap-platform-mac`
Expected: PASS（0 个运行，1 个 ignored）。
Run: `cargo test -p hawksnap-platform-mac -- --ignored`
Expected: 首次运行触发系统「屏幕录制」授权弹窗；授权并重跑后 PASS。若被拒绝：Expected FAIL with `CaptureError::Permission`。

- [ ] **Step 5: 提交**

```bash
git add crates/hawksnap-platform-mac Cargo.toml Cargo.lock
git commit -m "feat(platform-mac): xcap-backed ScreenGrabber with permission mapping"
```

---

### Task 6: PNG 裁剪 + 剪贴板 + 保存（core 纯函数）

**Files:**
- Create: `crates/hawksnap-core/src/crop.rs`
- Modify: `crates/hawksnap-core/src/lib.rs`（`pub mod crop;`）

**Interfaces:**
- Consumes: `geometry::Rect`
- Produces: `crop_png(png:&[u8], physical:&Rect)->Result<Vec<u8>,CropError>`——Task 7 的 `finish_capture` 命令调用；输入输出均为物理像素。

- [ ] **Step 1: 写失败测试**

```rust
// crop.rs
#[cfg(test)]
mod tests {
    use super::*;
    use hawksnap_core::geometry::Rect;

    fn solid_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn crops_to_exact_physical_rect() {
        // 4x4 全红图，裁 (1,1,2,2) → 2x2
        let out = crop_png(&solid_png(4, 4, [255, 0, 0, 255]), &Rect { x: 1, y: 1, w: 2, h: 2 }).unwrap();
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!((img.width(), img.height()), (2, 2));
    }

    #[test]
    fn out_of_bounds_returns_error() {
        let e = crop_png(&solid_png(4, 4, [0, 0, 0, 255]), &Rect { x: 3, y: 3, w: 5, h: 5 });
        assert!(matches!(e, Err(CropError::OutOfBounds)));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p hawksnap-core crop`
Expected: FAIL。

- [ ] **Step 3: 实现**

```rust
// crop.rs（置于 tests 之前）
use hawksnap_core::geometry::Rect;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CropError {
    #[error("source decode failed")]
    Decode,
    #[error("crop rect outside source")]
    OutOfBounds,
    #[error("encode failed")]
    Encode,
}

pub fn crop_png(png: &[u8], physical: &Rect) -> Result<Vec<u8>, CropError> {
    let img = image::load_from_memory(png).map_err(|_| CropError::Decode)?;
    let (sw, sh) = (img.width() as i32, img.height() as i32);
    if physical.x < 0 || physical.y < 0
        || physical.right() > sw || physical.bottom() > sh { return Err(CropError::OutOfBounds); }
    let cropped = img.crop_imm(
        physical.x as u32, physical.y as u32, physical.w, physical.h,
    );
    let mut out = Vec::new();
    cropped.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|_| CropError::Encode)?;
    Ok(out)
}
```

（`hawksnap-core` 内部引用自身路径：若 lib 名为 `hawksnap_core`，模块内 `use crate::geometry::Rect;`；跨 crate 用户 `use hawksnap_core::geometry::Rect;`。实现里统一用 `crate::geometry::Rect`。）

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p hawksnap-core`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/hawksnap-core
git commit -m "feat(core): physical-pixel png crop with bounds checks"
```

---

### Task 7: Tauri 命令层（capture_screens / finish_capture）

**Files:**
- Create: `app/src-tauri/src/commands.rs`
- Modify: `app/src-tauri/src/lib.rs`（注册命令）、`app/src-tauri/Cargo.toml`（依赖）、`app/src-tauri/tauri.conf.json`（asset protocol 白名单）

**Interfaces:**
- Consumes: `ScreenGrabber`/`ScreenFrame`、`crop_png`、`expand`/`NamingCtx`
- Produces（前端契约，Task 9 依赖）:
  - `#[derive(Serialize)] struct DisplayPayload { displayId: u64, x: i32, y: i32, width: u32, height: u32, scale: f64, pngUrl: String }`
  - 命令 `capture_screens() -> Result<Vec<DisplayPayload>, String>`（冻结帧 PNG 落盘到缓存目录，返回 `asset://` URL）
  - 命令 `finish_capture(req: FinishReq) -> Result<CaptureOutcome, String>`，其中
    `FinishReq { displayId: u64, rect: Rect /*逻辑坐标*/, copy: bool, save: bool }`
    `CaptureOutcome { copied: bool, savedTo: Option<String> }`

- [ ] **Step 1: 依赖与配置**

`app/src-tauri/Cargo.toml` 的 `[dependencies]`：

```toml
hawksnap-core.workspace = true
hawksnap-platform-mac.workspace = true
arboard = "3"
chrono = "0.4"
serde_json = "1"
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-global-shortcut = "2"
```

`tauri.conf.json` 的 `app.security.assetProtocol`：

```json
"assetProtocol": { "enable": true, "scope": ["$CACHE/**"] }
```

- [ ] **Step 2: 写失败测试（命令层纯逻辑：坐标换算与落盘命名）**

```rust
// commands.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_rect_maps_to_physical() {
        // 逻辑 (10,20,100,50) @2x → 物理 (20,40,200,100)
        let p = to_physical_rect(
            &hawksnap_core::geometry::Rect { x: 10, y: 20, w: 100, h: 50 }, 2.0);
        assert_eq!((p.x, p.y, p.w, p.h), (20, 40, 200, 100));
    }

    #[test]
    fn save_uses_template_and_avoids_collision() {
        let dir = std::env::temp_dir().join(format!("hs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = save_path_for(&dir, "Safari");
        assert!(first.to_string_lossy().contains("Safari"));
        std::fs::write(&first, b"1").unwrap();
        let second = save_path_for(&dir, "Safari");
        assert_ne!(first, second, "重名时必须生成不同文件名");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml`
Expected: FAIL（函数未定义）。

- [ ] **Step 4: 实现**

```rust
// commands.rs（置于 tests 之前）
use hawksnap_core::capture::ScreenGrabber;
use hawksnap_core::geometry::Rect;
use hawksnap_core::{crop::crop_png, naming::{expand, NamingCtx}};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPayload {
    pub display_id: u64, pub x: i32, pub y: i32,
    pub width: u32, pub height: u32, pub scale: f64, pub png_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishReq { pub display_id: u64, pub rect: Rect, pub copy: bool, pub save: bool }

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOutcome { pub copied: bool, pub saved_to: Option<String> }

pub fn to_physical_rect(logical: &Rect, scale: f64) -> Rect {
    Rect {
        x: (logical.x as f64 * scale).round() as i32,
        y: (logical.y as f64 * scale).round() as i32,
        w: (logical.w as f64 * scale).round() as u32,
        h: (logical.h as f64 * scale).round() as u32,
    }
}

pub fn save_path_for(dir: &std::path::Path, app_name: &str) -> std::path::PathBuf {
    let now = chrono::Local::now();
    let ctx = NamingCtx {
        app: app_name.to_string(),
        date: now.format("%Y-%m-%d").to_string(),
        time: now.format("%H%M%S").to_string(),
    };
    let name = expand("{app} {date} {time}", &ctx, |n| dir.join(format!("{n}.png")).exists());
    dir.join(format!("{name}.png"))
}

#[tauri::command]
pub async fn capture_screens(app: AppHandle) -> Result<Vec<DisplayPayload>, String> {
    let mut grabber = hawksnap_platform_mac::XcapGrabber::new();
    let frames = grabber.grab_all().map_err(|e| e.to_string())?;
    let cache = app.path().cache_dir().map_err(|e| e.to_string())?.join("hawksnap/grabs");
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(frames.len());
    for f in frames {
        let file = cache.join(format!("{}.png", f.display_id));
        std::fs::write(&file, &f.png).map_err(|e| e.to_string())?;
        out.push(DisplayPayload {
            display_id: f.display_id, x: f.x, y: f.y,
            width: f.width, height: f.height, scale: f.scale,
            png_url: format!("hawk-grab://{}", file.display()),
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn finish_capture(
    app: AppHandle, req: FinishReq,
) -> Result<CaptureOutcome, String> {
    // 1) 读冻结帧
    let cache = app.path().cache_dir().map_err(|e| e.to_string())?.join("hawksnap/grabs");
    let png = std::fs::read(cache.join(format!("{}.png", req.display_id))).map_err(|e| e.to_string())?;
    // 2) 逻辑→物理裁剪
    let frame_meta = FRAME_META.lock().unwrap();
    let scale = frame_meta.get(&req.display_id).map(|m| m.scale).ok_or("display not captured")?;
    let physical = to_physical_rect(&req.rect, scale);
    let cropped = crop_png(&png, &physical).map_err(|e| e.to_string())?;
    // 3) 剪贴板
    let mut copied = false;
    if req.copy {
        let img = image::load_from_memory(&cropped).map_err(|e| e.to_string())?;
        let (w, h) = (img.width(), img.height());
        let rgba = img.to_rgba8().into_raw();
        let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        cb.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: rgba.into() })
            .map_err(|e| e.to_string())?;
        copied = true;
    }
    // 4) 保存
    let mut saved_to = None;
    if req.save {
        let dir = dirs_picture_dir().ok_or("no picture dir")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = save_path_for(&dir, frontmost_app_name().unwrap_or_else(|| "HawkSnap".into()));
        std::fs::write(&path, &cropped).map_err(|e| e.to_string())?;
        saved_to = Some(path.to_string_lossy().into_owned());
    }
    Ok(CaptureOutcome { copied, saved_to })
}

use std::sync::Mutex;
use std::collections::HashMap;
static FRAME_META: Mutex<HashMap<u64, FrameMeta>> = Mutex::new(HashMap::new());
#[derive(Clone, Copy)] struct FrameMeta { scale: f64 }

fn dirs_picture_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Pictures/HawkSnap"))
}

fn frontmost_app_name() -> Option<String> { None /* Plan B 接 window 检测后替换 */ }
```

`app/src-tauri/src/lib.rs`：

```rust
mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::init())
        .invoke_handler(tauri::generate_handler![
            commands::capture_screens, commands::finish_capture
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

注意：`capture_screens` 成功后需把各显示器 `scale` 存入 `FRAME_META`（在循环里 `frame_meta.insert(f.display_id, FrameMeta{scale:f.scale});`）——实现时补上这一行（测试通过 `save_path_for`/`to_physical_rect` 覆盖，不依赖该静态表）。

- [ ] **Step 5: 运行确认通过**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml`
Expected: PASS（2 个测试）。

- [ ] **Step 6: 提交**

```bash
git add app/src-tauri
git commit -m "feat(app): capture_screens/finish_capture commands with crop+clipboard+save"
```

---

### Task 8: F1 热键 → 覆盖层窗口编排

**Files:**
- Create: `app/src-tauri/src/overlay.rs`
- Modify: `app/src-tauri/src/lib.rs`（挂接热键与窗口创建）

**Interfaces:**
- Consumes: `capture_screens` 返回的 `DisplayPayload`
- Produces: 覆盖层窗口 label 约定 `overlay-{displayId}`；Tauri 事件 `hawk://overlay-ready`（payload `DisplayPayload[]`）、`hawk://overlay-shown`（前端回发，性能埋点用）、`hawk://capture-done`（payload `CaptureOutcome`）。前端 Task 9 监听这些事件名。

- [ ] **Step 1: 写失败测试（热键→窗口编排的纯逻辑：payload 序列化契约）**

```rust
// overlay.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_serializes_camel_case() {
        let p = DisplayPayloadForTest {
            displayId: 7, x: 0, y: 0, width: 100, height: 50, scale: 2.0, pngUrl: "hawk-grab:///tmp/a.png".into(),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"displayId\":7"));
        assert!(s.contains("\"pngUrl\":\"hawk-grab:///tmp/a.png\""));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml overlay`
Expected: FAIL。

- [ ] **Step 3: 实现（热键注册 + 覆盖层窗口创建）**

```rust
// overlay.rs（置于 tests 之前）
use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPayloadForTest {
    pub display_id: u64, pub x: i32, pub y: i32,
    pub width: u32, pub height: u32, pub scale: f64, pub png_url: String,
}

pub fn register_hotkeys(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
    let gs = app.global_shortcut();
    let f1 = Shortcut::new(None, Code::F1);
    gs.register(f1).map_err(|e| format!("F1 注册失败（可能被占用）: {e}"))?;
    let app2 = app.clone();
    gs.on_shortcut(f1, move |_app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Err(e) = start_capture_flow(&app2) {
                let _ = tauri::Emitter::emit(app2, "hawk://capture-error", e);
            }
        }
    })?;
    Ok(())
}

pub fn start_capture_flow(app: &AppHandle) -> Result<(), String> {
    let displays = tauri::async_runtime::block_on(commands::capture_screens(app.clone()))?;
    for d in &displays {
        let label = format!("overlay-{}", d.display_id);
        if let Some(w) = app.get_webview_window(&label) { let _ = w.close(); }
        let win = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("overlay".into()))
            .position(d.x as f64, d.y as f64)
            .inner_size(d.width as f64, d.height as f64)
            .decorations(false).transparent(true).always_on_top(true)
            .skip_taskbar(true).resizable(false)
            .build().map_err(|e| e.to_string())?;
        let _ = win.set_focus();
    }
    app.emit("hawk://overlay-ready", displays).map_err(|e| e.to_string())
}

use crate::commands;
```

`lib.rs` 的 `run()` 中，Builder 追加：

```rust
.setup(|app| {
    use tauri::Manager;
    overlay::register_hotkeys(app.handle())?;
    Ok(())
})
```

（`overlay-ready` 事件在窗口 `overlay` 路由加载完成前可能先到——Task 9 前端在挂载时主动 `invoke('capture_screens_meta')`？不需要：改为前端挂载后 emit `hawk://overlay-shown`，Rust 侧收到后重发 payload。简化实现：`overlay-ready` 循环发 3 次、间隔 150ms，前端以收到的最后一次为准——记录在代码注释里。）

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml`
Expected: PASS。
手动冒烟：`npm run tauri dev`，授权后按 F1 → 出现透明全屏覆盖层窗口（Task 9 前是白/透明空窗）。

- [ ] **Step 5: 提交**

```bash
git add app/src-tauri
git commit -m "feat(app): F1 hotkey drives frozen-frame overlay windows per display"
```

---

### Task 9: 覆盖层前端（Canvas 冻结帧 + 框选 + 动作浮条）

**Files:**
- Create: `app/src/lib/tauri.ts`、`app/src/windows/overlay/index.tsx`、`app/src/windows/overlay/useSelection.ts`
- Modify: `app/src/main.tsx`（按路由参数区分窗口：URL query `?window=overlay`）、`app/index.html`（入口 query 透传——用 `?window=` 由 Rust 侧 `WebviewUrl::App("overlay?window=overlay".into())` 传入）

**Interfaces:**
- Consumes: 事件 `hawk://overlay-ready`（`DisplayPayload[]`）、命令 `finish_capture`（Task 7 契约）、`convertFileSrc`
- Produces: `useSelection(...)` 返回 `{rect, phase, pointerHandlers, confirm(), cancel()}`；回发 `hawk://overlay-shown`、`hawk://capture-done` 后自动关闭窗口（Rust 侧 Task 8 已有 `capture-done` 监听关窗——本任务前端 emit）。

- [ ] **Step 1: 写失败测试（useSelection 状态机）**

```ts
// app/src/windows/overlay/useSelection.test.tsx
import { act, renderHook } from "@testing-library/react";
import { useSelection } from "./useSelection";

describe("useSelection", () => {
  it("drag normalizes rect and tracks phases", () => {
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600 }));
    expect(result.current.phase).toBe("idle");
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 300, clientY: 200 } as PointerEvent));
    expect(result.current.phase).toBe("dragging");
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 100, clientY: 50 } as PointerEvent));
    expect(result.current.rect).toEqual({ x: 100, y: 50, w: 200, h: 150 });
    act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
    expect(result.current.phase).toBe("selected");
  });

  it("Enter confirms only when a rect exists", () => {
    const onConfirm = vi.fn();
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600, onConfirm }));
    act(() => result.current.confirm());
    expect(onConfirm).not.toHaveBeenCalled(); // 空选区不可确认
  });

  it("Esc cancels and resets", () => {
    const { result } = renderHook(() => useSelection({ width: 1000, height: 600 }));
    act(() => result.current.pointerHandlers.onPointerDown({ clientX: 10, clientY: 10 } as PointerEvent));
    act(() => result.current.pointerHandlers.onPointerMove({ clientX: 60, clientY: 40 } as PointerEvent));
    act(() => result.current.cancel());
    expect(result.current.phase).toBe("idle");
    expect(result.current.rect).toBeNull();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd app && npm test`
Expected: FAIL（useSelection 不存在）。

- [ ] **Step 3: 实现 useSelection**

```ts
// app/src/windows/overlay/useSelection.ts
import { useCallback, useEffect, useRef, useState } from "react";

export type SelRect = { x: number; y: number; w: number; h: number };
export type Phase = "idle" | "dragging" | "selected";

type Point = { clientX: number; clientY: number };

export function useSelection(opts: {
  width: number; height: number;
  onConfirm?: (r: SelRect) => void; onCancel?: () => void;
}) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [rect, setRect] = useState<SelRect | null>(null);
  const start = useRef<Point | null>(null);

  const norm = (a: Point, b: Point): SelRect => ({
    x: Math.max(0, Math.min(a.clientX, b.clientX)),
    y: Math.max(0, Math.min(a.clientY, b.clientY)),
    w: Math.abs(a.clientX - b.clientX),
    h: Math.abs(a.clientY - b.clientY),
  });

  const pointerHandlers = {
    onPointerDown: (e: Point) => { start.current = e; setPhase("dragging"); setRect(null); },
    onPointerMove: (e: Point) => {
      if (phase !== "dragging" || !start.current) return;
      setRect(norm(start.current, e));
    },
    onPointerUp: (_: Point) => {
      if (phase !== "dragging") return;
      setPhase(r => (r === "dragging" ? "selected" : r));
      start.current = null;
    },
  };

  const confirm = useCallback(() => {
    if (phase !== "selected" || !rect || rect.w < 2 || rect.h < 2) return;
    opts.onConfirm?.(rect);
  }, [phase, rect, opts]);

  const cancel = useCallback(() => {
    setPhase("idle"); setRect(null); start.current = null; opts.onCancel?.();
  }, [opts]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cancel();
      if (e.key === "Enter") confirm();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cancel, confirm]);

  return { phase, rect, pointerHandlers, confirm, cancel };
}
```

- [ ] **Step 4: 运行确认通过**

Run: `cd app && npm test`
Expected: PASS（3 个测试）。

- [ ] **Step 5: 实现覆盖层页面（Canvas + 浮条）**

```tsx
// app/src/windows/overlay/index.tsx
import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useSelection, SelRect } from "./useSelection";

type Display = { displayId: number; x: number; y: number; width: number; height: number; scale: number; pngUrl: string };

export default function OverlayWindow() {
  const [display, setDisplay] = useState<Display | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let off = () => {};
    listen<Display[]>("hawk://overlay-ready", (e) => {
      const mine = e.payload.find((d) => `overlay-${d.displayId}` === window.__LABEL__);
      if (mine) setDisplay(mine);
    }).then((un) => { off = un; });
    return () => off();
  }, []);

  const { phase, rect, pointerHandlers, confirm, cancel } = useSelection({
    width: display?.width ?? 0, height: display?.height ?? 0,
    onConfirm: async (r: SelRect) => {
      if (!display) return;
      const outcome = await invoke("finish_capture", {
        req: { displayId: display.displayId, rect: r, copy: true, save: true },
      });
      await invoke("emit_capture_done", { outcome }); // 见 Step 6 注释
    },
    onCancel: () => invoke("cancel_capture", { displayId: display?.displayId }),
  });

  // 冻结帧 + 遮罩 + 选区：rect/display 变化时重绘
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !display) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    canvas.width = display.width;
    canvas.height = display.height;
    const img = new Image();
    img.onload = () => {
      ctx.drawImage(img, 0, 0, display.width, display.height);
      ctx.fillStyle = "rgba(0,0,0,0.35)";
      ctx.fillRect(0, 0, display.width, display.height);
      if (rect && rect.w >= 2 && rect.h >= 2) {
        ctx.drawImage(img, rect.x, rect.y, rect.w, rect.h, rect.x, rect.y, rect.w, rect.h);
        ctx.strokeStyle = "#FFB020";
        ctx.lineWidth = 2;
        ctx.strokeRect(rect.x + 1, rect.y + 1, rect.w - 2, rect.h - 2);
        const label = `${rect.w}×${rect.h}`;
        ctx.font = "13px system-ui, sans-serif";
        const tw = ctx.measureText(label).width + 12;
        ctx.fillStyle = "#FFFFFF";
        ctx.fillRect(rect.x + rect.w - tw, rect.y + rect.h + 6, tw, 20);
        ctx.fillStyle = "#0D1522";
        ctx.fillText(label, rect.x + rect.w - tw + 6, rect.y + rect.h + 20);
      }
    };
    img.src = convertFileSrc(display.pngUrl.replace("hawk-grab://", ""));
  }, [rect, display, phase]);

  if (!display) return null;
  return (
    <canvas ref={canvasRef}
      style={{ position: "fixed", inset: 0, cursor: "crosshair" }}
      {...pointerHandlers}
      onDoubleClick={() => confirm()} />
  );
}
```

（说明：`window.__LABEL__` 由 Rust 侧在窗口创建后 `webview.eval("window.__LABEL__='overlay-{id}'")` 注入；`emit_capture_done`/`cancel_capture` 是 Task 8 `overlay.rs` 内追加的两个薄命令——转发事件并关闭对应覆盖层窗口。`pngUrl` 传输约定：命令返回 `hawk-grab://<绝对路径>`，前端剥前缀后经 `convertFileSrc` 转为 asset 协议加载。）

- [ ] **Step 6: Rust 侧补薄命令**

```rust
// overlay.rs 追加
#[tauri::command]
pub async fn cancel_capture(app: AppHandle, display_id: u64) -> Result<(), String> {
    close_overlay(&app, display_id)
}

#[tauri::command]
pub async fn emit_capture_done(app: AppHandle, outcome: commands::CaptureOutcome) -> Result<(), String> {
    app.emit("hawk://capture-done", &outcome).map_err(|e| e.to_string())
}

fn close_overlay(app: &AppHandle, display_id: u64) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&format!("overlay-{display_id}")) {
        w.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}
```

`lib.rs` 注册这两个命令；`hawk://capture-done` 到达时（overlay.rs 顶部 `listen` 于 setup）调用 `close_overlay`。

- [ ] **Step 7: 运行测试 + 手动冒烟**

Run: `cd app && npm test` → PASS。
Run: `npm run tauri dev` → 按 F1 → 冻结帧出现 → 拖拽框选（尺寸标签跟随）→ Enter → 剪贴板可粘贴出截图、`~/Pictures/HawkSnap/` 出现 PNG、覆盖层关闭；Esc → 覆盖层关闭无产物。

- [ ] **Step 8: 提交**

```bash
git add app/src app/src-tauri
git commit -m "feat(overlay): frozen-frame canvas selection with confirm/cancel actions"
```

---

### Task 10: 托盘常驻 + 退出

**Files:**
- Modify: `app/src-tauri/src/lib.rs`、`app/src-tauri/tauri.conf.json`（图标资源）、`app/src-tauri/icons/`（放入 design/logo/final 导出的 icon png）

**Interfaces:**
- Consumes: `start_capture_flow`
- Produces: 托盘菜单项「截图 (F1)」「退出」；macOS 激活策略 Accessory（无 Dock 图标、不影响覆盖层聚焦）。

- [ ] **Step 1: 图标就位**

```bash
cd /Users/guxiaolong/workspace/HawkSnap
rsvg-convert -w 512 -h 512 design/logo/final/icon.svg -o app/src-tauri/icons/icon.png
rsvg-convert -w 32 -h 32 design/logo/final/tray-mono.svg -o app/src-tauri/icons/tray-32.png
# 用 `npm run tauri icon app/src-tauri/icons/icon.png` 生成全套尺寸
```

- [ ] **Step 2: 实现（lib.rs setup 内）**

```rust
{
    use tauri::{menu::{Menu, MenuItem}, tray::TrayIconBuilder};
    let shot = MenuItem::with_id(app, "shot", "截图 (F1)", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 HawkSnap", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&shot, &quit])?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .on_menu_event(|app, ev| match ev.id.as_ref() {
            "shot" => { let _ = overlay::start_capture_flow(app); }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}
```

- [ ] **Step 3: 手动验证**

Run: `npm run tauri dev` → 托盘出现图标 → 主窗口关闭后应用仍驻留 → 菜单「截图 (F1)」等效热键 → 「退出」进程结束（`ps aux | grep hawksnap` 无残留）。

- [ ] **Step 4: 提交**

```bash
git add app/src-tauri
git commit -m "feat(tray): resident tray menu with capture shortcut and quit"
```

---

### Task 11: 性能埋点与基准脚本（G3 证据）

**Files:**
- Create: `app/src-tauri/src/perf.rs`、`scripts/bench-latency.sh`

**Interfaces:**
- Produces: `perf::mark(stage:&str)`（进程内时间戳）；CSV 输出 `~/Library/Caches/com.hawksnap.app/perf/*.csv`，列 `stage,ts_ms`。阶段名固定：`hotkey`、`grab_done`、`overlay_created`、`overlay_shown`、`clipboard_done`。

- [ ] **Step 1: 实现 perf.rs**

```rust
// perf.rs
use std::io::Write;
pub fn mark(stage: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let dir = std::env::temp_dir().join("hawksnap-perf");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
        .open(dir.join(format!("{}.csv", std::process::id()))) {
        let _ = writeln!(f, "{stage},{ts}");
    }
}
```

在 `start_capture_flow` 首行 `perf::mark("hotkey")`、抓屏后 `mark("grab_done")`、窗口 build 后 `mark("overlay_created")`；前端 `hawk://overlay-shown` 处 `mark("overlay_shown")`；`finish_capture` 剪贴板写入后 `mark("clipboard_done")`。

- [ ] **Step 2: 基准脚本**

```bash
#!/usr/bin/env bash
# scripts/bench-latency.sh — 手动跑 20 次 F1 截图后统计 P90（spec G3）
set -euo pipefail
echo "连按 20 次 F1（每次 Enter 确认），完成后回车继续..."
read -r
python3 - <<'EOF'
import csv, glob, statistics
rows = [list(csv.reader(open(f))) for f in glob.glob("/tmp/hawksnap-perf/*.csv")]
flat = [r for run in rows for r in run]
by = {}
for stage, ts in flat: by.setdefault(stage, []).append(int(ts))
def p90(xs): return sorted(xs)[int(len(xs)*0.9)]
h2o = [b-a for a,b in zip(by["hotkey"], by["overlay_shown"])]
c2c = [b-a for a,b in zip(by["overlay_shown"], by["clipboard_done"])]
print(f"热键→覆盖层 P90 = {p90(h2o)}ms (目标 ≤150)")
print(f"确认→剪贴板 P90 = {p90(c2c)}ms (目标 ≤100)")
EOF
```

- [ ] **Step 3: 手动验证 + 提交**

Run: `chmod +x scripts/bench-latency.sh && ./scripts/bench-latency.sh` → 输出两行 P90，均达标（dev 模式可能超标，记录数值并在 release 复测：`npm run tauri build` 后测产物）。

```bash
git add app/src-tauri/src/perf.rs scripts
git commit -m "feat(perf): latency instrumentation and P90 bench script"
```

---

### Task 12: 收尾——README、体积检查、tag

**Files:**
- Create: `README.md`（仓库根）、`scripts/check-size.sh`

- [ ] **Step 1: check-size.sh（G1）**

```bash
#!/usr/bin/env bash
# scripts/check-size.sh — 安装包体积断言（G1: ≤15MB）
set -euo pipefail
DMG=$(ls app/src-tauri/target/release/bundle/dmg/*.dmg 2>/dev/null | head -1)
[ -z "$DMG" ] && { echo "先运行 npm run tauri build"; exit 1; }
SIZE_MB=$(du -m "$DMG" | cut -f1)
echo "dmg = ${SIZE_MB}MB"
[ "$SIZE_MB" -le 15 ] || { echo "超过 15MB 预算"; exit 1; }
```

- [ ] **Step 2: README**

内容必须包含：项目一句话简介、Slogan、开发环境要求（macOS 12.3+、Node 20+、Rust stable）、`npm install`/`npm run tauri dev`、首次运行需授予「屏幕录制」权限的说明、当前进度（Plan A 完成 = F1 闭环）、后续计划链接到 `docs/superpowers/plans/`。

- [ ] **Step 3: 全量验证 + 提交 + tag**

```bash
cargo test --workspace && cd app && npm test && npm run build && cd ..
./scripts/check-size.sh
git add -A && git commit -m "chore: plan-a closeout readme and size gate" && git tag v0.1.0-alpha.1
```

---

## 后续计划（M1 剩余，另立文档）

- **Plan B**：智能吸附（AXUIElement 悬停高亮）、窗口/全屏模式、多显示器覆盖层、延时截图、方向键微调+磁吸接入、**选区确认动作浮条**（复制/保存按钮化 + 预留贴图/标注/OCR/GIF 动作位）。
- **Plan C**：贴图 Pin 窗口族（含 F2 热键）、历史简版（SQLite+缩略图+列表 UI）、主面板与设置页（热键自定义、输出格式 JPEG 质量、保存目录）。
- **Plan D**：i18n（zh-CN/en-US + key 覆盖 CI）、自动更新（tauri-plugin-updater）、macOS 权限引导、Windows/Linux CI 矩阵、G1–G8 全量验收 → 发布 0.1。

每份计划在上一份完成、验收数据归档后编写，保证任务基于真实代码而非假设。
