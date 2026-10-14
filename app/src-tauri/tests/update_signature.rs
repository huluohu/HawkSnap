//! G7（plan D3）：伪签名集成测试——坏签名必须 Err 且零安装。
//!
//! 证明链（按 tauri-plugin-updater 2.13.x 实证语义）：
//!   1. `check()` 只做公告获取 + 版本比较，不验签 → 从本地静态源拿到
//!      Ok(Some(update))，版本/当前版本/notes 字段如实回传；
//!   2. `download_and_install()` 下载后、安装前做 Ed25519（minisign）验签——
//!      服务端公告（latest.json）本身不签名，攻击者可篡改公告指向任意字节包，
//!      签名是最后一道闸：篡改包 + 合法签名 → Minisign InvalidSignature，
//!      Display 为 "The signature verification failed"（含 "signature"）→ Err；
//!   3. 验签失败发生在 install 之前 → 安装目标目录零写入（nothing installs），
//!      即「进程仍运行旧版本」的进程内语义。
//!
//! fixtures（tests/fixtures/，提交入库；均为 tauri signer 的原生产物形态——
//! 单行 base64，内容为 minisign 文本）：
//!   - `update-test-key.pub`：测试专用公钥（base64("untrusted comment: …\nRWs…" 两行
//!     文本)）。私钥从不入库——生成期仅存在于 /tmp，仅用于产出 .sig fixture。
//!   - `hawk-update.tar.gz`：真 gzip 的假更新包（内容永不被解压——验签先拒）。
//!   - `hawk-update.tar.gz.sig`：用上述测试私钥对【原始包】的合法签名（即 latest.json
//!     signature 字段原文）。
//! 测试运行时把服务的字节翻转一位（模拟被篡改的更新包/被劫持的分发通道），
//! latest.json 内存生成（签名与公告字段均"合法"，仅字节对不上）。
//!
//! 插件接线：`UpdaterExt::updater_builder()` 是唯一公开构造路径（`UpdaterBuilder::new`
//! 为 pub(crate)），需要插件状态挂在 App 上——用 tauri "test" feature 的 mock app
//! （无窗口、无事件循环；插件 setup 只 manage 状态，mock 下照常运行）。endpoint 与
//! pubkey 在 builder 上逐实例覆盖，测试完全不依赖 tauri.conf.json 的占位配置。

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use tauri_plugin_updater::UpdaterExt;
use tiny_http::{Header, Response, Server};

/// 每请求读超时：测试若因接线错误收不到预期请求，fail fast 而非挂死 CI。
const SERVER_TIMEOUT: Duration = Duration::from_secs(10);

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(name))
        .unwrap_or_else(|e| panic!("fixture {name} 缺失（应随仓库提交）: {e}"))
}

fn read_fixture_text(name: &str) -> String {
    String::from_utf8(read_fixture(name)).unwrap_or_else(|e| panic!("fixture {name} 不是 UTF-8: {e}"))
}

/// 起本地静态源：GET /latest.json → 内存生成的静态清单（指向同源篡改包），
/// GET /hawk-update.tar.gz → 篡改后的包字节。返回 endpoint URL 与服务线程。
fn serve_channel() -> (std::thread::JoinHandle<()>, String) {
    let server = Server::http("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("IP listener").port();

    // 公告：version 必须大于 mock app 的包版本 0.1.0，否则 comparator 判非更新。
    // signature 用 fixture 原文（对原始包的合法签名）；url 指向被篡改的字节——
    // 复刻真实攻击面：分发通道被劫持，签名与字节不再对应。
    let sig = read_fixture_text("hawk-update.tar.gz.sig");
    let mut artifact = read_fixture("hawk-update.tar.gz");
    let last = artifact.len() - 1;
    artifact[last] ^= 0xff; // 篡改：任何一位翻转都使 Ed25519 验签失败
    let artifact_url = format!("http://127.0.0.1:{port}/hawk-update.tar.gz");
    let target = tauri_plugin_updater::target().expect("当前平台可表达为 updater target");
    let latest = serde_json::json!({
        "version": "99.0.0",
        "notes": "fake release for G7 (never installed)",
        "pub_date": "2026-10-14T00:00:00Z",
        "platforms": {
            target: { "signature": sig, "url": artifact_url },
        },
    })
    .to_string();

    let handle = std::thread::spawn(move || loop {
        // recv_timeout 返回 Result<Option<Request>>：超时/错误 = 测试早已在断言处
        // 失败，服务线程静默退出（进程退出时一并回收）。
        let Ok(Some(request)) = server.recv_timeout(SERVER_TIMEOUT) else {
            return;
        };
        let path = request.url().split('?').next().unwrap_or("").to_string();
        let body: Vec<u8> = match path.as_str() {
            "/latest.json" => latest.clone().into_bytes(),
            "/hawk-update.tar.gz" => artifact.clone(),
            _ => Vec::new(),
        };
        if body.is_empty() {
            let _ = request.respond(Response::empty(404));
            continue;
        }
        let content_type = if path == "/latest.json" {
            "application/json"
        } else {
            "application/octet-stream"
        };
        let response = Response::from_data(body).with_header(
            Header::from_bytes("Content-Type", content_type).expect("静态 header"),
        );
        let _ = request.respond(response);
    });
    let endpoint = format!("http://127.0.0.1:{port}/latest.json");
    (handle, endpoint)
}

/// mock app + updater 插件：`updater_builder()` 需要插件 setup 已 manage 状态。
/// 上下文用本 crate 的真 tauri.conf.json（占位公钥 REPLACE_AT_RELEASE 是普通
/// String，反序列化/插件初始化照常——顺带证明占位配置不会让插件注册失败）；
/// endpoint/pubkey 在 builder 上逐实例覆盖为本地测试值，不外联。运行时用
/// tauri::test 的 MockRuntime（无窗口/事件循环；Wry 的 EventLoop 禁止在 cargo
/// test 的工作线程上创建）。generate_context! 在 debug 下不嵌前端资产，与
/// app_lib 同前提，CI 无 dist 目录可编译。
fn app_with_updater() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(tauri::generate_context!())
        .expect("mock app 构建")
}

