//! M6 基础录屏：SCStream 全屏采集 → AVAssetWriter mux MP4（h264 + AAC）。
//!
//! ## 架构裁定（任务简报预授权的判断点，详见任务报告）
//!
//! 原计划「SCStream → CVPixelBuffer → VideoToolbox 硬编 → 手写 MP4 mux」裁定为
//! **SCStream → AVAssetWriter（AVFileTypeMPEG4，AVVideoCodecH264 + AAC）**：SCStream
//! 交付的是原始 CVPixelBuffer（IOSurface-backed），AVAssetWriter 的视频 input（h264
//! output settings）内部即走 VideoToolbox 硬编，MP4 容器 mux 也是它原生职责——
//! QuickTime Player 录屏同款管线。显式 VTCompressionSession + 手写 muxer 两块深水区
//! 全部消解，且不引入任何自绘字节格式代码。
//!
//! ## 绑定选型（ladder rung 1，未降级）
//!
//! - `objc2-screen-capture-kit 0.3.2`（crates.io 已发布，与锁文件 objc2 0.6 生态同版）：
//!   覆盖 SCStream / SCStreamConfiguration（width/height/minimumFrameInterval/pixelFormat/
//!   capturesAudio/sampleRate/channelCount/excludesCurrentProcessAudio）/ SCContentFilter
//!   (initWithDisplay:excludingWindows:) / SCShareableContent（completion handler 版）/
//!   SCStreamOutput（stream:didOutputSampleBuffer:ofType:）/ SCStreamDelegate。**用 crate，
//!   不手写 extern**（ax.rs 模式仅用于 crate 缺口的 CGDisplayPixels{Wide,High}/
//!   CGMainDisplayID 三个 C 函数——ScreenCaptureKit 绑定不包含 CoreGraphics 显示查询）。
//! - `objc2-av-foundation 0.3.2`（已在依赖树）：AVAssetWriter / AVAssetWriterInput /
//!   AVAssetWriterInputPixelBufferAdaptor / AVMediaType / AVVideoCodecH264 全覆盖。
//!   AVAudioSettings.h 常量（AVFormatIDKey 等）未生成 → 键名/值以字符串/FourCC 硬编码
//!   （`kAudioFormatMPEG4AAC = 'aac '`）。
//! - `objc2-core-media` / `objc2-core-video`（已在依赖树）：CMTime / CMSampleBuffer
//!   （presentation_time_stamp / duration / create_copy_with_new_timing / image_buffer）/
//!   CVPixelBuffer（= CVImageBuffer = CVBuffer 的类型别名，CF 类型）/
//!   kCVPixelFormatType_32BGRA。
//! - `dispatch2 0.2`（已在依赖树）：SCStreamOutput 的 sampleHandlerQueue（视频/音频两条
//!   串行队列，audio append 不阻塞 video append）。
//! - **麦克风（ladder 深端）裁定：M6 出**。系统声走 SCStream 自带 `capturesAudio`
//!   （本 ladder 的首版口径）；麦克风需独立 AVAudioEngine tap + 双路 LPCM 手工混音
//!   成单 AudioBufferList 再重建 CMSampleBuffer——「绑定过深」判定命中，麦克风列 M2.1
//!   （计划原文：「若 AudioUnit 绑定过深 → 首版仅系统声（SCStream audio sample handler），
//!   麦克风列 M2.1」）。
//!
//! ## 暂停语义（时间戳平移，非丢帧空洞）
//!
//! 暂停 = 停止 append（回调直接丢弃帧）；恢复后把新帧时间戳整体平移掉暂停时长，
//! 时间线无空洞（无冻结静音段）。平移基准取**帧 PTS 本身**（[`PauseClock`]）：首个
//! 恢复后交付的帧与暂停前最后一帧的 PTS 差即「暂停 + 管线延迟」，计入累计平移量——
//! 不依赖外部时钟与 SCStream PTS 基准的一致性。每流一条单调门（[`StreamGate`]）
//! 兜住恢复瞬间的重复/回退时间戳（最多丢 1 帧，时间线保持严格递增——AVAssetWriter
//! 的硬要求）。
//!
//! ## 线程纪律
//!
//! SCStream/AVAssetWriter 均为线程安全 API，全程不涉主线程：setup 与 stop 在调用方
//! 线程（tauri async 命令的运行时线程 / 测试线程），帧回调在自建 dispatch 串行队列，
//! 共享状态一把 Mutex 串行化（phase 检查 + 时钟更新同锁，无 TOCTOU）。

use std::path::PathBuf;
use std::ptr::NonNull;

use chrono::{DateTime, Local};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass};

use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterInputPixelBufferAdaptor,
    AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaTypeAudio, AVMediaTypeVideo,
    AVVideoCodecTypeH264,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{kCMTimeInvalid, CMSampleBuffer, CMSampleTimingInfo, CMTime};
use objc2_core_video::kCVPixelFormatType_32BGRA;
use objc2::runtime::NSObject;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSURL, NSError, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate,
    SCStreamOutput, SCStreamOutputType,
};

// ---------------------------------------------------------------------------
// 纯逻辑（跨平台可测）：暂停时间戳平移 + 单调门 + 输出路径 + MP4 时长解析
// ---------------------------------------------------------------------------

/// 暂停平移时钟：把「交付帧的原始 PTS」映射为「无空洞时间线上的 PTS」。
///
/// - `accumulated_secs`：截至当前已累计的平移量（= 所有已完成暂停的时长之和）；
/// - `awaiting_first_frame`：恢复后置位，首个交付帧兑现本次暂停的平移量；
/// - `last_delivered_pts`：暂停前最后一帧的原始 PTS（视频/音频共享——两流 PTS 同源
///   主机时钟，共享基准让平移量严格一致，音画不漂移）。
///
/// 兑现公式：`accumulated += max(0, 首帧PTS - 暂停前末帧PTS)`——差值 = 暂停时长 +
/// 管线延迟（≤1 帧的过渡帧竞态，见模块注）。恢复后交付的帧才走本函数；暂停期间
/// 的帧由回调在 phase 检查处直接丢弃（不更新时钟）。
#[derive(Debug, Default)]
pub struct PauseClock {
    accumulated_secs: f64,
    awaiting_first_frame: bool,
    last_delivered_pts: Option<f64>,
}

