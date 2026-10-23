//! 模型与 ORT dylib 管理（M2 Task M4）：
//!
//! - 落点：调用方传入的模型目录（app 侧为 `app_data_dir()/models/`）。
//! - 下载：官方 release URL（RapidOCR v3.10.0 modelscope 通道 / microsoft onnxruntime
//!   v1.31.0 GitHub release）+ **SHA256 常量校验**（实现者于 2026-10-23 实测，来源与
//!   实测值见 task-4-report.md）。
//! - 断点续传：`.tmp` 半成品 + `Range: bytes={len}-`（服务端 206 → 续写；200 → 重来；
//!   416/异常 → 清空重来）。
//! - 校验失败语义：删除半成品 → 整体重下一次 → 仍不匹配 → Err（绝不落坏文件）。
//! - 进度：`on_progress` 回调（app 层转 `hawk://ocr-model-progress` 事件）。
//!
//! 外联边界（spec）：模型/ORT 二进制下载是 OCR 功能唯一的网络行为，仅由
//! `ocr_recognize` 首次触发，全部字节经 SHA256 校验后才参与推理。

use sha2::{Digest, Sha256};

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

// —— ORT dylib（microsoft/onnxruntime v1.31.0 官方 release；版本与 ort 2.0.0-rc.15
// —— 的 ort-sys 绑定目标 1.31.0 对齐）。归档/内件 SHA256 实测于 2026-10-23。 ——

/// 当前平台的 ORT 归档规格（档案 SHA256 + 归档内库路径 + 解包后库 SHA256）。
pub struct OrtSpec {
    pub url: &'static str,
    pub archive_name: &'static str,
    pub archive_sha256: &'static str,
    /// 归档内的库文件相对路径（tar 解包成员）。
    pub inner_path: &'static str,
    /// 解包落盘的文件名（models 目录内）。
    pub dylib_name: &'static str,
    /// 解包后库文件自身的 SHA256（二次校验：档案对 ≠ 内件坏）。
    pub dylib_sha256: &'static str,
    /// true = zip（bsdtar `tar -xf`）；false = tgz。
    pub is_zip: bool,
}

/// 按编译目标平台返回 ORT 归档规格。上游 v1.31.0 未发布 macOS x64 包 → 明确报错
/// （不做无校验的降级下载）。未覆盖目标同样显式 Err。
pub fn ort_spec() -> Result<OrtSpec, String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Ok(OrtSpec {
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/onnxruntime-osx-arm64-1.31.0.tgz",
            archive_name: "onnxruntime-osx-arm64-1.31.0.tgz",
            archive_sha256: "1031b36dd3bdaa8d976a6130d9cf2a05e9be320b93ae804f8427b91dc3432afc",
            inner_path: "onnxruntime-osx-arm64-1.31.0/lib/libonnxruntime.1.31.0.dylib",
            dylib_name: "libonnxruntime.1.31.0.dylib",
            dylib_sha256: "2a50a63e7f41f8d4d551d3ee47248df3864f6540109fce8ad520ca3b51af4b70",
            is_zip: false,
        })
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Err("ONNX Runtime 1.31.0 官方未发布 macOS x64 构建，OCR 暂不支持此平台".into())
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Ok(OrtSpec {
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/onnxruntime-linux-x64-1.31.0.tgz",
            archive_name: "onnxruntime-linux-x64-1.31.0.tgz",
            archive_sha256: "cc5c72baf5ae5c8238a6841f0897227be2d02826b9cf98eaf02fdefaeeb03a57",
            inner_path: "onnxruntime-linux-x64-1.31.0/lib/libonnxruntime.so.1.31.0",
            dylib_name: "libonnxruntime.so.1.31.0",
            dylib_sha256: "469849977cb60744ea160927e1a3a0643fd4c55c0aff743251c6d293402e334e",
            is_zip: false,
        })
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Ok(OrtSpec {
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/onnxruntime-linux-aarch64-1.31.0.tgz",
            archive_name: "onnxruntime-linux-aarch64-1.31.0.tgz",
            archive_sha256: "c5b8b3cca31f3d643a3b313b8f42e2d0f16a4fcefd02cda343d97bd5306afaab",
            inner_path: "onnxruntime-linux-aarch64-1.31.0/lib/libonnxruntime.so.1.31.0",
            dylib_name: "libonnxruntime.so.1.31.0",
            dylib_sha256: "595c7f7a6775f5c7b3d567c3ed94c04f722092575abcc092560f5208a54daf99",
            is_zip: false,
        })
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Ok(OrtSpec {
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/onnxruntime-win-x64-1.31.0.zip",
            archive_name: "onnxruntime-win-x64-1.31.0.zip",
            archive_sha256: "3958d8a44984160692c1b4ffd0de3cb48f8c39f9d34bb31e433db06e30b82195",
            inner_path: "onnxruntime-win-x64-1.31.0/lib/onnxruntime.dll",
            dylib_name: "onnxruntime.dll",
            dylib_sha256: "0c42dc4e278f0968fdf9ebf1b1e54a74c657a9942e70803e453c91993451cf4c",
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
        Err("OCR 暂不支持当前平台".into())
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
    std::fs::create_dir_all(dir).map_err(|e| format!("创建模型目录失败: {e}"))?;
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
        std::fs::create_dir_all(&stage).map_err(|e| format!("创建解包目录失败: {e}"))?;
        let flag = if spec.is_zip { "-xf" } else { "-xzf" };
        let out = std::process::Command::new("tar")
            .arg(flag)
            .arg(&archive)
            .arg("-C")
            .arg(&stage)
            .arg(spec.inner_path)
            .output()
            .map_err(|e| format!("启动 tar 解包失败: {e}"))?;
        let staged = stage.join(spec.inner_path);
        let extract_ok = out.status.success() && staged.is_file();
        if !extract_ok || !file_sha_matches(&staged, spec.dylib_sha256) {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let _ = std::fs::remove_dir_all(&stage);
            if !extract_ok {
                return Err(format!("解包 ONNX Runtime 归档失败: {}", stderr.trim()));
            }
            return Err("ONNX Runtime 库 SHA256 校验失败".into());
        }
        if let Err(e) = std::fs::rename(&staged, &dest) {
            // 跨卷 rename 兜底：copy + 清暂存。
            std::fs::copy(&staged, &dest).map_err(|e2| format!("落盘 ONNX Runtime 库失败: {e}/{e2}"))?;
        }
        let _ = std::fs::remove_dir_all(&stage);
        let _ = std::fs::remove_file(&archive);
        on_progress(Progress { file: 0, phase: ProgressPhase::Verify, downloaded: 1, total: 1 });
        Ok(())
    };

    if let Err(first) = attempt(on_progress) {
        if let Err(second) = attempt(on_progress) {
            return Err(format!("ONNX Runtime 下载失败: {first} / 重试仍失败: {second}"));
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
    std::fs::create_dir_all(dir).map_err(|e| format!("创建模型目录失败: {e}"))?;
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
                return Err(format!("下载 {name} 失败: {first} / 重试仍失败: {second}", name = spec.name));
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
    download_to(&tmp, &spec.url, file_idx, on_progress)?;
    {
        let total = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        on_progress(Progress { file: file_idx, phase: ProgressPhase::Verify, downloaded: total, total });
    }
    let actual = sha256_file(&tmp).ok_or_else(|| "校验失败：临时文件不可读".to_string())?;
    if !actual.eq_ignore_ascii_case(&spec.sha256) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "SHA256 校验失败（{name}）：期望 {want}，实测 {actual}",
            name = spec.name,
            want = spec.sha256
        ));
    }
    std::fs::rename(&tmp, &target).map_err(|e| format!("落盘 {name} 失败: {e}", name = spec.name))?;
    Ok(target)
}

