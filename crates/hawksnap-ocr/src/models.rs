//! 模型与 ORT dylib 管理（M2 Task M4）：
//!
//! - 落点：调用方传入的模型目录（app 侧为 `app_data_dir()/models/`）。
//! - 下载：官方源（RapidOCR v3.10.0 modelscope 通道 / ONNX Runtime 1.19.2 ——
//!   Linux·Windows 取 PyPI 官方 wheel，macOS 取 NuGet 官方 Microsoft.ML.OnnxRuntime
//!   包；**原 microsoft/onnxruntime GitHub release 源已整体下线，2026-11-20 换源**）
//!   + **SHA256 常量校验**（归档与解包内件双层；实测值见下）。
//! - 断点续传：`.tmp` 半成品 + `Range: bytes={len}-`（服务端 206 → 续写；200 → 重来；
//!   416/异常 → 清空重来）。
//! - 校验失败语义：删除半成品 → 整体重下一次 → 仍不匹配 → Err（绝不落坏文件）。
//! - 进度：`on_progress` 回调（app 层转 `hawk://ocr-model-progress` 事件）。
//!
//! 外联边界（spec）：模型/ORT 二进制下载是 OCR 功能唯一的网络行为，仅由
//! `ocr_recognize` 首次触发，全部字节经 SHA256 校验后才参与推理。

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 下载/校验进度（phase 对齐前端文案：download=下载中 verify=校验中）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressPhase {
    Download,
    Verify,
}

#[derive(Clone, Copy, Debug)]
pub struct Progress {
    /// specs 中的第几个文件（ORT 归档固定 0 号）。
    pub file: usize,
    pub phase: ProgressPhase,
    pub downloaded: u64,
    pub total: u64,
}

/// 单个待保障文件（目标 = `<dir>/<name>`）。
#[derive(Clone, Debug)]
pub struct FileSpec {
    pub name: String,
    pub url: String,
    /// 期望 SHA256（64 位小写 hex）。硬常量：值来自官方 release 实测（报告存档）。
    pub sha256: String,
}

// —— 官方源（ppocrv4 mobile 三件套 + CTC 字典）。RapidAI/RapidOCR v3.10.0 官方发布
// —— 通道（其 default_models.yaml 逐字引用的 URL，SHA256 与 YAML 内置值一致，且本
// —— 实现者另行独立实测复核，见 task-4-report.md）。 ——

pub const DET_URL: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv4/det/ch_PP-OCRv4_det_mobile.onnx";
pub const DET_SHA256: &str = "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9";
pub const CLS_URL: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv4/cls/ch_ppocr_mobile_v2.0_cls_mobile.onnx";
pub const CLS_SHA256: &str = "e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c";
pub const REC_URL: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv4/rec/ch_PP-OCRv4_rec_mobile.onnx";
pub const REC_SHA256: &str = "48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b";
/// CTC 解码字典（ppocr_keys_v1，6623 行；rec 输出 6625 类 = blank + 6623 + space）。
pub const DICT_URL: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/paddle/PP-OCRv4/rec/ch_PP-OCRv4_rec_mobile/ppocr_keys_v1.txt";
pub const DICT_SHA256: &str = "28b2362ad4ab2dc38769aa72feb535e3a9ddb3fd2a7585a05920e6393b1dc7f7";

// —— ORT 动态库（**ONNX Runtime 1.19.2**，2026-11-20 换源）——
//
// **为什么换源（真实供应链故障，端到端验收抓出）**：原规格指向
// `github.com/microsoft/onnxruntime/releases/download/v1.31.0/…`，而该仓库的
// releases **已整体下线**（2026-11-20 实测：所有版本 404、仓库页与 API 均 404，
// 两个独立网络路径一致）——意味着**所有用户首次 OCR 必然失败**（HTTP 404）。
//
// **新源与选型**：
// - Linux / Windows：**PyPI 官方 wheel**（`onnxruntime` 1.19.2）——capi 下是独立
//   的 C API 动态库（与 Python ABI 无关），12/10MB。
// - macOS：**NuGet 官方包** `Microsoft.ML.OnnxRuntime` 1.19.2 ——PyPI 的 macOS
//   wheel 里**没有**独立 dylib（只有 Python 扩展 .so），NuGet 包则按
//   `runtimes/<os>-<arch>/native/` 提供三平台原生库。
// - 版本 **1.19.2** 的理由：① ort-sys 2.0.0-rc.15 的 `ORT_API_VERSION = 17`
//   是**最低**版本要求 → ORT 1.19（API 19）向后兼容可用；② macOS 侧提供
//   `macosx_11_0_universal2`（macOS 11+），与我们声明的 macOS 12.3 下限相容
//   （ORT ≥1.20 的 wheel 只到 macOS 13+）。
// - 双层 SHA256：归档 + 解包内件，均实测于 2026-11-20。
//
// macOS x64 保留（NuGet 内含 osx-x64 库）；非上述目标显式 Err（不做无校验降级）。

