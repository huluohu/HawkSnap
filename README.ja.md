<div align="center">

<img src="docs/logo.png" alt="HawkSnap" width="140" />

# HawkSnap

### 鷹のごとく鋭く、ワンキーで撮る

*Press, frame, done.* ／ *按下快捷键、框住、完事。*

**無料 · 完全ローカル · 3 プラットフォーム（macOS / Windows / Linux）· MIT**

[![Release](https://img.shields.io/github/v/release/huluohu/HawkSnap)](https://github.com/huluohu/HawkSnap/releases)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-0a84ff)](#ダウンロード)
[![CI](https://github.com/huluohu/HawkSnap/actions/workflows/pipeline.yml/badge.svg)](https://github.com/huluohu/HawkSnap/actions/workflows/pipeline.yml)
[![License](https://img.shields.io/badge/license-MIT-34c759)](LICENSE)

</div>

---

[简体中文](README.md) | [繁體中文](README.zh-Hant.md) | [文言](README.classical.md) | [English](README.en.md) | **日本語**

<p>
  <img src="docs/screenshots/main.png" width="49%" alt="HawkSnap メインパネル（ライト）" />
  <img src="docs/screenshots/main-dark.png" width="49%" alt="HawkSnap メインパネル（ダーク）" />
</p>

HawkSnap は、無料・完全ローカル動作の軽量スクリーンショット＆画面録画ツールです。アカウント登録不要、クラウド非経由、テレメトリなし。

## 機能

- **スクリーンショット**：グローバルホットキーで画面をフリーズし、ドラッグで範囲選択（Enter / ダブルクリックで確定、Esc で取消）。確定するとクリップボードへコピーし、`~/Pictures/HawkSnap` に保存；
- **スマートスナップ**：カーソル下の要素やウィンドウがホバーでハイライトされ、クリックで選択（要素 → ウィンドウ → 全画面の 3 段階フォールバック）。矢印キーで 1px 移動（Shift で 10px）、ウィンドウ端 8px 以内で自動スナップ。範囲確定後はアクションバーからコピー / 保存 / 注釈へワンタップ；
- **ピン留め**：F2 で画像をデスクトップにピン留め——ドラッグで移動、スクロールで拡大縮小（10%–3200%）、Ctrl+スクロールで不透明度（20%–100%）、Esc で閉じる。複数枚は自動でカスケード配置；
- **注釈エディタ**：矩形 / 楕円 / 矢印 / ペン / 蛍光ペン / モザイク / 番号 / テキスト / 切り抜きの 9 種類のツール、無制限の取り消し・やり直し、⌘S で保存；
- **OCR 文字認識**：エディタ内で画像全体を認識、中国語・英語対応（PP-OCRv4、完全ローカル推論）。結果は行単位でコピー / txt でエクスポート。認識テキストは履歴の全文検索に反映；
- **画面録画**：F6 で範囲選択モードに入り（ドラッグ = 範囲、Enter / ダブルクリック = 全画面）、フローティングバーで計測・一時停止 / 再開 / 停止。システム音声を録音（AAC）。H.264 MP4 を `~/Movies/HawkSnap` に保存。**GIF 書き出し**（ループ、無音）と**カメラ PiP**（MP4 のみ）に対応；
- **スクロールキャプチャ**：トレイから範囲を選び、ページをスクロールするとテンプレートマッチングで長尺画像を連結（キャンバス上限 256MB / 40000px）；
- **キーキャスト**：録画中に押したキー組み合わせ（例：⌘⇧A）を画面隅に表示。読み取り専用イベントタップでキーを奪わない；
- **履歴**：全ショットをローカル SQLite に記録——サムネイルグリッド、中日英全文検索、再編集 / ピン留め / フォルダを表示；
- **遅延キャプチャ**：トレイメニューで 1 / 2 / 3 / 5 秒を選択、カウントダウン中のキャンセル可；
- **ブラウザ拡張**：Chrome / Edge 拡張からページ全体や選択範囲をワンクリックでキャプチャし、デスクトップアプリへ直接送信（Native Messaging ブリッジ）；
- **自動アップデート**：Ed25519 署名付きの更新チャネル。不正署名の更新はインストール前に拒否；
- **バイリンガル UI**：中文 / English。システム言語に追従、手動切替も可。

## ダウンロード

[Releases](https://github.com/huluohu/HawkSnap/releases) からお使いのプラットフォーム用インストーラを入手：

| プラットフォーム | インストーラ | 自動更新 |
| --- | --- | --- |
| macOS 12.3+（Apple Silicon / Intel） | dmg | ✅ |
| Windows 10 1903+ / 11（x64） | NSIS セットアップ / MSI | ✅ |
| Linux（x64） | AppImage / deb / rpm | ✅ |

アプリ内の「設定 → 自動アップデート」から新バージョンの確認・インストールができます。

## プラットフォーム別の対応

| 機能 | macOS | Windows / Linux |
| --- | --- | --- |
| スクリーンショット（範囲 / 全画面） | ✅ | ✅ クロスプラットフォームバックエンド |
| ウィンドウ自動認識（ホバー選択） | ✅ | ❌（手動ドラッグにフォールバック） |
| 録画 / PiP / スクロールキャプチャ / ピン留め / キーキャスト | ✅ | ❌（明示的にエラー、サイレント未対応なし） |
| OCR / 履歴 / エディタ / ブラウザ拡張 | ✅ | ✅ |

## 使い方

| ホットキー | 機能 | 説明 |
| --- | --- | --- |
| **fn+F1**（または F1*） | キャプチャ | 標準の F1 キーを登録。macOS の初期設定では F1 は輝度キーのため、fn+F1 でキャプチャ。輝度調整はそのまま機能 |
| **F2**（変更可） | ピン留め | 直近のショットまたはクリップボードの画像をピン留め |
| **F6**（変更可） | 録画 | 範囲選択モードへ。録画中にもう一度押すと停止 |

ホットキーはメインパネルの設定で変更可能（競合する場合は保存時に通知）。メインパネルはトレイに常駐し、トレイメニューから開きます。

## 権限について（macOS）

- **画面収録**：初回キャプチャ時にシステムが要求します。表示されない・キャプチャに失敗する場合は、システム設定 → プライバシーとセキュリティ → 画面収録 で HawkSnap を有効化し、アプリを再起動してください。
- **アクセシビリティ**：ホバースナップとキーキャストに必要（システム設定 → プライバシーとセキュリティ → アクセシビリティ）。未許可でもドラッグでの範囲指定は可能です（ホバーハイライトのみ無効）。
- **カメラ**：録画の PiP に必要。未許可でも録画自体は正常で、カメラ映像が乗らないだけです。

## 既知の制限

- Windows のインストーラはコード署名なし（初回起動時に SmartScreen が警告）；macOS dmg は公証なし。
- GIF 書き出しは無音で PiP 非対応。GIF のフレームキューには上限（約 22 フレーム）があり、長時間録画の GIF は終盤のみ（MP4 は常に全編）。
- スクロールキャプチャは縦方向のみ。繰り返し要素（チャット吹き出しなど）は「誤って連結しないため」連結を拒否することがあります。
- OCR モデルは初回使用時に約 60MB をダウンロード（以降オフライン可）；OCR は Intel Mac 非対応。
- 録画中にアプリを終了すると MP4 が壊れます（終了時にファイナライズされないため）。
- Rust 側のエラーメッセージは現在中国語のみ（UI はバイリンガル）。

## ソースからビルド

```bash
# macOS 12.3+ / Node.js 20+ / Rust stable
cd app
npm install
npm run tauri dev      # 開発
npm run tauri build    # ビルド（成果物はワークスペースルートの target/release/bundle/）
```

Linux ビルドには `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev` などが必要です（`.github/workflows/pipeline.yml` の Linux ステップを参照）。

## ライセンス

[MIT](LICENSE)。サードパーティの表示は [LICENSE-THIRD-PARTY.md](LICENSE-THIRD-PARTY.md) を参照。
