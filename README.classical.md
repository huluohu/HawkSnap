<div align="center">

<img src="docs/logo.png" alt="HawkSnap" width="140" />

# HawkSnap · 鷹截

### 準若鷹瞵，一擊而獲

*Press, frame, done.* ／ *鷹瞵鶚視，少縱即逝；一按而擒之。*

**無償 · 盡在爾機 · 三平台（macOS / Windows / Linux）· MIT 之許**

[![Release](https://img.shields.io/github/v/release/huluohu/HawkSnap)](https://github.com/huluohu/HawkSnap/releases)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-0a84ff)](#獲置)
[![CI](https://github.com/huluohu/HawkSnap/actions/workflows/pipeline.yml/badge.svg)](https://github.com/huluohu/HawkSnap/actions/workflows/pipeline.yml)
[![License](https://img.shields.io/badge/license-MIT-34c759)](LICENSE)

</div>

---

[简体中文](README.md) | [繁體中文](README.zh-Hant.md) | **文言** | [English](README.en.md) | [日本語](README.ja.md)

<p>
  <img src="docs/screenshots/main.png" width="49%" alt="主面板" />
  <img src="docs/screenshots/main-dark.png" width="49%" alt="主面板，夜色" />
</p>

鷹截者，截屏録屏之輕器也。不設帳籍，不寄雲端，不窺行止，諸事皆辦於君之機，纖毫不外洩。

## 其能

- **截屏**：按鍵而屏凍，框其所欲（回車、雙擊以定之，Esc 而罷），既定則畫入剪貼，兼藏諸 `~/Pictures/HawkSnap`；
- **智吸**：目之所臨，窗素自彰其界，一擊而取，毋庸框畫（微者、窗、全屏，三級遞降）；以方向鍵移之，毫釐可辨（shift 則十之），近窗八像素則自附如磁；既框則浮條並出，複製、儲藏、註記，一擊可達；
- **釘圖**：按 F2 而釘畫於案——曳之則行，輪轉則縮（一成至三十二倍），ctrl 合輪則調其淡（二成至全），Esc 而去；眾釘自爲錯落；
- **註記**：矩、橢、矢、筆、螢、瓃、序、文、翦，凡九器；悔之復之，無所限；⌘S 而成；
- **識文**：器中舉圖而識，漢英兼通（推演盡在本地）；所得逐行可録，可導爲 txt，且入史而備檢；
- **録屏**：按 F6 而入選區之式（曳則録其域，回車、雙擊則全屏），浮條計時，可暫可續可止；系聲並録；成 H.264 之 MP4，藏諸 `~/Movies/HawkSnap`；可出 GIF（循環而無聲），可添鏡中之影於畫隅（惟 MP4）；
- **長圖**：自托盤入，框其域，縱卷而覽，撮而接之，雖萬像素可成一圖（上界 256MB、四萬像素）；
- **鍵示**：録時按鍵自見於隅（如 ⌘⇧A），旁觀而毫不奪鍵；
- **史册**：每截自入於册（本地 SQLite），縮圖成陣，漢英全文可檢，可重編、可釘、可啓其目；
- **延時**：托盤擇一、二、三、五秒，倒數未畢可罷；
- **瀏覽之佐**：Chrome / Edge 有插件，一鍵取整頁或一隅，徑致桌面端（設 Native Messaging 之橋）；
- **自新**：更新之道以 Ed25519 爲憑，僞簽者拒而弗納；
- **重譯**：華文 / English 隨宜而切。

## 獲置

詣 [Releases](https://github.com/huluohu/HawkSnap/releases)，各取其器：

| 平台 | 其器 | 自新 |
| --- | --- | --- |
| macOS 12.3+（Apple Silicon / Intel） | dmg | ✅ |
| Windows 10 1903+ / 11（x64） | NSIS / MSI | ✅ |
| Linux（x64） | AppImage / deb / rpm | ✅ |

器中「設定 → 自新」可自檢而自新。

## 諸平台之能

| 所能 | macOS | Windows / Linux |
| --- | --- | --- |
| 截屏（域 / 全屏） | ✅ | ✅ 跨平台之徑 |
| 窗之自辨（懸停而選） | ✅ | ❌（降而手畫） |
| 録屏 / 鏡影 / 長圖 / 釘圖 / 鍵示 | ✅ | ❌（明告不可，不作僞） |
| 識文 / 史册 / 註記 / 瀏覽之佐 | ✅ | ✅ |

## 用之法

| 鍵 | 其事 | 釋 |
| --- | --- | --- |
| **fn+F1**（或 F1*） | 截屏 | 所錄者標準 F1 也；macOS 之常，F1 爲調光之鍵，故按 fn+F1 以截，調光如故，兩不相奪 |
| **F2**（可自擇） | 釘圖 | 釘近截之圖，或剪貼之畫 |
| **F6**（可自擇） | 録屏 | 入選區之式；録中再按則止 |

諸鍵皆可於主面板之設定自擇（與他器相衝則弗許，必告焉）。主面板棲於托盤，啓之由托盤之單。

## 權限（macOS）

- **屏幕録製**：初截則系統索之；若索而不出，或截而不成，往 系統設置 → 隱私與安全性 → 屏幕録製，許 HawkSnap（開發之時，則許其所承之終端），既許，重啓其器。
- **輔助功能**：懸停之彰、按鍵之示所需（系統設置 → 隱私與安全性 → 輔助功能）。未許則框選如常，惟懸停不彰。
- **鏡（攝像頭）**：録而添影者所需；未許則録自如，惟成片無鏡影。

## 所限

- Windows 之器未經碼簽（初啓則 SmartScreen 警之）；macOS 之 dmg 未經公證。
- GIF 無聲，且無鏡影；抽幀之隊有界（約廿二幀），長録所出惟收其末（MP4 恒全）。
- 長圖惟縱卷一軸；同構相環之內容（如談話之泡）或拒不接——寧闕毋重影。
- 識文之模型初用須聯網而取，約六十兆，既取則可離；Intel 之 macOS 不供識文。
- 録中遽退其器，則 MP4 損（收尾不逮，後版本圖之）。
- Rust 側之文案暫用漢語（界面已重譯）。

## 自源碼營建

```bash
# macOS 12.3+ / Node.js 20+ / Rust stable
cd app
npm install
npm run tauri dev      # 試之
npm run tauri build    # 營之（成器在 workspace 根 target/release/bundle/）
```

Linux 營建須 `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev` 諸件（見 `.github/workflows/pipeline.yml` Linux 之步）。

## 其許

[MIT](LICENSE)。第三方之件，聲明在 [LICENSE-THIRD-PARTY.md](LICENSE-THIRD-PARTY.md)。
