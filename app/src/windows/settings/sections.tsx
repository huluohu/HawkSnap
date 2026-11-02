// ============================================================================
// HawkSnap 设置窗口 —— 分区内容（8 个分区）+ 共用 UI 原子
// ----------------------------------------------------------------------------
// 为什么要本地文案表：设置窗口**不允许**改 app/src/locales/*.json（主面板的
// i18n 词典是共享资产），因此本页自带一份 zh/en 双语表（COPY），组件内按
// useLocale() 选择。语言态复用 lib/i18n.ts 的模块级订阅（useSyncExternalStore +
// subscribe/getLanguage）——它已经把 "system" 三态按 navigator.language 解析成
// zh-CN / en-US，本页只需再映射成 "zh" | "en" 两个分支。
//
// 契约（Rust 侧已实现，命令名逐字对齐）：
//   get_settings / set_settings        设置读写（camelCase 字段）
//   permission_status                 权限状态 { screenRecording, accessibility }
//   open_permission_settings           打开系统设置面板 { which }
//   open_external                     打开外部链接 { url }（仅 http/https）
//   check_for_updates / install_update / restart_app  自动更新三段式
//   open_main_panel              打开主面板（历史浏览面）
//   hawk://update-progress             更新进度事件 { downloaded, total }
// ============================================================================
import {
  useCallback,
  useEffect,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  getLanguage,
  setLanguage,
  subscribe,
  type LanguageSetting,
} from "../../lib/i18n";

// —— 语言 ——

export type Locale = "zh" | "en";

/** 生效语言（zh-CN → zh，其余 → en）。 */
export function useLocale(): Locale {
  const lang = useSyncExternalStore(subscribe, getLanguage);
  return lang === "zh-CN" ? "zh" : "en";
}

// —— 文案表（zh / en 键必须完全对齐；en 的类型即由 zh 推导，漏键即编译报错）——

