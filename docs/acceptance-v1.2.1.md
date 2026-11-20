# 真机验收说明（v1.2.1 审计轮）

> 2026-11-20 第二轮审计战役（86 条发现 → 六波修复）后的验收说明。
> 记录**哪些已在真机验证、哪些需要你本机确认、怎么确认**。

---

## 一、已在真机验证的部分（CI 三平台真机 runner）

CI 的 `smoke` job 在 **macos-15 / ubuntu-24.04 / windows-2022** 上启动真实应用
（真实 WKWebView / WebKitGTK / WebView2），判定依据是前端握手日志
`[hawksnap] frontend ready: main`。

**已实证**：

| 验收项 | 证据 | 结果 |
|---|---|---|
| CSP 不导致白屏（script-src 放行 bundle） | 三平台均出现握手行 | ✅ |
| CSP 放行 IPC（connect-src） | 握手本身是一次 invoke 往返 | ✅ |
| 应用启动无 panic、进程存活 | 脚本判据 | ✅ |
| Linux 托盘/DBus/Xvfb 环境可启动 | Linux 腿握手成功 | ✅ |
| 媒体路径白名单逻辑 | `validate_media_path_in` 单测 ×5（三平台跑） | ✅ |
| 错误码与词典双向对齐 | `check-i18n` errcode 门（162 码 ↔ 162 键） | ✅ |

**尚未运行时实证**（需要真实素材，CI 里没有）：

| 验收项 | 为什么 CI 验不了 | 怎么确认 |
|---|---|---|
| 历史网格缩略图加载（`img-src` 的 asset 协议） | CI 里历史库为空，无图可加载 | 历史里有图 → 打开主面板看缩略图 |
| 覆盖层冻结帧加载（`fetch` asset 协议） | CI 无屏幕录制权限，抓不到帧 | 按 F1 → 覆盖层应显示冻结帧 |
| 媒体白名单的**端到端**路径（重编辑/贴图/OCR） | 需要真实截图文件 | 历史里任一张 → 重新编辑 / 贴图 / OCR |

---

## 二、你本机的验收脚本（我不代跑）

按项目约定，构建/启动/录屏类操作**不在你的工作机上跑**（会占屏、抢焦点、抢 CPU）。
所以验收动作已写成自包含脚本，你方便时跑一次即可：

```bash
cd /path/to/HawkSnap
bash scripts/acceptance-mac.sh            # 构建 + 启动 + 采集（默认 debug 构建）
bash scripts/acceptance-mac.sh --no-build # 复用已有构建
```

产物落在 `/tmp/hawksnap-acceptance-<时间戳>/`：

| 文件 | 内容 |
|---|---|
| `REPORT.md` | 自动汇总：前端握手是否出现、进程是否存活、panic/错误行、**6 条人工确认项** |
| `app.log` | 应用 stdout/stderr（含握手行） |
| `panel.png` | 主面板截图 |
| `perms.txt` | 截图/缩略图/录屏产物的权限位（期望 `-rw-------`） |

**跑完把结果告诉我**（贴 `REPORT.md` 内容，或把整个目录打包给我），我据此判读。

### 人工确认项（脚本无法自动判定）

1. 设置窗内容正常渲染（不是空白页）——CSP 覆盖第二个窗口
2. 历史里任一张 → 「重新编辑」→ 编辑器加载出图（媒体白名单路径）
3. 历史里任一张 → 「贴图」→ 贴图窗显示（macOS 专属）
4. 历史里任一张 → 「OCR」→ 识别出文字（白名单 + 错误码双语）
5. 故意触发失败（把保存目录设成不可写再截图）→ 横幅是**当前语言**文案，不是中文/原始哨兵
6. 触发错误横幅后等 ~10s → 横幅自动消失

---

## 三、CI 里的真机冒烟门（已合入，长期生效）

```
smoke (macos-15 / ubuntu-24.04 / windows-2022)
  ├─ npm ci → 前端 build
  ├─ cargo build -p hawksnap --features tauri/custom-protocol   ← 缺该 feature 会回落 devUrl → 必白屏
  └─ bash scripts/smoke-launch.sh
        ├─ HAWKSNAP_SMOKE=1 启动（自动打开主面板）
        ├─ 等 [hawksnap] frontend ready: main（≤45s）
        ├─ 判进程存活 + 无 panic
        └─ 收尾（Linux 走 xvfb-run + dbus-run-session）
```

`HAWKSNAP_SMOKE=1` 是自检开关（沿用仓库既有 `HAWKSNAP_BENCH` / `HAWKSNAP_G4` 约定），
生产运行零影响。零窗口架构下启动只起托盘，没有这个开关就无从观察「前端跑起来没有」。

---

## 四、已知未修项（审计登记，本轮未做）

详见项目记忆的「登记未修」清单，要点：GIF 32MB 队列预算截断（长录制 GIF 只剩最后
~1 秒）、收尾链未 spawn_blocking 化、抓帧整帧 PNG 白编码、`editor_save` 走 base64
过 IPC、贴图窗口图像双份驻留等。
