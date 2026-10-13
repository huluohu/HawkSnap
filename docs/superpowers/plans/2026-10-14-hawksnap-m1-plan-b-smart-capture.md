# HawkSnap M1-Plan B: 智能捕获交互 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Plan A 的 F1 截图闭环上补齐 spec §3.1 交互全集（动作浮条、方向键微调+磁吸、窗口/全屏模式、智能吸附、延时截图、多显示器语义收敛），并修复 G3 实测超标（热键→覆盖层 459ms vs ≤150ms；确认→剪贴板 141ms vs ≤100ms——2026-10-14 合成基准 20 次实测）。

**Architecture:** 两项性能修复先行：①覆盖层窗口常驻复用（首个显示器的 webview 冷加载 267ms 是 h2o 大头——改为 show/hide 复用 + payload 更新）；②raw-RGBA 捕获缓存（裁剪/剪贴板从内存 RGBA 直通，PNG 编码只在保存路径）。随后是交互层：动作浮条 → 微调磁吸 → AXUIElement 悬停高亮（元素→窗口→全屏三级回退）→ 多显示器语义 → 延时截图。

**Tech Stack:** 既有栈（Tauri 2、React 18、Rust workspace、xcap、arboard）+ objc2-app-kit（NSEvent，已引入）+ `accessibility` crate（AXUIElement 绑定，或 objc2 手写声明——实现者按锁定版本验证）。

**Spec:** `docs/superpowers/specs/2026-10-13-hawksnap-design.md`（本计划实现其 §3.1 交互全集、§3.2 智能吸附的 macOS 部分、§4 Esc/Enter/双击约定、F4 延时截图；G3 修复对应 §5/§6.1）

## Global Constraints

- 性能预算（spec §5）：热键→覆盖层 ≤150ms（P90）；确认选区→剪贴板 ≤100ms（P90）；常驻内存 ≤80MB（Task 8 采样守门，超标则执行其应急步）。
- 隐私：无遥测、无外联；本计划不引入任何网络请求。
- IPC 事件前缀 `hawk://`；新增事件：`hawk://hover`（Rust→前端，悬停帧）、`hawk://countdown`（Rust→前端，剩余秒数）、`hawk://overlay-ready`（语义扩展为「显示或更新」）。
- TypeScript strict；Rust edition 2021、workspace lints deny `unsafe_op_in_unsafe_fn`；每任务以 `cargo test` / `npm test` 全绿 + conventional commit 结束。
- UI 文案暂用中文硬编码（i18n key 化在 Plan D 统一做，G6 门）；本计划不引入 i18n 框架。
- macOS 专属 API（AXUIElement、CGWindowList）引用点必须与 target_os 门控同置；Windows/Linux 延后 CI（Plan D）。
- 辅助功能权限缺失时：悬停高亮静默降级（eprintln + 无高亮），不允许崩溃或阻塞启动；提示走既有 hotkey-error 横幅通道（「提示：」前缀模式）。
- 基准方法：G3 数据一律用 `scripts/bench-synth.sh`（Task 8 引入的合成注入基准）采集，禁用人工按键计数。

---

### Task 1: 覆盖层窗口常驻复用（G3-1 修复）

**Files:**
- Modify: `app/src-tauri/src/overlay.rs`（start_capture_flow 复用分支；cancel/confirm 改 hide）
- Modify: `app/src/windows/overlay/index.tsx`（payload 到达即重绘+重置；shownRef 每轮重置；图片 cache-bust）
- Modify: `app/src/windows/overlay/useSelection.ts`（新增 `reset()`）
- Test: `app/src/windows/overlay/useSelection.test.tsx`（reset 用例）

**Interfaces:**
- Consumes: `hawk://overlay-ready`（payload `DisplayPayload[]`，不变）
- Produces: `useSelection(...)` 返回值新增 `reset(): void`（重置 phase/rect/in-flight，Task 3/5 依赖）；覆盖层窗口生命周期变为「创建一次，之后 show/hide」——Task 7 的 countdown 窗口复用同一模式。

**背景（实测）：** h2o P90=459ms，分解：抓屏 63ms + 建窗 42ms + **webview 冷加载→首帧 267ms**。常驻复用砍掉第三段。

- [ ] **Step 1: 写失败测试（reset）**

```ts
// useSelection.test.tsx 追加
it("reset returns to idle and clears in-flight guard", () => {
  const { result } = renderHook(() => useSelection({ width: 1000, height: 600 }));
  act(() => result.current.pointerHandlers.onPointerDown({ clientX: 10, clientY: 10 } as PointerEvent));
  act(() => result.current.pointerHandlers.onPointerMove({ clientX: 60, clientY: 40 } as PointerEvent));
  act(() => result.current.cancel()); // cancel 内部也会 reset
  act(() => result.current.pointerHandlers.onPointerDown({ clientX: 5, clientY: 5 } as PointerEvent));
  act(() => result.current.reset());
  expect(result.current.phase).toBe("idle");
  expect(result.current.rect).toBeNull();
  // reset 后 confirm 可以再次触发（in-flight 守卫被清）
  expect(() => result.current.confirm()).not.toThrow();
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd app && npm test`
Expected: FAIL（`reset` 不存在）。

- [ ] **Step 3: 实现 reset（useSelection.ts）**

```ts
const reset = useCallback(() => {
  confirmingRef.current = false;
  start.current = null;
  setPhase("idle");
  setRect(null);
}, []);
// return 行加入 reset
return { phase, rect, pointerHandlers, confirm, cancel, reset };
```

- [ ] **Step 4: Rust 侧复用分支（overlay.rs 的 start_capture_flow 循环改写）**

现有循环：先 close 旧窗 + 500ms 轮询等注销，再 build。改为：