/// HTTP 下载到 `tmp`（断点续传）。流式写盘 + 进度回调；HTTP 错误统一 Err(String)。
fn download_to(
    tmp: &std::path::Path,
    url: &str,
    file_idx: usize,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), String> {
    use std::io::{Read as _, Write as _};

    let offset: u64 = std::fs::metadata(tmp).map(|m| m.len()).unwrap_or(0);
    let agent = ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(20)))
        .build()
        .new_agent();
    let mut req = agent.get(url);
    if offset > 0 {
        req = req.header("Range", &format!("bytes={offset}-"));
    }
    let mut resp = req.call().map_err(|e| format!("HTTP 请求失败: {e}"))?;
    let mut status = resp.status().as_u16();
    match status {
        200 | 206 => {}
        416 => {
            // Range 不可满足（半成品不小于远端）：清空后全量重来。
            std::fs::File::create(tmp).map_err(|e| format!("重建临时文件失败: {e}"))?;
            resp = agent.get(url).call().map_err(|e| format!("HTTP 请求失败: {e}"))?;
            status = resp.status().as_u16();
        }
        s => return Err(format!("HTTP {s}")),
    }
    // 206：追加续写；200/416 重来：截断重写。
    let total_from_header: u64 = resp
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .map(|len: u64| if status == 206 { len + offset } else { len })
        .unwrap_or(0);
    let mut body = resp.into_body();
    let mut reader = body.as_reader();
    let mut file = if status == 206 {
        std::fs::OpenOptions::new().create(true).append(true).open(tmp)
    } else {
        std::fs::File::create(tmp)
    }
    .map_err(|e| format!("打开临时文件失败: {e}"))?;
    let mut buf = [0u8; 64 * 1024];
    let mut written: u64 = if status == 206 { offset } else { 0 };
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("下载中断: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| format!("写入临时文件失败: {e}"))?;
        written += n as u64;
        on_progress(Progress {
            file: file_idx,
            phase: ProgressPhase::Download,
            downloaded: written,
            total: total_from_header.max(written),
        });
    }
    file.flush().map_err(|e| format!("写入临时文件失败: {e}"))?;
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

/// 目标文件存在且 SHA256 匹配。
pub fn file_sha_matches(path: &std::path::Path, sha256: &str) -> bool {
    match sha256_file(path) {
        Some(actual) => actual.eq_ignore_ascii_case(sha256),
        None => false,
    }
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
        assert!(err.contains("SHA256 校验失败"), "got: {err}");
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
}