impl PauseClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// 进入暂停（回调侧丢帧即可；保留方法为语义显式 + 可测）。
    pub fn on_pause(&mut self) {}

    /// 恢复：下一帧交付时兑现平移。
    pub fn on_resume(&mut self) {
        self.awaiting_first_frame = true;
    }

    /// 交付一帧：返回平移后的 PTS。暂停期间不得调用（调用方先查 phase）。
    pub fn on_delivered(&mut self, pts_secs: f64) -> f64 {
        if self.awaiting_first_frame {
            if let Some(last) = self.last_delivered_pts {
                // max(0)：防御性兜底（时钟回拨/乱序），不把负差值计成倒退。
                self.accumulated_secs += (pts_secs - last).max(0.0);
            }
            self.awaiting_first_frame = false;
        }
        self.last_delivered_pts = Some(pts_secs);
        pts_secs - self.accumulated_secs
    }
}

/// 单流单调门：AVAssetWriter 要求同 input 的样本时间戳严格递增，重复/回退帧直接
/// 拒收。恢复后的首帧平移后恰等于暂停前末帧位置（时间线无缝），对本流即「重复」
/// → 丢 1 帧（≤33ms@30fps，时间线仍无空洞）；视频/音频各持一门，互不误伤。
#[derive(Debug, Default)]
pub struct StreamGate {
    last_shifted: Option<f64>,
}

impl StreamGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Some(shifted) = 允许 append；None = 丢弃（重复/回退）。
    pub fn accept(&mut self, shifted_secs: f64) -> Option<f64> {
        if let Some(last) = self.last_shifted {
            // 容差 1µs：浮点经 CMTime 600 基准量化后可能出现同位微差。
            if shifted_secs <= last + 1e-6 {
                return None;
            }
        }
        self.last_shifted = Some(shifted_secs);
        Some(shifted_secs)
    }
}

/// 录屏输出路径：`<base>/<yyyy-MM-dd>-<HHmmss>.mp4`；目录不存在则创建（含父目录）。
/// 创建失败（权限等）→ Err，调用方拒绝启动（不静默换路径）。
pub fn recording_output_path(
    base_dir: &std::path::Path,
    now: DateTime<Local>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(base_dir)
        .map_err(|e| format!("创建录屏目录 {} 失败: {e}", base_dir.display()))?;
    Ok(base_dir.join(format!("{}.mp4", now.format("%Y-%m-%d-%H%M%S"))))
}

/// MP4 容器时长（秒）：扫描 `mvhd` atom——version 0（32 位时长）与 version 1（64 位）
/// 两种布局。跨平台纯解析（无 ffprobe/Spotlight 依赖），集成测试与 soak 脚本共用；
/// 非 MP4/损坏文件 → None。
pub fn mp4_duration_secs(data: &[u8]) -> Option<f64> {
    let pos = data.windows(4).position(|w| w == b"mvhd")?;
    // mvhd box：[size4]["mvhd"][version1][flags3][ctime][mtime][timescale][duration]
    //   v0 = ctime4 + mtime4 + timescale4 + duration4
    //   v1 = ctime8 + mtime8 + timescale4 + duration8
    let v = *data.get(pos + 4)? as u32;
    let (timescale_off, duration_off, duration_len) = if v == 1 {
        (pos + 4 + 4 + 16, pos + 4 + 4 + 16 + 4, 8)
    } else {
        (pos + 4 + 4 + 8, pos + 4 + 4 + 8 + 4, 4)
    };
    let ts = data.get(timescale_off..timescale_off + 4)?;
    let timescale = u32::from_be_bytes(ts.try_into().ok()?);
    if timescale == 0 {
        return None;
    }
    let d = data.get(duration_off..duration_off + duration_len)?;
    let duration = if duration_len == 8 {
        u64::from_be_bytes(d.try_into().ok()?) as f64
    } else {
        u32::from_be_bytes(d.try_into().ok()?) as f64
    };
    Some(duration / timescale as f64)
}

/// [`mp4_duration_secs`] 的文件路径封装（测试/脚本侧用）。
pub fn mp4_duration_secs_at(path: &std::path::Path) -> Option<f64> {
    mp4_duration_secs(&std::fs::read(path).ok()?)
}

// ---------------------------------------------------------------------------
// macOS 管线（SCStream → AVAssetWriter）
// ---------------------------------------------------------------------------

const OUTPUT_TIMESCALE: i32 = 600;

// —— AVAudioSettings.h 常量未生成（objc2-av-foundation 只生成类/协议）→ 硬编码 ——
const AV_FORMAT_ID_KEY: &str = "AVFormatIDKey";
const AV_SAMPLE_RATE_KEY: &str = "AVSampleRateKey";
const AV_NUMBER_OF_CHANNELS_KEY: &str = "AVNumberOfChannelsKey";
/// kAudioFormatMPEG4AAC = 'aac '（AudioToolbox 四字符码）。
const K_AUDIO_FORMAT_MPEG4_AAC: i64 = 0x6161_6320;

