# HawkSnap Plan A 跟进任务：F1 媒体键兼容 + 单实例守卫 + README

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement task-by-task.

**Goal:** 让 F1 在 macOS 默认设置（F1 行 = 媒体键）下可用：媒体键路径监听亮度下键并触发截图；启动时检测功能键模式并给出可见提示；阻止第二个实例重复注册热键；README 补充说明。

**背景（2026-10-13 排障结论）：** 代码链路完好（HID 注入键码 122 可完整触发截图闭环），但 macOS 默认 `com.apple.keyboard.fnState` 未设置 = F1 行作媒体键，物理 F1 只发亮度系统事件、键码 122 不投递。另发现 macOS 允许跨进程重复注册同一 Carbon 热键（`eventHotKeyExistsErr` 仅限同进程），双实例会同时弹覆盖层。

**Spec:** 沿用 `docs/superpowers/specs/2026-10-13-hawksnap-design.md`（§4 F1 区域截图、§9 热键占用可见反馈/权限引导）。

## Global Constraints（继承 Plan A）

- Rust edition 2021、workspace lints `unsafe_op_in_unsafe_fn = deny`；TS strict。
- 无遥测、无外联；不引入网络请求。
- 新依赖只允许 macOS 系统框架绑定（core-graphics / objc2 系），并在报告说明版本。
- 每个任务以 `cargo test --workspace` / `cd app && npm test` 全绿 + conventional commit 结束。
- 媒体键监听需要的「辅助功能」权限属 spec §6.3 已声明项（权限引导 UI 在 Plan D），本任务只做日志级降级，不弹权限窗。

---

### Task 13: F1 媒体键兼容 + 单实例守卫 + README

**Files:**
- Modify: `app/src-tauri/src/overlay.rs`（媒体键 tap + fnState 提示 + 触发去重）、`app/src-tauri/src/lib.rs`（setup 挂接 + 单实例守卫调用）、`app/src-tauri/Cargo.toml`（如需新增绑定依赖）、`README.md`
- Create: `app/src-tauri/src/single_instance.rs`（锁文件守卫）

**Interfaces:**
- Produces: `register_media_key_f1(app: &AppHandle) -> Result<(), String>`（亮度下键 → 触发与 F1 相同的截图流程；tap 不可用返回 Ok 并仅 eprintln，不阻塞启动）；`fnstate_hint() -> Option<String>`（fnState 未开启 → Some(提示文案)，经现有 hotkey-error 横幅通道显示）；`single_instance::acquire_lock() -> Result<LockGuard, String>`（flock，持锁守卫进程生命周期；第二实例 Err 退出）。

- [ ] **Step 1: 写失败测试（纯逻辑三件）**

```rust
// overlay.rs 内 media 模块
#[cfg(test)]
mod media_tests {
    use super::media::*;

    #[test]
    fn parses_brightness_down_from_system_defined() {
        // NSEvent systemDefined: data1 高 16 位为 NX 键类型，低 16 位为按下/释放标志
        let data1 = (22i64 << 16) | 0x000A; // NX_KEYTYPE_BRIGHTNESS_DOWN=22, keyDown flag=0xA
        assert_eq!(parse_media_key(data1), Some(MediaKey::BrightnessDown));
    }

    #[test]
    fn ignores_other_media_keys_and_releases() {
        assert_eq!(parse_media_key((23i64 << 16) | 0x000A), None); // BRIGHTNESS_UP
        assert_eq!(parse_media_key((22i64 << 16) | 0x000B), None); // keyUp flag
    }

    #[test]
    fn hint_only_when_fnstate_not_standard() {
        assert!(fnstate_hint(Some(false)).is_some()); // 媒体键模式 → 提示
        assert!(fnstate_hint(None).is_some());        // 未设置（默认）→ 提示
        assert!(fnstate_hint(Some(true)).is_none());  // 标准功能键 → 无提示
    }
}
```

```rust
// single_instance.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_releasable() {
        let dir = std::env::temp_dir().join(format!("hs-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.lock");
        let g1 = acquire_lock_at(&path).expect("first acquire");
        assert!(acquire_lock_at(&path).is_err(), "second acquire must fail");
        drop(g1);
        assert!(acquire_lock_at(&path).is_ok(), "reacquire after drop");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml`
Expected: FAIL（`media`/`single_instance` 未定义）。

- [ ] **Step 3: 实现**