/// 当前平台的 ORT 归档规格（档案 SHA256 + 归档内库路径 + 解包后库 SHA256）。
pub struct OrtSpec {
    pub url: &'static str,
    pub archive_name: &'static str,
    pub archive_sha256: &'static str,
    /// 归档内的库文件相对路径（zip/tar 解包成员）。
    pub inner_path: &'static str,
    /// 解包落盘的文件名（models 目录内）。
    pub dylib_name: &'static str,
    /// 解包后库文件自身的 SHA256（二次校验：档案对 ≠ 内件坏）。
    pub dylib_sha256: &'static str,
    /// true = zip（`tar -xf`）；false = tgz。
    pub is_zip: bool,
}

/// 按编译目标平台返回 ORT 归档规格。未覆盖目标显式 Err（不静默降级）。
pub fn ort_spec() -> Result<OrtSpec, String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Ok(OrtSpec {
            url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.19.2/microsoft.ml.onnxruntime.1.19.2.nupkg",
            archive_name: "microsoft.ml.onnxruntime.1.19.2.nupkg",
            archive_sha256: "fa133c8fe6238445a0dd3e8447baa5c7d16f418caa8e576c7e253ba77b27b61b",
            inner_path: "runtimes/osx-arm64/native/libonnxruntime.dylib",
            dylib_name: "libonnxruntime.dylib",
            dylib_sha256: "183dca3132c8f0e2f0a1a30da3bcf7dee634cd6a7d5bf99b47f3d7ffe2791799",
            is_zip: true,
        })
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Ok(OrtSpec {
            url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.19.2/microsoft.ml.onnxruntime.1.19.2.nupkg",
            archive_name: "microsoft.ml.onnxruntime.1.19.2.nupkg",
            archive_sha256: "fa133c8fe6238445a0dd3e8447baa5c7d16f418caa8e576c7e253ba77b27b61b",
            inner_path: "runtimes/osx-x64/native/libonnxruntime.dylib",
            dylib_name: "libonnxruntime.dylib",
            dylib_sha256: "f06335b436437d9efcdbc4ced5f1c27f1bc3ced9ffddf1c1913e7200613ea2c1",
            is_zip: true,
        })
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Ok(OrtSpec {
            url: "https://files.pythonhosted.org/packages/60/ce/be4152da5c1030ab5a159a4a792ed9abad6ba498d79ef0aeba593ff7b5bf/onnxruntime-1.19.2-cp311-cp311-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl",
            archive_name: "onnxruntime-1.19.2-cp311-cp311-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl",
            archive_sha256: "a36511dc07c5c964b916697e42e366fa43c48cdb3d3503578d78cef30417cb84",
            inner_path: "onnxruntime/capi/libonnxruntime.so.1.19.2",
            dylib_name: "libonnxruntime.so.1.19.2",
            dylib_sha256: "4ff01702cba83d4162e87752deb6f09a8bb9297185880e2b54067ef3ffb334f1",
            is_zip: true,
        })
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Ok(OrtSpec {
            url: "https://files.pythonhosted.org/packages/f3/78/e29f5fb76e0f6524f3520e8e5b9d53282784b45d14068c5112db9f712b0a/onnxruntime-1.19.2-cp311-cp311-manylinux_2_27_aarch64.manylinux_2_28_aarch64.whl",
            archive_name: "onnxruntime-1.19.2-cp311-cp311-manylinux_2_27_aarch64.manylinux_2_28_aarch64.whl",
            archive_sha256: "c1dfe4f660a71b31caa81fc298a25f9612815215a47b286236e61d540350d7b6",
            inner_path: "onnxruntime/capi/libonnxruntime.so.1.19.2",
            dylib_name: "libonnxruntime.so.1.19.2",
            dylib_sha256: "4a8b12f35d553899a064ae577dc6d9be44c47a5ba4530ac0cf2e1454e4567afc",
            is_zip: true,
        })
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Ok(OrtSpec {
            url: "https://files.pythonhosted.org/packages/1e/f5/9d995a685f97508b3254f17015b4a78641b0625e79480a7aed7a7a105d7c/onnxruntime-1.19.2-cp311-cp311-win_amd64.whl",
            archive_name: "onnxruntime-1.19.2-cp311-cp311-win_amd64.whl",
            archive_sha256: "1c3e5d415b78337fa0b1b75291e9ea9fb2a4c1f148eb5811e7212fed02cfffa8",
            inner_path: "onnxruntime/capi/onnxruntime.dll",
            dylib_name: "onnxruntime.dll",
            dylib_sha256: "d1b2538368cc16979a6e4506eb01094f17ac3c3e7fe5dfd2aec35409c10b4dc0",
            is_zip: true,
        })
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    {
        Err(hawksnap_core::errcode::coded("ocr_platform_unsupported"))
    }
}

/// 三模型 + 字典规格（固定顺序：det/cls/rec/dict，进度 file 下标 = base + i）。
pub fn model_specs() -> Vec<FileSpec> {
    vec![
        FileSpec { name: "ch_PP-OCRv4_det_mobile.onnx".into(), url: DET_URL.into(), sha256: DET_SHA256.into() },
        FileSpec { name: "ch_ppocr_mobile_v2.0_cls_mobile.onnx".into(), url: CLS_URL.into(), sha256: CLS_SHA256.into() },
        FileSpec { name: "ch_PP-OCRv4_rec_mobile.onnx".into(), url: REC_URL.into(), sha256: REC_SHA256.into() },
        FileSpec { name: "ppocr_keys_v1.txt".into(), url: DICT_URL.into(), sha256: DICT_SHA256.into() },
    ]
}