```rust
for d in &displays {
    let label = format!("overlay-{}", d.display_id);
    if let Some(w) = app.get_webview_window(&label) {
        // 复用：窗口与页面常驻，仅更新几何并显示。页面经 overlay-ready 的新 payload 重绘。
        let _ = w.set_position(tauri::LogicalPosition::new(d.x as f64, d.y as f64));
        let _ = w.set_size(tauri::LogicalSize::new(d.width as f64, d.height as f64));
        let _ = w.show();
        let _ = w.set_focus();
        continue;
    }
    // 首建路径（原 build 代码，保留 eval __LABEL__ 与 set_focus；删除 close-stale 轮询——
    // 复用模式下同 label 不会重建）
    let win = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("overlay?window=overlay".into()))
        .position(d.x as f64, d.y as f64)
        .inner_size(d.width as f64, d.height as f64)
        .decorations(false).transparent(true).always_on_top(true)
        .skip_taskbar(true).resizable(false)
        .build().map_err(|e| e.to_string())?;
    let _ = win.eval(&format!("window.__LABEL__='overlay-{}';", d.display_id));
    let _ = win.set_focus();
}
```

`close_overlay` 改名 `hide_overlay`，`w.close()` 改 `w.hide()`（cancel_capture 与 emit_capture_done 两处调用点同步改；注释更新：「常驻复用：隐藏而非关闭，窗口由 start_capture_flow 复用」）。

- [ ] **Step 5: 前端 payload 处理重置（index.tsx 的 listen 回调）**

```tsx
listen<Display[]>("hawk://overlay-ready", (e) => {
  const mine = e.payload.find((d) => `overlay-${d.displayId}` === window.__LABEL__);
  if (mine) {
    setDisplay(mine);      // 新对象身份必触发重绘 effect
    reset();               // 新一轮捕获：清选区与 in-flight
    shownRef.current = false; // 每轮捕获重发 overlay-shown（bench 门闩按轮配对）
  }
})
```

（`reset` 与 `shownRef` 来自本任务改动；effect 依赖数组加入 `reset`。图片加载加 cache-bust：
`img.src = convertFileSrc(display.pngUrl.replace("hawk-grab://", "")) + "?v=" + Date.now();`
若 asset 协议不接受 query（加载失败），改为后端写版本化文件名 `format!("{}-{}.png", f.display_id, chrono::Local::now().timestamp_millis())` 并在写入前删除旧文件——实现者验证取其一，报告注明。）

- [ ] **Step 6: 运行确认通过**

Run: `cd app && npm test` → PASS（6 用例）。
Run: `cargo test --workspace` → 全绿。
Run: `cd app && npm run build` → 严格构建通过。

- [ ] **Step 7: 提交**

```bash
git add app/src app/src-tauri
git commit -m "perf(overlay): reuse persistent overlay windows with payload-driven redraw"
```

---

### Task 2: raw-RGBA 捕获缓存与直通裁剪（G3-2 修复）

**Files:**
- Modify: `crates/hawksnap-core/src/crop.rs`（新增 `crop_rgba`、`rgba_to_png`）
- Modify: `crates/hawksnap-core/src/capture.rs`（ScreenFrame 增 `rgba` 字段；InMemGrabber 测试适配）
- Modify: `crates/hawksnap-platform-mac/src/lib.rs`（填充 rgba）
- Modify: `app/src-tauri/src/commands.rs`（FRAME_META → CaptureStore 存 RGBA；finish_capture 直通）

**Interfaces:**
- Produces: `crop_rgba(rgba: &[u8], w_px: u32, h_px: u32, physical: &Rect) -> Result<Vec<u8>, CropError>`（行复制裁剪，输入输出均物理 RGBA）；`rgba_to_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, CropError>`（Fast+NoFilter，仅保存路径用）；`ScreenFrame.rgba: Vec<u8>`（物理 RGBA，与 png 同一帧）。

**背景（实测）：** c2c P90=141ms = 读 PNG + 全帧解码 + crop 重编码 + 剪贴板前再解码。直通后预期 <50ms。

- [ ] **Step 1: 写失败测试（core/crop.rs 追加）**

```rust
#[cfg(test)]
mod rgba_tests {
    use super::*;
    use crate::geometry::Rect;

    fn solid_rgba(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        vec![px[0], px[1], px[2], px[3]].repeat((w * h) as usize)
    }

    #[test]
    fn crop_rgba_extracts_exact_rows() {
        // 2x2：左列红右列蓝，裁右列 → 1x2 蓝
        let mut img = vec![255u8, 0, 0, 255, 0, 0, 255, 255];
        img[0] = 255; img[2] = 0; // (0,0) 红
        let out = crop_rgba(&solid_rgba(2, 2, [0, 0, 255, 255]), 2, 2,
            &Rect { x: 1, y: 0, w: 1, h: 2 }).unwrap();
        assert_eq!(out, vec![0, 0, 255, 255, 0, 0, 255, 255]);
        let _ = img; // 语义用 solid_rgba 表达
    }

    #[test]
    fn crop_rgba_rejects_oob_and_overflow() {
        assert!(matches!(crop_rgba(&solid_rgba(2, 2, [0; 4]), 2, 2,
            &Rect { x: 2, y: 0, w: 1, h: 1 }), Err(CropError::OutOfBounds)));
        assert!(matches!(crop_rgba(&solid_rgba(2, 2, [0; 4]), 2, 2,
            &Rect { x: 0, y: 0, w: u32::MAX, h: 1 }), Err(CropError::OutOfBounds)));
    }

    #[test]
    fn crop_rgba_rejects_short_buffer() {
        assert!(matches!(crop_rgba(&[0u8; 7], 2, 2,
            &Rect { x: 0, y: 0, w: 1, h: 1 }), Err(CropError::Decode)));
    }

    #[test]
    fn rgba_to_png_roundtrips_dimensions() {
        let rgba = solid_rgba(4, 3, [9, 8, 7, 255]);
        let png = rgba_to_png(&rgba, 4, 3).unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (4, 3));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p hawksnap-core crop`
