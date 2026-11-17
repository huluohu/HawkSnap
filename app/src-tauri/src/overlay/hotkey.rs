// —— 非 macOS 平台放行约定（M4 P0 评审修复轮）——
//
// 本文件含 macOS 专属能力（录屏 / 画中画 / 贴图 / 按键显示 / 长截图 等）。非 macOS
// 构建下这些项**无人调用**——这是「能力缺席」的正常结果，不是代码缺陷。故按项逐个
// 放行：`#[cfg_attr(not(target_os = "macos"), allow(dead_code))]`（仅 macOS 分支才用
// 的局部变量用 `unused_variables`，仅 macOS 才用的导入用 `unused_imports`）。
//
// **逐项**放行而非 crate 级 blanket allow：crate 级 `allow(dead_code, unused_*)`
// 会把**未来新增**的同类问题一并吞掉；逐项放行后，新增项若在非 macOS 上真的无人用，
// 依然照报。macOS 构建下这些属性恒不生效（`not(target_os = "macos")` 为假）——
// macOS 的 lint 纪律零变化。

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::AppHandle;

use super::flow::start_capture_flow;
#[cfg(target_os = "macos")]
use super::hover;

// 热键注册失败的 pull 通道：setup 阶段的 hawk://hotkey-error emit 早于前端 listen 注册，
// 必丢；错误同时落入此静态，前端挂载时经 hotkey_error 命令拉取补显（读取即清除）。
static HOTKEY_ERROR: Mutex<Option<HotkeyError>> = Mutex::new(None);

/// Plan B Task 5：智能吸附 AX 悬停探测注册。
///
/// 辅助功能未授予 → 整条链路缺席（裁定 1）：eprintln + `Ok(())`，不装监听、不弹授权窗，
/// 桌面上不留任何提示；拖拽框选与窗口级磁吸不受影响。
/// 必须在主线程调用（事件监听挂在 AppKit 事件循环上，与热键注册同线程纪律）。
#[cfg(target_os = "macos")]
pub fn register_hover_probe(app: &AppHandle) -> Result<(), String> {
    if !hawksnap_platform_mac::ax::ax_trusted() {
        eprintln!("[hawksnap] 未授予辅助功能：悬停元素高亮不可用（拖拽框选不受影响）");
        return Ok(());
    }
    hover::install_hover_monitors(app);
    Ok(())
}

pub fn register_hotkeys(app: &AppHandle) -> Result<(), HotkeyError> {
    // C7：热键自设置读取（settings.json 缺失/损坏 → Settings::default = F1/F2/F6），
    // 注册失败语义与拆分前一致：Err 上抛 + 落 pull 通道（HOTKEY_ERROR，首错优先），
    // lib.rs 的 emit/拉面板逻辑不变。
    let settings = crate::settings::Settings::load(app);
    reregister_from_settings(app, &settings).inspect_err(|err| {
        let mut slot = HOTKEY_ERROR.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            // 首错优先（通道槽位只有一个）：register_in_order 返回的即注册序首个错误
            *slot = Some(err.clone());
        }
    })
}

/// 热键角色：注册序 capture → pin → record 下决定组合键触发哪个处理器。
/// 小写序列化（"capture"/"pin"/"record"）：hotkey-error 载荷的 kind 字段，前端据此
/// 归因横幅（截图/贴图/录屏——D0-⑤）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyKind {
    Capture,
    Pin,
    Record,
}

/// 注册失败的候选键建议（§4「给出候选键」）：静态表——逐个试注册动态探测可用组合
/// 的代价与复杂度不成比例，候选只做引导。沿用 Snipaste 风格的同修饰系（Cmd/Alt + 数字
/// 与功能键），实际占用情况以再次尝试为准。
fn candidates_for(kind: HotkeyKind) -> &'static str {
    match kind {
        HotkeyKind::Capture => "候选键：Cmd+Shift+1 / Cmd+Alt+1 / F7",
        HotkeyKind::Pin => "候选键：Cmd+Shift+2 / Cmd+Alt+2 / F8",
        HotkeyKind::Record => "候选键：Cmd+Shift+3 / Cmd+Alt+3 / F9",
    }
}