#[test]
fn tampered_package_is_rejected_with_signature_error_and_nothing_installs() {
    let (_server_thread, endpoint) = serve_channel();
    let app = app_with_updater();

    // 插件约定的 pubkey 形态：fixture 原文（tauri signer 产物本身即 base64(
    // minisign 文本) 的单行串——插件 verify_signature 先 base64 解码一层再按
    // minisign 文本解析，因此直接喂文件原文，不可再编码一层）。
    let pubkey = read_fixture_text("update-test-key.pub").trim().to_string();

    // 安装目标隔离：executable_path 指向临时 .app 束（不存在），macOS 语义下
    // extract_path 解析为该 .app 目录——若测试翻车真的发生安装，会在检测点现形。
    let install_root = std::env::temp_dir().join(format!(
        "hawksnap-g7-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&install_root).unwrap();
    let fake_exe = install_root.join("HawkSnap.app/Contents/MacOS/HawkSnap");

    let updater = app
        .handle()
        .updater_builder()
        .endpoints(vec![endpoint.parse().expect("endpoint 是合法 URL")])
        .expect("endpoint 设置")
        .pubkey(pubkey)
        .executable_path(&fake_exe)
        .build()
        .expect("updater 构建");

    // ① 公告获取成功：check 不验签，返回 Some(update)。
    let update = tauri::async_runtime::block_on(updater.check())
        .expect("check 对可达通道必须成功")
        .expect("99.0.0 > 0.1.0-alpha.3，必须判定为更新");
    assert_eq!(update.version, "99.0.0");
    assert_eq!(
        update.current_version,
        app.package_info().version.to_string(),
        "当前版本取自包信息"
    );
    assert_eq!(
        update.body.as_deref(),
        Some("fake release for G7 (never installed)")
    );

    // ② 篡改包：下载后验签失败 → Err 且错误含 "signature"。
    let result = tauri::async_runtime::block_on(update.download_and_install(|_, _| {}, || {}));
    let err = result.expect_err("被篡改的更新包必须被签名校验拒绝");
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("signature"),
        "错误信息应指明签名校验失败，实得：{msg}"
    );

    // ③ 零安装：验签先于 install → 安装目标目录从未被创建（旧版本语义不受扰）。
    assert!(
        !install_root.join("HawkSnap.app").exists(),
        "验签失败后不得有任何安装落盘"
    );
    let _ = std::fs::remove_dir_all(&install_root);
}

/// 负向对照：fixture 自身（未篡改）签名与公钥在【原字节】上必须验签通过——
/// 证明 ② 的 Err 源于「字节与签名不对应」这一被测语义，而非 fixture 本身坏掉
/// （否则上面的 Err 可能是钥匙格式错误之类的假阳性）。
#[test]
fn fixture_signature_is_genuine_over_original_bytes() {
    // 最小复刻插件 verify_signature 的解码序：两个 fixture 都是 base64(minisign
    // 文本) 的单行串 → 先 base64 解一层，再按 minisign 两行/四行文本解析。
    let pubkey_text = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(read_fixture_text("update-test-key.pub").trim())
            .expect(".pub fixture 是合法 base64"),
    )
    .expect("解码后为 minisign 文本");
    let pubkey =
        minisign_verify::PublicKey::decode(&pubkey_text).expect("fixture 公钥是合法 minisign 文本");
    let sig_text = read_fixture_text("hawk-update.tar.gz.sig");
    let sig_b64 = base64::engine::general_purpose::STANDARD
        .decode(sig_text.trim())
        .expect("tauri 约定：signature 字段是 base64(sig 文件全文)");
    let sig_lines = String::from_utf8(sig_b64).expect("解码后为 minisign 文本");
    let signature = minisign_verify::Signature::decode(&sig_lines).expect("合法 minisign 签名");

    let artifact = read_fixture("hawk-update.tar.gz");
    pubkey
        .verify(&artifact, &signature, false)
        .expect("原始 fixture 字节必须能被 fixture 签名验证通过");
}