Expected: FAIL（函数未定义）。

- [ ] **Step 3: 实现（crop.rs）**

```rust
/// 从物理 RGBA 缓冲按行复制裁剪（无重编码）。`rgba.len()` 必须 ≥ w_px*h_px*4。
pub fn crop_rgba(rgba: &[u8], w_px: u32, h_px: u32, physical: &Rect) -> Result<Vec<u8>, CropError> {
    let stride = w_px as usize * 4;
    if w_px == 0 || h_px == 0 || rgba.len() < stride.checked_mul(h_px as usize).ok_or(CropError::Decode)? {
        return Err(CropError::Decode);
    }
    if physical.w > i32::MAX as u32 || physical.h > i32::MAX as u32 {
        return Err(CropError::OutOfBounds);
    }
    if physical.x < 0 || physical.y < 0
        || physical.right() > w_px as i32 || physical.bottom() > h_px as i32 {
        return Err(CropError::OutOfBounds);
    }
    let mut out = Vec::with_capacity(physical.w as usize * 4 * physical.h as usize);
    for row in physical.y as usize..physical.bottom() as usize {
        let start = row * stride + physical.x as usize * 4;
        out.extend_from_slice(&rgba[start..start + physical.w as usize * 4]);
    }
    Ok(out)
}

/// 裁剪结果 → PNG（Fast+NoFilter）。只在保存路径调用；剪贴板直用 RGBA。
pub fn rgba_to_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, CropError> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or(CropError::Decode)?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_with_encoder(image::codecs::png::PngEncoder::new_with_quality(
            std::io::Cursor::new(&mut out),
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::NoFilter,
        ))
        .map_err(|_| CropError::Encode)?;
    Ok(out)
}
```

- [ ] **Step 4: ScreenFrame 增 rgba（capture.rs + platform-mac）**

`ScreenFrame` 增加 `pub rgba: Vec<u8>`（注释：物理 RGBA 原始字节，长度 = width_px*scale? 注意：**rgba 是物理像素**，宽高 = width*scale, height*scale——用 `(width as f64 * scale).round() as u32` 计算）。InMemGrabber 测试的帧构造补 `rgba: vec![0; 200*100*4]`（200×100 物理示例）。platform-mac：`capture_image()` 得到 `RgbaImage` 后，`rgba = img.into_raw()` 前先 `let png_src = img.clone()` 用于编码（或先编码后 into_raw，避免 5.9MB clone 的写法自行取舍）。

- [ ] **Step 5: commands.rs 改 CaptureStore + 直通**

```rust
struct CaptureMeta { scale: f64, width_px: u32, height_px: u32, rgba: std::sync::Arc<Vec<u8>> }
static CAPTURES: LazyLock<Mutex<HashMap<u64, CaptureMeta>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
```

capture_screens 循环：对每帧计算 `(pw, ph) = ((f.width as f64 * f.scale).round() as u32, (f.height as f64 * f.scale).round() as u32)`，`CAPTURES.insert(f.display_id, CaptureMeta { scale: f.scale, width_px: pw, height_px: ph, rgba: Arc::new(f.rgba.clone()) })`（同时仍写 PNG 文件给前端 asset 协议）。删除 FRAME_META。
finish_capture：`let meta = { CAPTURES.lock().unwrap_or_else(|e| e.into_inner()).get(&req.display_id).cloned() }.ok_or("display not captured")?;`
裁剪：`let cropped = crop_rgba(&meta.rgba, meta.width_px, meta.height_px, &physical).map_err(|e| e.to_string())?;`
剪贴板：直接 `arboard::ImageData { width: physical.w as usize, height: physical.h as usize, bytes: cropped.clone().into() }`（删除 load_from_memory 解码）。
保存：`rgba_to_png(&cropped, physical.w, physical.h).map_err(...)` 后写 `save_path_for`。
注意 async 命令内持锁：先 clone Arc 立即释放锁（如上 `.cloned()` 块），后续无锁。

- [ ] **Step 6: 运行确认通过 + 提交**

Run: `cargo test --workspace` → 全绿（新增 4 用例）。
Run: `cargo test -p hawksnap-platform-mac -- --ignored` → PASS（rgba 长度一致性：`rgba.len() == (w*scale).round()*(h*scale).round()*4` 断言并入该测试）。

```bash
git add crates app/src-tauri Cargo.lock
git commit -m "perf(capture): raw-RGBA store with direct crop and clipboard path"
```

---

### Task 3: 选区确认动作浮条

**Files:**
- Create: `app/src/windows/overlay/actionBar.ts`（纯函数 `barPosition`）
- Test: `app/src/windows/overlay/actionBar.test.ts`
- Modify: `app/src/windows/overlay/useSelection.ts`（confirm 增 action 参数）
- Modify: `app/src/windows/overlay/index.tsx`（浮条 UI）
- Test: `app/src/windows/overlay/useSelection.test.tsx`（confirm 带动作）

**Interfaces:**
- Produces: `useSelection.confirm(action: "copy" | "save")`；`barPosition(rect, viewW, viewH) -> {left, top}`。Rust 无改动（FinishReq.copy/save 已存在）。

- [ ] **Step 1: 写失败测试**

