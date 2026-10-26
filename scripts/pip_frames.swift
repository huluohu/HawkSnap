// scripts/pip_frames.swift — N3 F9 证据：AVAssetImageGenerator 抽帧到 PNG（控制器抽帧目检）。
// 用法: swift pip_frames.swift <mp4> <outdir> <t1> [t2] [t3] ...
// 输出: <outdir>/frame-<t>.png；至少一个时间戳失败 → exit 1。

import AVFoundation
import AppKit

guard CommandLine.arguments.count >= 4 else {
    fputs("usage: pip_frames.swift <mp4> <outdir> <t1> [t2] ...\n", stderr)
    exit(2)
}
let url = URL(fileURLWithPath: CommandLine.arguments[1])
let outdir = CommandLine.arguments[2]
let times = CommandLine.arguments[3...].compactMap { Double($0) }
guard !times.isEmpty else {
    fputs("no valid timestamps\n", stderr)
    exit(2)
}
try? FileManager.default.createDirectory(atPath: outdir, withIntermediateDirectories: true)

let asset = AVURLAsset(url: url)
let gen = AVAssetImageGenerator(asset: asset)
gen.appliesPreferredTrackTransform = true
gen.requestedTimeToleranceBefore = .zero
gen.requestedTimeToleranceAfter = .zero

var ok = 0
for t in times {
    do {
        let cg = try gen.copyCGImage(at: CMTime(seconds: t, preferredTimescale: 600), actualTime: nil)
        let rep = NSBitmapImageRep(cgImage: cg)
        guard let data = rep.representation(using: .png, properties: [:]) else {
            fputs("png encode failed at t=\(t)\n", stderr)
            continue
        }
        let path = "\(outdir)/frame-\(String(format: "%.2f", t)).png"
        try data.write(to: URL(fileURLWithPath: path))
        print("[pip-frames] \(path) \(cg.width)x\(cg.height)")
        ok += 1
    } catch {
        fputs("[pip-frames] extract failed at t=\(t): \(error)\n", stderr)
    }
}
exit(ok == times.count ? 0 : 1)