/// 录屏元信息（stop 后供 app 层历史入库 w/h）。
#[derive(Debug, Clone, Copy)]
pub struct RecordingInfo {
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

/// 共享运行态：帧回调（两条 dispatch 队列）与控制方（start/pause/resume/stop）交汇。
/// 一把锁串行化 phase 检查 + 时钟更新（模块注的 TOCTOU 论证基于此）。
struct RecShared {
    /// 0 = recording（正常 append），1 = paused（回调丢帧），2 = stopping（回调丢帧）。
    phase: std::sync::atomic::AtomicU8,
    state: std::sync::Mutex<Option<RecSession>>,
}

struct RecSession {
    clock: PauseClock,
    video_gate: StreamGate,
    audio_gate: StreamGate,
    session_started: bool,
    writer: Option<Retained<AVAssetWriter>>,
    adaptor: Option<Retained<AVAssetWriterInputPixelBufferAdaptor>>,
    audio_input: Option<Retained<AVAssetWriterInput>>,
    stream: Option<Retained<SCStream>>,
    video_queue: Option<dispatch2::DispatchRetained<dispatch2::DispatchQueue>>,
    audio_queue: Option<dispatch2::DispatchRetained<dispatch2::DispatchQueue>>,
    /// 管线错误（writer append 失败 / stream didStopWithError）：stop 时上抛。
    error: Option<String>,
    info: RecordingInfo,
    output_path: PathBuf,
}

// —— ScreenCaptureKit 不覆盖 CoreGraphics 显示查询 → ax.rs 同款裸 extern（3 个 C 函数）——
mod cg_display_ffi {
    // CGDirectDisplayID = UInt32（CGDisplayConfiguration.h）。
    pub type CGDirectDisplayID = u32;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        /// 主显示器 id（SCDisplay 列表无主屏标记，以 CG 口径为准）。
        pub fn CGMainDisplayID() -> CGDirectDisplayID;
        /// 物理像素宽（SCDisplay.width 是 points；SCStreamConfiguration.width 要物理像素）。
        pub fn CGDisplayPixelsWide(display: CGDirectDisplayID) -> usize;
        pub fn CGDisplayPixelsHigh(display: CGDirectDisplayID) -> usize;
    }
}

// —— 帧回调对象（SCStreamOutput + SCStreamDelegate）——

struct OutputIvars {
    shared: std::sync::Arc<RecShared>,
}

define_class!(
    // SAFETY: NSObject 无子类化约束；本对象不实现 Drop（Retained 归 SCStream 与
    // ScreenRecorder 双方持有，正常引用计数释放）。
    #[unsafe(super(NSObject))]
    #[name = "HawkRecordOutput"]
    #[ivars = OutputIvars]
    struct HawkRecordOutput;

    unsafe impl NSObjectProtocol for HawkRecordOutput {}

    unsafe impl SCStreamOutput for HawkRecordOutput {
        // 帧回调：视频（SCStreamOutputTypeScreen，IOSurface CVPixelBuffer）与音频
        // （SCStreamOutputTypeAudio，LPCM AudioBufferList）共用本入口，按 type 分流。
        #[allow(non_snake_case)]
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn stream_didOutputSampleBuffer_ofType(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            r#type: SCStreamOutputType,
        ) {
            self.handle_sample(sample_buffer, r#type);
        }
    }

    unsafe impl SCStreamDelegate for HawkRecordOutput {
        // 流自发终止（显示器拔出/TCC 撤销等）：记错误，等用户侧 stop 收口上报。
        #[allow(non_snake_case)]
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn stream_didStopWithError(&self, _stream: &SCStream, error: &NSError) {
            let ivars = self.ivars();
            let mut session = ivars.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = session.as_mut() {
                if s.error.is_none() {
                    s.error = Some(format!("SCStream 异常终止: {error}"));
                }
            }
        }
    }
);

impl HawkRecordOutput {
    fn new(shared: std::sync::Arc<RecShared>) -> Retained<Self> {
        // alloc → set_ivars → 父类 init（define_class 的 ivars 初始化契约，同 pin.rs）。
        let this = Self::alloc().set_ivars(OutputIvars { shared });
        unsafe { msg_send![super(this), init] }
    }

    fn handle_sample(&self, sb: &CMSampleBuffer, r#type: SCStreamOutputType) {
        let ivars = self.ivars();
        let mut session = ivars.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = session.as_mut() else { return };

        // 暂停/停止中：直接丢帧（时间戳平移保时间线无空洞，见模块注）。
        let phase = ivars
            .shared
            .phase
            .load(std::sync::atomic::Ordering::SeqCst);
        if phase != 0 {
            return;
        }

        let pts_raw = unsafe { sb.presentation_time_stamp().seconds() };
        if !pts_raw.is_finite() {
            return; // kCMTimeInvalid 等（理论不可达，防御）
        }
        let shifted = s.clock.on_delivered(pts_raw);

        let gate_shifted = if r#type == SCStreamOutputType::Screen {
            s.video_gate.accept(shifted)
        } else {
            s.audio_gate.accept(shifted)
        };
        let Some(shifted) = gate_shifted else { return };

        if !s.session_started {
            // 首样本落位 = 会话起点（startWriting 之后的 startSessionAtSourceTime
            // 把绝对主机时钟 PTS 相对化为 0 起；MP4 时长 = 平移后样本跨度）。
            let Some(writer) = s.writer.as_ref() else { return };
            unsafe {
                writer.startSessionAtSourceTime(CMTime::with_seconds(shifted, OUTPUT_TIMESCALE));
            }
            s.session_started = true;
        }

        if r#type == SCStreamOutputType::Screen {
            self.append_video(s, sb, shifted);
        } else {
            self.append_audio(s, sb, shifted);
        }
    }

    /// 视频帧：IOSurface CVPixelBuffer 直通 adaptor（h264 编码在 writer 内部走 VT）。
    fn append_video(&self, s: &mut RecSession, sb: &CMSampleBuffer, shifted: f64) {
        let (Some(adaptor), Some(writer)) = (s.adaptor.as_ref(), s.writer.as_ref()) else {
            return;
        };
        // 实时背压（评审修复，与音频路径 :append_audio 同策略）：expectsMediaData
        // InRealTime 下编码器瞬时积压的 NO 是良性丢帧（≤1 帧 @30fps），不是会话故障。
        // 恢复瞬间首帧 PTS 也可能以微差落在会话起点之下（音频先赢 startSession 的
        // ordering race）→ 同样表现为一次良性 NO——一律静默丢弃；只有 writer 进入
        // Failed（status/error 为 AVAssetWriter 文档明示的线程安全对）才记会话错误，
        // 杜绝「一次良性 NO → stop 永远 Err」与首错覆盖后错两个问题。
        if !unsafe { adaptor.assetWriterInput().isReadyForMoreMediaData() } {
            return;
        }
        // CMSampleBufferGetImageBuffer 返回的 CVImageBuffer 随样本缓冲存活（此处已
        // retain）；CVPixelBuffer 是 CVImageBuffer 的类型别名（同 CF 类型），直通。
        let Some(img) = (unsafe { sb.image_buffer() }) else { return };
        let ok = unsafe {
            adaptor.appendPixelBuffer_withPresentationTime(
                &img,
                CMTime::with_seconds(shifted, OUTPUT_TIMESCALE),
            )
        };
        if !ok && unsafe { writer.status() } == AVAssetWriterStatus::Failed {
            if s.error.is_none() {
                s.error = Some(format!(
                    "视频帧写入失败（writer Failed）: {:?}",
                    unsafe { writer.error() }.map(|e| e.to_string())
                ));
            }
        }
    }

    /// 音频帧：LPCM 样本带原始时间戳 → 重建平移时间戳的副本再 append
    /// （appendSampleBuffer 用样本自带时序，无法旁路指定 PTS；writer 侧 AAC 转码）。
    fn append_audio(&self, s: &mut RecSession, sb: &CMSampleBuffer, shifted: f64) {
        let Some(input) = s.audio_input.as_ref() else { return };
        // 实时背压：input 缓冲满（AAC 编码积压）时丢帧——比无界积压更符合直播语义。
        if !unsafe { input.isReadyForMoreMediaData() } {
            return;
        }
        // SAFETY: 纯常量读取/纯值构造（extern static kCMTimeInvalid 由系统保证有效）。
        let timing = CMSampleTimingInfo {
            duration: unsafe { sb.duration() },
            presentationTimeStamp: unsafe { CMTime::with_seconds(shifted, OUTPUT_TIMESCALE) },
            decodeTimeStamp: unsafe { kCMTimeInvalid },
        };
        // Option<CFRetained<T>> 的 ABI 即裸指针（niche 优化），可安全 reinterpret 为
        // CF 风格的 out 参数槽位。
        let mut out: Option<CFRetained<CMSampleBuffer>> = None;
        let out_ptr = NonNull::from(&mut out).cast::<*mut CMSampleBuffer>();
        let status = unsafe { CMSampleBuffer::create_copy_with_new_timing(None, sb, 1, &timing, out_ptr) };
        if status != 0 || out.is_none() {
            if s.error.is_none() {
                s.error = Some(format!("音频样本时间戳重建失败 (osstatus={status})"));
            }
            return;
        }
        let Some(new_sb) = out else { return };
        let ok = unsafe { input.appendSampleBuffer(&new_sb) };
        if !ok {
            if s.error.is_none() {
                s.error = Some("音频样本写入失败".into());
            }
        }
    }
}