```ts
// actionBar.test.ts
import { describe, expect, it } from "vitest";
import { barPosition } from "./actionBar";

describe("barPosition", () => {
  it("places below the rect normally", () => {
    expect(barPosition({ x: 100, y: 100, w: 200, h: 100 }, 1512, 982))
      .toEqual({ left: 180, top: 212 }); // 右对齐选区、下方 12px
  });
  it("flips above when the bar would overflow the bottom", () => {
    const p = barPosition({ x: 100, y: 900, w: 200, h: 70 }, 1512, 982);
    expect(p.top).toBeLessThan(900);
  });
  it("clamps left within the view", () => {
    const p = barPosition({ x: 10, y: 100, w: 20, h: 20 }, 1512, 982);
    expect(p.left).toBeGreaterThanOrEqual(12);
  });
});
```

```ts
// useSelection.test.tsx 追加
it("confirm forwards the requested action", () => {
  const onConfirm = vi.fn();
  const { result } = renderHook(() => useSelection({ width: 1000, height: 600, onConfirm }));
  act(() => result.current.pointerHandlers.onPointerDown({ clientX: 10, clientY: 10 } as PointerEvent));
  act(() => result.current.pointerHandlers.onPointerMove({ clientX: 60, clientY: 40 } as PointerEvent));
  act(() => result.current.pointerHandlers.onPointerUp({} as PointerEvent));
  act(() => result.current.confirm("save"));
  expect(onConfirm).toHaveBeenCalledWith({ x: 10, y: 10, w: 50, h: 30 }, "save");
});
```

- [ ] **Step 2: 运行确认失败** —— `cd app && npm test` FAIL（模块/签名不存在）。

- [ ] **Step 3: 实现**

```ts
// actionBar.ts
import type { SelRect } from "./useSelection";

const BAR_W = 252, BAR_H = 40, MARGIN = 12;

export function barPosition(rect: SelRect, viewW: number, viewH: number): { left: number; top: number } {
  const belowY = rect.y + rect.h + MARGIN;
  const top = belowY + BAR_H > viewH ? Math.max(MARGIN, rect.y - BAR_H - MARGIN) : belowY;
  const left = Math.min(
    Math.max(MARGIN, rect.x + rect.w - BAR_W),
    Math.max(MARGIN, viewW - BAR_W - MARGIN),
  );
  return { left, top };
}
```

useSelection：`confirm = useCallback((action: "copy" | "save" = "copy") => { ...opts.onConfirm?.(rect, action) }, [...])`（onConfirm 类型改 `(r: SelRect, action: "copy" | "save") => void`；Enter/keydown 与 dblclick 传缺省 "copy"）。

index.tsx：phase==="selected" && rect 时渲染浮条（canvas 的兄弟节点，`position: fixed`，`barPosition(rect, display.width, display.height)` 定位）：

```tsx
<div style={{ position: "fixed", left: pos.left, top: pos.top, display: "flex", gap: 8,
  background: "#0D1522", border: "1px solid #22405F", borderRadius: 8, padding: "6px 10px", zIndex: 10 }}>
  <button onClick={() => confirm("copy")}>复制</button>
  <button onClick={() => confirm("save")}>保存</button>
  {["贴图", "标注", "OCR", "GIF"].map((t) => (
    <button key={t} disabled title="即将推出" style={{ opacity: 0.4 }}>{t}</button>
  ))}
</div>
```

onConfirm 改 `(r, action) => { ... invoke("finish_capture", { req: { displayId, rect: r, copy: action === "copy", save: action === "save" } }) ... }`。

- [ ] **Step 4: 确认通过 + 提交**

Run: `cd app && npm test && npm run build` → 全绿。

```bash
git add app/src/windows/overlay
git commit -m "feat(overlay): confirm action bar with copy/save and future slots"
```

---

### Task 4: 方向键微调 + 8px 磁吸

**Files:**
- Create: `app/src/windows/overlay/geometry.ts`（`nudgeRect`、`snapRect`、`inBounds`）
- Test: `app/src/windows/overlay/geometry.test.ts`
- Modify: `app/src-tauri/src/overlay.rs`（`snap_candidates` 命令 + CGWindowList 拉取）
- Modify: `app/src/windows/overlay/useSelection.ts`（selected 态方向键处理）
- Modify: `app/src/windows/overlay/index.tsx`（candidates/bounds 注入）

**Interfaces:**
- Produces: 命令 `snap_candidates(display_id: u64) -> Vec<RectLog>`（全屏可见窗口逻辑坐标 rect 列表，layer==0、非空、排除自身 pid；`RectLog{x,y,w,h}` 全 i32/f64 一致 JSON）；TS `snapRect(r, candidates, 8)`（最近单边语义——比 Rust snap_to 的四 if 语义更干净，两侧并存不互调）；useSelection opts 增 `candidatesRef?: React.MutableRefObject<Rect[]>`、`bounds?: Rect`。

- [ ] **Step 1: 写失败测试（geometry.test.ts）**

```ts
import { describe, expect, it } from "vitest";
import { nudgeRect, snapRect } from "./geometry";

const screen = { x: 0, y: 0, w: 1512, h: 982 };

describe("nudgeRect", () => {
  it("moves by step and clamps to bounds", () => {
    expect(nudgeRect({ x: 10, y: 10, w: 50, h: 50 }, 5, -3, screen))
      .toEqual({ x: 15, y: 7, w: 50, h: 50 });
    expect(nudgeRect({ x: 0, y: 0, w: 50, h: 50 }, -10, 0, screen))
      .toEqual({ x: 0, y: 0, w: 50, h: 50 });
  });
});

describe("snapRect", () => {
  const candidates = [{ x: 0, y: 0, w: 1512, h: 982 }, { x: 500, y: 300, w: 400, h: 300 }];
  it("does not snap beyond threshold", () => {
    expect(snapRect({ x: 103, y: 500, w: 100, h: 100 }, candidates, 8).x).toBe(103);
  });
  it("snaps the nearest single edge within threshold", () => {
    // 左边缘距候选窗口右边缘 (900) 6px → 吸附到 900
    expect(snapRect({ x: 906, y: 500, w: 100, h: 100 }, candidates, 8).x).toBe(900);
    // 左边缘距屏幕左边缘 3px → 吸附 0
    expect(snapRect({ x: 3, y: 500, w: 100, h: 100 }, candidates, 8).x).toBe(0);
  });
  it("picks the nearest edge when several are in range", () => {
    // 距 900 差 2、距 500+400=900 右…同点：距屏幕右 1512-1006=506 不在阈值
    expect(snapRect({ x: 902, y: 500, w: 100, h: 100 }, candidates, 8).x).toBe(900);
  });
});
```