const zh = {
  "app.name": "猎影 HawkSnap",
  "app.running": "运行中",
  "nav.searchPlaceholder": "搜索设置",
  "nav.general": "通用",
  "nav.capture": "截屏",
  "nav.annotate": "标注",
  "nav.history": "历史记录",
  "nav.shortcuts": "键盘快捷键",
  "nav.permissions": "权限",
  "nav.menubar": "菜单栏",
  "nav.about": "关于",
  "nav.empty": "没有匹配的分区",
  "action.save": "保存",
  "action.saveTip": "保存设置",
  "action.saving": "保存中…",
  "action.saved": "已保存",
  "action.prev": "上一个分区",
  "action.next": "下一个分区",
  "action.details": "查看说明",
  "action.open": "打开",
  "action.restore": "恢复默认",
  "error.save": "保存失败：{message}",
  "error.generic": "操作失败：{message}",
  "error.permissionOpen": "打开系统设置失败：{message}",
  "error.link": "打开链接失败：{message}",
  "general.title": "通用",
  "general.appearance.title": "外观与语言",
  "general.appearance.desc": "控制设置窗口的配色与界面语言。",
  "general.appearance.label": "外观",
  "general.appearance.system": "跟随系统",
  "general.appearance.light": "浅色",
  "general.appearance.dark": "深色",
  "general.appearance.info": "「跟随系统」会随 macOS 的浅色 / 深色模式自动切换设置窗口的配色。",
  "general.language.label": "界面语言",
  "general.language.system": "跟随系统",
  "general.language.zh": "简体中文",
  "general.language.en": "English",
  "general.language.info": "「跟随系统」按 macOS 的语言偏好选择中文或英文界面。",
  "general.output.title": "输出",
  "general.output.desc": "截图与录屏的默认落盘格式。",
  "general.output.formatLabel": "输出格式",
  "general.output.png": "PNG",
  "general.output.jpeg": "JPEG",
  "general.output.formatInfo": "PNG 无损、体积较大，适合截图；JPEG 体积小，适合分享与录屏。",
  "general.output.qualityLabel": "JPEG 质量",
  "general.output.qualityInfo": "数值越高画质越好、体积越大；仅在输出格式为 JPEG 时生效。",
  "general.storage.title": "存储与启动",
  "general.storage.desc": "截图保存位置与开机行为。",
  "general.storage.saveDirLabel": "保存目录",
  "general.storage.saveDirPlaceholder": "~/Pictures/HawkSnap",
  "general.storage.saveDirInfo": "留空即使用默认目录 ~/Pictures/HawkSnap。",
  "general.storage.launchLabel": "开机自启",
  "general.storage.launchDesc": "登录 macOS 时自动启动猎影。",
  "general.update.title": "软件更新",
  "general.update.desc": "检查新版本并下载安装。",
  "general.update.autoLabel": "自动更新",
  "general.update.autoDesc": "启动时自动检查新版本，发现后在此提示下载。",
  "general.update.check": "检查更新",
  "general.update.checking": "正在检查…",
  "general.update.latest": "已是最新版本",
  "general.update.available": "发现新版本 {version}",
  "general.update.install": "下载并安装",
  "general.update.downloading": "正在下载并安装…",
  "general.update.progress": "已下载 {done} / {total}",
  "general.update.installed": "更新已安装，重启后生效",
  "general.update.restart": "重启以完成更新",
  "general.update.notConfigured": "更新通道尚未配置，暂时无法检查更新",
  "general.update.error": "更新操作失败：{message}",
  "capture.title": "截屏",
  "capture.hotkey.title": "截图热键",
  "capture.hotkey.desc": "全局生效，在任何应用中按下即可唤起截图。",
  "capture.hotkey.label": "截图",
  "capture.hotkey.info": "点击胶囊后按下想要的组合键；按下 Esc 取消。修改后在顶部点保存生效。",
  "capture.hotkey.setShortcut": "设置快捷键",
  "capture.hotkey.pressCombo": "按下组合键…",
  "capture.format.title": "默认截图格式",
  "capture.format.desc": "在「通用」分区选择 PNG 或 JPEG，并可调整 JPEG 质量。",
  "capture.format.current": "当前格式",
  "capture.modes.title": "截图模式",
  "capture.modes.desc": "在覆盖层中选择要截取的范围。",
  "capture.modes.regionTitle": "区域截图",
  "capture.modes.regionDesc": "拖拽选择任意矩形区域。",
  "capture.modes.windowTitle": "窗口截图",
  "capture.modes.windowDesc": "悬停高亮目标窗口，单击直接截取。",
  "capture.modes.screenTitle": "全屏截图",
  "capture.modes.screenDesc": "截取当前显示器的整块屏幕。",
  "capture.delay.title": "延时截图",
  "capture.delay.desc": "给界面一点准备时间再抓屏。",
  "capture.delay.itemTitle": "倒计时 1 / 2 / 3 / 5 秒",
  "capture.delay.itemDesc": "从菜单栏选择「延时截图」后开始倒计时，结束后自动抓屏；倒计时期间可随时取消。",
  "annotate.title": "标注",
  "annotate.tools.title": "编辑器工具",
  "annotate.tools.desc": "截图保存前可在标注编辑器中加工。",
  "annotate.tool.rect": "矩形",
  "annotate.tool.rectDesc": "框选并高亮一块区域。",
  "annotate.tool.ellipse": "椭圆",
  "annotate.tool.ellipseDesc": "圈出需要强调的重点。",
  "annotate.tool.arrow": "箭头",
  "annotate.tool.arrowDesc": "指向画面中的具体细节。",
  "annotate.tool.pen": "画笔",
  "annotate.tool.penDesc": "自由手绘标注。",
  "annotate.tool.highlighter": "荧光笔",
  "annotate.tool.highlighterDesc": "半透明笔迹划过文字段落。",
  "annotate.tool.mosaic": "马赛克",
  "annotate.tool.mosaicDesc": "遮挡画面中的隐私信息。",
  "annotate.tool.step": "序号",
  "annotate.tool.stepDesc": "按顺序给步骤编号。",
  "annotate.tool.text": "文字",
  "annotate.tool.textDesc": "在画面上添加文字说明。",
  "annotate.tool.crop": "裁剪",
  "annotate.tool.cropDesc": "裁掉画面中不需要的部分。",
  "annotate.shortcuts.title": "编辑器快捷键",
  "annotate.shortcuts.desc": "标注编辑器中可用的快捷键。",
  "annotate.shortcuts.undo": "撤销",
  "annotate.shortcuts.redo": "重做",
  "annotate.shortcuts.save": "保存",
  "history.title": "历史记录",
  "history.limit.title": "历史上限",
  "history.limit.desc": "只保留最近的记录，超出上限后自动淘汰最旧的条目。",
  "history.limit.value": "当前上限：{count} 条",
  "history.storage.title": "存储位置",
  "history.storage.desc": "历史索引保存在应用数据目录；截图文件与缩略图保存在「通用」分区设置的目录中（缩略图位于其 thumbs/ 子目录）。",
  "history.search.title": "文本搜索",
  "history.search.desc": "截图中的文字经 OCR 识别后进入全文索引，可在主面板直接搜索画面里的文字。",
  "history.openPanel": "在主面板中打开历史记录",
  "history.openPanelDesc": "打开主面板查看全部历史条目。",
  "shortcuts.title": "键盘快捷键",
  "shortcuts.global.title": "全局快捷键",
  "shortcuts.global.desc": "以下组合键在系统全局生效。点击胶囊后按下想要的组合键，按下 Esc 取消。",
  "shortcuts.slot.capture": "截图",
  "shortcuts.slot.pin": "贴图",
  "shortcuts.slot.record": "录屏",
  "shortcuts.captureInfo": "截图热键：按下后进入覆盖层，可选区域、窗口或全屏。",
  "shortcuts.pinInfo": "贴图热键：把最近一次截图钉在桌面最上层，方便对照查看。",
  "shortcuts.recordInfo": "录屏热键：开始 / 停止录制屏幕，录制中可通过悬浮条暂停。",
  "permissions.title": "权限",
  "permissions.status.title": "权限状态",
  "permissions.status.desc": "猎影需要以下两项 macOS 权限才能正常工作。",
  "permissions.screen.label": "屏幕录制",
  "permissions.screen.granted": "已授权",
  "permissions.screen.denied": "未授权",
  "permissions.screen.action": "打开系统设置",
  "permissions.screen.desc": "用于抓取屏幕内容与录制视频。",
  "permissions.accessibility.label": "辅助功能",
  "permissions.accessibility.granted": "已授权",
  "permissions.accessibility.denied": "未授权",
  "permissions.accessibility.action": "打开系统设置",
  "permissions.accessibility.desc": "用于智能吸附时的悬停取词与按键显示。",
  "permissions.unknown": "状态未知",
  "permissions.recheck": "重新检查",
  "permissions.usage.title": "权限用途",
  "permissions.usage.desc": "为什么需要这些权限，以及不授权会失去什么。",
  "menubar.title": "菜单栏",
  "menubar.items.title": "菜单项",
  "menubar.items.desc": "点击菜单栏中的猎影图标可打开以下菜单。",
  "menubar.item.capture": "截图",
  "menubar.item.captureDesc": "立即开始一次截图。",
  "menubar.item.delay": "延时截图",
  "menubar.item.delayDesc": "倒计时结束后自动截图。",
  "menubar.item.scrollshot": "长截图",
  "menubar.item.scrollshotDesc": "拖选区域后滚动页面，自动拼接成长图。",
  "menubar.item.scrollshotStop": "停止长截图",
  "menubar.item.scrollshotStopDesc": "结束当前的长截图拼接。",
  "menubar.item.pinCloseAll": "关闭全部贴图",
  "menubar.item.pinCloseAllDesc": "一次性收起所有钉在桌面的贴图。",
  "menubar.item.panel": "主面板",
  "menubar.item.panelDesc": "打开历史记录与快捷操作面板。",
  "menubar.item.settings": "设置",
  "menubar.item.settingsDesc": "打开本设置窗口。",
  "menubar.item.quit": "退出",
  "menubar.item.quitDesc": "退出猎影 HawkSnap。",
  "menubar.behavior.title": "常驻行为",
  "menubar.behavior.desc": "猎影常驻菜单栏（Windows / Linux 为系统托盘），macOS 下不显示 Dock 图标；关闭本窗口不会退出应用，退出请用菜单栏的「退出」。",
  "about.title": "关于",
  "about.app.title": "猎影 HawkSnap",
  "about.app.desc": "轻量的截图、标注与录屏工具，支持 macOS / Windows / Linux。",
  "about.license.label": "许可",
  "about.app.license": "MIT 许可",
  "about.app.version": "版本 {version}",
  "about.links.title": "链接",
  "about.links.desc": "源码、下载与许可声明。",
  "about.link.repo": "源代码仓库",
  "about.link.repoDesc": "在 GitHub 上查看源码、反馈问题与参与贡献。",
  "about.link.site": "官方网站",
  "about.link.siteDesc": "产品介绍、下载与更新日志。",
  "about.link.thirdParty": "第三方组件声明",
  "about.link.thirdPartyDesc": "本项目依赖的第三方组件及其许可。",
  "about.credits.title": "依赖致谢",
  "about.credits.desc": "猎影建立在以下开源项目之上。",
  "about.credit.tauri": "Tauri",
  "about.credit.tauriDesc": "跨平台桌面应用框架（MIT / Apache-2.0）。",
  "about.credit.react": "React",
  "about.credit.reactDesc": "界面构建库（MIT）。",
  "about.credit.rust": "Rust",
  "about.credit.rustDesc": "后端与原生能力实现语言（MIT / Apache-2.0）。",
  "about.credit.gif": "gif",
  "about.credit.gifDesc": "GIF 编码（MIT / Apache-2.0，替代此前的 gifski）。",
} as const;

export type CopyKey = keyof typeof zh;