/// 热键注册失败（D0-⑤）：首错的消息 + 角色归因。同时是 hawk://hotkey-error 事件与
/// hotkey_error 命令的载荷（serde camelCase → `{kind, message}`）；Display 只出 message
/// ——lib.rs 的 eprintln 日志口径与拆分前一致（组合键本就写在 message 里）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyError {
    pub kind: HotkeyKind,
    pub message: String,
}

impl std::fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 按注册序（capture → pin → record）逐项调用 register_fn；全部尝试（前项失败不阻断
/// 后项——拆分前 F1 被占用时 F2 仍注册成功的行为保留），任一失败 → 返回首个错误
/// （带角色归因，D0-⑤）。与 AppHandle 解耦（app 由 register_fn 闭包捕获）：settings.rs
/// 的保存管线与单测共用。
pub(crate) fn register_in_order(
    settings: &crate::settings::Settings,
    mut register_fn: impl FnMut(&str, HotkeyKind) -> Result<(), String>,
) -> Result<(), HotkeyError> {
    let jobs = [
        (settings.hotkey_capture.as_str(), HotkeyKind::Capture),
        (settings.hotkey_pin.as_str(), HotkeyKind::Pin),
        (settings.hotkey_record.as_str(), HotkeyKind::Record),
    ];
    // §4 库内互斥校验：同一组合绑多个功能时，第二个注册必失败且报「可能被占用」——
    // 其实是**自己占了自己**，误导用户去查外部占用。注册前先做一次库内查重，命中即
    // 归因到后一个角色，文案明说「重复绑定」。
    for i in 0..jobs.len() {
        for j in (i + 1)..jobs.len() {
            if jobs[i].0.eq_ignore_ascii_case(jobs[j].0) && !jobs[j].0.trim().is_empty() {
                return Err(HotkeyError {
                    kind: jobs[j].1,
                    message: format!(
                        "热键 {} 重复绑定（{} 与 {} 设了同一个组合），请为每个功能设置不同的组合键",
                        jobs[j].0,
                        kind_label(jobs[i].1),
                        kind_label(jobs[j].1),
                    ),
                });
            }
        }
    }
    let mut first_err: Option<HotkeyError> = None;
    for (combo, kind) in jobs {
        if let Err(e) = register_fn(combo, kind) {
            if first_err.is_none() {
                first_err = Some(HotkeyError { kind, message: e });
            }
        }
    }
    first_err.map_or(Ok(()), Err)
}

/// 角色的用户可见名（重复绑定文案归因用）。Rust 侧文案当前为中文单语（错误码化
/// 归后续文案统一轮），此处与横幅既有中文口径一致。
fn kind_label(kind: HotkeyKind) -> &'static str {
    match kind {
        HotkeyKind::Capture => "截图",
        HotkeyKind::Pin => "贴图",
        HotkeyKind::Record => "录屏",
    }
}