/// 运行中的录屏器：`start` → `pause`/`resume`（幂等）→ `stop`（阻塞收尾）。
/// app 层持 `Mutex<Option<ScreenRecorder>>`。
pub struct ScreenRecorder {
    shared: std::sync::Arc<RecShared>,
    // SCStream 对 output 对象持强引用；本侧 Retained 仅保证 start→stop 期间对象
    // 引用计数不落零（防御 stopCapture 后的残余回调）。
    _output: Retained<HawkRecordOutput>,
}

// SAFETY（Send）：本类型在 start（async 命令线程）与 pause/resume/stop（另一命令
// 线程）之间**移动**——Retained 移动仅转移指针所有权，访问全部经方法调用。所持
// ObjC 对象的跨线程可用性：AVAssetWriter / AVAssetWriterInput / AVAssetWriterInput
// PixelBufferAdaptor 为 Apple 头文件明示「thread safe」的类；SCStream 为 ScreenCaptureKit
// 的跨线程控制面（start/stop 任意线程）；HawkRecordOutput 的 ivars 是 Arc<RecShared>
// （见下），对象本身的回调全部发生在 SCStream 派发的队列上。共享态 RecShared 的
// 可变字段（RecSession）只在 Mutex 持锁路径上触碰，phase 是 AtomicU8。
unsafe impl Send for RecShared {}
unsafe impl Send for ScreenRecorder {}

impl ScreenRecorder {
    /// 启动全屏（主显示器）录制：SCShareableContent 枚举 → 主显示器 filter →
    /// SCStream（视频 BGRA@30fps + 系统声 48k/2ch）→ AVAssetWriter（h264+AAC → MP4）。
    /// 阻塞至 startCapture 完成（~百 ms 级，调用方为 async 命令线程，不卡主线程）。
    pub fn start(output_path: PathBuf) -> Result<ScreenRecorder, String> {
        if output_path.exists() {
            return Err(format!("输出文件已存在: {}", output_path.display()));
        }
        let shared = std::sync::Arc::new(RecShared {
            phase: std::sync::atomic::AtomicU8::new(0),
            state: std::sync::Mutex::new(None),
        });
        let output = HawkRecordOutput::new(shared.clone());

        // 1. 可共享内容（completion handler 异步 → channel 同步收口）。
        let content = get_shareable_content()?;
        let displays = unsafe { content.displays() };
        let main_id = unsafe { cg_display_ffi::CGMainDisplayID() };
        let mut picked: Option<Retained<objc2_screen_capture_kit::SCDisplay>> = None;
        let count = displays.len();
        for i in 0..count {
            let d = displays.objectAtIndex(i);
            if unsafe { d.displayID() } == main_id {
                picked = Some(d);
                break;
            }
            if picked.is_none() {
                picked = Some(d);
            }
        }
        let display = picked.ok_or_else(|| "未找到可捕获的显示器".to_string())?;
        let display_id = unsafe { display.displayID() };
        let (w, h) = (
            unsafe { cg_display_ffi::CGDisplayPixelsWide(display_id) },
            unsafe { cg_display_ffi::CGDisplayPixelsHigh(display_id) },
        );
        if w == 0 || h == 0 {
            return Err(format!("显示器物理尺寸异常: {w}x{h}"));
        }

        // 2. filter = 主显示器、排除窗口列表为空（不排除任何窗口 → 全桌面）。
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingWindows(
                objc2_screen_capture_kit::SCContentFilter::alloc(),
                &display,
                &NSArray::new(),
            )
        };

