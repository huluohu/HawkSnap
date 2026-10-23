# M2 Task M6 Report: 基础录屏（SCStream + AVAssetWriter + 系统声）

## 交付

- `crates/hawksnap-platform-mac/src/record.rs`（新模块）：
  - **纯逻辑**（跨平台可测，15 测试）：`PauseClock`（暂停时间戳平移）、`StreamGate`
    （单流单调门）、`recording_output_path`（`~/Movies/HawkSnap/yyyy-MM-dd-HHmmss.mp4`）、
    `mp4_duration_secs`（moov/mvhd 离线解析，v0/v1 双布局——ffprobe/Spotlight 双缺席
    下的确定性验收路径）。
  - **macOS 管线**：`ScreenRecorder::start/pause/resume/stop`——SCShareableContent
    （CGMainDisplayID 择主显）→ SCContentFilter(initWithDisplay:excludingWindows:[]) →
    SCStream（BGRA@30fps + capturesAudio 48k/2ch/excludesCurrentProcessAudio）→
    AVAssetWriter（AVFileTypeMPEG4：h264 视频 input + AAC 音频 input）→
    `~/Movies/HawkSnap/*.mp4`。帧回调 = `HawkRecordOutput`（define_class 实现
    SCStreamOutput + SCStreamDelegate），视频/音频两条 dispatch 串行队列。
- app 层 `app/src-tauri/src/recording.rs`：状态机（Idle/Recording/Paused，纯转移函数
  + 5 测试）、`record_start/pause/resume/stop/status` 命令（全 async——stop 阻塞收尾
  ≤25s 不卡事件循环）、每秒 `hawk://record-tick {seconds, paused}` 广播线程、悬浮条
  窗口（280×56 深色实心常驻复用 + 主显顶部居中）、`record_stop` → history 入库
  （`app="recording"` 标记、thumb 留空——M0 网格空 thumbPath 占位分支已在）。
- F6 重接线：hotkey.rs Record 分支从 coming-soon 横幅改为 `recording::toggle`
  （空闲启动 / 录制暂停中停止），错误走 `hawk://record-error`（panel::show_record_error
  懒拉主面板，与 capture-error 同款不静默收尾）；App.tsx coming-soon 监听/横幅移除，
  record-error 横幅接入。settings.slot.record 文案去掉「（占位）」。
- 前端 `app/src/windows/recording-bar/index.tsx` + `?window=recording-bar` 路由 +
  `capabilities/recording-bar.json`（core:default）+ 7 个 i18n key 双语
  （record.barLabel/pause/resume/stop/paused、banner.recordError、settings.slot.record）。
- 历史网格：`app === "recording"` 条目渲染 🎬 占位，隐藏 贴图/重新编辑（MP4 非编辑器
  输入），打开目录/删除照常。
- `scripts/soak-record.sh`（G5）：单进程跑满全程（内存增长在同进程累积才可测），
  驱动 soak ignored 测试自采样 RSS/CPU → CSV，脚本断言进程存活 + 内存增长 <10%；
  `HAWKSNAP_SOAK_HOURS` 默认 72，短模式（0.001h）自测通过；`HAWKSNAP_SOAK_KEEP=1`
  保留产物供人工复核。

## 绑定可用性矩阵（ladder 裁定依据）

| 需求 | crate | 结论 |
|---|---|---|
| SCStream/Config/Filter/ShareableContent/Output/Delegate | **objc2-screen-capture-kit 0.3.2**（唯一新增 crate，objc2 0.6 生态同版） | **全覆盖 → ladder rung 1（用 crate，未手写 extern）** |
| AVAssetWriter/Input/PixelBufferAdaptor/AVMediaType/AVFileType/AVVideoCodecTypeH264 | objc2-av-foundation 0.3.2（已在树） | 全覆盖；AVAudioSettings.h 常量未生成 → 键名/FourCC 硬编码（`kAudioFormatMPEG4AAC='aac '`） |
| CMTime/CMSampleBuffer（PTS/duration/create_copy_with_new_timing/image_buffer） | objc2-core-media 0.3.2（已在树） | 全覆盖 |
| CVPixelBuffer/kCVPixelFormatType_32BGRA | objc2-core-video 0.3.2（已在树） | 全覆盖（CVPixelBuffer=CVImageBuffer=CVBuffer 类型别名，CF 类型直通 adaptor） |
| CGMainDisplayID/CGDisplayPixels{Wide,High} | 手写 3 个 C extern（ax.rs/permissions.rs 同款） | ScreenCaptureKit 绑定不含 CG 显示查询 |
| 帧回调队列 | dispatch2 0.3（sck 0.3.2 要求 ≥0.3；wry 树内 0.2 为旧命名 Queue，双版本共存） | DispatchQueue::new(label, SERIAL) |