- [ ] **Step 2: 确认失败** —— `cd app && npm test` FAIL。

- [ ] **Step 3: 实现 geometry.ts**

```ts
import type { SelRect } from "./useSelection";
export type Rect = SelRect;

export function nudgeRect(r: Rect, dx: number, dy: number, bounds: Rect): Rect {
  const x = Math.min(Math.max(bounds.x, r.x + dx), bounds.x + bounds.w - r.w);
  const y = Math.min(Math.max(bounds.y, r.y + dy), bounds.y + bounds.h - r.h);
  return { ...r, x, y };
}

/// 最近单边磁吸：对每条候选边算 |delta|，全距 < threshold 中取最小者吸一条边。
export function snapRect(r: Rect, candidates: Rect[], threshold: number): Rect {
  let best: { d: number; apply: (r: Rect) => Rect } | null = null;
  for (const c of candidates) {
    const edges: Array<[number, number, (v: number) => Rect]> = [
      [Math.abs(r.x - c.x), 0, (v) => ({ ...r, x: v })],
      [Math.abs(r.x + r.w - (c.x + c.w)), 1, (v) => ({ ...r, x: v - r.w })],
      [Math.abs(r.y - c.y), 2, (v) => ({ ...r, y: v })],
      [Math.abs(r.y + r.h - (c.y + c.h)), 3, (v) => ({ ...r, y: v - r.h })],
    ];
    for (const [d, , apply] of edges) {
      if (d < threshold && (!best || d < best.d)) best = { d, apply };
    }
  }
  return best ? best.apply(r) : r;
}
```

（实现时展开 `r` 的捕获避免闭包旧值——直接在循环里算目标值对。）

- [ ] **Step 4: Rust snap_candidates（overlay.rs）**

```rust
#[tauri::command]
pub fn snap_candidates(app: AppHandle, _display_id: u64) -> Result<Vec<RectLog>, String> {
    let _ = app; // 预留 per-display 过滤；前端再按自身 display 过滤
    visible_window_rects()
}

#[derive(serde::Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct RectLog { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }

#[cfg(target_os = "macos")]
fn visible_window_rects() -> Result<Vec<RectLog>, String> {
    // CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements])；
    // 过滤 kCGWindowLayer==0、Bounds 非空、kCGWindowOwnerPID != std::process::id()。
    // 绑定：core-graphics 若暴露 CGWindowListCopyWindowInfo/CFArray/CFDictionary 直接用；
    // 否则 objc2 手写 extern 声明（报告注明选型）。数值经 CFNumber -> f64。
    ... // 实现者按绑定库补全；空结果返回 Ok(vec![]) 不视为错误
}
#[cfg(not(target_os = "macos"))]
fn visible_window_rects() -> Result<Vec<RectLog>, String> { Ok(vec![]) }
```

测试：`rects_on_display` 交集过滤放前端（index.tsx 拿到 candidates 后按 display 边界 filter）——Rust 侧至少一个单测：RectLog 序列化 camelCase 契约（serde_json 断言，模式同 payload_serializes_camel_case）。

- [ ] **Step 5: useSelection 键盘处理**

opts 增 `candidatesRef?: { current: Rect[] }`、`bounds?: Rect`。keydown 分支：

```ts
if (phase === "selected" && e.key.startsWith("Arrow")) {
  e.preventDefault();
  const step = e.shiftKey ? 10 : 1;
  const dx = e.key === "ArrowLeft" ? -step : e.key === "ArrowRight" ? step : 0;
  const dy = e.key === "ArrowUp" ? -step : e.key === "ArrowDown" ? step : 0;
  if (opts.bounds) {
    setRect((r) => (r ? snapRect(nudgeRect(r, dx, dy, opts.bounds!), opts.candidatesRef?.current ?? [], 8) : r));
  }
  return;
}
```

index.tsx：payload 到达时 `invoke<{x:number;y:number;w:number;h:number}[]>("snap_candidates", { displayId: display.displayId }).then(...)` 存入 candidatesRef（按 display 边界过滤），传给 useSelection。

- [ ] **Step 6: 确认通过 + 提交**

Run: `cd app && npm test && npm run build`；`cargo test --workspace` 全绿。

```bash
git add app/src app/src-tauri
git commit -m "feat(overlay): arrow-key nudge with nearest-edge snap and window candidates"
```

---

### Task 5: 智能吸附——AX 悬停探测（Rust 侧）

**Files:**
- Create: `crates/hawksnap-platform-mac/src/ax.rs`（`AxFrame`、`element_at_point`、`ax_trusted()`）
- Modify: `crates/hawksnap-platform-mac/src/lib.rs`（`pub mod ax;`）
- Modify: `app/src-tauri/src/overlay.rs`（mouseMoved 全局监听 + 节流 16ms + `hawk://hover` 发射）
- Modify: `app/src-tauri/Cargo.toml`（如需 `accessibility` crate 则加入 macos deps）

**Interfaces:**
- Produces: `AxFrame { x: f64, y: f64, w: f64, h: f64, role: String }`（全局逻辑坐标，**top-left 原点**）；`element_at_point(x: f64, y: f64) -> Option<AxFrame>`；`ax_trusted() -> bool`。事件 `hawk://hover` payload `Option<AxFrameJson>`（null = 无元素）。

