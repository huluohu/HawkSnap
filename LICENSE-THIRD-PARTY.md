# 第三方组件许可声明（HawkSnap）

HawkSnap 以 **MIT** 授权（完整文本见仓库根的 [`LICENSE`](LICENSE)）。本文件声明分发时
需要特别留意的第三方组件；完整依赖清单与精确版本见仓库根的 `Cargo.lock`（各组件的
完整许可文本随其上游仓库发布）。

## 许可口径

- **无 copyleft 组件**：2026-10-31 起 HawkSnap 整体按 MIT 分发，依赖树已将唯一的
  copyleft 触发源 [gifski](https://github.com/AnimatedPNG/gifski)（AGPL-3.0）替换为
  [`gif`](https://github.com/image-rs/image-gif)（MIT / Apache-2.0，内建 NeuQuant
  量化）。`scripts/release-readiness.sh` 的检查项 (h) 把守该口径。
- 其余依赖均为 MIT / Apache-2.0 / BSD / ISC / Unicode-3.0 等宽松许可，不要求源码
  公开。逐项清单以 `Cargo.lock` 与各 crate 的上游仓库为准；本项目未对任何组件的
  许可文本做修改。

## 运行时随附组件

- **WebView**：macOS 由系统 WKWebView 提供；Windows 由系统 WebView2 Runtime 提供
  （缺省时安装器走 Microsoft 官方 bootstrapper 下载）。其许可各自归属 Apple /
  Microsoft，非本项目分发物。
