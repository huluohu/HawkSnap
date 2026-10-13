// scripts/bench_synth.swift — 合成 F1→拖拽→双击 注入基准（免人工按键，控制器 2026-10-14 验证序列固化）
// 用法: swift bench_synth.swift [轮数N]   （默认 20）
// 每轮: F1(keycode 122, 按下-抬起间隔 50ms) → 1.5s → 短拖拽（起点偏移随轮次变化防同名）
//       → 0.6s → 拖拽中心双击（clickState=2——合成事件不会被系统合并成双击，
//       必须显式置 2 才会被识别，这是合成双击的关键） → 1.2s
// 循环前先跑 1 轮预热（不计入统计，砍掉每轮首测的冷加载扰动）。
// 进度走 stderr，汇总走 stdout。

import CoreGraphics
import Foundation

let parsed = CommandLine.arguments.count > 1 ? (Int(CommandLine.arguments[1]) ?? 20) : 20
let n = max(1, parsed)

func postKey(_ keyCode: CGKeyCode, keyDown: Bool) {
    let src = CGEventSource(stateID: .hidSystemState)
    let ev = CGEvent(keyboardEventSource: src, virtualKey: keyCode, keyDown: keyDown)
    ev?.post(tap: .cghidEventTap)
}

func postMouse(_ type: CGEventType, at point: CGPoint, clicks: Int64 = 1) {
    let ev = CGEvent(mouseEventSource: nil, mouseType: type,
                     mouseCursorPosition: point, mouseButton: .left)
    ev?.setIntegerValueField(.mouseEventClickState, value: clicks)
    ev?.post(tap: .cghidEventTap)
}

func oneRound(_ i: Int) {
    let sx = CGFloat(600 + (i % 5) * 30)
    let sy = CGFloat(400 + (i % 4) * 25)
    let start = CGPoint(x: sx, y: sy)
    let end = CGPoint(x: sx + 120, y: sy + 90)
    let center = CGPoint(x: sx + 60, y: sy + 45)

    postKey(122, keyDown: true) // F1
    Thread.sleep(forTimeInterval: 0.05)
    postKey(122, keyDown: false)
    Thread.sleep(forTimeInterval: 1.5)

    // 拖拽：moved → down → dragged → up（间隙 ~100-150ms）
    postMouse(.mouseMoved, at: start)
    Thread.sleep(forTimeInterval: 0.12)
    postMouse(.leftMouseDown, at: start)
    Thread.sleep(forTimeInterval: 0.15)
    postMouse(.leftMouseDragged, at: end)
    Thread.sleep(forTimeInterval: 0.12)
    postMouse(.leftMouseUp, at: end)
    Thread.sleep(forTimeInterval: 0.6)

    // 双击确认：clickState=2（见文件头注释）
    postMouse(.leftMouseDown, at: center, clicks: 2)
    Thread.sleep(forTimeInterval: 0.06)
    postMouse(.leftMouseUp, at: center, clicks: 2)
    Thread.sleep(forTimeInterval: 1.2)
}

fputs("预热 1 轮（不计入统计）...\n", stderr)
oneRound(0)
for i in 1...n {
    fputs("round \(i)/\(n) ...\n", stderr)
    oneRound(i)
}
print("完成：预热 1 轮 + 正式 \(n) 轮，共 \(n + 1) 次 F1→拖拽→双击")
