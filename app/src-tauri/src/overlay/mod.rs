//! overlay 模块门面：实现按关注点拆分为子模块（flow/hover/delay/candidates/hotkey）。
//! 本文件只承载子模块声明与既有 `overlay::X` 路径的重导出——对外路径零变化。
mod candidates;
mod delay;
mod flow;
#[cfg(target_os = "macos")]
mod hover;
mod hotkey;

// 公开 API 重导出：lib.rs 与 crate 内既有的 overlay::X 路径保持不变。
pub use candidates::snap_candidates;
pub use delay::{cancel_delayed_capture, countdown_remaining, start_delayed_capture};
pub use flow::{cancel_capture, emit_capture_done, start_capture_flow};
// reregister_from_settings / reregister_with 只在 hotkey.rs 内部链路使用（setup 经
// register_hotkeys），不在此重导出（重导出无人引用会挂 unused_imports）。
pub use hotkey::{hotkey_error, register_hotkeys, register_one, HotkeyKind};
// 保存管线（settings.rs apply_pipeline）按注册序逐项注册热键：crate 内共享，不外暴。
pub(crate) use hotkey::register_in_order;
#[cfg(target_os = "macos")]
pub use hotkey::{register_hover_probe, register_media_key_f1};

pub(crate) use flow::ANY_OVERLAY_VISIBLE;

// #[tauri::command] 生成的隐藏宏（__cmd__X 包装器 / __tauri_command_name_X 命令名匹配）
// 定义在命令所在的子模块，而 generate_handler![overlay::X] 会展开出 overlay::__cmd__X!()
// 与 overlay::__tauri_command_name_X!()——故必须一并重导出到 overlay 命名空间，
// lib.rs 的命令注册路径才能继续解析（见任务报告）。
#[doc(hidden)]
pub use candidates::{__cmd__snap_candidates, __tauri_command_name_snap_candidates};
#[doc(hidden)]
pub use delay::{
    __cmd__cancel_delayed_capture, __cmd__countdown_remaining, __cmd__start_delayed_capture,
    __tauri_command_name_cancel_delayed_capture, __tauri_command_name_countdown_remaining,
    __tauri_command_name_start_delayed_capture,
};
#[doc(hidden)]
pub use flow::{
    __cmd__cancel_capture, __cmd__emit_capture_done, __tauri_command_name_cancel_capture,
    __tauri_command_name_emit_capture_done,
};
#[doc(hidden)]
pub use hotkey::{__cmd__hotkey_error, __tauri_command_name_hotkey_error};