/// 注册单个组合并挂对应处理器（生产 register_fn）。Shortcut::from_str 是插件的热键
/// 字符串解析器（"F1"、"Cmd+Shift+A"，大小写不敏感）。on_shortcut 注册快捷键并挂处理器
/// （勿再单独 register：同一组合二次注册会被 macOS 以 eventHotKeyExistsErr 拒绝）。
/// 热键回调在主线程（Carbon 事件）触发，而窗口建窗/关窗类操作禁止在主线程做
/// （tauri-runtime-wry 在主线程处理 Close 消息会直接 panic），故处理器只 spawn 独立
/// 线程执行，热键处理立即返回。
pub fn register_one(app: &AppHandle, combo: &str, kind: HotkeyKind) -> Result<(), String> {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
    let shortcut =
        Shortcut::from_str(combo).map_err(|e| format!("热键 {combo} 无法解析: {e}"))?;
    let gs = app.global_shortcut();
    let app2 = app.clone();
    // 拥有化组合串：Pin 探针的闭包要求 'static（&str 参数活不过本函数体）。
    let combo_owned = combo.to_string();
    match kind {
        HotkeyKind::Capture => gs
            .on_shortcut(shortcut, move |_app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    let app = app2.clone();
                    std::thread::spawn(move || trigger_capture_flow(&app));
                }
            })
            .map_err(|e| format!("{combo} 注册失败（可能被占用）: {e}；{}", candidates_for(kind))),
        HotkeyKind::Pin => gs
            .on_shortcut(shortcut, move |_app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    // 探针（常驻，廉价）：E2E 判别「pin 回调没进来」vs「进来了但没出窗」。
                    // 注意 global-hotkey 0.8 的 Code::F2 → Carbon 0x78=120（99 是 kVK_F3）。
                    // D0-①：探针不再写死 F2，回显本次实际注册的组合（combo 即设置值）。
                    eprintln!("[hawksnap] {combo_owned} 热键触发（pin_latest）");
                    let app = app2.clone();
                    std::thread::spawn(move || {
                        // v1.2 守卫（审计修复）：覆盖层进行中（截图/录制/长截图选区）按 F2
                        // 此前会贴出**整屏冻结帧**叠在选区 UI 上——两套全屏 UI 不叠屏（M2
                        // 守卫 a 同款口径，F6 同守卫）。拒绝并横幅引导。
                        let overlay_visible = super::flow::ANY_OVERLAY_VISIBLE
                            .load(std::sync::atomic::Ordering::SeqCst);
                        if overlay_visible {
                            crate::panel::show_capture_error(
                                &app,
                                "截图选区进行中：先按 Esc 结束选区，再按贴图键".into(),
                            );
                            return;
                        }
                        if let Err(e) = crate::pins::pin_latest(app.clone()) {
                            // 错误横幅可见化（show_capture_error 先懒拉主面板）：此前裸 emit
                            // 在面板不存在时静默丢失——启动零窗口架构下正是常态。
                            crate::panel::show_capture_error(&app, e);
                        }
                    });
                }
            })
            .map_err(|e| format!("{combo} 注册失败（可能被占用）: {e}；{}", candidates_for(kind))),
        HotkeyKind::Record => gs
            .on_shortcut(shortcut, move |_app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    eprintln!("[hawksnap] {combo_owned} 热键触发（record）");
                    let app = app2.clone();
                    std::thread::spawn(move || {
                        // bench 轮次保护（D0-③，与 F1/托盘入口同款）：基准模式进行中
                        // 忽略外部触发——录制选区覆盖层是全屏 UI，混入 bench 轮会污染
                        // 时延样本（M2 的 F6 toggle 无此守卫；N1 起覆盖层参与，补齐）。
                        if crate::bench_mode::is_running() {
                            return;
                        }
                        // N1：F6 完整路由（intent::f6_route 纯函数，热键侧分派）——
                        // 录制/暂停中 → 停止（M6 toggle 的停止路径原样保留，即便覆盖层
                        // 可见也放行去停：两态经对向守卫后不再共存，此处防御性放行）；
                        // 空闲 → 进入录制选区模式（**旧「直接全屏录制」语义废除**，plan
                        // Interfaces 裁定：F6 统一进入选区模式，拖选=区域、Enter/双击=
                        // 全屏快捷确认、Esc=取消；record_start API 保留给兼容）；
                        // 覆盖层可见（截图或录制选区进行中）→ 拒绝横幅（M2 守卫 a：
                        // 两套全屏 UI 不叠屏；反馈走 capture-error 通道，与 M2 同口径）。
                        use super::intent::{f6_route, F6Route};
                        let overlay_visible = super::flow::ANY_OVERLAY_VISIBLE
                            .load(std::sync::atomic::Ordering::SeqCst);
                        match f6_route(
                            crate::recording::recording_active(),
                            overlay_visible,
                        ) {
                            F6Route::StopRecording => {
                                if let Err(e) = crate::recording::record_stop(app.clone()) {
                                    // 错误横幅可见化（show_record_error，非主线程合法），
                                    // 拉起失败仅 eprintln，横幅退化为 push 通道尽力投递。
                                    crate::panel::show_record_error(&app, e);
                                }
                            }
                            F6Route::StartRecordSelection => {
                                if let Err(e) =
                                    super::flow::start_record_selection_flow(&app)
                                {
                                    crate::panel::show_record_error(&app, e);
                                }
                            }
                            F6Route::Reject(msg) => {
                                crate::panel::show_capture_error(&app, msg.to_string());
                            }
                        }
                    });
                }
            })
            .map_err(|e| format!("{combo} 注册失败（可能被占用）: {e}；{}", candidates_for(kind))),
    }
}

