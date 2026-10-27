//! N4 keycast **窗口 capability 契约**（评审修复 Critical 的守卫）。
//!
//! Tauri 的 ACL 按**窗口标签**授权：每个 webview 只能调用其 capability 覆盖的命令。
//! keycast 小窗要收 `hawk://keycast`（JS `listen`）→ 必须有覆盖标签 `keycast` 的
//! capability 且含 `core:default`（→ `core:event:default` → `allow-listen`）。
//! 缺了它：`listen()` 的 promise 在运行时被 ACL 拒绝 → 窗口**永远收不到事件**
//! （Rust 侧 tap 正常工作、窗口也开着，但小窗永远空白）——正是本任务评审发现的缺陷。
//!
//! 本测试走**真实编译期 ACL**：`tauri::generate_context!()` 内嵌 tauri-build 从
//! `app/src-tauri/capabilities/*.json` 解析出的清单 → `RuntimeAuthority::resolve_access`
//! 判定（不是「JSON 文件存在」这种弱断言）。反向对照断言（未登记标签拿不到监听权）
//! 保证本断言有判别力——若 ACL 恒放行，测试同样会红。
//!
//! 独立集成测试 crate（非单元测试）的原因：`generate_context!` 宏每 crate 只能展开
//! 一次（`embed_info_plist_bytes` 定义 `_EMBED_INFO_PLIST` 符号）——lib.rs 已占用。

use tauri::ipc::Origin;

/// 事件监听命令名（core:event 插件的 listen；ACL 解析后为 `plugin:event|listen`）。
const LISTEN_CMD: &str = "plugin:event|listen";

#[test]
fn keycast_window_capability_grants_event_listen() {
    // 真实 context（含 capabilities/ 目录解析结果）——编译期产物。
    let mut ctx: tauri::Context<tauri::Wry> = tauri::generate_context!();
    let acl = ctx.runtime_authority_mut();

    // ① keycast 标签被授予事件监听 → 小窗能收 hawk://keycast。
    assert!(
        acl.resolve_access(LISTEN_CMD, "keycast", "keycast", &Origin::Local).is_some(),
        "keycast 窗口 capability 缺失：core:event:allow-listen 未授予标签 `keycast`"
    );
    // ② 反向对照：未登记的标签拿不到监听权（证明 ACL 真按标签区分，而非恒放行）。
    assert!(
        acl.resolve_access(LISTEN_CMD, "no-such-window", "no-such-window", &Origin::Local)
            .is_none(),
        "ACL 未按窗口标签区分——本断言失去判别力"
    );
    // ③ 既有窗口标签不受影响（回归：recording-bar 仍能 listen hawk://record-tick）。
    assert!(
        acl.resolve_access(LISTEN_CMD, "recording-bar", "recording-bar", &Origin::Local)
            .is_some(),
        "recording-bar 的 capability 回归（core:default 应覆盖其标签）"
    );
}