        // 3. 配置：30fps BGRA + 系统声（48k/2ch，排除本进程自己的音频）。
        let config = unsafe {
            let cfg = SCStreamConfiguration::new();
            cfg.setWidth(w);
            cfg.setHeight(h);
            cfg.setMinimumFrameInterval(CMTime::new(1, 30));
            cfg.setPixelFormat(kCVPixelFormatType_32BGRA);
            cfg.setShowsCursor(true);
            cfg.setCapturesAudio(true);
            cfg.setSampleRate(48000);
            cfg.setChannelCount(2);
            cfg.setExcludesCurrentProcessAudio(true);
            cfg
        };

        // 4. AVAssetWriter + 双 input（h264 视频 + AAC 音频 → MP4 容器）。
        let path_str = output_path
            .to_str()
            .ok_or_else(|| format!("输出路径含非法字符: {}", output_path.display()))?;
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path_str), false);
        let writer = unsafe {
            AVAssetWriter::assetWriterWithURL_fileType_error(
                &url,
                AVFileTypeMPEG4.unwrap(),
            )
        }
        .map_err(|e| format!("AVAssetWriter 初始化失败: {e}"))?;

        // 注意：AVVideoCodecH264 常量的值是 "avc1"（非字面 "AVVideoCodecH264"）——
        // 字典的 KEY 是头文件宏名字面量，VALUE 必须读真常量。
        let video_settings = settings_dict(&[
            ("AVVideoCodecKey", string_const(unsafe { AVVideoCodecTypeH264 })),
            ("AVVideoWidthKey", number_value(w as i64)),
            ("AVVideoHeightKey", number_value(h as i64)),
        ]);
        let video_input = unsafe {
            AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                AVMediaTypeVideo.unwrap(),
                Some(&video_settings),
            )
        };
        unsafe { video_input.setExpectsMediaDataInRealTime(true) };

        let adaptor = unsafe {
            AVAssetWriterInputPixelBufferAdaptor::assetWriterInputPixelBufferAdaptorWithAssetWriterInput_sourcePixelBufferAttributes(
                &video_input,
                Some(&settings_dict(&[
                    ("kCVPixelBufferPixelFormatTypeKey", number_value(kCVPixelFormatType_32BGRA as i64)),
                    ("kCVPixelBufferWidthKey", number_value(w as i64)),
                    ("kCVPixelBufferHeightKey", number_value(h as i64)),
                ])),
            )
        };

        let audio_settings = settings_dict(&[
            (AV_FORMAT_ID_KEY, number_value(K_AUDIO_FORMAT_MPEG4_AAC)),
            (AV_SAMPLE_RATE_KEY, number_value(48000)),
            (AV_NUMBER_OF_CHANNELS_KEY, number_value(2)),
        ]);
        let audio_input = unsafe {
            AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                AVMediaTypeAudio.unwrap(),
                Some(&audio_settings),
            )
        };
        unsafe { audio_input.setExpectsMediaDataInRealTime(true) };

        if !unsafe { writer.canAddInput(&video_input) } {
            return Err("AVAssetWriter 拒绝视频 input".into());
        }
        unsafe { writer.addInput(&video_input) };
        if !unsafe { writer.canAddInput(&audio_input) } {
            return Err("AVAssetWriter 拒绝音频 input".into());
        }
        unsafe { writer.addInput(&audio_input) };
        // startWriting：Unknown → Writing（首样本 PTS 即会话起点——writer 对未显式
        // startSessionAtSourceTime 的场景以首个 append 样本的时间开场，正是我们要的
        // 「首帧落位 = t0」语义；显式调用反而抛 NSInternalInconsistencyException
        // （status 必须已是 Writing，实测见任务报告））。
        if !unsafe { writer.startWriting() } {
            return Err(format!(
                "AVAssetWriter startWriting 失败: {:?}",
                unsafe { writer.error() }.map(|e| e.to_string())
            ));
        }

        // 5. SCStream 组装 + 双输出队列（视频/音频互不阻塞）。
        let video_queue = dispatch2::DispatchQueue::new(
            "com.hawksnap.record.video",
            dispatch2::DispatchQueueAttr::SERIAL,
        );
        let audio_queue = dispatch2::DispatchQueue::new(
            "com.hawksnap.record.audio",
            dispatch2::DispatchQueueAttr::SERIAL,
        );
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &config,
                Some(ProtocolObject::from_ref(&*output)),
            )
        };
        unsafe {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*output),
                    SCStreamOutputType::Screen,
                    Some(&*video_queue),
                )
                .map_err(|e| format!("注册视频输出失败: {e}"))?;
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*output),
                    SCStreamOutputType::Audio,
                    Some(&*audio_queue),
                )
                .map_err(|e| format!("注册音频输出失败: {e}"))?;
        }

        // 6. startCapture（completion 同步等待）。
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let block = block2::RcBlock::new(move |err: *mut NSError| {
            let _ = tx.send(if err.is_null() {
                Ok(())
            } else {
                // SAFETY: 非空即系统投递的合法 NSError。
                Err(format!("startCapture 失败: {}", unsafe { &*err }))
            });
        });
        unsafe { stream.startCaptureWithCompletionHandler(Some(&block)) };
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "startCapture 完成回调超时".to_string())??;

        // 全部就绪 → 装载会话（帧回调自此有完整上下文）。
        *shared.state.lock().unwrap_or_else(|e| e.into_inner()) = Some(RecSession {
            clock: PauseClock::new(),
            video_gate: StreamGate::new(),
            audio_gate: StreamGate::new(),
            session_started: false,
            writer: Some(writer),
            adaptor: Some(adaptor),
            audio_input: Some(audio_input),
            stream: Some(stream),
            video_queue: Some(video_queue),
            audio_queue: Some(audio_queue),
            error: None,
            info: RecordingInfo {
                width: w as u32,
                height: h as u32,
                has_audio: true,
            },
            output_path,
        });

        Ok(ScreenRecorder {
            shared,
            _output: output,
        })
    }

    /// 暂停：回调丢帧。幂等（重复暂停无害）。
    pub fn pause(&self) {
        self.shared
            .phase
            .store(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(s) = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            s.clock.on_pause();
        }
    }

    /// 恢复：时间戳平移在下一帧兑现。幂等。
    pub fn resume(&self) {
        if let Some(s) = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            s.clock.on_resume();
        }
        self.shared
            .phase
            .store(0, std::sync::atomic::Ordering::SeqCst);
    }

    /// 录屏元信息（帧尺寸等；stop 前后均可读）。
    pub fn info(&self) -> Option<RecordingInfo> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.info)
    }

    /// 管线错误（SCStream 自发终止等）；None = 运行正常。
    pub fn take_error(&self) -> Option<String> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .and_then(|s| s.error.take())
    }

    /// 收尾：stopCapture → finishWriting（阻塞等待，≤15s）。返回录屏元信息；
    /// 错误时由调用方清理半成品文件（错误消息含原因）。
    pub fn stop(&self) -> Result<RecordingInfo, String> {
        self.shared
            .phase
            .store(2, std::sync::atomic::Ordering::SeqCst);
        let mut session = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut s) = session.take() else {
            return Err("录屏会话不存在（重复停止）".into());
        };

        // 1. 停流（completion 同步收口；此间残余回调因 phase=2 丢帧）。
        if let Some(stream) = s.stream.take() {
            let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
            let block = block2::RcBlock::new(move |err: *mut NSError| {
                let _ = tx.send(if err.is_null() {
                    Ok(())
                } else {
                    Err(format!("stopCapture 失败: {}", unsafe { &*err }))
                });
            });
            unsafe { stream.stopCaptureWithCompletionHandler(Some(&block)) };
            let _ = rx.recv_timeout(std::time::Duration::from_secs(10)); // 超时仍继续收尾
        }
        // 2. 撤输出队列（停流完成后残余回调已排空；队列对象随会话释放）。
        drop(s.video_queue.take());
        drop(s.audio_queue.take());

        // 3. 无帧保护：会话从未落位（首帧未到）→ cancel + 删半成品 + 报错。
        if !s.session_started {
            if let Some(writer) = s.writer.take() {
                unsafe { writer.cancelWriting() };
            }
            let _ = std::fs::remove_file(&s.output_path);
            return Err("录屏未捕获到任何画面（首帧未到）".into());
        }

        // 4. finishWriting（阻塞等 mux 落盘）。
        let writer = s.writer.take().ok_or("AVAssetWriter 缺失")?;
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let block = block2::RcBlock::new(move || {
            let _ = tx.send(());
        });
        unsafe { writer.finishWritingWithCompletionHandler(&block) };
        if rx.recv_timeout(std::time::Duration::from_secs(15)).is_err() {
            return Err("finishWriting 超时（15s）".into());
        }

        let status = unsafe { writer.status() };
        if status != AVAssetWriterStatus::Completed {
            let detail = unsafe { writer.error() }
                .map(|e| e.to_string())
                .unwrap_or_else(|| "未知错误".into());
            return Err(format!("MP4 收尾失败: {detail}"));
        }
        if let Some(err) = s.error.take() {
            return Err(format!("录屏管线曾报错（文件已生成，请核查）: {err}"));
        }
        Ok(s.info)
    }
}

