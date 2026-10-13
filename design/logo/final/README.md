# HawkSnap 品牌套件（Logo 定稿：C 快门鹰眼）

2026-10-13 定稿。概念探索稿见 `../`（A 鹰眼取景框 / B H鹰嘴字母标 / C 快门鹰眼[定稿] / D 鹰击截图框）。

## 设计语义

六条弦线交织成相机光圈（快门），中心是琥珀色鹰眼（瞳孔 + 高光）——
「快门」表达截图/录屏的本质动作，「鹰眼」承载 Slogan 的「准」。

## 色板

| 用途 | 色值 |
|---|---|
| 鹰隼蓝（背景渐变起点） | `#22405F` |
| 鹰隼蓝（背景渐变终点） | `#0C1624` |
| 深色外观背景渐变 | `#1B3450` → `#080F1A` |
| 鹰眼琥珀（高光 → 主色 → 暗部） | `#FFE08A` → `#FFB020` → `#EF7A00` |
| 瞳孔/墨色 | `#0D1522` |
| 弦线/外环 | `#FFFFFF`（深色外观用 `#F5F7FA`） |

## 文件清单

| 文件 | 用途 |
|---|---|
| `icon.svg` | 应用图标母版（512×512，浅色外观） |
| `icon-dark.svg` | 应用图标（深色外观变体，更深背景） |
| `tray-mono.svg` | 托盘/菜单栏单色图标（`currentColor`，自适应明暗） |
| `lockup.svg` | 横排组合（图标 + HawkSnap + Slogan） |

## 导出命令（构建期使用）

```bash
# PNG 各尺寸
for s in 16 32 64 128 256 512 1024; do
  rsvg-convert -w $s -h $s icon.svg -o icon-$s.png
done

# macOS .icns（需要 iconutil + png2icns 或直接用 1024 png 在 Xcode 生成）
# Windows .ico（需要 ImageMagick）
magick icon-256.png -define icon:auto-resize=256,128,64,48,32,16 icon.ico
```

## 使用规范

- 最小可用尺寸：彩色 16px（此时仅保留外环+中心圆轮廓），托盘单色 16px。
- 安全边距：图标内容与容器边缘 ≥ 图标边长的 8%。
- 琥珀色仅用于鹰眼与强调场景，不得作为大面积底色。
- Lockup 中文字使用系统字体栈（SF Pro Display / Segoe UI / PingFang SC），不引入外部字体依赖。