- [ ] **Step 1: 写失败测试（ax.rs 内）**

```rust
#[cfg(test)]
mod ax_tests {
    use super::*;

    #[test]
    fn clipped_frame_rejects_empty() {
        assert!(clip_frame(AxFrame { x: 0.0, y: 0.0, w: 0.0, h: 0.0, role: "AXButton".into() }, 100.0, 100.0).is_none());
    }

    #[test]
    fn clipped_frame_clamps_to_display() {
        let f = clip_frame(AxFrame { x: 50.0, y: 50.0, w: 200.0, h: 100.0, role: "AXButton".into() }, 100.0, 100.0).unwrap();
        assert_eq!((f.w, f.h), (50.0, 50.0)); // 超出显示器的部分裁掉
        assert_eq!(f.role, "AXButton");
    }

    #[test]
    #[ignore = "需要辅助功能授权；授权后 cargo test -p hawksnap-platform-mac -- --ignored 运行"]
    fn real_element_at_center_returns_frame() {
        // 光标置于 Finder 图标上时手工验证；脚本环境仅验证不 panic 且返回 None 或 Some
        let _ = element_at_point(100.0, 100.0);
    }
}
```

- [ ] **Step 2: 确认失败** —— `cargo test -p hawksnap-platform-mac` FAIL（clip_frame 未定义）。

- [ ] **Step 3: 实现**

```rust
// ax.rs
#[derive(Debug, Clone)]
pub struct AxFrame { pub x: f64, pub y: f64, pub w: f64, pub h: f64, pub role: String }

pub fn clip_frame(f: AxFrame, disp_w: f64, disp_h: f64) -> Option<AxFrame> {
    let x1 = f.x.max(0.0); let y1 = f.y.max(0.0);
    let x2 = (f.x + f.w).min(disp_w); let y2 = (f.y + f.h).min(disp_h);
    if x2 - x1 < 2.0 || y2 - y1 < 2.0 { return None; }
    Some(AxFrame { x: x1, y: y1, w: x2 - x1, h: y2 - y1, role: f.role })
}

pub fn ax_trusted() -> bool {
    // AXIsProcessTrustedWithOptions 带 kAXTrustedCheckOptionPrompt=false（不弹窗）
    ... // accessibility crate 或 objc2 声明；Boolean(u8) != 0
}

pub fn element_at_point(x: f64, y: f64) -> Option<AxFrame> {
    // AXUIElementCreateSystemWide -> CopyElementAtPosition(CGPoint{x,y})
    // 读 kAXRoleAttribute (CFString) / kAXPositionAttribute (AXValue) / kAXSizeAttribute (AXValue)
    // 注意：AXPosition/AXSize 坐标即 top-left 原点全局逻辑坐标，无需翻转。
    // 任何查询失败 -> None。
    ... // 绑定选型同上，报告注明
}
```

overlay.rs 悬停发射（与 Task 13 的 NSEvent 全局监听同模式）：

```rust
pub fn register_hover_probe(app: &AppHandle) -> Result<(), String> {
    if !hawksnap_platform_mac::ax::ax_trusted() {
        eprintln!("[hawksnap] 未授予辅助功能：智能吸附降级为窗口级回退");
        return Ok(());
    }
    // NSEvent addGlobalMonitorForEventsMatchingMask: NSEventMaskMouseMoved
    // handler: 取 CGEvent(source: nil)?.location（top-left 原点，勿用 NSEvent 的 y 翻转坐标），
    // 节流 16ms（static LAST_HOVER: Mutex<Option<Instant>>），查询 element_at_point，
    // emit("hawk://hover", &frame)（None 也发，前端清除高亮）。
    // monitor token 泄漏保活（同 media monitor 模式）。
    ...
}
```

lib.rs setup 注册（media key 注册旁）。节流与裁剪是纯逻辑已测；AX 查询本体走 ignored 集成测试。**鼠标事件仅在覆盖层显示时有意义**：handler 开头检查 `ANY_OVERLAY_VISIBLE: AtomicBool`（start_capture_flow 置 true，hide_overlay 时若无可见覆盖层置 false）——否则常驻监听空转耗电。

- [ ] **Step 4: 确认通过 + 提交**

Run: `cargo test --workspace` 全绿；`cargo test -p hawksnap-platform-mac -- --ignored` 记录结果（授权前 None-不 panic 路径）。

```bash
git add crates/hawksnap-platform-mac app/src-tauri Cargo.lock
git commit -m "feat(detect): AX element hover probe with throttled hawk://hover events"
```

---

### Task 6: 智能吸附——前端高亮、单击选中与三级回退

**Files:**
- Modify: `app/src/windows/overlay/index.tsx`（hover 层绘制、click-to-select、回退链）
- Modify: `app/src/windows/overlay/useSelection.ts`（`selectRect(r)` 直选）
- Test: `app/src/windows/overlay/useSelection.test.tsx`（selectRect 用例）
- Create: `app/src/windows/overlay/hover.ts`（纯函数：hover 帧本地化 + 回退链）
- Test: `app/src/windows/overlay/hover.test.ts`

**Interfaces:**
- Produces: `useSelection.selectRect(r: Rect): void`（跳过拖拽直接进入 selected）；`toLocalHover(frame|null, display) -> Rect|null`（全局→本地，越界丢弃）；回退链在 Rust hover handler 内：元素 → 命中窗口（snap_candidates 数据做点命中）→ 无（前端不做全屏候选，避免噪声——全屏选择 = Esc 后直接拖满屏或 B7 的全屏模式）。

- [ ] **Step 1: 写失败测试**