/// OCR 全量就绪后的产物路径。
#[derive(Clone, Debug)]
pub struct ModelPaths {
    pub det: std::path::PathBuf,
    pub cls: std::path::PathBuf,
    pub rec: std::path::PathBuf,
    pub dict: std::path::PathBuf,
    pub ort_dylib: std::path::PathBuf,
}

/// 确保模型目录就绪（幂等、带 SHA256 门）：ORT dylib（归档下载+解包+内件校验）→
/// 三模型 + 字典。`on_progress` 可为 None（测试/离线复检路径）。
pub fn ensure_models(
    dir: &std::path::Path,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<ModelPaths, String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_models_dir_failed", &[("detail", &e.to_string())]))?;
    let ort = ensure_ort_dylib(dir, on_progress)?;
    let mut paths = ensure_files(dir, &model_specs(), 1, on_progress)?;
    Ok(ModelPaths {
        det: paths.remove(0),
        cls: paths.remove(0),
        rec: paths.remove(0),
        dict: paths.remove(0),
        ort_dylib: ort,
    })
}

/// 保障 ORT dylib：dest 存在且 SHA256 匹配 → 直接过；否则下载归档（SHA256 校验 +
/// 续传）→ 系统 tar 解出内件 → 内件 SHA256 校验 → 落盘 dest → 清理归档与暂存目录。
/// 任一环失败/SHA 不匹配：整体重试一次（删半成品重下），再失败即 Err。
pub fn ensure_ort_dylib(
    dir: &std::path::Path,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<std::path::PathBuf, String> {
    let spec = ort_spec()?;
    let dest = dir.join(spec.dylib_name);
    if file_sha_matches(&dest, spec.dylib_sha256) {
        return Ok(dest);
    }
    // 已存在但坏（或版本不匹配）：删除（只能重下整体归档修复）。
    let _ = std::fs::remove_file(&dest);

    let attempt = |on_progress: &mut dyn FnMut(Progress)| -> Result<(), String> {
        let archive_spec = FileSpec {
            name: spec.archive_name.into(),
            url: spec.url.into(),
            sha256: spec.archive_sha256.into(),
        };
        let archive = download_verified(dir, &archive_spec, 0, on_progress)?;
        // 解包到暂存目录（tar 支持单成员解包），校验内件 SHA256 后落盘。
        let stage = dir.join(format!(".ort-extract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&stage);
        std::fs::create_dir_all(&stage)
            .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_stage_dir_failed", &[("detail", &e.to_string())]))?;
        let flag = if spec.is_zip { "-xf" } else { "-xzf" };
        let out = std::process::Command::new("tar")
            .arg(flag)
            .arg(&archive)
            .arg("-C")
            .arg(&stage)
            .arg(spec.inner_path)
            .output()
            .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_tar_spawn_failed", &[("detail", &e.to_string())]))?;
        let staged = stage.join(spec.inner_path);
        let extract_ok = out.status.success() && staged.is_file();
        if !extract_ok || !file_sha_matches(&staged, spec.dylib_sha256) {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let _ = std::fs::remove_dir_all(&stage);
            if !extract_ok {
                return Err(hawksnap_core::errcode::coded_kv(
                    "ocr_unpack_failed",
                    &[("detail", stderr.trim())],
                ));
            }
            return Err(hawksnap_core::errcode::coded("ocr_ort_checksum_failed"));
        }
        if let Err(e) = std::fs::rename(&staged, &dest) {
            // 跨卷 rename 兜底：copy + 清暂存。
            std::fs::copy(&staged, &dest).map_err(|e2| {
                hawksnap_core::errcode::coded_kv(
                    "ocr_ort_install_failed",
                    &[("detail", &format!("{e}/{e2}"))],
                )
            })?;
        }
        let _ = std::fs::remove_dir_all(&stage);
        let _ = std::fs::remove_file(&archive);
        on_progress(Progress { file: 0, phase: ProgressPhase::Verify, downloaded: 1, total: 1 });
        Ok(())
    };

    if let Err(first) = attempt(on_progress) {
        if let Err(second) = attempt(on_progress) {
            // 内层错误**直接透传**（2026-11-20 CI 实测）：first/second 多已是哨兵码
            //（如 ocr_sha_mismatch）——再包一层 coded_kv 会经 sanitize_kv 剥掉内层
            // 的 < >，前端看到的是剥壳后的原始码文本。首试原因记日志（诊断面），
            // 回执给更具体的内层码。
            eprintln!("[hawksnap] ORT 首试失败（重试后仍败）: {first}");
            return Err(second);
        }
    }
    Ok(dest)
}

/// 保障一组文件（幂等）：存在 + SHA 匹配 → 短路；否则下载校验（续传）。单个文件
/// 校验失败 → 删半成品重下一次 → 仍失败 Err。`file_base` 为进度事件起始下标。
pub fn ensure_files(
    dir: &std::path::Path,
    specs: &[FileSpec],
    file_base: usize,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<Vec<std::path::PathBuf>, String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_models_dir_failed", &[("detail", &e.to_string())]))?;
    let mut out = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        let idx = file_base + i;
        let target = dir.join(&spec.name);
        if file_sha_matches(&target, &spec.sha256) {
            out.push(target);
            continue;
        }
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_file(dir.join(tmp_name(&spec.name)));
        if let Err(first) = download_verified(dir, spec, idx, on_progress) {
            // 重试前清掉续传基线（半成品可能被写坏）。
            let _ = std::fs::remove_file(dir.join(tmp_name(&spec.name)));
            if let Err(second) = download_verified(dir, spec, idx, on_progress) {
                // 内层错误直接透传（同 ensure_ort_dylib 注：嵌套哨兵会被 sanitize
                // 剥壳）；首试原因记日志。
                eprintln!("[hawksnap] 模型 {} 首试失败（重试后仍败）: {first}", spec.name);
                return Err(second);
            }
        }
        out.push(target);
    }
    Ok(out)
}

fn tmp_name(name: &str) -> String {
    format!("{name}.tmp")
}

/// 下载（支持从 `.tmp` 续传）→ SHA256 校验 → 原子 rename 到目标名。
fn download_verified(
    dir: &std::path::Path,
    spec: &FileSpec,
    file_idx: usize,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<std::path::PathBuf, String> {
    let target = dir.join(&spec.name);
    let tmp = dir.join(tmp_name(&spec.name));
    download_to(&tmp, &spec.url, file_idx, DOWNLOAD_STALL_TIMEOUT, on_progress)?;
    {
        let total = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        on_progress(Progress { file: file_idx, phase: ProgressPhase::Verify, downloaded: total, total });
    }
    let actual = sha256_file(&tmp)
        .ok_or_else(|| hawksnap_core::errcode::coded("ocr_tmp_unreadable"))?;
    if !actual.eq_ignore_ascii_case(&spec.sha256) {
        let _ = std::fs::remove_file(&tmp);
        return Err(hawksnap_core::errcode::coded_kv(
            "ocr_sha_mismatch",
            &[("name", &spec.name), ("want", &spec.sha256), ("actual", &actual)],
        ));
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        hawksnap_core::errcode::coded_kv(
            "ocr_model_install_failed",
            &[("name", &spec.name), ("detail", &e.to_string())],
        )
    })?;
    Ok(target)
}

/// 单次读停滞判定窗口：两次 body 读之间超过该时长无任何新字节 → 判定下载停滞
/// （Err）。**选型依据（M7 评审遗留 Important，读 ureq 3.4.2 源码定案）**：ureq 3
/// 没有 stall 型（按次读）超时旋钮——`timeout_recv_body` 是整个 body 的**总时长**
/// 预算（config.rs 文档明示 "The budget is not restarted for each read"），
/// `timeout_global` / `timeout_per_call` 同为总时长；只设 `timeout_connect` 时
/// mid-body 停滞的 socket read 永久阻塞（transport 层读超时由预算推导，无预算即
/// None），下载线程永不返回、`ocr_recognize` 卡死。总时长旋钮对 ~36MB 的 ORT 归档
/// + 慢链路会误杀，故用 pump 线程 + `recv_timeout` 自实现 stall 语义：只要字节
/// 持续到达就永不超时。
const DOWNLOAD_STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// HTTP 下载到 `tmp`（断点续传）。流式写盘 + 进度回调；HTTP 错误统一 Err(String)。
/// `stall` 为读停滞窗口（生产传 [`DOWNLOAD_STALL_TIMEOUT`]，测试可收窄）。
fn download_to(
    tmp: &std::path::Path,
    url: &str,
    file_idx: usize,
    stall: std::time::Duration,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), String> {
    use std::io::{Read as _, Write as _};

    let offset: u64 = std::fs::metadata(tmp).map(|m| m.len()).unwrap_or(0);
    let agent = ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(20)))
        // 响应头限时：堵「连接建立后服务端永不回包」的挂死（60s 对模型 CDN 宽裕）。
        .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
        // 全链路兜底（DNS→body 收尾）30min：涓流服务端的最终止损，同时保证 stall
        // 判定后 pump 线程的阻塞 socket read 会在预算尽头被 ureq 拆除（读超时由
        // 预算推导）——看门狗线程滞留有界。30min 对 36.5MB 归档容纳 ~20KB/s 链路。
        .timeout_global(Some(std::time::Duration::from_secs(30 * 60)))
        .build()
        .new_agent();
    let mut req = agent.get(url);
    if offset > 0 {
        req = req.header("Range", &format!("bytes={offset}-"));
    }
    let mut resp = req
        .call()
        .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_http_failed", &[("detail", &e.to_string())]))?;
    let mut status = resp.status().as_u16();
    match status {
        200 | 206 => {}
        416 => {
            // Range 不可满足（半成品不小于远端）：清空后全量重来。
            std::fs::File::create(tmp)
                .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_tmpfile_create_failed", &[("detail", &e.to_string())]))?;
            resp = agent
                .get(url)
                .call()
                .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_http_failed", &[("detail", &e.to_string())]))?;
            status = resp.status().as_u16();
        }
        s => return Err(hawksnap_core::errcode::coded_kv("ocr_http_status", &[("detail", &s.to_string())])),
    }
    // 206：追加续写；200/416 重来：截断重写。
    let total_from_header: u64 = resp
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .map(|len: u64| if status == 206 { len + offset } else { len })
        .unwrap_or(0);
    // stall 看门狗（M7）：owned reader（`into_reader` 给出 'static、可跨线程——
    // ureq body 文档明示）移入 pump 线程持续读，主循环 `recv_timeout(stall)` 消费。
    // pump 线程被服务端断流卡在 read 时主循环照常超时返回 Err；线程本体最迟在
    // timeout_global 预算尽头随 socket 读超时退出（见上注），不会无限堆积。
    let reader = resp.into_body().into_reader();
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<Vec<u8>>>();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(Ok(Vec::new())); // EOF 哨兵（空块）
                    break;
                }
                Ok(n) => {
                    if tx.send(Ok(buf[..n].to_vec())).is_err() {
                        break; // 主循环已放弃（stall/写盘失败）：接收端已 drop
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        }
    });
    let mut file = if status == 206 {
        std::fs::OpenOptions::new().create(true).append(true).open(tmp)
    } else {
        std::fs::File::create(tmp)
    }
    .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_tmpfile_create_failed", &[("detail", &e.to_string())]))?;
    let mut written: u64 = if status == 206 { offset } else { 0 };
    loop {
        use std::sync::mpsc::RecvTimeoutError;
        let chunk = match rx.recv_timeout(stall) {
            Ok(Ok(c)) if c.is_empty() => break, // EOF
            Ok(Ok(c)) => c,
            Ok(Err(e)) => {
                return Err(hawksnap_core::errcode::coded_kv(
                    "ocr_download_interrupted",
                    &[("detail", &e.to_string())],
                ))
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(hawksnap_core::errcode::coded_kv(
                    "ocr_download_stalled",
                    &[("detail", &stall.as_secs().to_string())],
                ));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(hawksnap_core::errcode::coded("ocr_download_thread_lost"))
            }
        };
        file.write_all(&chunk)
            .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_tmpfile_write_failed", &[("detail", &e.to_string())]))?;
        written += chunk.len() as u64;
        on_progress(Progress {
            file: file_idx,
            phase: ProgressPhase::Download,
            downloaded: written,
            total: total_from_header.max(written),
        });
    }
    file.flush()
        .map_err(|e| hawksnap_core::errcode::coded_kv("ocr_tmpfile_write_failed", &[("detail", &e.to_string())]))?;
    Ok(())
}