const en: Record<CopyKey, string> = {
  "app.name": "HawkSnap",
  "app.running": "Running",
  "nav.searchPlaceholder": "Search settings",
  "nav.general": "General",
  "nav.capture": "Capture",
  "nav.annotate": "Annotate",
  "nav.history": "History",
  "nav.shortcuts": "Keyboard Shortcuts",
  "nav.permissions": "Permissions",
  "nav.menubar": "Menu Bar",
  "nav.about": "About",
  "nav.empty": "No matching sections",
  "action.save": "Save",
  "action.saveTip": "Save settings",
  "action.saving": "Saving…",
  "action.saved": "Saved",
  "action.prev": "Previous section",
  "action.next": "Next section",
  "action.details": "Show details",
  "action.open": "Open",
  "action.restore": "Restore default",
  "error.save": "Save failed: {message}",
  "error.generic": "Action failed: {message}",
  "error.permissionOpen": "Could not open System Settings: {message}",
  "error.link": "Could not open link: {message}",
  "general.title": "General",
  "general.appearance.title": "Appearance & Language",
  "general.appearance.desc": "Controls the palette and the interface language.",
  "general.appearance.label": "Appearance",
  "general.appearance.system": "Match System",
  "general.appearance.light": "Light",
  "general.appearance.dark": "Dark",
  "general.appearance.info": "“Match System” follows the macOS light / dark mode for this window.",
  "general.language.label": "Interface language",
  "general.language.system": "Match System",
  "general.language.zh": "简体中文",
  "general.language.en": "English",
  "general.language.info": "“Match System” picks Chinese or English from the macOS language preference.",
  "general.output.title": "Output",
  "general.output.desc": "Default file format for captures and recordings.",
  "general.output.formatLabel": "Output format",
  "general.output.png": "PNG",
  "general.output.jpeg": "JPEG",
  "general.output.formatInfo": "PNG is lossless but larger — good for screenshots; JPEG is small — good for sharing and recordings.",
  "general.output.qualityLabel": "JPEG quality",
  "general.output.qualityInfo": "Higher values keep more detail but produce bigger files; only used when the format is JPEG.",
  "general.storage.title": "Storage & Startup",
  "general.storage.desc": "Where captures are saved and whether HawkSnap starts with macOS.",
  "general.storage.saveDirLabel": "Save folder",
  "general.storage.saveDirPlaceholder": "~/Pictures/HawkSnap",
  "general.storage.saveDirInfo": "Leave it empty to use the default folder ~/Pictures/HawkSnap.",
  "general.storage.launchLabel": "Launch at login",
  "general.storage.launchDesc": "Start HawkSnap automatically when you log in to macOS.",
  "general.update.title": "Software Update",
  "general.update.desc": "Check for new versions and install them.",
  "general.update.autoLabel": "Automatic updates",
  "general.update.autoDesc": "Check for new versions on launch and offer the download here.",
  "general.update.check": "Check for Updates",
  "general.update.checking": "Checking…",
  "general.update.latest": "You are up to date",
  "general.update.available": "Version {version} is available",
  "general.update.install": "Download and Install",
  "general.update.downloading": "Downloading and installing…",
  "general.update.progress": "{done} of {total} downloaded",
  "general.update.installed": "Update installed — restart to finish",
  "general.update.restart": "Restart to Finish Update",
  "general.update.notConfigured": "The update channel is not configured yet",
  "general.update.error": "Update failed: {message}",
  "capture.title": "Capture",
  "capture.hotkey.title": "Capture Hotkey",
  "capture.hotkey.desc": "Works globally — press it in any app to start a capture.",
  "capture.hotkey.label": "Capture",
  "capture.hotkey.info": "Click the chip, then press the combination you want. Press Esc to cancel. Save at the top to apply.",
  "capture.hotkey.setShortcut": "Set Shortcut",
  "capture.hotkey.pressCombo": "Press a shortcut…",
  "capture.format.title": "Default Capture Format",
  "capture.format.desc": "Pick PNG or JPEG in the General section and adjust the JPEG quality there.",
  "capture.format.current": "Current format",
  "capture.modes.title": "Capture Modes",
  "capture.modes.desc": "Choose what to capture from the overlay.",
  "capture.modes.regionTitle": "Region",
  "capture.modes.regionDesc": "Drag to select any rectangular area.",
  "capture.modes.windowTitle": "Window",
  "capture.modes.windowDesc": "Hover to highlight a window, click to capture it.",
  "capture.modes.screenTitle": "Full Screen",
  "capture.modes.screenDesc": "Capture the entire display.",
  "capture.delay.title": "Delayed Capture",
  "capture.delay.desc": "Give the screen a moment before it is captured.",
  "capture.delay.itemTitle": "1 / 2 / 3 / 5 second countdown",
  "capture.delay.itemDesc": "Pick “Delayed capture” from the menu bar to start a countdown; the capture fires when it ends, and you can cancel at any time.",
  "annotate.title": "Annotate",
  "annotate.tools.title": "Editor Tools",
  "annotate.tools.desc": "Polish a capture in the annotation editor before saving.",
  "annotate.tool.rect": "Rectangle",
  "annotate.tool.rectDesc": "Box and highlight an area.",
  "annotate.tool.ellipse": "Ellipse",
  "annotate.tool.ellipseDesc": "Circle what matters.",
  "annotate.tool.arrow": "Arrow",
  "annotate.tool.arrowDesc": "Point at a specific detail.",
  "annotate.tool.pen": "Pen",
  "annotate.tool.penDesc": "Draw freehand.",
  "annotate.tool.highlighter": "Highlighter",
  "annotate.tool.highlighterDesc": "Mark up text with translucent ink.",
  "annotate.tool.mosaic": "Mosaic",
  "annotate.tool.mosaicDesc": "Hide private information.",
  "annotate.tool.step": "Step number",
  "annotate.tool.stepDesc": "Number the steps in order.",
  "annotate.tool.text": "Text",
  "annotate.tool.textDesc": "Add a text note to the image.",
  "annotate.tool.crop": "Crop",
  "annotate.tool.cropDesc": "Trim the parts you do not need.",
  "annotate.shortcuts.title": "Editor Shortcuts",
  "annotate.shortcuts.desc": "Shortcuts available inside the annotation editor.",
  "annotate.shortcuts.undo": "Undo",
  "annotate.shortcuts.redo": "Redo",
  "annotate.shortcuts.save": "Save",
  "history.title": "History",
  "history.limit.title": "History Limit",
  "history.limit.desc": "Only the most recent entries are kept; older ones are dropped automatically.",
  "history.limit.value": "Current limit: {count} entries",
  "history.storage.title": "Storage Location",
  "history.storage.desc": "The history index lives in the app data folder; captures and their thumbnails are saved to the folder set in General (thumbnails in its thumbs/ subfolder).",
  "history.search.title": "Text Search",
  "history.search.desc": "Recognized text is indexed with OCR, so you can search the words inside a capture from the main panel.",
  "history.openPanel": "Open History in the main panel",
  "history.openPanelDesc": "Open the main panel to see every history entry.",
  "shortcuts.title": "Keyboard Shortcuts",
  "shortcuts.global.title": "Global Shortcuts",
  "shortcuts.global.desc": "These combinations work system-wide. Click a chip, then press the combination you want; press Esc to cancel.",
  "shortcuts.slot.capture": "Capture",
  "shortcuts.slot.pin": "Pin",
  "shortcuts.slot.record": "Record",
  "shortcuts.captureInfo": "Capture hotkey: opens the overlay, where you pick a region, a window or the full screen.",
  "shortcuts.pinInfo": "Pin hotkey: keeps the latest capture on top of other windows for easy comparison.",
  "shortcuts.recordInfo": "Record hotkey: starts and stops a screen recording; pause from the recording bar.",
  "permissions.title": "Permissions",
  "permissions.status.title": "Permission Status",
  "permissions.status.desc": "HawkSnap needs the following two macOS permissions to work properly.",
  "permissions.screen.label": "Screen Recording",
  "permissions.screen.granted": "Granted",
  "permissions.screen.denied": "Not granted",
  "permissions.screen.action": "Open System Settings",
  "permissions.screen.desc": "Required to capture screen content and to record video.",
  "permissions.accessibility.label": "Accessibility",
  "permissions.accessibility.granted": "Granted",
  "permissions.accessibility.denied": "Not granted",
  "permissions.accessibility.action": "Open System Settings",
  "permissions.accessibility.desc": "Required for hover picking while smart snapping and for the key display overlay.",
  "permissions.unknown": "Status unknown",
  "permissions.recheck": "Re-check",
  "permissions.usage.title": "What the Permissions Are For",
  "permissions.usage.desc": "Why each permission is needed, and what stops working without it.",
  "menubar.title": "Menu Bar",
  "menubar.items.title": "Menu Items",
  "menubar.items.desc": "Click the HawkSnap icon in the menu bar to open this menu.",
  "menubar.item.capture": "Capture",
  "menubar.item.captureDesc": "Start a capture right away.",
  "menubar.item.delay": "Delayed capture",
  "menubar.item.delayDesc": "Capture after a countdown.",
  "menubar.item.scrollshot": "Scrolling shot",
  "menubar.item.scrollshotDesc": "Pick a region, scroll the page, get one long image.",
  "menubar.item.scrollshotStop": "Stop scrolling shot",
  "menubar.item.scrollshotStopDesc": "End the current scrolling capture.",
  "menubar.item.pinCloseAll": "Close all pins",
  "menubar.item.pinCloseAllDesc": "Dismiss every pinned image at once.",
  "menubar.item.panel": "Main panel",
  "menubar.item.panelDesc": "Open the history and quick-action panel.",
  "menubar.item.settings": "Settings",
  "menubar.item.settingsDesc": "Open this settings window.",
  "menubar.item.quit": "Quit",
  "menubar.item.quitDesc": "Quit HawkSnap.",
  "menubar.behavior.title": "Background Behavior",
  "menubar.behavior.desc": "HawkSnap lives in the menu bar (system tray on Windows / Linux) and shows no Dock icon on macOS. Closing this window does not quit the app — use “Quit” in the menu instead.",
  "about.title": "About",
  "about.app.title": "HawkSnap",
  "about.app.desc": "A lightweight screenshot, annotation and screen recording tool for macOS, Windows and Linux.",
  "about.license.label": "License",
  "about.app.license": "MIT License",
  "about.app.version": "Version {version}",
  "about.links.title": "Links",
  "about.links.desc": "Source code, downloads and license notices.",
  "about.link.repo": "Source repository",
  "about.link.repoDesc": "Browse the source, report issues and contribute on GitHub.",
  "about.link.site": "Official website",
  "about.link.siteDesc": "Product page, downloads and changelog.",
  "about.link.thirdParty": "Third-party notices",
  "about.link.thirdPartyDesc": "The third-party components this project builds on and their licenses.",
  "about.credits.title": "Credits",
  "about.credits.desc": "HawkSnap stands on the shoulders of these open source projects.",
  "about.credit.tauri": "Tauri",
  "about.credit.tauriDesc": "The cross-platform desktop framework (MIT / Apache-2.0).",
  "about.credit.react": "React",
  "about.credit.reactDesc": "The interface library (MIT).",
  "about.credit.rust": "Rust",
  "about.credit.rustDesc": "The language behind the backend and native features (MIT / Apache-2.0).",
  "about.credit.gif": "gif",
  "about.credit.gifDesc": "GIF encoding (MIT / Apache-2.0, replacing the former gifski).",
};

