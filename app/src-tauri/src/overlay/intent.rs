//! N1 录制选区模式：覆盖层用途互斥状态机（Capture vs Record）。
//!
//! M2 之前覆盖层只有一个用途（截图）；N1 起覆盖层是**截图与录制选区共用的
//! 全屏选区 UI**——同一套十字准线/遮罩/磁吸，只是提示文案不同、动作浮条在
//! 录制模式缺席、确认路由不同（capture → finish_capture，record →
//! record_start_region）。两种用途同一时刻至多一轮流程（M2 互斥守卫 a/b 保证
//! 不叠屏），故状态槽是 `Mutex<Option<CaptureIntent>>` 单槽：
//!
//! - 触发点写入：F1/媒体键/托盘/面板/延时 → [`CaptureIntent::Capture`]；
//!   F6 空闲触发 → [`CaptureIntent::Record`]（F6 完整语义见 [`f6_route`]）。
//! - 覆盖层就绪后经 overlay-ready 载荷的 `intent` 字段（`"capture"|"record"`）
//!   交给前端路由确认——Rust 侧静态槽是单一事实源，载荷值由它派生。
//! - 出口清除：hide_all_overlays（确认/取消/录制启动的全部收口路径都经它）+
//!   launch 失败路径。槽位空 = 无进行中的选区流程。

use std::sync::Mutex;

/// 覆盖层本轮用途（N1 互斥枚举，plan Interfaces：CaptureMode/RecordMode 互斥）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureIntent {
    /// 截图流程（M2 及之前覆盖层的唯一语义，行为零变化）。
    Capture,
    /// 录制选区流程（F6 统一进入：拖选=区域、Enter/双击=全屏快捷确认、Esc=取消）。
    Record,
    /// 长截图选区流程（N5：托盘「长截图」进入；确认后进入滚动采集会话）。
    Scrollshot,
}

impl CaptureIntent {
    /// overlay-ready 载荷 `intent` 字段值（camelCase 契约为单个小写词；
    /// 前端 `display.intent === "record"` 判录制模式）。
    pub fn as_payload_str(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Record => "record",
            Self::Scrollshot => "scrollshot",
        }
    }
}

/// 进程级状态槽（None = 无进行中的选区流程）。与 HOTKEY_ERROR/DELAY_ACTIVE 同款
/// static Mutex 单槽模式；写入方是两条流程入口（互斥守卫保证不并发），清除方是
/// hide_all_overlays 与 launch 失败路径。
static INTENT: Mutex<Option<CaptureIntent>> = Mutex::new(None);

fn lock_intent() -> std::sync::MutexGuard<'static, Option<CaptureIntent>> {
    INTENT.lock().unwrap_or_else(|e| e.into_inner())
}

/// 写入本轮用途（流程入口在**任何重活之前**先落槽——capture_screens 百 ms 级阻塞
/// 期间状态即已确定，与载荷发射的先后竞态无关：载荷值由入口参数直传，槽位供
/// 守卫/测试与未来 Rust 侧路由消费）。
pub fn set(intent: CaptureIntent) {
    *lock_intent() = Some(intent);
}

/// 当前用途（None = 无进行中的选区流程）。
#[cfg_attr(not(test), allow(dead_code))] // 生产读取方暂缺：入口写入 + 出口清除即闭环，
                                         // 槽位的存在性由 f6_route 守卫矩阵与状态机测试锚定。
pub fn current() -> Option<CaptureIntent> {
    *lock_intent()
}

/// 清除（hide_all_overlays 与 launch 失败路径调用）。
pub fn clear() {
    *lock_intent() = None;
}

/// F6 的完整路由（N1 裁定，纯函数可测）：录制中 → 停止（toggle 停止语义保留）；
/// 空闲 + 覆盖层可见 → 拒绝（M2 守卫 a：两套全屏 UI 不叠屏）；空闲 + 无覆盖层 →
/// 进入录制选区模式。**旧「直接全屏录制」语义废除**（plan Interfaces 裁定：
/// F6 统一进入选区模式，Snipaste 语义一致；record_start API 保留给兼容）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum F6Route {
    /// 录制/暂停中 → 停止录制（M6 toggle 的停止路径原样保留）。
    StopRecording,
    /// 空闲 → 启动录制选区覆盖层（start_record_selection_flow）。
    StartRecordSelection,
    /// 覆盖层可见 → 拒绝启动（文案 = M2 守卫 a 原文）。
    Reject(&'static str),
}

/// [`F6Route`] 决策函数：守卫文案复用 recording::f6_start_guard_msg（M2 单一来源）。
pub fn f6_route(recording_active: bool, overlay_visible: bool) -> F6Route {
    if recording_active {
        return F6Route::StopRecording;
    }
    match crate::recording::f6_start_guard_msg(overlay_visible) {
        Some(msg) => F6Route::Reject(msg),
        None => F6Route::StartRecordSelection,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 状态机纯测试（set/clear/roundtrip）：进程级单槽——本用例是**唯一**触碰
    /// INTENT 的测试（无并发写方），先取后放不重入锁。
    #[test]
    fn intent_slot_set_current_clear_roundtrip() {
        clear();
        assert_eq!(current(), None, "空槽基线");
        set(CaptureIntent::Capture);
        assert_eq!(current(), Some(CaptureIntent::Capture));
        // 互斥枚举：写入 Record 覆写 Capture（同槽单值，不共存）——对应「快速
        // 连按 F1 再 F6」竞态下后到入口覆盖前值的语义（F6 那次实际会被 M2 守卫
        // 拒绝，此处是槽位层面的兜底语义）。
        set(CaptureIntent::Record);
        assert_eq!(current(), Some(CaptureIntent::Record));
        // N5：长截图用途同槽写入（三种用途互斥，同一时刻至多一轮流程）。
        set(CaptureIntent::Scrollshot);
        assert_eq!(current(), Some(CaptureIntent::Scrollshot));
        set(CaptureIntent::Record);
        assert_eq!(current(), Some(CaptureIntent::Record));
        clear();
        assert_eq!(current(), None);
    }

    /// 载荷契约：intent 枚举 → overlay-ready 载荷的小写单词（前端判录制模式的
    /// 唯一依据，与 DisplayPayload.intent 的序列化形状一致）。
    #[test]
    fn intent_payload_str_is_lowercase_single_word() {
        assert_eq!(CaptureIntent::Capture.as_payload_str(), "capture");
        assert_eq!(CaptureIntent::Record.as_payload_str(), "record");
        // N5：长截图用途的载荷值。
        assert_eq!(CaptureIntent::Scrollshot.as_payload_str(), "scrollshot");
    }

    /// F6 路由矩阵（N1 裁定 + M2 守卫复用）：
    /// 录制中 → 停止（即便覆盖层可见也放行去停——M2 的防御性放行语义保留）；
    /// 空闲 + 覆盖层可见 → 拒绝；空闲 + 无覆盖层 → 录制选区。
    #[test]
    fn f6_route_matrix() {
        assert_eq!(f6_route(true, false), F6Route::StopRecording);
        assert_eq!(f6_route(true, true), F6Route::StopRecording);
        assert_eq!(
            f6_route(false, true),
            F6Route::Reject("请先完成或取消当前截图再录屏")
        );
        assert_eq!(f6_route(false, false), F6Route::StartRecordSelection);
    }
}
