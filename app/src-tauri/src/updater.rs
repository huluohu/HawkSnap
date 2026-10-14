//! 自动更新（plan D3，spec §3.7）：tauri-plugin-updater 官方插件 + GitHub Releases
//! 静态 latest.json 通道，Ed25519 签名校验由插件内置（minisign）。
//!
//! 命令口径（spec §5：更新检查是唯一新增外联，可配置）：
//! - `check_for_updates(manual)`：settings.update_enabled=false 且非手动 → Ok(None)
//!   静默；manual=true 覆盖开关（手动检查是用户显式动作）。下载不在本命令做（保持
//!   check 快），交给 `install_update`。
//! - `install_update`：重新 check → 下载（进度经 hawk://update-progress 事件）
//!   → 插件验签（Ed25519）→ 安装。验签失败 Err，绝不落盘（G7 集成测试证明）。
//! - `restart_app`：安装完成后前端提示重启，用户点「立即重启」才调（不自动重启）。
//!
//! 占位公钥（REPLACE_AT_RELEASE，D7/发布时由控制器替换为真实公钥）的行为——按插件
//! 源码实证（tauri-plugin-updater 2.13.x）：公钥是纯 String 配置，插件 init 与 check()
//! 均不解析它，只在 download 后的 verify_signature 才解码验签。因此占位符不会让插件
//! 注册失败、不会让应用启动失败（沿 C7 autostart「非致命」先例）；但任何下载都会在
//! 验签一步被拒（公钥 base64 解码失败 → Err，fail-closed）。为不给用户「发现新版本
//! 却装不了」的死路，check_for_updates/install_update 在检测到占位/空公钥时短路：
//! 自动检查静默 Ok(None)，手动检查返回哨兵错误（前端映射为「更新功能未配置」）。

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

use crate::settings::Settings;

/// tauri.conf.json plugins.updater.pubkey 的流程性占位（计划 D3：密钥对生成是控制器
/// 手工步骤，D7 前一直随 conf 发布）。
const PUBKEY_PLACEHOLDER: &str = "REPLACE_AT_RELEASE";

/// 更新通道未配置（占位/空公钥）时的哨兵错误串。Rust 侧文案暂不 i18n（D1 裁定），
/// 但这个 Err 会被前端特判映射为 t("settings.updates.notConfigured")，其余 Err 原样
/// 展示（外面包 banner.updateError）。
pub const ERR_NOT_CONFIGURED: &str = "updater_not_configured";

/// 下载进度事件载荷（hawk://update-progress）。downloaded 为累计字节数（插件回调
/// 给的是单 chunk 长度，install_update 内累计），total 为 Content-Length，可能缺失。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

/// 可安装更新的摘要（check_for_updates 的 Ok(Some) 载荷）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    pub current_version: String,
}

/// 更新通道是否已配置（公钥在盘且非占位）。tauri.conf.json 的 plugins 配置经
/// PluginConfig(pub HashMap<String, JsonValue>) 可读；缺节/缺键/占位都算未配置。
fn channel_configured(app: &AppHandle) -> bool {
    let pubkey = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|v| v.get("pubkey"))
        .and_then(|v| v.as_str());
    match pubkey {
        Some(k) => {
            let k = k.trim();
            !k.is_empty() && k != PUBKEY_PLACEHOLDER
        }
        None => false,
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 检查更新（不下载）。update_enabled=false 且 !manual → Ok(None)；manual=true 覆盖
/// 开关。通道未配置：自动检查静默跳过（log 留痕），手动检查返回哨兵错误给前端映射。
#[tauri::command(async)]
pub async fn check_for_updates(
    app: AppHandle,
    manual: bool,
) -> Result<Option<UpdateInfo>, String> {
    if !manual && !Settings::load(&app).update_enabled {
        return Ok(None);
    }
    if !channel_configured(&app) {
        eprintln!("[hawksnap] 更新通道未配置（公钥占位/缺失），跳过更新检查");
        return if manual {
            Err(ERR_NOT_CONFIGURED.to_string())
        } else {
            Ok(None)
        };
    }
    let update = app
        .updater()
        .map_err(err)?
        .check()
        .await
        .map_err(err)?;
    Ok(update.map(|u| UpdateInfo {
        version: u.version,
        notes: u.body,
        current_version: u.current_version,
    }))
}

/// 下载并安装更新（check → download_and_install）。验签失败 → Err 且零落盘（插件在
/// verify_signature 通过后才动安装目录，G7 集成测试证明）。进度经
/// hawk://update-progress 推送；进度事件按 ≥512KB 步进节流，避免大更新打爆 IPC。
/// 安装成功后不自动重启（macOS/Linux 需 relaunch，前端提示用户点「立即重启」）。
#[tauri::command(async)]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if !channel_configured(&app) {
        return Err(ERR_NOT_CONFIGURED.to_string());
    }
    let update = app
        .updater()
        .map_err(err)?
        .check()
        .await
        .map_err(err)?;
    // 竞态兜底：前端只在「发现新版本」后启用安装按钮，此处 None 说明公告已撤/
    // 已装——按无事发生收场（前端随后回到「已是最新」态），不算错误。
    let Some(update) = update else {
        return Ok(());
    };
    let handle = app.clone();
    let mut downloaded: u64 = 0;
    let mut last_emitted: u64 = 0;
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                // 节流：距上次发射 ≥512KB 才推（或已接近完成由最终状态覆盖）。
                if downloaded - last_emitted >= 512 * 1024 {
                    last_emitted = downloaded;
                    let _ = handle.emit(
                        "hawk://update-progress",
                        UpdateProgress { downloaded, total },
                    );
                }
            },
            || {},
        )
        .await
        .map_err(err)?;
    let _ = app.emit(
        "hawk://update-progress",
        UpdateProgress {
            downloaded,
            total: Some(downloaded), // downloaded == total → 前端渲染 100%
        },
    );
    Ok(())
}

/// 安装完成后由前端「立即重启」按钮调用（AppHandle::restart 不返回，进程即换新）。
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_configured_marker_is_stable() {
        // 前端特判依赖此串（i18n.test.ts 同步断言）。改动 = 双端同改。
        assert_eq!(ERR_NOT_CONFIGURED, "updater_not_configured");
    }
}