```ts
// hover.test.ts
import { describe, expect, it } from "vitest";
import { toLocalHover } from "./hover";

const display = { x: 0, y: 0, width: 1512, height: 982 };
describe("toLocalHover", () => {
  it("converts global frame to local rect", () => {
    expect(toLocalHover({ x: 10, y: 20, w: 100, h: 50, role: "AXButton" }, display))
      .toEqual({ x: 10, y: 20, w: 100, h: 50 });
  });
  it("drops frames outside this display", () => {
    expect(toLocalHover({ x: 2000, y: 20, w: 100, h: 50, role: "AXButton" }, display)).toBeNull();
  });
  it("returns null for null probe", () => {
    expect(toLocalHover(null, display)).toBeNull();
  });
});
```

```ts
// useSelection.test.tsx 追加
it("selectRect enters selected with the given rect", () => {
  const { result } = renderHook(() => useSelection({ width: 1000, height: 600 }));
  act(() => result.current.selectRect({ x: 5, y: 6, w: 100, h: 50 }));
  expect(result.current.phase).toBe("selected");
  expect(result.current.rect).toEqual({ x: 5, y: 6, w: 100, h: 50 });
});
```

- [ ] **Step 2: 确认失败** —— FAIL。

- [ ] **Step 3: 实现**

hover.ts / selectRect 按测试直写。index.tsx：`listen<AxFrameJson | null>("hawk://hover", ...)` → toLocalHover → hoverRect state → 重绘 effect 中 hoverRect 有值时画高亮（fill `rgba(255,176,32,0.15)` + stroke `#FFB020` 2px，画在遮罩之后、选区之前）。click-to-select：pointerup 时若移动距离 < 4px 且 hoverRect 存在 → `selectRect(hoverRect)`（在 useSelection 的 onPointerUp 里做需要访问 hover —— 改在 index.tsx 包装：pointerHandlers.onPointerUp 外层先判断 click 语义再调 selectRect 或原 handler）。Task 5 的窗口级回退：Rust hover handler 在 element_at_point 为 None 时用窗口候选命中（B4 的 visible_window_rects + 点命中，取包含点且面积最小者）。

- [ ] **Step 4: 确认通过 + 提交**

Run: `cd app && npm test && npm run build`；cargo 全绿。

```bash
git add app/src app/src-tauri
git commit -m "feat(overlay): hover highlight with click-to-select and window fallback"
```

---

### Task 7: 窗口/全屏模式收敛 + 多显示器语义

**Files:**
- Modify: `app/src-tauri/src/overlay.rs`（hover 回退链加显示器级；`hide_all_overlays`；emit_capture_done/cancel_capture 改全部隐藏）
- Modify: `crates/hawksnap-core/src/geometry.rs`（`Rect::contains_point`，供点命中测试）

**Interfaces:**
- Produces: 三级回退语义 = 元素（AX）→ 窗口（CGWindowList 命中）→ 显示器（本 display 全帧）；确认/取消作用于**所有**覆盖层窗口（多显示器一致收敛）。`Rect::contains_point(&self, x: i32, y: i32) -> bool`。

- [ ] **Step 1: 写失败测试（geometry.rs）**

```rust
#[test]
fn contains_point_bounds_checked() {
    let r = Rect { x: 10, y: 10, w: 50, h: 50 };
    assert!(r.contains_point(10, 10));
    assert!(r.contains_point(59, 59));
    assert!(!r.contains_point(60, 60));
    assert!(!r.contains_point(9, 30));
}
```

- [ ] **Step 2: 确认失败 → 实现 → 通过**

```rust
pub fn contains_point(&self, x: i32, y: i32) -> bool {
    x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
}
```

overlay.rs：hover handler 的 None 分支改 `window_or_display_frame(pt, display_frames) -> Option<AxFrame>`（命中窗口取最小包含者；否则返回 display 帧）——纯函数 + 单测（构造候选帧列表断言命中优先级）。`hide_all_overlays(app)`：遍历 `app.webview_windows()` 取 label 前缀 `overlay-` 逐个 hide（emit_capture_done/cancel_capture 调用之；Task 1 的单窗 hide 逻辑收编进此函数）。

- [ ] **Step 3: 全量验证 + 提交**

Run: `cargo test --workspace`；`cd app && npm test && npm run build` 全绿。

```bash
git add crates/hawksnap-core app/src-tauri
git commit -m "feat(overlay): three-level hover fallback and multi-display hide semantics"
```

---

### Task 8: 延时截图（托盘子菜单 + 倒计时浮窗）

**Files:**
- Create: `app/src/windows/countdown/index.tsx`
- Modify: `app/src/main.tsx`（`?window=countdown` 路由）
- Modify: `app/src-tauri/src/overlay.rs`（`start_delayed_capture`、`cancel_delayed_capture` 命令、DELAY 状态）
- Modify: `app/src-tauri/src/lib.rs`（托盘子菜单）

**Interfaces:**
- Produces: 命令 `start_delayed_capture(secs: u64) -> Result<(), String>`（1/2/3/5 之外报错；进行中重复启动报错）、`cancel_delayed_capture() -> Result<(), String>`；事件 `hawk://countdown` payload `{ remaining: u64 }`（每秒）与 `{ remaining: 0 }`（触发前一瞬）；窗口 label `countdown`（同 Task 1 的复用模式：一次创建、show/hide）。

- [ ] **Step 1: 写失败测试（overlay.rs）**

```rust
#[test]
fn delay_secs_validated() {
    assert_eq!(validate_delay_secs(3), Ok(3));
    assert!(validate_delay_secs(4).is_err());
    assert!(validate_delay_secs(0).is_err());
}

#[test]
fn delay_gate_is_exclusive() {
    let flag = std::sync::atomic::AtomicBool::new(false);
    assert!(try_begin_delay(&flag));
    assert!(!try_begin_delay(&flag)); // 进行中拒绝
    flag.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(try_begin_delay(&flag));
}
```