## 架构裁定：SCStream → AVAssetWriter（替换 VT 硬编 + 手写 mux）

计划原文管线「SCStream → CVPixelBuffer → VideoToolbox H264 → MP4 分段写盘」裁定为
**SCStream → AVAssetWriter**（简报预授权的判断点）：SCStream 交付原始 BGRA
CVPixelBuffer，AVAssetWriter 的 h264 output settings input **内部即走 VideoToolbox
硬编**（Apple 文档明示 H.264/HEVC 编码器），MP4 容器 mux 也是其原生职责——QuickTime
Player 录屏同款栈。显式 VTCompressionSession + 手写 MP4 muxer 两块深水区全部消解，
零自绘字节格式代码。

**踩坑实录（AVAssetWriter 会话时序，两处 NSInternalInconsistencyException）**：
1. `startSessionAtSourceTime` 在 status=Unknown 时调用 → "Cannot call method when
   status is 0"——**必须先 `startWriting()`**（Unknown → Writing）。
2. `startWriting` 后仍需在**首个样本 append 前**显式 `startSessionAtSourceTime`
   （adaptor/appendSampleBuffer 都在无会话时抛 "Must start a session first"）。
   最终时序：startWriting（start 内）→ 首样本回调里懒调 startSessionAtSourceTime
   （绝对主机时钟 PTS 相对化为 0 起）→ append。
3. completion handler 交付的 SCShareableContent 是 **+0 引用**：`Retained::from_raw`
   （adopt）会过度释放 → 随机 SIGTRAP/SIGABRT；改 `Retained::retain` 后稳定。
4. `AVVideoCodecH264` 常量的**值是 "avc1"** 而非字面 "AVVideoCodecH264"（键名字面量
   ≠ 值）——字典 value 必须读真常量，否则 canAddInput 拒绝。
5. objc2 类型保守 `!Send`：AVAssetWriter/SCStream 均 Apple 明示 thread-safe →
   `unsafe impl Send for RecShared/ScreenRecorder`（SAFETY 注释在案）。

## 麦克风裁定（ladder）：M6 不做，系统声 only

系统声走 SCStream 自带 `capturesAudio`（ladder 首版口径）；麦克风需独立 AVAudioEngine
tap + 双路 LPCM 手工混音成单 AudioBufferList 再重建 CMSampleBuffer——计划原文「绑定
过深 → 首版仅系统声，麦克风列 M2.1」判定命中。Info.plist 无需 NSMicrophoneUsageDescription
（无麦克风 TCC 触达）。**TCC**：复用 Plan D4 的 `screen_recording_prechecked` 预检，
未授予 → 深链系统设置 + Err「屏幕录制权限未授予：已为你打开系统设置，授权后重新按
F6 即可」。

## 暂停语义：时间戳平移（非丢帧空洞）

暂停 = 回调丢帧（phase 原子检查）；恢复后 `PauseClock` 把新帧 PTS 整体平移掉暂停
时长：`accumulated += max(0, 首帧PTS − 暂停前末帧PTS)`（视频/音频共享 last_pts 基准，
音画同步平移不漂移；不依赖外部时钟与 SCStream PTS 基准一致性）。每流一条 `StreamGate`
单调门兜恢复瞬间的重复/回退时间戳（丢 ≤1 帧，时间线严格递增——AVAssetWriter 硬要求）。
纯逻辑 5 测试 + 真实 2s/2s/2s 时长断言双覆盖（下表实测 4.005s）。

## 范围裁定（scope cut，显著记录）

**M6 首版 = F6 直接录制整个主显示器，无区域选区**——区域录制需要 overlay 选区模式
状态机改造（覆盖层正服务于截图流），值得独立任务；README + 本报告 + recording.rs
模块注三处记录，区域录制归 M3 计划。悬浮条位置 = 主显顶部居中（不遮挡内容区）。

## TDD 证据

先测后实的纯逻辑面（文件内测试与实现同 commit 交付）：