/// 文件 SHA256（小写 hex）；不存在/不可读 → None。
pub fn sha256_file(path: &std::path::Path) -> Option<String> {
    use std::io::Read as _;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(format!("{:x}", h.finalize()))
}

/// SHA256 结果缓存（M3 继承项⑤）：key = 路径，value = (mtime, hash)。
///
/// `ensure_models` 在**每次** `ocr_recognize` 调用时复查全部产物（ORT dylib ~40MB +
/// 三模型 + 字典共 5 个文件），全量重哈希实测 ~200-400ms/次（逐调用，M2 终审
/// ledgered debt）；而产物只写一次、此后只读——mtime 未变即直接复用上次哈希，
/// 热路径 ~0ms。HashMap（而非单条目）覆盖全部 5 个工件；文件被替换/触碰 →
/// mtime 变化 → 重算并覆盖条目（失效正确性由 sha_cache_invalidates_on_mtime_change
/// 单测守护：等长改写 + mtime 显式前移，规避 FS 时间粒度）。
static SHA_CACHE: OnceLock<Mutex<HashMap<std::path::PathBuf, (std::time::SystemTime, String)>>> =
    OnceLock::new();

/// 带缓存的文件 SHA256（小写 hex）；不存在/不可读（含无 mtime）→ None。
/// 缓存键 = (路径, mtime)：mtime 相同 → 命中复用；变化/首见 → 全量计算后入缓存。
/// `sha256_file` 保持无缓存原样（下载 `.tmp` 等一次性文件不走缓存，无收益且
/// 避免缓存膨胀）。
pub fn sha256_file_cached(path: &std::path::Path) -> Option<String> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let mut cache = SHA_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((cached_mtime, hash)) = cache.get(path) {
        if *cached_mtime == mtime {
            return Some(hash.clone());
        }
    }
    let hash = sha256_file(path)?;
    cache.insert(path.to_path_buf(), (mtime, hash.clone()));
    Some(hash)
}