/// C7：按设置重注册全部热键——先 unregister_all 旧组合，再按 capture → pin → record
/// 注册新组合。任一失败 → Err（调用方中止保存；Err 携带角色归因，D0-⑤）。注意：失败时
/// 旧组合已注销、新组合半数可能已注册（运行态半新半旧），设置文件保持旧值——重启或下次
/// 成功保存自愈（计划裁定语义，不做旧热键回滚注册）。注册经插件 run_main_thread 交付
/// 主线程（2.4.1 register_internal/unregister_all 均已自带），任意线程可调——
/// set_settings(async) 从 tokio 线程调用安全。
pub fn reregister_from_settings(
    app: &AppHandle,
    settings: &crate::settings::Settings,
) -> Result<(), HotkeyError> {
    reregister_with(app, settings, register_one)
}

/// reregister 的可注入缝（计划 C7 裁定）：register_fn 由调用方注入，测试注入 fake
/// （AppHandle 不可造，热键字符串经 settings.rs 的 register_in_order 间接验证）。
pub fn reregister_with(
    app: &AppHandle,
    settings: &crate::settings::Settings,
    register_fn: impl Fn(&AppHandle, &str, HotkeyKind) -> Result<(), String>,
) -> Result<(), HotkeyError> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    app.global_shortcut()
        .unregister_all()
        // 注销失败发生在任何角色注册之前、无从归因到单个组合——按注册序归首个角色
        //（capture），仅为满足 HotkeyError 的 kind 字段；文案保持原样。
        .map_err(|e| HotkeyError {
            kind: HotkeyKind::Capture,
            message: format!("注销旧热键失败: {e}"),
        })?;
    register_in_order(settings, |combo, kind| register_fn(app, combo, kind))
}

/// 截图热键触发入口：300ms 去重窗内只启动一次截图流程（快速连按去抖；Task 13 时代
/// 该窗口挡的是「Carbon F1 与媒体键双通道竞态」，媒体键通道已按用户要求移除——
/// 系统亮度键完整归还系统，见 README「截图热键：fn+F1」）。任意线程可调：函数体只做
/// 原子置位 + spawn，start_capture_flow 永远跑在 spawn 出的非主线程上（主线程建窗/关窗
/// 会 panic）。
pub(crate) fn trigger_capture_flow(app: &AppHandle) {
    // bench 守卫（D0-③，Plan C 终审）：基准模式进行中忽略外部触发（防一轮被污染）——
    // bench 轮次直接调 start_capture_flow、不经本入口，自身不受影响。托盘/面板入口
    // 各有同款守卫（lib.rs / panel.rs），热键通道在此统一拦截。
    if crate::bench_mode::is_running() {
        return;
    }
    if !arm_once(&TRIGGER_INFLIGHT) {
        return; // 去重窗口内已有触发在跑
    }
    let app2 = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = start_capture_flow(&app2) {
            // 热键失败横幅可见化：主面板懒创建，未开面板时 push 事件无窗可显——
            // emit 前先拉起（show_capture_error 内 ensure_main_panel，本线程非主线程合法）
            crate::panel::show_capture_error(&app2, e);
        }
    });
    // 去重窗按时长解除（而非按流程结束）
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        disarm(&TRIGGER_INFLIGHT);
    });
}

/// 300ms 去重闸门（进程级）：窗口内重复触发返回 false。
/// `arm_once` 是纯状态转换（可测）。
pub(crate) fn arm_once(flag: &std::sync::atomic::AtomicBool) -> bool {
    use std::sync::atomic::Ordering;
    !flag.swap(true, Ordering::SeqCst)
}

pub(crate) fn disarm(flag: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering;
    flag.store(false, Ordering::SeqCst);
}

pub(crate) static TRIGGER_INFLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 拉取并清除最近一次热键注册错误（None = 无错误）；补上 setup 期早于前端 listen 的丢失。
/// 载荷 `{kind, message}`（camelCase，D0-⑤）：前端按 kind 归因横幅。
#[tauri::command]
pub fn hotkey_error() -> Option<HotkeyError> {
    HOTKEY_ERROR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

// —— 去重闸门纯逻辑测试 ——
#[cfg(test)]
mod dedupe_tests {
    use super::{arm_once, disarm};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn dedupe_arms_once() {
        // 去重闸门纯逻辑：首次置位成功，窗口内（未 disarm 前）再触发被拒
        let flag = AtomicBool::new(false);
        assert!(arm_once(&flag));
        assert!(!arm_once(&flag));
        disarm(&flag);
        assert!(arm_once(&flag));
    }
}