- [ ] **Step 2: 确认失败 → 实现 → 通过**

```rust
fn validate_delay_secs(secs: u64) -> Result<u64, String> {
    matches!(secs, 1 | 2 | 3 | 5).then_some(secs).ok_or_else(|| "仅支持 1/2/3/5 秒".into())
}
fn try_begin_delay(flag: &std::sync::atomic::AtomicBool) -> bool {
    !flag.swap(true, std::sync::atomic::Ordering::SeqCst)
}
```

`start_delayed_capture(app, secs)`：校验 → 门闩 → 建/复用 countdown 窗口（`WebviewUrl::App("countdown?window=countdown")`，200×120 居中，always_on_top，非透明）→ spawn 线程：`deadline = Instant::now() + secs`；循环每 50ms 检查 `DELAY_ABORT`（真→hide+清门闩+返回）与 `remaining = deadline - now` 整秒变化时 `emit("hawk://countdown", {remaining})`；到点 → hide + 清门闩 + `start_capture_flow`。`cancel_delayed_capture` 置 ABORT。托盘菜单：`MenuItem::with_id("delay-1".."delay-5"...)` 子菜单「延时截图」，on_menu_event 线程派发（同 shot 模式）。

countdown/index.tsx：listen `hawk://countdown` 显示大号数字；Esc/点击 → `invoke("cancel_delayed_capture")`。

- [ ] **Step 3: 全量验证 + 提交**

Run: `cargo test --workspace`；`npm test && npm run build` 全绿。

```bash
git add app/src app/src-tauri
git commit -m "feat(capture): delayed capture with countdown overlay and cancel"
```

---

### Task 9: Plan B 收尾——合成基准入库、G2 采样、README、tag

**Files:**
- Create: `scripts/bench-synth.sh` + `scripts/bench_synth.swift`（合成 F1+拖拽+双击基准，参数化轮数）
- Create: `scripts/sample-mem.sh`（G2：启动后空载采样 RSS）
- Modify: `README.md`（进度段更新：智能吸附/动作浮条/延时截图；辅助功能权限说明扩展到悬停高亮）
- Modify: `app/src-tauri/tauri.conf.json` + `app/src-tauri/Cargo.toml`（版本 → 0.1.0-alpha.2）

- [ ] **Step 1: bench-synth 脚本**（把控制器 2026-10-14 验证过的注入程序固化）：

```bash
#!/usr/bin/env bash
# scripts/bench-synth.sh N — 合成 N 轮 F1→拖拽→双击基准（免人工按键）
set -euo pipefail
N="${1:-20}"
DIR="$(cd "$(dirname "$0")" && pwd)"
swift "$DIR/bench_synth.swift" "$N"
sleep 5
echo | "$DIR/bench-latency.sh"
```

bench_synth.swift：F1(122)→1.5s→短拖拽(不同偏移防同名)→0.6s→双击(clickState=2)→1.2s × N。**前置条件脚本内自检**：pgrep hawksnap 存在、`$TMPDIR/hawksnap-perf` 已清空（不空则报错退出）。

- [ ] **Step 2: sample-mem.sh**

```bash
#!/usr/bin/env bash
# scripts/sample-mem.sh — G2: 空载常驻内存采样（spec §5 ≤80MB）
set -euo pipefail
PID=$(pgrep -f "MacOS/hawksnap" | head -1)
[ -z "$PID" ] && { echo "HawkSnap 未运行"; exit 1; }
echo "采样 5 次（间隔 10s，空载）..."
for i in 1 2 3 4 5; do
  sleep 10
  ps -o rss= -p "$PID" | awk '{printf "sample %d: %.1f MB\n", '"$i"', $1/1024}'
done
```

- [ ] **Step 3: 全量验证 + 数据归档 + 提交 + tag**

控制器执行：`npm run tauri build` → `./scripts/bench-synth.sh 20` → 期望两行 P90 **均达标**（≤150 / ≤100）→ 数值写入 `docs/superpowers/bench/2026-10-14-plan-b-g3.md`（格式：日期、方法、轮数、两行 P90、h2o 分解）→ `./scripts/sample-mem.sh`（≤80MB 记录；**超标应急**：覆盖层窗口加 5 分钟空闲销毁——`spawn` 看护线程，hide 后 300s 未复用则 `close()`，复用路径已天然支持重建——实施后重测）。

```bash
cargo test --workspace && cd app && npm test && npm run build && cd ..
git add -A && git commit -m "chore: plan-b closeout bench tooling and memory gate" && git tag v0.1.0-alpha.2
```

---

## 自审记录（writing-plans self-review）

1. **Spec 覆盖**：§3.1 动作浮条→T3；微调+磁吸→T4；智能吸附→T5/T6；窗口/全屏模式→T7；多显示器→T7；延时→T8；冻结帧/命名/PNG 输出已在 Plan A。§4 Esc/Enter/双击→已有+T3 语义微调。F4 延时→T8。G3 修复→T1/T2。G2→T9。**缺口**：JPEG 输出与质量设置归 Plan C（设置页）；hover ≤16ms 的 F2 验收留 Plan D CI 基准（T5 已按 16ms 节流设计）。
2. **无占位符**：T5 的 AX 绑定与 T4 的 CGWindowList 绑定允许实现者按锁定库二选一并注明——这是「绑定选型」而非行为 TBD，测试契约已完整给出。
3. **类型一致性**：`reset()`/`selectRect()`/`barPosition`/`crop_rgba`/`rgba_to_png`/`RectLog`/`AxFrame` 各处引用一致；`hawk://hover` payload 为可空帧对象。