/// 目标文件存在且 SHA256 匹配（M3 继承项⑤起走 mtime 缓存——models/dylib 复查
/// 热路径 ~0ms，见 [`SHA_CACHE`] 注）。
pub fn file_sha_matches(path: &std::path::Path, sha256: &str) -> bool {
    sha256_file_cached(path).is_some_and(|actual| actual.eq_ignore_ascii_case(sha256))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hs-ocr-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 本地 HTTP 源（tiny_http）：给定字节，可响应 Range 续传（206）。返回
    /// (base_url, 结束信号发送端)。drop 发送端即停线程。
    fn serve(
        tag: &str,
        body: Vec<u8>,
        range_support: bool,
    ) -> (String, mpsc::Sender<()>, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_string();
        let (tx, rx) = mpsc::channel::<()>();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hits2 = hits.clone();
        std::thread::spawn(move || loop {
            if rx.recv_timeout(std::time::Duration::from_millis(100)).is_ok() {
                break;
            }
            let req = match server.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(Some(r)) => r,
                _ => continue,
            };
            hits2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let range = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Range"))
                .map(|h| h.value.as_str().to_string());
            let (status, payload) = match (range_support, range) {
                (true, Some(r)) if r.starts_with("bytes=") => {
                    let start: u64 = r.trim_start_matches("bytes=")
                        .split('-')
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    (206u16, body[start as usize..].to_vec())
                }
                _ => (200u16, body.clone()),
            };
            let len = payload.len();
            let mut resp = tiny_http::Response::from_data(payload)
                .with_status_code(status);
            resp.add_header(
                tiny_http::Header::from_bytes(&b"Content-Length"[..], len.to_string().as_bytes())
                    .unwrap(),
            );
            let _ = req.respond(resp);
        });
        (format!("http://{addr}/{tag}"), tx, hits)
    }

    fn sha256_hex(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        format!("{:x}", h.finalize())
    }

    /// 新鲜下载：字节原样落盘 + 目标文件存在 + 无 .tmp 残留。
    #[test]
    fn downloads_file_and_verifies_sha() {
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let (url, stop, hits) = serve("fresh", body.clone(), true);
        let dir = temp_dir("fresh");
        let spec = FileSpec { name: "model.bin".into(), url: url.clone(), sha256: sha256_hex(&body) };
        let paths = ensure_files(&dir, &[spec], 0, &mut |_| {}).unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), body, "字节原样落盘");
        assert!(!dir.join("model.bin.tmp").exists(), "无 .tmp 残留");
        assert!(hits.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        stop.send(()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 幂等短路：文件已就绪（SHA 匹配）→ 零网络请求。
    #[test]
    fn ready_file_short_circuits_network() {
        let body = b"stable-bytes".to_vec();
        let (url, stop, hits) = serve("shortcircuit", body.clone(), true);
        let dir = temp_dir("shortcircuit");
        std::fs::write(dir.join("model.bin"), &body).unwrap();
        let spec = FileSpec { name: "model.bin".into(), url: url.clone(), sha256: sha256_hex(&body) };
        ensure_files(&dir, &[spec], 0, &mut |_| {}).unwrap();
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0, "零网络请求");
        stop.send(()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// SHA 不匹配：删除半成品 → 重下一次 → 仍坏 → Err；坏文件绝不落盘。
    #[test]
    fn sha_mismatch_retries_once_then_errors() {
        let good = b"good-bytes-for-the-model".to_vec();
        let bad = b"BAD-bytes-for-the-model".to_vec();
        // 服务端永远发坏字节；spec 期望好字节的 SHA。
        let (url, stop, hits) = serve("mismatch", bad, true);
        let dir = temp_dir("mismatch");
        let spec = FileSpec { name: "model.bin".into(), url: url.clone(), sha256: sha256_hex(&good) };
        let err = ensure_files(&dir, &[spec], 0, &mut |_| {}).unwrap_err();
        assert!(err.contains("<hserr:ocr_sha_mismatch>"), "got: {err}");
        assert!(!dir.join("model.bin").exists(), "坏文件不落盘");
        assert!(!dir.join("model.bin.tmp").exists(), "半成品清理");
        assert!(hits.load(std::sync::atomic::Ordering::SeqCst) >= 2, "重试一次");
        stop.send(()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 断点续传：预置半截 .tmp → 206 续传 → 拼出完整字节且 SHA 通过。
    #[test]
    fn resumes_from_partial_tmp() {
        let body: Vec<u8> = (0..80_000u32).map(|i| (i * 7 % 253) as u8).collect();
        let (url, stop, hits) = serve("resume", body.clone(), true);
        let dir = temp_dir("resume");
        let half = body.len() / 2;
        std::fs::write(dir.join("model.bin.tmp"), &body[..half]).unwrap();
        let spec = FileSpec { name: "model.bin".into(), url: url.clone(), sha256: sha256_hex(&body) };
        ensure_files(&dir, &[spec], 0, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), body, "续传拼出完整字节");
        assert!(hits.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        stop.send(()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 服务端不支持 Range（恒 200）→ 全量重下，结果仍正确。
    #[test]
    fn falls_back_when_range_unsupported() {
        let body = b"no-range-server-payload".to_vec();
        let (url, stop, _) = serve("norange", body.clone(), false);
        let dir = temp_dir("norange");
        std::fs::write(dir.join("model.bin.tmp"), b"stale-partial").unwrap();
        let spec = FileSpec { name: "model.bin".into(), url: url.clone(), sha256: sha256_hex(&body) };
        ensure_files(&dir, &[spec], 0, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), body);
        stop.send(()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 已存在但坏（SHA 不匹配）的目标文件：删除重下修复。
    #[test]
    fn corrupt_target_is_redownloaded() {
        let body = b"intact-model-bytes".to_vec();
        let (url, stop, _) = serve("corrupt", body.clone(), true);
        let dir = temp_dir("corrupt");
        std::fs::write(dir.join("model.bin"), b"corrupted!!").unwrap();
        let spec = FileSpec { name: "model.bin".into(), url: url.clone(), sha256: sha256_hex(&body) };
        ensure_files(&dir, &[spec], 0, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), body);
        stop.send(()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 进度回调：Download 阶段 downloaded 单调 + 最终到 total；随后有 Verify 事件。
    #[test]
    fn progress_events_flow() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 255) as u8).collect();
        let (url, stop, _) = serve("progress", body.clone(), true);
        let dir = temp_dir("progress");
        let spec = FileSpec { name: "model.bin".into(), url: url.clone(), sha256: sha256_hex(&body) };
        let (tx, rx) = mpsc::channel();
        let mut sink = move |p: Progress| {
            let _ = tx.send(p);
        };
        ensure_files(&dir, &[spec], 0, &mut sink).unwrap();
        drop(sink);
        let evs: Vec<Progress> = rx.iter().collect();
        assert!(!evs.is_empty(), "必须发进度");
        let downloads: Vec<&Progress> = evs.iter().filter(|e| e.phase == ProgressPhase::Download).collect();
        assert!(downloads.len() > 1, "多次进度（分块流式）");
        let mut last = 0u64;
        for e in &downloads {
            assert!(e.downloaded >= last, "downloaded 单调");
            last = e.downloaded;
        }
        assert_eq!(downloads.last().unwrap().downloaded as usize, body.len(), "最终到 total");
        assert!(evs.iter().any(|e| e.phase == ProgressPhase::Verify), "有校验事件");
        stop.send(()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// mid-body 停滞看门狗（M7）：响应头 + 首批字节正常送达，随后服务端「断流不断
    /// 连」长时间不发数据 → download_to 必须在 stall 窗口内 Err（配置 1s + 断言上界
    /// 5s），而不是让读线程永久挂死。修复前形态：只设 timeout_connect，socket read
    /// 无限阻塞，ocr_recognize 永不返回。
    ///
    /// fixture 坑（实测）：声明长度必须 < 32KB——tiny_http 对 ≥32KB（chunked_threshold
    /// 默认值）的响应走 chunked 编码，chunked_transfer::Encoder 内部 8KB 缓冲 + io::copy
    /// 每次 8KB 读，首块恰好只够填缓冲、永远不触发 send()，字节滞留到 respond 收尾
    /// 才 flush，客户端看不见「先有数据后断流」，stall 无从观察。identity 编码直写
    /// BufWriter（≥1KB 即穿透），T+0 就有字节到达。
    #[test]
    fn download_stall_times_out() {
        /// 首块立即送达，之后的 read 长睡——正是「中途断流」的服务端形态。
        struct StallingReader {
            first: Vec<u8>,
            delivered: bool,
        }
        impl std::io::Read for StallingReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if !self.delivered {
                    self.delivered = true;
                    let n = self.first.len().min(buf.len());
                    buf[..n].copy_from_slice(&self.first[..n]);
                    return Ok(n);
                }
                std::thread::sleep(std::time::Duration::from_secs(3));
                Ok(0) // 睡醒后 EOF（客户端早已因 stall 放弃，此分支只为收尾线程）
            }
        }

        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_string();
        let (stop, rx) = mpsc::channel::<()>();
        std::thread::spawn(move || loop {
            if rx.recv_timeout(std::time::Duration::from_millis(100)).is_ok() {
                break;
            }
            let req = match server.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(Some(r)) => r,
                _ => continue,
            };
            let first: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();
            // 声明长度 16KB：identity 编码（< 32KB 阈值）且比实际多——客户端必须
            // 继续等更多字节（stall 才可观察）。
            let resp = tiny_http::Response::new(
                tiny_http::StatusCode(200),
                vec![],
                StallingReader { first, delivered: false },
                Some(16 * 1024),
                None,
            );
            let _ = req.respond(resp);
        });

        let dir = temp_dir("stall");
        let tmp = dir.join("model.bin.tmp");
        let start = std::time::Instant::now();
        let err = download_to(
            &tmp,
            &format!("http://{addr}/stall"),
            0,
            std::time::Duration::from_secs(1),
            &mut |_| {},
        )
        .unwrap_err();
        // 错误码化（2026-11-20）：停滞断言走哨兵码（词典 err.ocr_download_stalled 双语）。
        assert!(err.contains("<hserr:ocr_download_stalled>"), "必须是停滞错误，got: {err}");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "必须有界时间内返回，实测 {:?}",
            start.elapsed()
        );
        let _ = stop.send(());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// tar 解包链路：用系统 tar 现场打一个 tgz（含子目录），ensure_ort_dylib 的
    /// 解包半程（extract + 内件 SHA 校验 + 落盘）按同款命令验证可行。
    #[test]
    fn tar_extraction_handles_nested_dylib_path() {
        let dir = temp_dir("tar");
        let payload = b"fake-dylib-bytes".to_vec();
        // 构造 onnxruntime-fake-1.31.0/lib/libonnxruntime.1.31.0.dylib 结构。
        let root = dir.join("onnxruntime-fake-1.31.0/lib");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("libonnxruntime.1.31.0.dylib"), &payload).unwrap();
        let tgz = dir.join("fake.tgz");
        let st = std::process::Command::new("tar")
            .args(["-czf"])
            .arg(&tgz)
            .arg("-C")
            .arg(&dir)
            .arg("onnxruntime-fake-1.31.0")
            .output()
            .unwrap();
        assert!(st.status.success(), "测试环境必须有可用 tar");
        std::fs::remove_dir_all(dir.join("onnxruntime-fake-1.31.0")).unwrap();

        // 与 ensure_ort_dylib 相同的解包半程。
        let stage = dir.join(".stage");
        std::fs::create_dir_all(&stage).unwrap();
        let out = std::process::Command::new("tar")
            .args(["-xzf"])
            .arg(&tgz)
            .arg("-C")
            .arg(&stage)
            .arg("onnxruntime-fake-1.31.0/lib/libonnxruntime.1.31.0.dylib")
            .output()
            .unwrap();
        assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        let staged = stage.join("onnxruntime-fake-1.31.0/lib/libonnxruntime.1.31.0.dylib");
        assert_eq!(std::fs::read(&staged).unwrap(), payload);
        assert!(file_sha_matches(&staged, &sha256_hex(&payload)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ort_spec：本平台必须给出规格或明确平台错误（不允许 panic）。
    #[test]
    fn ort_spec_is_total() {
        let r = ort_spec();
        #[cfg(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
            all(target_os = "windows", target_arch = "x86_64")
        ))]
        assert!(r.is_ok(), "本平台应有规格: {:?}", r.err());
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        assert!(r.is_err(), "macos x64 上游未发布，必须明确报错");
        let _ = r;
    }

    /// 模型规格的 URL 全部 https + SHA 常量格式合法（64 hex）。
    #[test]
    fn model_specs_are_wellformed() {
        for s in model_specs() {
            assert!(s.url.starts_with("https://"), "{}", s.url);
            assert_eq!(s.sha256.len(), 64, "{}", s.name);
            assert!(s.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    /// 防回退哨兵：官方 URL / SHA 常量字面值被改动即红（评审可读性）。
    #[test]
    fn official_constants_unchanged() {
        assert!(DET_URL.contains("modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/"));
        assert_eq!(DET_SHA256, "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9");
        assert_eq!(REC_SHA256, "48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b");
        assert_eq!(CLS_SHA256, "e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c");
        assert_eq!(DICT_SHA256, "28b2362ad4ab2dc38769aa72feb535e3a9ddb3fd2a7585a05920e6393b1dc7f7");
        let _ = std::io::stdout(); // 静默未用导入（Write 在部分 cfg 下仅测试用）
        let _ = std::io::stderr().write_all(b"");
    }

    /// M3 继承项⑤：SHA 缓存命中与失效正确性——mtime 未变 → 复用；mtime 变化
    /// （等长改写 + set_modified 显式前移，规避 FS 时间粒度）→ 必须重算出**新**
    /// 哈希；文件消失 → None（缓存不投毒）。
    #[test]
    fn sha_cache_invalidates_on_mtime_change() {
        let dir = temp_dir("sha-cache");
        let f = dir.join("model.bin");
        std::fs::write(&f, b"cached-content").unwrap();
        // 首算并入缓存：值与直算一致。
        let h1 = sha256_file_cached(&f).unwrap();
        assert_eq!(h1, sha256_hex(b"cached-content"));
        // 命中路径：未触碰文件再取，仍同值（此时走缓存——无独立可观测面，值等价即契约）。
        assert_eq!(sha256_file_cached(&f).unwrap(), h1);
        // 失效路径：等长改写（corrupted-content 与 cached-content 同为 14 字节），
        // mtime 显式前移 10s（若只依赖 FS 自动 mtime，粗粒度文件系统可能同刻）。
        std::fs::write(&f, b"corrupted-content").unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&f).unwrap();
        file.set_modified(
            std::time::SystemTime::now() + std::time::Duration::from_secs(10),
        )
        .unwrap();
        drop(file);
        let h2 = sha256_file_cached(&f).unwrap();
        assert_ne!(h2, h1, "mtime 变化必须重算（缓存不得吞掉文件损坏）");
        assert_eq!(h2, sha256_hex(b"corrupted-content"), "重算结果与直算一致");
        // 文件消失 → None；对不存在路径不缓存、不 panic。
        std::fs::remove_file(&f).unwrap();
        assert!(sha256_file_cached(&f).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// file_sha_matches 走缓存后语义不变：匹配 / 不匹配 / 文件缺失三态。
    #[test]
    fn file_sha_matches_uses_cache_with_same_semantics() {
        let dir = temp_dir("sha-matches");
        let f = dir.join("model.bin");
        std::fs::write(&f, b"stable-bytes").unwrap();
        assert!(file_sha_matches(&f, &sha256_hex(b"stable-bytes")));
        assert!(!file_sha_matches(&f, &sha256_hex(b"other-bytes")));
        assert!(!file_sha_matches(&dir.join("gone.bin"), &sha256_hex(b"x")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
