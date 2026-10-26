// scripts/pip_region_motion.swift — N3 F9 证据：PiP 区域运动熵（程序化「贴片可见」）。
// 用法: swift pip_region_motion.swift <mp4> <x> <y> <size> <t1> <t2>
// 行为: 抽 t1/t2 两帧，在 rect（视频像素空间，左上原点）内逐像素求均值绝对差
//       （mean abs RGB diff），stdout 打印 `motion=<f64>`。相机/注入贴片是活动
//       内容 → 高；静态桌面角落（无 PiP 基线）→ ≈0。exit 0。

import AVFoundation
import CoreVideo

guard CommandLine.arguments.count == 7,
      let x = Int(CommandLine.arguments[2]),
      let y = Int(CommandLine.arguments[3]),
      let size = Int(CommandLine.arguments[4]),
      let t1 = Double(CommandLine.arguments[5]),
      let t2 = Double(CommandLine.arguments[6])
else {
    fputs("usage: pip_region_motion.swift <mp4> <x> <y> <size> <t1> <t2>\n", stderr)
    exit(2)
}

let url = URL(fileURLWithPath: CommandLine.arguments[1])
let asset = AVURLAsset(url: url)
let gen = AVAssetImageGenerator(asset: asset)
gen.appliesPreferredTrackTransform = true
gen.requestedTimeToleranceBefore = .zero
gen.requestedTimeToleranceAfter = .zero

func frame(_ t: Double) throws -> (data: [UInt8], bpr: Int, w: Int, h: Int) {
    let cg = try gen.copyCGImage(at: CMTime(seconds: t, preferredTimescale: 600), actualTime: nil)
    let w = cg.width, h = cg.height
    var buf = [UInt8](repeating: 0, count: w * h * 4)
    let cs = CGColorSpaceCreateDeviceRGB()
    let ctx = CGContext(data: &buf, width: w, height: h, bitsPerComponent: 8,
                        bytesPerRow: w * 4, space: cs,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    guard let ctx = ctx else { throw NSError(domain: "ctx", code: 1) }
    ctx.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
    return (buf, w * 4, w, h)
}

guard let a = try? frame(t1), let b = try? frame(t2) else {
    fputs("frame extraction failed\n", stderr)
    exit(1)
}
// CG 上下文数据行序为顶下行（premultipliedLast RGBA），rect 左上原点直接用。
var total = 0.0
var count = 0
for j in y..<min(y + size, a.h) {
    for i in x..<min(x + size, a.w) {
        let o = j * a.bpr + i * 4
        let dr = abs(Int(a.data[o]) - Int(b.data[o]))
        let dg = abs(Int(a.data[o + 1]) - Int(b.data[o + 1]))
        let db = abs(Int(a.data[o + 2]) - Int(b.data[o + 2]))
        total += Double(dr + dg + db) / 3.0
        count += 1
    }
}
guard count > 0 else {
    fputs("rect empty (w=\(a.w) h=\(a.h) rect=\(x),\(y),\(size))\n", stderr)
    exit(1)
}
print(String(format: "motion=%.4f", total / Double(count)))