| 面 | 测试 | 结果 |
|---|---|---|
| PauseClock 平移数学 | 直通/暂停平移/多次累计/负差值兜底/首帧即恢复 | 5 ✓ |
| StreamGate 单调门 | 递增接受/重复丢弃/回退丢弃/1µs 容差 | 2 ✓ |
| 输出路径 | `2026-10-23-090507.mp4` 格式 + 目录创建 | 1 ✓ |
| mvhd 解析 | v0（600/2400→4s）/v1（48000/192000→4s）/垃圾拒绝 | 3 ✓ |
| 状态机（app 层） | 合法转移/非法拒绝/幂等方向/计时扣除×4/HOME 口径 | 5 ✓ |

`cargo test --workspace`：**151 passed, 0 failed**（app 80 + core 26 + ocr 28 +
platform 15 + update_signature 2）；`npm test` 120 passed；`npm run build` ✓；
`check:i18n` OK（A⊆Z、A⊆E 且 Z==E，动态 key WARN 为既有 hotkey.kind.* 约定项）。

## 真实录制集成测试输出（本机实跑，屏幕录制已授权）

```
cargo test -p hawksnap-platform-mac --lib real_recording_produces_valid_mp4 -- --ignored
  [m6] real recording: 1169812 bytes, duration=5.027s (target 5s)     ✓ (>50KB + ftyp 魔数 + mvhd ≈5s±1.5)
cargo test -p hawksnap-platform-mac --lib real_pause_resume_timeline_gapless -- --ignored
  [m6] pause/resume: duration=4.005s (target ≈4s, 2s pause excluded)  ✓（验收口径 ±300ms 实测 5ms）
```

产物二级验证（3s 录制 + afinfo/atom 扫描）：

```
xxd: 00000000  0000 001c 6674 7970 6d70 3432 …   → "ftypmp42" ✓
afinfo: Num Tracks: 2 — video(avc1) + audio(aac, 2ch 48000Hz) ✓（系统声实测在轨）
atom 计数: trak=2 vide=1 soun=1 avc1=1 mp4a=1 ✓
```

## F7 CPU 抽测（1080p30 档，`ps -o rss=,%cpu=`，soak 采样路径）

30s 录制（3024×1964 物理像素 @30fps，实际高于 1080p 口径）：

```
2026-10-24T03:35:07 …,84800 KB, 4.3%
2026-10-24T03:35:17 …,50752 KB, 5.4%
2026-10-24T03:35:27 …,50768 KB, 4.9%
```

**CPU 4.3–5.4%，预算 ≤8% 达标**（注意口径：测试进程 RSS 含 libtest harness，
非应用常驻进程——G5 正式启测时 soak 脚本采的是同口径，纵向可比）。

## G5 soak 脚本

`scripts/soak-record.sh`：`HAWKSNAP_SOAK_HOURS`（默认 72）单进程单会话跑满全程
（内存增长才在同进程累积），每小时 RSS/CPU 自采样 → CSV，退出码 0（崩溃=0）+
首尾 RSS 增长 <10% 双判定。短模式（0.001h）自测通过：`PASS：进程存活全程 + 内存
增长 <10%`（+0.4%/3 样本）。首次运行 TCC 可能弹屏幕录制授权（测试二进制身份随
rebuild 变化），控制器执行时注意。

## 遗留 / 风险

1. **暂停竞态偏斜 ≤1 帧/次**：恢复瞬间在途帧的 PTS 差含管线延迟，平移量偏差有界
   （≤1 帧 @30fps ≈ 33ms），实测 4.005s 证明无感；音频/视频共享基准不漂移。
2. **录制中 TCC 撤销/显示器拔出**：SCStreamDelegate `stream:didStopWithError:` 记错
   误，悬浮条继续计时，用户 stop 时上抛「文件已生成，请核查」——M6 不做自动终止。
3. **音频背压丢帧**：AAC 编码积压时 `isReadyForMoreMediaData=false` 丢音频帧（直播
   语义，比无界积压诚实）；极端负载下可能有静音间隙。
4. **历史条目动作**：录屏条目无缩略图（thumb 空 → 🎬 占位）、无 重新编辑/贴图；
   双击贴图等后续按 spec 归 M3。
5. 测试二进制 TCC 身份随 rebuild 变化（macOS 签名机制），ignored 集成测试在 CI 无
   授权环境恒 skip（`#[ignore]`），与 XcapGrabber 真录屏测试同口径。