export const COPY: Record<Locale, Record<CopyKey, string>> = { zh, en };

export type CopyVars = Record<string, string | number>;

/** {name} 占位符插值（缺参占位符原样保留）。 */
function fill(template: string, vars?: CopyVars): string {
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (m, name) =>
    Object.prototype.hasOwnProperty.call(vars, name) ? String(vars[name]) : m,
  );
}

export type TFn = (key: CopyKey, vars?: CopyVars) => string;

/** 取当前语言下的文案（组件内直接用，不经 locales 词典）。 */
export function useT(): TFn {
  const locale = useLocale();
  return useCallback<TFn>(
    (key, vars) => fill(COPY[locale][key], vars),
    [locale],
  );
}

// —— 设置模型（与 Rust Settings 的 camelCase 字段逐项对齐；saveDir 归一为 string）——

export interface SettingsForm {
  hotkeyCapture: string;
  hotkeyPin: string;
  hotkeyRecord: string;
  outputFormat: string;
  jpegQuality: number;
  saveDir: string;
  launchAtLogin: boolean;
  appearance: string;
  language: string;
  updateEnabled: boolean;
}

export type HotkeySlot = "hotkeyCapture" | "hotkeyPin" | "hotkeyRecord";

export interface SectionProps {
  form: SettingsForm;
  setField: <K extends keyof SettingsForm>(key: K, value: SettingsForm[K]) => void;
  capturing: HotkeySlot | null;
  setCapturing: (slot: HotkeySlot | null) => void;
  notifyError: (message: string) => void;
}

// —— 图标（24×24 线性图标，stroke=currentColor）——

export type IconName =
  | "search"
  | "chevronLeft"
  | "chevronRight"
  | "gear"
  | "info"
  | "save"
  | "external"
  | "sliders"
  | "camera"
  | "pen"
  | "clock"
  | "keyboard"
  | "shield"
  | "menuBar"
  | "folder"
  | "appMark"
  | "infoCircle";

const ICON_SHAPES: Record<IconName, ReactNode> = {
  search: (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="M15.8 15.8 20 20" />
    </>
  ),
  chevronLeft: <path d="M14.5 6.5 9 12l5.5 5.5" />,
  chevronRight: <path d="M9.5 6.5 15 12l-5.5 5.5" />,
  gear: (
    <>
      <circle cx="12" cy="12" r="3.2" />
      <path d="M12 3.6v2.1M12 18.3v2.1M4.9 7.9l1.8 1.1M17.3 15l1.8 1.1M4.9 16.1l1.8-1.1M17.3 9l1.8-1.1" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 11.2v5M12 7.9h.01" />
    </>
  ),
  save: (
    <>
      <path d="M5 5.5h10.2L19.5 9.8V18a1.5 1.5 0 0 1-1.5 1.5H6A1.5 1.5 0 0 1 4.5 18V7A1.5 1.5 0 0 1 6 5.5Z" />
      <path d="M8 5.5v4h6v-4M8 19.5v-5h8v5" />
    </>
  ),
  folder: (
    <>
      <path d="M3.5 6.5A1.5 1.5 0 0 1 5 5h4l2 2.5h8a1.5 1.5 0 0 1 1.5 1.5V18a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 18Z" />
    </>
  ),
  appMark: (
    // 光圈 + 鹰眼（真应用图标的简化线稿：外圈光圈叶片 + 中间金色瞳孔）
    <>
      <circle cx="12" cy="12" r="8.2" />
      <path d="M12 3.8 12 12 18.4 15.9M12 12 7.2 18.1M12 12 5.6 8.1M12 12 16.8 5.9" />
      <circle cx="12" cy="12" r="3.2" />
    </>
  ),
  external: (
    <>
      <path d="M14 4.5h5.5V10" />
      <path d="M19.5 4.5 11 13" />
      <path d="M18 13.5V19H5V6h5.5" />
    </>
  ),
  sliders: (
    <>
      <path d="M4 8h9M17 8h3M4 16h3M11 16h9" />
      <circle cx="15" cy="8" r="2" />
      <circle cx="9" cy="16" r="2" />
    </>
  ),
  camera: (
    <>
      <path d="M4 8.6A2.6 2.6 0 0 1 6.6 6h1.1l1.1-1.5a1 1 0 0 1 .8-.4h4.8a1 1 0 0 1 .8.4L16.3 6h1.1A2.6 2.6 0 0 1 20 8.6v7.8a2.6 2.6 0 0 1-2.6 2.6H6.6A2.6 2.6 0 0 1 4 16.4Z" />
      <circle cx="12" cy="12.4" r="3.2" />
    </>
  ),
  pen: (
    <>
      <path d="M4 20l1.2-4.2L16.4 4.6a2 2 0 0 1 2.8 0l.2.2a2 2 0 0 1 0 2.8L8.2 18.8 4 20Z" />
      <path d="M14.8 6.2l3 3" />
    </>
  ),
  clock: (
    <>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 7.6V12l2.8 1.8" />
    </>
  ),
  keyboard: (
    <>
      <rect x="3" y="6.5" width="18" height="11" rx="2.5" />
      <path d="M7 10.2h.01M10.5 10.2h.01M14 10.2h.01M17.5 10.2h.01M8 13.8h8" />
    </>
  ),
  shield: (
    <>
      <path d="M12 3.5l7 2.4v5.3c0 4.2-2.9 7.6-7 9.3-4.1-1.7-7-5.1-7-9.3V5.9Z" />
      <path d="M9 12.2l2.2 2.2 4.2-4.4" />
    </>
  ),
  menuBar: (
    <>
      <path d="M3.5 12.5h17c0 3.6-2.7 6.5-6 6.5h-5c-3.3 0-6-2.9-6-6.5Z" />
      <path d="M8 9.4h.01M12 9.4h.01M16 9.4h.01" />
    </>
  ),
  infoCircle: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 11.2v5M12 7.9h.01" />
    </>
  ),
};