```rust
// overlay.rs 追加（或独立 media 子模块）
pub(crate) mod media {
    /// NSEvent systemDefined 的 data1：高 16 位 = NX 键类型；bit3..0 非 0 视为 keyDown
    /// （NX_KEYDOWN_MASK=0x0A | 0x0100.. 位组合，工程上以低位非零且含 0x2 判按下，
    /// 实现按 platform 惯例：`(data1 & 0x0000FFFF) != 0` 视为 keyDown）。
    pub const NX_KEYTYPE_BRIGHTNESS_DOWN: i64 = 22;

    #[derive(Debug, PartialEq)]
    pub enum MediaKey { BrightnessDown }

    pub fn parse_media_key(data1: i64) -> Option<MediaKey> {
        let key = (data1 & 0xFFFF_0000) >> 16;
        let pressed = (data1 & 0x0000_FFFF) != 0;
        if pressed && key == NX_KEYTYPE_BRIGHTNESS_DOWN { Some(MediaKey::BrightnessDown) } else { None }
    }

    /// true=标准功能键；None=未设置（默认媒体键模式）。
    pub fn fnstate_hint(fnstate: Option<bool>) -> Option<String> {
        match fnstate {
            Some(true) => None,
            _ => Some("F1 当前是亮度键：可在 系统设置→键盘 开启「将 F1、F2 等键用作标准功能键」，\
                     或在 系统设置→隐私与安全性→辅助功能 中添加 HawkSnap（推荐，开箱即用）。".into()),
        }
    }
}
```

媒体键 tap：优先 `core-graphics`（CGEventTap，`kCGEventTapOptionListenOnly` + `kCGSessionEventTap`，
mask 只含 `NSEventMaskSystemDefined`（1<<14））；若绑定不全改用 objc2 + NSEvent
`addGlobalMonitorForEventsMatchingMask`。要求：
- tap 回调运行在其创建线程的 CFRunLoop 上——在独立 `std::thread` 中创建 tap 并 `CFRunLoopRun()`，
  回调里解析事件，命中 BrightnessDown 时 `std::thread::spawn` 走与 F1 完全相同的
  `start_capture_flow`（勿在 tap 回调线程直接跑流程——它同样不得在主线程建窗/关窗）。
- `CGEventTapCreate` 返回 NULL（无辅助功能权限）→ `eprintln!` + 返回 `Ok(())`，不阻塞启动。
- tap/handler 引用需进程生命周期存活（static 或 leaked Box）。

fnState 读取：`Command::new("defaults").args(["read", "-g", "com.apple.keyboard.fnState"])`，
退出码 0 且输出 trim=="1" → Some(true)；退出码非 0（键不存在）→ None；输出其它 → Some(false)。
setup 中若 `fnstate_hint(...)` 有值 → 走现有 `hawk://hotkey-error` 横幅通道 + eprintln
（与「注册失败」提示区分开：此为模式提示，不是错误文案，前缀用「提示：」）。

```rust
// single_instance.rs
use std::fs::File;
pub struct LockGuard { _file: File }
pub fn acquire_lock_at(path: &std::path::Path) -> Result<LockGuard, String> {
    let file = File::create(path).map_err(|e| e.to_string())?;
    file.try_lock().map_err(|e| format!("另一实例正在运行: {e}"))?;
    Ok(LockGuard { _file: File::try_clone(&file).map_err(|e| e.to_string())? })
}
```

（`File::try_lock/try_clone` 为 std 1.89+ API；守卫结构持有已锁定文件的 clone 保住 flock。
实现若发现 try_lock 语义不可靠，可改 `libc::flock`，在报告注明。）

lib.rs setup 首行：`single_instance::acquire_lock_at(&lock 路径)` 失败 → `eprintln!` + `app.exit(0)`；
锁路径 `std::env::temp_dir().join("hawksnap.lock")`。成功则把 guard 泄漏成 'static（进程生命周期）。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml` → PASS（原 4 + 新 3 = 7）。
Run: `cargo test --workspace` → 全绿。

- [ ] **Step 5: README 补充**

「屏幕录制权限」小节追加两条：① macOS 默认 F1 是亮度键——开「标准功能键」或授予「辅助功能」
（媒体键路径）二选一；② 授予「辅助功能」的方法（隐私与安全性 → 辅助功能 → 添加 HawkSnap）。

- [ ] **Step 6: 手动/半自动验证（控制器执行）**

- `Swift` 注入 `CGEventCreateSystemDefinedEvent`（NX_KEYTYPE_BRIGHTNESS_DOWN, keyDown）→ 触发截图流程
  （观测 $TMPDIR/hawksnap-perf 出现 hotkey 行）。
- 双开二进制 → 第二实例立即退出且 eprintln「另一实例正在运行」。

- [ ] **Step 7: 提交**

```bash
git add app/src-tauri README.md
git commit -m "feat(hotkey): media-key F1 fallback, fnstate hint, single-instance guard"
```
