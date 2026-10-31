<div align="center">

<img src="docs/logo.png" alt="HawkSnap" width="140" />

# HawkSnap · 獵影

### 如鷹之準，一按即截

*Press, frame, done.* ／ *按下快速鍵、框住、完事。*

**免費 · 純本地 · 三平台（macOS / Windows / Linux）· MIT**

[![Release](https://img.shields.io/github/v/release/huluohu/HawkSnap)](https://github.com/huluohu/HawkSnap/releases)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-0a84ff)](#下載)
[![CI](https://github.com/huluohu/HawkSnap/actions/workflows/pipeline.yml/badge.svg)](https://github.com/huluohu/HawkSnap/actions/workflows/pipeline.yml)
[![License](https://img.shields.io/badge/license-MIT-34c759)](LICENSE)

</div>

---

[简体中文](README.md) | **繁體中文** | [文言](README.classical.md) | [English](README.en.md) | [日本語](README.ja.md)

<p>
  <img src="docs/screenshots/main.png" width="49%" alt="HawkSnap 主面板（淺色）" />
  <img src="docs/screenshots/main-dark.png" width="49%" alt="HawkSnap 主面板（深色）" />
</p>

HawkSnap（中文名**獵影**）是一款免費、純本地的輕量級截屏 + 錄屏工具——無帳號、無雲端、無遙測。

## 功能

- **截圖**：全域快速鍵呼出凍結畫面覆蓋層，拖曳框選（Enter / 雙擊確認，Esc 取消），確認後進剪貼簿並儲存到 `~/Pictures/HawkSnap`；
- **智慧吸附**：游標所至，元素或視窗自動高亮，單擊即選（元素 → 視窗 → 全螢幕三級回退）；方向鍵 1px 微調（Shift 10px），貼近視窗邊緣 8px 自動吸附；框選後動作浮條一鍵複製 / 儲存 / 標註；
- **圖釘**：F2 把圖片釘在桌面——按住拖曳、滾輪縮放（10%–3200%）、Ctrl+滾輪調透明度、Esc 關閉，多釘自動錯開；
- **標註編輯器**：矩形 / 橢圓 / 箭頭 / 畫筆 / 螢光筆 / 馬賽克 / 序號 / 文字 / 裁剪共 9 種工具，復原重做無步數限制，⌘S 儲存；
- **OCR 文字辨識**：編輯器內整圖辨識，中英雙語（PP-OCRv4，純本地推理），結果逐行點擊複製 / 匯出 txt，文字進歷史全文檢索；
- **錄屏**：F6 進入選區模式（拖曳 = 區域，Enter / 雙擊 = 全螢幕），懸浮條計時 + 暫停 / 恢復 / 停止；系統聲音隨錄（AAC）；H.264 MP4 存至 `~/Movies/HawkSnap`；可選 **GIF 匯出**（無限循環，無音訊）與**視訊畫中畫**（僅 MP4）；
- **長截圖**：托盤入口，框選區域後捲動頁面，模板比對拼接成長圖（畫布上限 256MB / 40000px）；
- **按鍵顯示**：錄製中即時顯示按下的組合鍵（如 ⌘⇧A），唯讀旁路不攔截按鍵；
- **歷史記錄**：每次截圖自動入庫（本地 SQLite），縮圖網格 + 中英文全文搜尋 + 重新編輯 / 釘選 / 開啟資料夾；
- **延時截圖**：托盤選單可選 1 / 2 / 3 / 5 秒倒數；
- **瀏覽器外掛**：Chrome / Edge 外掛一鍵截取整頁或選區，直接傳送到桌面端編輯（Native Messaging 橋）；
- **自動更新**：Ed25519 簽章的更新通道，壞簽章的更新包拒絕安裝；
- **雙語介面**：中文 / English，跟隨系統語言或手動切換。

## 下載

從 [Releases](https://github.com/huluohu/HawkSnap/releases) 下載對應平台的安裝包：

| 平台 | 安裝包 | 自動更新 |
| --- | --- | --- |
| macOS 12.3+（Apple Silicon / Intel） | dmg | ✅ |
| Windows 10 1903+ / 11（x64） | NSIS 安裝器 / MSI | ✅ |
| Linux（x64） | AppImage / deb / rpm | ✅ |

應用內「設定 → 自動更新」可檢查並安裝新版本。

## 平台能力差異

| 能力 | macOS | Windows / Linux |
| --- | --- | --- |
| 截圖（區域 / 全螢幕） | ✅ | ✅ 跨平台後端 |
| 視窗自動辨識（懸停選窗） | ✅ | ❌（降級為手動畫框） |
| 錄屏 / 畫中畫 / 長截圖 / 圖釘 / 按鍵顯示 | ✅ | ❌（入口明確報錯，不靜默缺失） |
| OCR / 歷史 / 編輯器 / 瀏覽器外掛 | ✅ | ✅ |

## 使用

| 快速鍵 | 功能 | 說明 |
| --- | --- | --- |
| **fn+F1**（或 F1*） | 截圖 | 註冊的是標準 F1 鍵；macOS 預設 F1 是亮度鍵，按 fn+F1 觸發截圖，系統亮度調節不受影響 |
| **F2**（可自訂） | 圖釘 | 釘選最近截圖或剪貼簿圖片 |
| **F6**（可自訂） | 錄屏 | 進入選區模式；錄製中再按停止 |

快速鍵均可在主面板設定中自訂（衝突時儲存失敗並提示）。主面板常駐托盤，從托盤選單開啟。

## 權限說明（macOS）

- **螢幕錄製**：首次截圖時系統會請求；若未彈窗或截圖失敗，前往 系統設定 → 隱私與安全性 → 螢幕錄製 開啟 HawkSnap，然後重新啟動應用程式。
- **輔助功能**：智慧吸附懸停高亮與按鍵顯示需要（系統設定 → 隱私與安全性 → 輔助功能）。未授權時拖曳框選照常可用，只是沒有懸停高亮。
- **攝影機**：錄製畫中畫需要；未授權時錄製本身不受影響，只是成片不含相機畫面。

## 已知限制

- Windows 安裝器未程式碼簽章（首次執行 SmartScreen 會告警）；macOS dmg 未做公證。
- GIF 匯出無音訊、不含畫中畫；GIF 抽幀佇列有界（約 22 幀），長時間錄製產出的 GIF 只涵蓋收尾窗口（MP4 始終是全程）。
- 長截圖只支援單軸縱向捲動；週期性內容（聊天氣泡等）可能被拒絕拼接（寧可不拼，絕不產出重影圖）。
- OCR 模型首次使用需連網下載約 60MB（此後離線可用）；OCR 不支援 Intel macOS。
- 錄製中結束應用程式會產生損壞的 MP4（結束時不收尾錄製器）。
- Rust 端錯誤文案暫為中文（介面已雙語）。

## 從原始碼建置

```bash
# macOS 12.3+ / Node.js 20+ / Rust stable
cd app
npm install
npm run tauri dev      # 開發
npm run tauri build    # 建置（產物在 workspace 根 target/release/bundle/）
```

Linux 建置需要 `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev` 等（見 `.github/workflows/pipeline.yml` 的 Linux 步驟）。

## 授權

[MIT](LICENSE)。第三方元件聲明見 [LICENSE-THIRD-PARTY.md](LICENSE-THIRD-PARTY.md)。
