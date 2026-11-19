# 第三方组件许可声明（HawkSnap）

HawkSnap 以 **MIT** 授权（完整文本见仓库根的 [`LICENSE`](LICENSE)）。本文件声明分发时
需要特别留意的第三方组件；完整依赖清单与精确版本见仓库根的 `Cargo.lock`（各组件的
完整许可文本随其上游仓库发布）。

## 许可口径

- **无强 copyleft 依赖（GPL / AGPL）**：2026-10-31 起 HawkSnap 整体按 MIT 分发，
  Cargo 依赖树已将唯一的 copyleft 触发源 [gifski](https://github.com/AnimatedPNG/gifski)
  （AGPL-3.0）替换为 [`gif`](https://github.com/image-rs/image-gif)（MIT / Apache-2.0，
  内建 NeuQuant 量化）。`scripts/release-readiness.sh` 的检查项 (h) 把守该口径。
- 其余 Cargo/npm 依赖均为 MIT / Apache-2.0 / BSD / ISC / Unicode-3.0 等宽松许可，
  不要求源码公开。逐项清单以 `Cargo.lock` 与各 crate 的上游仓库为准；本项目未对
  任何组件的许可文本做修改。
- **Linux 版动态链接系统 LGPL 库**（2026-11-20 措辞修正——此前「无 copyleft 组件」
  的表述失实）：`gtk3` / `webkit2gtk-4.1`（LGPL-2.1+）/ `libayatana-appindicator`
  （LGPL-2.1 / LGPL-3）/ `librsvg`（LGPL-2.1+）等由 Tauri 在 Linux 平台的运行时
  依赖引入。它们是**系统库而非本项目分发物**，按 LGPL 的动态链接条款以未修改的
  系统库形态使用，不影响应用本体按 MIT 分发；各库许可归属其上游。

## 运行时随附/按需下载组件

- **WebView**：macOS 由系统 WKWebView 提供；Windows 由系统 WebView2 Runtime 提供
  （缺省时安装器走 Microsoft 官方 bootstrapper 下载）。其许可各自归属 Apple /
  Microsoft，非本项目分发物。
- **OCR 运行时（首次使用时按需下载，SHA256 双重校验）**：[ONNX Runtime]
  （https://github.com/microsoft/onnxruntime，MIT）；PP-OCR 系列模型与字典
  （Apache-2.0，来自 RapidOCR 发布件）。下载源为 modelscope.cn 与 github.com
  官方 release，无其他遥测外联。
