#!/usr/bin/env bash
# scripts/sample-ocr-mem.sh — G2: OCR 常驻内存实测（M4 遗留 Important #2 的 M7 收口）
# 装载三模型 + 双推理，ps 采样 RSS 四时点（init 前/后、首推理后、第二次推理后），
# 断言 init+首推理合计增长 <300MB；>150MB 的 idle 卸载裁定见测试输出与 task-7-report。
# 首次运行需网络下载官方模型（~60MB+36MB，缓存于 $TMPDIR/hs-ocr-mem-cache，复跑免下）。
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test -p hawksnap-ocr -- --ignored ocr_resident_memory --nocapture