// —— macOS 侧构造辅助 ——

/// getShareableContent 同步收口（completion handler → channel）。
fn get_shareable_content() -> Result<Retained<SCShareableContent>, String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<Retained<SCShareableContent>, String>>();
    let block = block2::RcBlock::new(
        move |content: *mut SCShareableContent, err: *mut NSError| {
            let res = if err.is_null() {
                // SAFETY: err 为空即系统交付的合法 +1 引用。
                match unsafe { Retained::retain(content) } {
                    Some(c) => Ok(c),
                    None => Err("getShareableContent 返回空内容".to_string()),
                }
            } else {
                // SAFETY: 非空即系统投递的合法 NSError。
                Err(format!("getShareableContent 失败: {}", unsafe { &*err }))
            };
            let _ = tx.send(res);
        },
    );
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            false, true, &block,
        );
    }
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| "getShareableContent 回调超时".to_string())?
}

type SettingsValue = Retained<objc2::runtime::AnyObject>;

fn string_const(s: Option<&'static NSString>) -> SettingsValue {
    // SAFETY: 常量为系统初始化的非空静态；NSString → AnyObject 向上转型静态成立。
    let s = s.expect("AVFoundation 常量缺失");
    unsafe { Retained::cast_unchecked::<objc2::runtime::AnyObject>(s.to_owned()) }
}

fn number_value(n: i64) -> SettingsValue {
    // SAFETY: 同上（NSNumber → AnyObject）。
    unsafe { Retained::cast_unchecked::<objc2::runtime::AnyObject>(NSNumber::numberWithLongLong(n)) }
}

fn settings_dict(
    entries: &[(&str, SettingsValue)],
) -> Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> {
    let keys: Vec<Retained<NSString>> = entries.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| k.as_ref()).collect();
    let value_refs: Vec<&objc2::runtime::AnyObject> =
        entries.iter().map(|(_, v)| &**v).collect();
    NSDictionary::from_slices(&key_refs, &value_refs)
}

