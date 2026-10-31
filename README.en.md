<div align="center">

<img src="docs/logo.png" alt="HawkSnap" width="140" />

# HawkSnap

### Sharp as a hawk. One press, done.

*Press, frame, done.* ／ *按下快捷键、框住、完事。*

**Free · Fully local · Three platforms (macOS / Windows / Linux) · MIT**

[![Release](https://img.shields.io/github/v/release/huluohu/HawkSnap)](https://github.com/huluohu/HawkSnap/releases)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-0a84ff)](#download)
[![CI](https://github.com/huluohu/HawkSnap/actions/workflows/pipeline.yml/badge.svg)](https://github.com/huluohu/HawkSnap/actions/workflows/pipeline.yml)
[![License](https://img.shields.io/badge/license-MIT-34c759)](LICENSE)

</div>

---

[简体中文](README.md) | [繁體中文](README.zh-Hant.md) | [文言](README.classical.md) | **English** | [日本語](README.ja.md)

<p>
  <img src="docs/screenshots/main.png" width="49%" alt="HawkSnap main panel (light)" />
  <img src="docs/screenshots/main-dark.png" width="49%" alt="HawkSnap main panel (dark)" />
</p>

HawkSnap is a free, fully local, lightweight screen-capture and screen-recording tool — no account, no cloud, no telemetry.

## Features

- **Screenshot**: a global hotkey freezes the screen into an overlay; drag to select (Enter / double-click to confirm, Esc to cancel). Shots go to the clipboard and `~/Pictures/HawkSnap`;
- **Smart snapping**: whatever is under the cursor — element or window — highlights as you hover; click to select (element → window → full screen, three-level fallback). Arrow keys nudge by 1px (Shift for 10px); edges snap within 8px of a window. After framing, an action bar offers copy / save / annotate;
- **Pin**: F2 pins an image to your desktop — drag to move, scroll to zoom (10%–3200%), Ctrl+scroll for opacity (20%–100%), Esc to close; multiple pins cascade automatically;
- **Annotation editor**: rectangle / ellipse / arrow / pen / highlighter / mosaic / counter / text / crop — 9 tools, unlimited undo-redo, ⌘S to save;
- **OCR**: full-image text recognition inside the editor, Chinese and English (PP-OCRv4, fully local inference); copy line by line, export as txt; recognized text feeds full-text search in history;
- **Screen recording**: F6 enters selection mode (drag = region, Enter / double-click = full screen) with a floating bar showing elapsed time plus pause / resume / stop; system audio included (AAC); H.264 MP4 saved to `~/Movies/HawkSnap`; optional **GIF export** (looping, silent) and **camera picture-in-picture** (MP4 only);
- **Scrolling screenshot**: pick a region from the tray, scroll the page, and template matching stitches a long image (canvas cap 256MB / 40000px);
- **Keycast**: while recording, pressed key combinations appear on screen (e.g. ⌘⇧A) via a read-only event tap that never intercepts keys;
- **History**: every shot lands in a local SQLite library — thumbnail grid, Chinese and English full-text search, re-edit / pin / reveal in Finder;
- **Delayed capture**: pick 1 / 2 / 3 / 5 seconds from the tray menu, cancelable during the countdown;
- **Browser extension**: a Chrome / Edge extension grabs a full page or a selection and sends it straight to the desktop app (via the Native Messaging bridge);
- **Auto-update**: an Ed25519-signed update channel; updates with bad signatures are rejected before install;
- **Bilingual UI**: 中文 / English, following the system language or set manually.

## Download

Grab the installer for your platform from [Releases](https://github.com/huluohu/HawkSnap/releases):

| Platform | Installer | Auto-update |
| --- | --- | --- |
| macOS 12.3+ (Apple Silicon / Intel) | dmg | ✅ |
| Windows 10 1903+ / 11 (x64) | NSIS setup / MSI | ✅ |
| Linux (x64) | AppImage / deb / rpm | ✅ |

Check and install new versions in-app under “Settings → Auto-update”.

## Platform differences

| Capability | macOS | Windows / Linux |
| --- | --- | --- |
| Screenshot (region / full screen) | ✅ | ✅ cross-platform backend |
| Window auto-detection (hover pick) | ✅ | ❌ (falls back to manual framing) |
| Recording / PiP / scrolling shot / pin / keycast | ✅ | ❌ (entry points fail explicitly, never silently) |
| OCR / history / editor / browser extension | ✅ | ✅ |

## Usage

| Hotkey | Action | Notes |
| --- | --- | --- |
| **fn+F1** (or F1*) | Capture | Registers the standard F1 key. On macOS, F1 is a brightness key by default — press fn+F1 to capture; brightness keeps working |
| **F2** (customizable) | Pin | Pins the latest shot or a clipboard image |
| **F6** (customizable) | Record | Enters selection mode; press again while recording to stop |

All hotkeys can be customized in the main panel’s settings (a conflicting binding fails to save with a notice). The main panel lives in the tray; open it from the tray menu.

## Permissions (macOS)

- **Screen recording**: the system asks on first capture. If no prompt appears or capture fails, open System Settings → Privacy & Security → Screen Recording, enable HawkSnap, then restart the app.
- **Accessibility**: needed for hover snapping and keycast (System Settings → Privacy & Security → Accessibility). Without it, drag capture still works — just no hover highlight.
- **Camera**: needed for recording picture-in-picture. Without it, recording works fine — only the camera overlay is missing.

## Known limitations

- Windows installers are not code-signed (SmartScreen warns on first run); macOS dmg is not notarized.
- GIF export is silent and excludes picture-in-picture; the GIF frame queue is bounded (~22 frames), so long recordings yield a GIF covering only the closing window (MP4 is always the full take).
- Scrolling screenshots support vertical scrolling only; repetitive layouts (chat bubbles, galleries) may be rejected rather than stitched wrong.
- OCR models download ~60MB on first use (offline afterwards); OCR is not available on Intel Macs.
- Quitting the app mid-recording produces a broken MP4 (the recorder isn’t finalized on quit).
- Rust-side error strings are Chinese for now (the UI is bilingual).

## Building from source

```bash
# macOS 12.3+ / Node.js 20+ / Rust stable
cd app
npm install
npm run tauri dev      # develop
npm run tauri build    # build (artifacts under the workspace-root target/release/bundle/)
```

Linux builds need `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`, etc. (see the Linux steps in `.github/workflows/pipeline.yml`).

## License

[MIT](LICENSE). Third-party notices in [LICENSE-THIRD-PARTY.md](LICENSE-THIRD-PARTY.md).