export function Icon({ name }: { name: IconName }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {ICON_SHAPES[name]}
    </svg>
  );
}


// 编辑器快捷键的修饰键随平台（与编辑器实现一致：mac 走 metaKey，其余走 ctrlKey）。
const MOD_KEY = /Mac|iP(hone|ad|od)/.test(navigator.platform) ? "⌘" : "Ctrl";

// —— 共用 UI 原子 ——

export function Card({
  icon,
  title,
  desc,
  trailing,
  children,
}: {
  icon: IconName;
  title: string;
  desc?: string;
  trailing?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <section className="hs-card">
      <header className="hs-card-head">
        <span className="hs-card-icon">
          <Icon name={icon} />
        </span>
        <div className="hs-card-head-text">
          <h2 className="hs-card-title">{title}</h2>
          {desc ? <p className="hs-card-desc">{desc}</p> : null}
        </div>
        {trailing}
      </header>
      {children ? <div className="hs-card-body">{children}</div> : null}
    </section>
  );
}

/**
 * 设置行：左 = 标题（+ ⓘ 说明），右 = 控件；行间由 CSS 画 1px 细分隔线。
 * ⓘ 与齿轮按钮共用同一个展开态（点谁都能翻出说明行）。
 */
function Row({
  label,
  info,
  gearTip,
  children,
}: {
  label: string;
  info?: string;
  gearTip?: string;
  children?: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const toggle = useCallback(() => setOpen((v) => !v), []);
  return (
    <div className="hs-row">
      <div className="hs-row-head">
        <span className="hs-row-label">{label}</span>
        {info ? (
          <button
            type="button"
            className={`hs-info-btn${open ? " is-open" : ""}`}
            aria-expanded={open}
            aria-label={info}
            onClick={toggle}
          >
            <Icon name="info" />
          </button>
        ) : null}
      </div>
      <div className="hs-row-control">
        {children}
        {info && gearTip ? (
          <button
            type="button"
            className="hs-icon-btn"
            title={gearTip}
            aria-label={gearTip}
            aria-expanded={open}
            onClick={toggle}
          >
            <Icon name="gear" />
          </button>
        ) : null}
      </div>
      {open && info ? <p className="hs-info-box">{info}</p> : null}
    </div>
  );
}

export function Rows({ children }: { children: ReactNode }) {
  return <div className="hs-rows">{children}</div>;
}

/** 说明清单（只讲事实，不造假开关）。 */
export function List({ head, children }: { head?: string; children: ReactNode }) {
  return (
    <ul className="hs-list">
      {head ? (
        <li className="hs-list-item hs-list-head">
          <span className="hs-list-title">{head}</span>
        </li>
      ) : null}
      {children}
    </ul>
  );
}

export function ListItem({ title, desc }: { title: string; desc: string }) {
  return (
    <li className="hs-list-item">
      <span className="hs-list-title">{title}</span>
      <span className="hs-list-desc">{desc}</span>
    </li>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={`hs-toggle${checked ? " is-on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="hs-toggle-knob" />
    </button>
  );
}

export interface RadioOption {
  value: string;
  label: string;
}

/** 分段式单选（radio 语义 + macOS segmented 观感）。 */
export function RadioGroup({
  name,
  value,
  options,
  onChange,
}: {
  name: string;
  value: string;
  options: RadioOption[];
  onChange: (next: string) => void;
}) {
  return (
    <div className="hs-segment" role="radiogroup">
      {options.map((opt) => (
        <label
          key={opt.value}
          className={`hs-segment-item${value === opt.value ? " is-active" : ""}`}
        >
          <input
            className="hs-segment-input"
            type="radio"
            name={name}
            value={opt.value}
            checked={value === opt.value}
            onChange={() => onChange(opt.value)}
          />
          <span className="hs-segment-dot" aria-hidden="true" />
          <span className="hs-segment-label">{opt.label}</span>
        </label>
      ))}
    </div>
  );
}

// —— 热键 ——

const MODIFIER_SYMBOL: Record<string, string> = {
  Cmd: "⌘",
  Command: "⌘",
  Meta: "⌘",
  Shift: "⇧",
  Alt: "⌥",
  Option: "⌥",
  Ctrl: "⌃",
  Control: "⌃",
};

/** "Cmd+Shift+A" → ["⌘","⇧","A"]（胶囊展示用）。 */
export function hotkeyParts(combo: string): string[] {
  return combo
    .split("+")
    .map((p) => p.trim())
    .filter((p) => p !== "")
    .map((p) => {
      const sym = MODIFIER_SYMBOL[p];
      if (sym) return sym;
      if (p === "Space") return "Space";
      return p.length === 1 ? p.toUpperCase() : p;
    });
}

const MODIFIER_ONLY_KEYS = new Set([
  "Meta",
  "Shift",
  "Alt",
  "Control",
  "CapsLock",
  "Fn",
  "FnLock",
  "Hyper",
  "Symbol",
]);

/**
 * 组合键 → 插件词汇表字符串（global-hotkey parse_hotkey：修饰键在前 + 主键，
 * "+" 连接；主键大小写不敏感）。与主面板 App.tsx 同款逻辑（保持两处生成的
 * 字符串格式一致，否则服务端解析口径会分叉）。
 */
export function comboFromEvent(e: KeyboardEvent): string | null {
  if (MODIFIER_ONLY_KEYS.has(e.key)) return null;
  const mods: string[] = [];
  if (e.metaKey) mods.push("Cmd");
  if (e.shiftKey) mods.push("Shift");
  if (e.altKey) mods.push("Alt");
  if (e.ctrlKey) mods.push("Ctrl");
  let key: string;
  if (/^Digit\d$/.test(e.code)) key = e.code.slice(5);
  else if (e.key === " ") key = "Space";
  else if (e.key.length === 1) key = e.key.toUpperCase();
  else key = e.key;
  return [...mods, key].join("+");
}

export function HotkeyChip({
  combo,
  capturing,
  onStart,
  onCancel,
  setLabel,
  pressLabel,
}: {
  combo: string;
  capturing: boolean;
  onStart: () => void;
  onCancel: () => void;
  setLabel: string;
  pressLabel: string;
}) {
  if (capturing) {
    return (
      <button type="button" className="hs-chip" onClick={onCancel}>
        <span className="hs-chip-hint">{pressLabel}</span>
      </button>
    );
  }
  const parts = hotkeyParts(combo);
  if (parts.length === 0) {
    return (
      <button type="button" className="hs-chip hs-chip-empty" onClick={onStart}>
        {setLabel}
      </button>
    );
  }
  return (
    <button type="button" className="hs-chip" title={combo} onClick={onStart}>
      {parts.map((p, i) => (
        <kbd className="hs-key" key={`${p}-${i}`}>
          {p}
        </kbd>
      ))}
    </button>
  );
}

// —— 自动更新（D3）—— 与后端 updater.rs 对齐 ——

interface UpdateInfo {
  version: string;
  notes: string | null;
  currentVersion: string;
}

interface UpdateProgress {
  downloaded: number;
  total: number | null;
}

type UpdateState =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "latest" }
  | { phase: "available"; info: UpdateInfo }
  | { phase: "downloading"; progress: UpdateProgress | null }
  | { phase: "installed" };

// 后端哨兵（updater.rs ERR_NOT_CONFIGURED）：更新通道未配置是常态（占位公钥期），
// 映射为可读文案而非原始错误串。
const ERR_UPDATER_NOT_CONFIGURED = "updater_not_configured";

const fmtMb = (bytes: number): string => (bytes / 1024 / 1024).toFixed(1);

// —— 分区 1/8：通用 General ——

export function GeneralSection({ form, setField }: SectionProps) {
  const tx = useT();
  const [update, setUpdate] = useState<UpdateState>({ phase: "idle" });
  const [updateError, setUpdateError] = useState<string | null>(null);

  // 下载进度 push 通道（hawk://update-progress）；无事件时显示不确定态文案。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<UpdateProgress>("hawk://update-progress", (e) => {
      if (disposed) return;
      setUpdate((s) =>
        s.phase === "downloading" ? { phase: "downloading", progress: e.payload } : s,
      );
    })
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const updateErrorText = (e: unknown): string =>
    String(e) === ERR_UPDATER_NOT_CONFIGURED
      ? tx("general.update.notConfigured")
      : tx("general.update.error", { message: String(e) });

  // 手动检查（manual=true 覆盖「自动更新」开关——用户显式动作）。
  const checkForUpdates = () => {
    setUpdateError(null);
    setUpdate({ phase: "checking" });
    invoke<UpdateInfo | null>("check_for_updates", { manual: true })
      .then((info) =>
        setUpdate(info ? { phase: "available", info } : { phase: "latest" }),
      )
      .catch((e) => {
        setUpdate({ phase: "idle" });
        setUpdateError(updateErrorText(e));
      });
  };

  // 下载并安装：进度经 hawk://update-progress；成功 → installed（提示重启）。
  const installUpdate = () => {
    setUpdateError(null);
    setUpdate({ phase: "downloading", progress: null });
    invoke<void>("install_update")
      .then(() => setUpdate({ phase: "installed" }))
      .catch((e) => {
        setUpdate({ phase: "idle" });
        setUpdateError(updateErrorText(e));
      });
  };

  // 立即重启（后端直接替换进程，不返回）。
  const restartApp = () => {
    invoke<void>("restart_app").catch((e) => setUpdateError(updateErrorText(e)));
  };

  // 启动静默检查（manual=false → 受「自动更新」开关门控；通道未配置时后端返回
  // Ok(None) 而非错误，故此处不弹错）。
  useEffect(() => {
    let disposed = false;
    invoke<UpdateInfo | null>("check_for_updates", { manual: false })
      .then((info) => {
        if (disposed || !info) return;
        setUpdate((s) => (s.phase === "idle" ? { phase: "available", info } : s));
      })
      .catch(() => {});
    return () => {
      disposed = true;
    };
  }, []);

  const jpegSelected = form.outputFormat === "jpeg";

  return (
    <div className="hs-cards">
      <Card
        icon="sliders"
        title={tx("general.appearance.title")}
        desc={tx("general.appearance.desc")}
      >
        <Rows>
          <Row label={tx("general.appearance.label")} info={tx("general.appearance.info")}>
            <RadioGroup
              name="set-appearance"
              value={form.appearance}
              options={[
                { value: "system", label: tx("general.appearance.system") },
                { value: "light", label: tx("general.appearance.light") },
                { value: "dark", label: tx("general.appearance.dark") },
              ]}
              onChange={(v) => setField("appearance", v)}
            />
          </Row>
          <Row label={tx("general.language.label")} info={tx("general.language.info")}>
            <RadioGroup
              name="set-language"
              value={form.language}
              options={[
                { value: "system", label: tx("general.language.system") },
                { value: "zh-CN", label: tx("general.language.zh") },
                { value: "en-US", label: tx("general.language.en") },
              ]}
              onChange={(v) => {
                setField("language", v);
                // 语言即时生效（本窗口的 i18n 模块态；与主面板互不干扰）。
                setLanguage(v as LanguageSetting);
              }}
            />
          </Row>
        </Rows>
      </Card>

      <Card icon="camera" title={tx("general.output.title")} desc={tx("general.output.desc")}>
        <Rows>
          <Row label={tx("general.output.formatLabel")} info={tx("general.output.formatInfo")}>
            <RadioGroup
              name="set-output-format"
              value={form.outputFormat}
              options={[
                { value: "png", label: tx("general.output.png") },
                { value: "jpeg", label: tx("general.output.jpeg") },
              ]}
              onChange={(v) => setField("outputFormat", v)}
            />
          </Row>
          {jpegSelected ? (
            <Row label={tx("general.output.qualityLabel")} info={tx("general.output.qualityInfo")}>
              <span className="hs-slider">
                <input
                  type="range"
                  min={1}
                  max={100}
                  value={form.jpegQuality}
                  aria-label={tx("general.output.qualityLabel")}
                  onChange={(e) => setField("jpegQuality", Number(e.target.value))}
                />
                <span className="hs-slider-value">{form.jpegQuality}</span>
              </span>
            </Row>
          ) : null}
        </Rows>
      </Card>

      <Card
        icon="folder"
        title={tx("general.storage.title")}
        desc={tx("general.storage.desc")}
      >
        <Rows>
          <Row label={tx("general.storage.saveDirLabel")} info={tx("general.storage.saveDirInfo")}>
            <input
              className="hs-input"
              type="text"
              value={form.saveDir}
              placeholder={tx("general.storage.saveDirPlaceholder")}
              aria-label={tx("general.storage.saveDirLabel")}
              onChange={(e) => setField("saveDir", e.target.value)}
            />
            <button
              type="button"
              className="hs-btn"
              onClick={() => setField("saveDir", "")}
              title={tx("general.storage.saveDirInfo")}
            >
              {tx("action.restore")}
            </button>
          </Row>
          <Row label={tx("general.storage.launchLabel")} info={tx("general.storage.launchDesc")}>
            <Toggle
              checked={form.launchAtLogin}
              onChange={(v) => setField("launchAtLogin", v)}
              label={tx("general.storage.launchLabel")}
            />
          </Row>
          <Row label={tx("general.update.autoLabel")} info={tx("general.update.autoDesc")}>
            <Toggle
              checked={form.updateEnabled}
              onChange={(v) => setField("updateEnabled", v)}
              label={tx("general.update.autoLabel")}
            />
          </Row>
        </Rows>
      </Card>

      <Card
        icon="infoCircle"
        title={tx("general.update.title")}
        desc={tx("general.update.desc")}
      >
        <div className="hs-btn-row">
          <button
            type="button"
            className="hs-btn"
            onClick={checkForUpdates}
            disabled={update.phase === "checking" || update.phase === "downloading"}
          >
            {update.phase === "checking"
              ? tx("general.update.checking")
              : tx("general.update.check")}
          </button>
          {update.phase === "available" ? (
            <button type="button" className="hs-btn hs-btn-primary" onClick={installUpdate}>
              {tx("general.update.install")}
            </button>
          ) : null}
          {update.phase === "installed" ? (
            <button type="button" className="hs-btn hs-btn-primary" onClick={restartApp}>
              {tx("general.update.restart")}
            </button>
          ) : null}
          {update.phase === "latest" ? (
            <span className="hs-status-text hs-status-text-ok">{tx("general.update.latest")}</span>
          ) : null}
          {update.phase === "available" ? (
            <span className="hs-status-text">
              {tx("general.update.available", { version: update.info.version })}
            </span>
          ) : null}
          {update.phase === "downloading" ? (
            <span className="hs-status-text">
              {update.progress && update.progress.total != null
                ? tx("general.update.progress", {
                    done: fmtMb(update.progress.downloaded),
                    total: fmtMb(update.progress.total),
                  })
                : tx("general.update.downloading")}
            </span>
          ) : null}
          {update.phase === "installed" ? (
            <span className="hs-status-text hs-status-text-ok">
              {tx("general.update.installed")}
            </span>
          ) : null}
        </div>
        {update.phase === "available" && update.info.notes ? (
          <p className="hs-note">{update.info.notes}</p>
        ) : null}
        {updateError ? (
          <p className="hs-note" role="alert">
            {updateError}
          </p>
        ) : null}
      </Card>
    </div>
  );
}

// —— 分区 2/8：截屏 Capture ——

export function CaptureSection({ form, capturing, setCapturing }: SectionProps) {
  const tx = useT();
  return (
    <div className="hs-cards">
      <Card
        icon="keyboard"
        title={tx("capture.hotkey.title")}
        desc={tx("capture.hotkey.desc")}
      >
        <Rows>
          <Row
            label={tx("capture.hotkey.label")}
            info={tx("capture.hotkey.info")}
            gearTip={tx("action.details")}
          >
            <HotkeyChip
              combo={form.hotkeyCapture}
              capturing={capturing === "hotkeyCapture"}
              onStart={() => setCapturing("hotkeyCapture")}
              onCancel={() => setCapturing(null)}
              setLabel={tx("capture.hotkey.setShortcut")}
              pressLabel={tx("capture.hotkey.pressCombo")}
            />
          </Row>
        </Rows>
      </Card>

      <Card
        icon="camera"
        title={tx("capture.format.title")}
        desc={tx("capture.format.desc")}
      >
        <Rows>
          <Row label={tx("capture.format.current")}>
            <span className="hs-status-text">
              {form.outputFormat === "jpeg"
                ? `${tx("general.output.jpeg")} · ${form.jpegQuality}`
                : tx("general.output.png")}
            </span>
          </Row>
        </Rows>
      </Card>

      <Card icon="sliders" title={tx("capture.modes.title")} desc={tx("capture.modes.desc")}>
        <List>
          <ListItem title={tx("capture.modes.regionTitle")} desc={tx("capture.modes.regionDesc")} />
          <ListItem title={tx("capture.modes.windowTitle")} desc={tx("capture.modes.windowDesc")} />
          <ListItem title={tx("capture.modes.screenTitle")} desc={tx("capture.modes.screenDesc")} />
        </List>
      </Card>

      <Card icon="clock" title={tx("capture.delay.title")} desc={tx("capture.delay.desc")}>
        <List>
          <ListItem
            title={tx("capture.delay.itemTitle")}
            desc={tx("capture.delay.itemDesc")}
          />
        </List>
      </Card>
    </div>
  );
}

// —— 分区 3/8：标注 Annotate ——

export function AnnotateSection(_props: SectionProps) {
  const tx = useT();
  return (
    <div className="hs-cards">
      <Card icon="pen" title={tx("annotate.tools.title")} desc={tx("annotate.tools.desc")}>
        <List>
          <ListItem title={tx("annotate.tool.rect")} desc={tx("annotate.tool.rectDesc")} />
          <ListItem title={tx("annotate.tool.ellipse")} desc={tx("annotate.tool.ellipseDesc")} />
          <ListItem title={tx("annotate.tool.arrow")} desc={tx("annotate.tool.arrowDesc")} />
          <ListItem title={tx("annotate.tool.pen")} desc={tx("annotate.tool.penDesc")} />
          <ListItem
            title={tx("annotate.tool.highlighter")}
            desc={tx("annotate.tool.highlighterDesc")}
          />
          <ListItem title={tx("annotate.tool.mosaic")} desc={tx("annotate.tool.mosaicDesc")} />
          <ListItem title={tx("annotate.tool.step")} desc={tx("annotate.tool.stepDesc")} />
          <ListItem title={tx("annotate.tool.text")} desc={tx("annotate.tool.textDesc")} />
          <ListItem title={tx("annotate.tool.crop")} desc={tx("annotate.tool.cropDesc")} />
        </List>
      </Card>

      <Card
        icon="keyboard"
        title={tx("annotate.shortcuts.title")}
        desc={tx("annotate.shortcuts.desc")}
      >
        <Rows>
          <Row label={tx("annotate.shortcuts.undo")}>
            <span className="hs-chip">
              <kbd className="hs-key">{MOD_KEY}</kbd>
              <kbd className="hs-key">Z</kbd>
            </span>
          </Row>
          <Row label={tx("annotate.shortcuts.redo")}>
            <span className="hs-chip">
              <kbd className="hs-key">{MOD_KEY}</kbd>
              <kbd className="hs-key">⇧</kbd>
              <kbd className="hs-key">Z</kbd>
            </span>
          </Row>
          <Row label={tx("annotate.shortcuts.save")}>
            <span className="hs-chip">
              <kbd className="hs-key">{MOD_KEY}</kbd>
              <kbd className="hs-key">S</kbd>
            </span>
          </Row>
        </Rows>
      </Card>
    </div>
  );
}

// —— 分区 4/8：历史记录 History ——

const HISTORY_LIMIT = 500;

export function HistorySection({ notifyError }: SectionProps) {
  const tx = useT();

  const openInPanel = () => {
    // 失败必须 catch，绝不白屏。
    invoke<void>("open_main_panel")
      .catch((e) => notifyError(tx("error.generic", { message: String(e) })));
  };

  return (
    <div className="hs-cards">
      <Card icon="clock" title={tx("history.limit.title")} desc={tx("history.limit.desc")}>
        <Rows>
          <Row label={tx("history.limit.title")}>
            <span className="hs-status-text">
              {tx("history.limit.value", { count: HISTORY_LIMIT })}
            </span>
          </Row>
        </Rows>
      </Card>

      <Card icon="folder" title={tx("history.storage.title")} desc={tx("history.storage.desc")} />

      <Card icon="search" title={tx("history.search.title")} desc={tx("history.search.desc")} />

      <Card icon="sliders" title={tx("history.openPanel")} desc={tx("history.openPanelDesc")}>
        <div className="hs-btn-row">
          <button type="button" className="hs-btn" onClick={openInPanel}>
            {tx("action.open")}
          </button>
        </div>
      </Card>
    </div>
  );
}

// —— 分区 5/8：键盘快捷键 Shortcuts ——

export function ShortcutsSection({ form, capturing, setCapturing }: SectionProps) {
  const tx = useT();

  const slotRow = (
    slot: HotkeySlot,
    label: string,
    info: string,
    combo: string,
  ) => (
    <Row label={label} info={info} gearTip={tx("action.details")}>
      <HotkeyChip
        combo={combo}
        capturing={capturing === slot}
        onStart={() => setCapturing(slot)}
        onCancel={() => setCapturing(null)}
        setLabel={tx("capture.hotkey.setShortcut")}
        pressLabel={tx("capture.hotkey.pressCombo")}
      />
    </Row>
  );

  return (
    <div className="hs-cards">
      <Card
        icon="keyboard"
        title={tx("shortcuts.global.title")}
        desc={tx("shortcuts.global.desc")}
      >
        <Rows>
          {slotRow(
            "hotkeyCapture",
            tx("shortcuts.slot.capture"),
            tx("shortcuts.captureInfo"),
            form.hotkeyCapture,
          )}
          {slotRow(
            "hotkeyPin",
            tx("shortcuts.slot.pin"),
            tx("shortcuts.pinInfo"),
            form.hotkeyPin,
          )}
          {slotRow(
            "hotkeyRecord",
            tx("shortcuts.slot.record"),
            tx("shortcuts.recordInfo"),
            form.hotkeyRecord,
          )}
        </Rows>
      </Card>
    </div>
  );
}

// —— 分区 6/8：权限 Permissions ——

interface PermissionStatus {
  screenRecording: boolean;
  accessibility: boolean;
}

export function PermissionsSection({ notifyError }: SectionProps) {
  const tx = useT();
  const [perm, setPerm] = useState<PermissionStatus | null>(null);
  const [permError, setPermError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    invoke<PermissionStatus>("permission_status")
      .then((s) => {
        setPerm(s);
        setPermError(null);
      })
      .catch((e) => setPermError(tx("error.generic", { message: String(e) })));
  }, [tx]);

  useEffect(() => {
    refresh();
    // 从系统设置返回后复检（窗口获得焦点即刷新）。
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refresh]);

  const openPermission = (which: "screenRecording" | "accessibility") => {
    invoke<void>("open_permission_settings", { which })
      .then(() => refresh())
      .catch((e) => notifyError(tx("error.permissionOpen", { message: String(e) })));
  };

  const permRow = (
    which: "screenRecording" | "accessibility",
    label: string,
    grantedText: string,
    deniedText: string,
    actionText: string,
    desc: string,
  ) => {
    const known = perm !== null;
    const granted = perm !== null && perm[which] === true;
    return (
      <Row label={label} info={desc}>
        <span className="hs-status">
          <span
            className={`hs-status-dot ${
              !known
                ? "hs-status-dot-unknown"
                : granted
                  ? "hs-status-dot-ok"
                  : "hs-status-dot-warn"
            }`}
            aria-hidden="true"
          />
          <span
            className={
              granted ? "hs-status-text hs-status-text-ok" : "hs-status-text hs-status-text-warn"
            }
          >
            {!known ? tx("permissions.unknown") : granted ? grantedText : deniedText}
          </span>
        </span>
        <button
          type="button"
          className="hs-btn"
          onClick={() => openPermission(which)}
          title={actionText}
        >
          {actionText}
        </button>
      </Row>
    );
  };

  return (
    <div className="hs-cards">
      <Card
        icon="shield"
        title={tx("permissions.status.title")}
        desc={tx("permissions.status.desc")}
      >
        <Rows>
          {permRow(
            "screenRecording",
            tx("permissions.screen.label"),
            tx("permissions.screen.granted"),
            tx("permissions.screen.denied"),
            tx("permissions.screen.action"),
            tx("permissions.screen.desc"),
          )}
          {permRow(
            "accessibility",
            tx("permissions.accessibility.label"),
            tx("permissions.accessibility.granted"),
            tx("permissions.accessibility.denied"),
            tx("permissions.accessibility.action"),
            tx("permissions.accessibility.desc"),
          )}
        </Rows>
        <div className="hs-btn-row" style={{ marginTop: 10 }}>
          <button type="button" className="hs-btn" onClick={refresh}>
            {tx("permissions.recheck")}
          </button>
        </div>
        {permError ? (
          <p className="hs-note" role="alert">
            {permError}
          </p>
        ) : null}
      </Card>

      <Card icon="infoCircle" title={tx("permissions.usage.title")} desc={tx("permissions.usage.desc")}>
        <List>
          <ListItem
            title={tx("permissions.screen.label")}
            desc={tx("permissions.screen.desc")}
          />
          <ListItem
            title={tx("permissions.accessibility.label")}
            desc={tx("permissions.accessibility.desc")}
          />
        </List>
      </Card>
    </div>
  );
}

// —— 分区 7/8：菜单栏 Menu Bar ——

export function MenuBarSection(_props: SectionProps) {
  const tx = useT();
  return (
    <div className="hs-cards">
      <Card icon="menuBar" title={tx("menubar.items.title")} desc={tx("menubar.items.desc")}>
        <List>
          <ListItem title={tx("menubar.item.capture")} desc={tx("menubar.item.captureDesc")} />
          <ListItem title={tx("menubar.item.delay")} desc={tx("menubar.item.delayDesc")} />
          <ListItem title={tx("menubar.item.scrollshot")} desc={tx("menubar.item.scrollshotDesc")} />
          <ListItem title={tx("menubar.item.scrollshotStop")} desc={tx("menubar.item.scrollshotStopDesc")} />
          <ListItem title={tx("menubar.item.pinCloseAll")} desc={tx("menubar.item.pinCloseAllDesc")} />
          <ListItem title={tx("menubar.item.panel")} desc={tx("menubar.item.panelDesc")} />
          <ListItem title={tx("menubar.item.settings")} desc={tx("menubar.item.settingsDesc")} />
          <ListItem title={tx("menubar.item.quit")} desc={tx("menubar.item.quitDesc")} />
        </List>
      </Card>

      <Card
        icon="infoCircle"
        title={tx("menubar.behavior.title")}
        desc={tx("menubar.behavior.desc")}
      />
    </div>
  );
}

// —— 分区 8/8：关于 About ——

const URL_REPO = "https://github.com/huluohu/HawkSnap";
const URL_SITE = "https://huluohu.github.io/HawkSnap/";
const URL_THIRD_PARTY =
  "https://github.com/huluohu/HawkSnap/blob/main/LICENSE-THIRD-PARTY.md";

export function AboutSection({ notifyError }: SectionProps) {
  const tx = useT();
  const [version, setVersion] = useState("");

  useEffect(() => {
    let disposed = false;
    // 应用版本来自 Tauri 核心（plugin:app|version，core:default 已授权）；
    // 取不到就留空——不编造版本号。
    getVersion()
      .then((v) => {
        if (!disposed) setVersion(v);
      })
      .catch(() => {});
    return () => {
      disposed = true;
    };
  }, []);

  // 外链：优先走 opener 插件（capabilities/default.json 已给 settings 窗口
  // opener:default 权限）；插件不可用时回落到后端 open_external 命令。
  const openExternal = (url: string) => {
    openUrl(url)
      .catch(() => invoke<void>("open_external", { url }))
      .catch((e) => notifyError(tx("error.link", { message: String(e) })));
  };

  const linkRow = (label: string, desc: string, url: string) => (
    <div className="hs-row">
      <div className="hs-row-head">
        <span className="hs-row-label">{label}</span>
      </div>
      <div className="hs-row-control">
        <span className="hs-status-text">{desc}</span>
        <button
          type="button"
          className="hs-icon-btn"
          title={url}
          aria-label={`${label} — ${tx("action.open")}`}
          onClick={() => openExternal(url)}
        >
          <Icon name="external" />
        </button>
      </div>
    </div>
  );

  return (
    <div className="hs-cards">
      <Card icon="appMark" title={tx("about.app.title")} desc={tx("about.app.desc")}>
        <Rows>
          <Row label={tx("about.app.title")}>
            <span className="hs-status-text">
              {version ? tx("about.app.version", { version }) : "—"}
            </span>
          </Row>
          <Row label={tx("about.license.label")}>
            <span className="hs-status-text">{tx("about.app.license")}</span>
          </Row>
        </Rows>
      </Card>

      <Card icon="external" title={tx("about.links.title")} desc={tx("about.links.desc")}>
        <Rows>
          {linkRow(tx("about.link.repo"), tx("about.link.repoDesc"), URL_REPO)}
          {linkRow(tx("about.link.site"), tx("about.link.siteDesc"), URL_SITE)}
          {linkRow(tx("about.link.thirdParty"), tx("about.link.thirdPartyDesc"), URL_THIRD_PARTY)}
        </Rows>
      </Card>

      <Card icon="shield" title={tx("about.credits.title")} desc={tx("about.credits.desc")}>
        <List>
          <ListItem title={tx("about.credit.tauri")} desc={tx("about.credit.tauriDesc")} />
          <ListItem title={tx("about.credit.react")} desc={tx("about.credit.reactDesc")} />
          <ListItem title={tx("about.credit.rust")} desc={tx("about.credit.rustDesc")} />
          <ListItem title={tx("about.credit.gif")} desc={tx("about.credit.gifDesc")} />
        </List>
      </Card>
    </div>
  );
}