// ---------------------------------------------------------------------------
// 测试：纯逻辑（跨平台）+ 真实录制集成（#[ignore]，需屏幕录制权限，开发机直跑）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // —— PauseClock：时间戳平移数学 ——

    #[test]
    fn clock_passes_through_without_pause() {
        let mut c = PauseClock::new();
        assert_eq!(c.on_delivered(100.0), 100.0);
        assert_eq!(c.on_delivered(100.033), 100.033);
        assert_eq!(c.on_delivered(100.066), 100.066);
    }

    #[test]
    fn clock_shifts_by_pause_gap() {
        // 暂停前 10.0/10.033 → 暂停 2s → 恢复后首帧 12.05（含管线延迟 50ms）：
        // 平移量 = 12.05 - 10.033 = 2.017 → 首帧平移后 = 10.033（恰在暂停前末帧
        // 位置，由 gate 丢弃），下一帧 12.083 → 10.066 = 暂停前节奏，时间线无空洞。
        let mut c = PauseClock::new();
        c.on_delivered(10.0);
        c.on_delivered(10.033);
        c.on_pause();
        c.on_resume();
        assert!((c.on_delivered(12.05) - 10.033).abs() < 1e-9);
        assert!((c.on_delivered(12.083) - 10.066).abs() < 1e-9);
        assert!((c.on_delivered(12.116) - 10.099).abs() < 1e-9);
    }

    #[test]
    fn clock_accumulates_across_multiple_pauses() {
        let mut c = PauseClock::new();
        c.on_delivered(5.0);
        c.on_pause();
        c.on_resume();
        c.on_delivered(6.0); // +1.0 → shifted 5.0
        c.on_delivered(6.033); // 5.033
        c.on_pause();
        c.on_resume();
        c.on_delivered(9.0); // += 9.0-6.033 → 累计 3.967 → shifted 5.033
        assert!((c.on_delivered(9.033) - 5.066).abs() < 1e-9);
    }

    #[test]
    fn clock_no_negative_shift_on_clock_regression() {
        // 防御：恢复后首帧 PTS 早于暂停前末帧（理论不可达）→ 平移量不倒退。
        let mut c = PauseClock::new();
        c.on_delivered(10.0);
        c.on_pause();
        c.on_resume();
        c.on_delivered(9.5); // 负差值 → 不计入
        assert_eq!(c.on_delivered(9.6), 9.6);
    }

    #[test]
    fn clock_resume_without_prior_frames() {
        // 首帧即恢复后交付（start 前 pause 无意义但幂等路径可达）→ 平移 0。
        let mut c = PauseClock::new();
        c.on_pause();
        c.on_resume();
        assert_eq!(c.on_delivered(3.0), 3.0);
    }

    // —— StreamGate：单调门 ——

    #[test]
    fn gate_accepts_increasing_and_drops_duplicate() {
        let mut g = StreamGate::new();
        assert_eq!(g.accept(10.0), Some(10.0));
        assert_eq!(g.accept(10.033), Some(10.033));
        // 恢复后首帧平移到暂停前末帧位置 = 重复 → 丢
        assert_eq!(g.accept(10.033), None);
        // 回退 → 丢
        assert_eq!(g.accept(10.020), None);
        assert_eq!(g.accept(10.066), Some(10.066));
    }

    #[test]
    fn gate_epsilon_tolerates_sub_microsecond_drift() {
        let mut g = StreamGate::new();
        assert_eq!(g.accept(1.0), Some(1.0));
        assert_eq!(g.accept(1.0 + 5e-7), None); // 0.5µs 内视为同帧
        assert_eq!(g.accept(1.0 + 1e-5), Some(1.0 + 1e-5));
    }

    // —— 输出路径格式 ——

    #[test]
    fn output_path_format_and_dir_creation() {
        let base = std::env::temp_dir().join(format!("hawksnap-m6-{}", std::process::id()));
        let now = Local.with_ymd_and_hms(2026, 10, 23, 9, 5, 7).unwrap();
        let p = recording_output_path(&base, now).unwrap();
        assert_eq!(
            p.file_name().unwrap().to_str().unwrap(),
            "2026-10-23-090507.mp4"
        );
        assert!(base.is_dir(), "目录应随路径创建");
        let _ = std::fs::remove_dir_all(&base);
    }

    // —— MP4 mvhd 时长解析 ——

    #[test]
    fn mp4_duration_parses_version0_mvhd() {
        // 手工最小 moov/mvhd：version 0，timescale=600，duration=2400 → 4.0s
        let mut b: Vec<u8> = vec![0, 0, 0, 0]; // 占位头部（ftyp 可缺席——解析只找 mvhd）
        b.extend_from_slice(b"ftypisom");
        b.extend_from_slice(&[0, 0, 0, 108]); // box size（任意，解析不看）
        b.extend_from_slice(b"moov");
        b.extend_from_slice(&[0, 0, 0, 100]);
        b.extend_from_slice(b"mvhd");
        b.extend_from_slice(&[0, 0, 0, 0]); // version 0 + flags
        b.extend_from_slice(&[0; 8]); // ctime + mtime
        b.extend_from_slice(&600u32.to_be_bytes()); // timescale
        b.extend_from_slice(&2400u32.to_be_bytes()); // duration
        b.extend_from_slice(&[0; 80]); // 其余字段（rate/volume/pre_defined...）
        assert!((mp4_duration_secs(&b).unwrap() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn mp4_duration_parses_version1_mvhd() {
        let mut b: Vec<u8> = Vec::new();
        b.extend_from_slice(b"xxmvhd");
        b.extend_from_slice(&[1, 0, 0, 0]); // version 1
        b.extend_from_slice(&[0; 16]); // ctime + mtime (64-bit each)
        b.extend_from_slice(&48000u32.to_be_bytes());
        b.extend_from_slice(&192000u64.to_be_bytes());
        b.extend_from_slice(&[0; 80]);
        assert!((mp4_duration_secs(&b).unwrap() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn mp4_duration_rejects_garbage() {
        assert!(mp4_duration_secs(b"not an mp4 at all").is_none());
        // timescale = 0 → None
        let mut b: Vec<u8> = b"mvhd".to_vec();
        b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // v0 + times
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&100u32.to_be_bytes());
        assert!(mp4_duration_secs(&b).is_none());
    }

    // —— 真实录制集成（#[ignore]，开发机已授予屏幕录制权限时直跑） ——

    /// 录制固定时长 → 断言 MP4 ftyp 魔数 + mvhd 时长 ≈ 录制时长。
    /// HAWKSNAP_RECORD_SECS 控制时长（soak 脚本注入 3600 做 72h 分段）。
    #[test]
    #[ignore = "需要 macOS 屏幕录制权限 + 真实显示器；cargo test -p hawksnap-platform-mac -- --ignored 运行"]
    fn real_recording_produces_valid_mp4() {
        let secs: u64 = std::env::var("HAWKSNAP_RECORD_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);
        let dir = std::env::temp_dir().join(format!("hawksnap-m6-real-{}", std::process::id()));
        let now = Local::now();
        let path = recording_output_path(&dir, now).unwrap();
        let rec = ScreenRecorder::start(path.clone()).expect("start");
        std::thread::sleep(std::time::Duration::from_secs(secs));
        let info = rec.stop().expect("stop");
        assert!(info.width >= 1 && info.height >= 1);
        assert!(info.has_audio);

        let data = std::fs::read(&path).expect("read output");
        let min_bytes = if secs >= 5 { 50_000 } else { 5_000 };
        assert!(data.len() > min_bytes, "录屏应 >{min_bytes} 字节，实际 {}", data.len());
        // MP4 ftyp 魔数（offset 4..8）
        assert_eq!(&data[4..8], b"ftyp", "应为 MP4 容器");
        // mvhd 时长 ≈ 录制时长（±1.5s：首帧落位 + mux 收尾的固有偏移）
        let dur = mp4_duration_secs(&data).expect("mvhd 解析");
        println!("[m6] real recording: {} bytes, duration={dur:.3}s (target {secs}s)", data.len());
        assert!(
            (dur - secs as f64).abs() <= 1.5,
            "时长 {dur}s 应 ≈ {secs}s ±1.5"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 暂停/恢复时间线检查：录 2s → 暂停 2s → 恢复 2s → 停止，mvhd 时长应 ≈4s
    /// （暂停平移生效、无空洞）——任务验收口径 ±300ms，此处放 ±0.6s 容忍 CI 波动。
    #[test]
    #[ignore = "需要 macOS 屏幕录制权限 + 真实显示器；cargo test -p hawksnap-platform-mac -- --ignored 运行"]
    fn real_pause_resume_timeline_gapless() {
        let dir = std::env::temp_dir().join(format!("hawksnap-m6-pause-{}", std::process::id()));
        let path = recording_output_path(&dir, Local::now()).unwrap();
        let rec = ScreenRecorder::start(path.clone()).expect("start");
        std::thread::sleep(std::time::Duration::from_secs(2));
        rec.pause();
        std::thread::sleep(std::time::Duration::from_secs(2));
        rec.resume();
        std::thread::sleep(std::time::Duration::from_secs(2));
        rec.stop().expect("stop");

        let dur = mp4_duration_secs_at(&path).expect("mvhd 解析");
        println!("[m6] pause/resume: duration={dur:.3}s (target ≈4s, 2s pause excluded)");
        assert!(
            (dur - 4.0).abs() <= 0.6,
            "暂停 2s 不应计入时长：期望 ≈4s，实际 {dur}s"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// G5 soak 驱动段：HAWKSNAP_RECORD_SECS（默认 5，soak 注入 3600）单段录制 +
    /// 自采样 RSS/CPU → CSV 追加（脚本见 scripts/soak-record.sh）。同进程跑满全程
    /// （72h = 本测试单次运行 72h），内存增长 <10% 由脚本对 CSV 断言。
    #[test]
    #[ignore = "soak 专用；scripts/soak-record.sh 驱动"]
    fn soak_segment_records_and_samples() {
        use std::io::Write;
        let secs: u64 = std::env::var("HAWKSNAP_RECORD_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);
        let dir = std::env::temp_dir().join(format!("hawksnap-m6-soak-{}", std::process::id()));
        let path = recording_output_path(&dir, Local::now()).unwrap();
        let rec = ScreenRecorder::start(path).expect("start");

        let csv = std::env::var("HAWKSNAP_SOAK_CSV").unwrap_or_default();
        let pid = std::process::id();
        // 每小时采样一次（短模式按 secs/8 采样，保证短跑也有 2-3 行）。
        let sample_every = std::env::var("HAWKSNAP_SOAK_SAMPLE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600)
            .max(1);
        let mut elapsed: u64 = 0;
        while elapsed < secs {
            let step = sample_every.min(secs - elapsed);
            std::thread::sleep(std::time::Duration::from_secs(step));
            elapsed += step;
            if csv.is_empty() {
                continue;
            }
            let out = std::process::Command::new("ps")
                .args(["-o", "rss=,%cpu=", "-p", &pid.to_string()])
                .output()
                .ok();
            if let Some(o) = out {
                // ps 输出按空白分列：rss_kb、cpu_percent → 显式拼 CSV（表头由
                // soak-record.sh 写入：time,pid,rss_kb,cpu_percent）。
                let line = String::from_utf8_lossy(&o.stdout).into_owned();
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 2 {
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&csv)
                    {
                        let _ = writeln!(
                            f,
                            "{},{},{},{}",
                            chrono::Local::now().to_rfc3339(),
                            pid,
                            cols[0],
                            cols[1]
                        );
                    }
                }
            }
        }
        rec.stop().expect("stop");
        // HAWKSNAP_SOAK_KEEP=1：保留产物（验收期 mdls/QuickTime 人工复核用）。
        if std::env::var("HAWKSNAP_SOAK_KEEP").is_err() {
            let _ = std::fs::remove_dir_all(&dir);
        } else {
            println!("[m6] soak 产物保留在 {dir:?}");
        }
    }
}
