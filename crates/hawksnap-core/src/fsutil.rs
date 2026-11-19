//! 私密文件落盘助手（2026-11-20 安全审计修复）。
//!
//! 截屏/录制缩略图/OCR 导出等产物可能包含高度敏感的屏幕内容。Unix 下统一以
//! **0600（文件）/ 0700（本次新建的目录）** 落盘，防同机其他本地用户、备份或
//! 同步代理读取；非 Unix 平台保持默认写（Windows 用户 profile 目录自身有
//! ACL 兜底，无位概念）。
//!
//! 与 `native_protocol::write_secret_file` 的分工：那一个面向「令牌配置」——
//! 需要同目录 tmp+rename **原子替换**语义；本模块面向「内容产物」——直接写
//! 目标路径（保存侧已有 expand 冲突递增，编辑器覆盖已有自己的 tmp+rename），
//! 只负责权限收紧。

use std::path::Path;

/// 写「仅本人可读」的字节文件。Unix：`mode(0o600)` 建文件 + 写后
/// `set_permissions` 复核——覆盖**已存在**文件时 `mode()` 不生效（OpenOptions
/// 的 mode 只作用于新建），必须显式收紧。非 Unix：plain write。
pub fn write_private_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)?;
        f.sync_all().ok();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

/// `create_dir_all` + 「仅当本次新建」时把目录权限收紧到 0700（Unix）。已存在
/// 的目录**不改**权限——可能是用户自建或与其他产物共用（与
/// `native_protocol::set_dir_mode` 同一纪律）。
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    let existed = dir.exists();
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        if !existed {
            use std::os::unix::fs::PermissionsExt;
            // 收紧失败不致命：文件层已有 0600 兜底。
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = existed;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "hawksnap-fsutil-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn new_file_is_owner_only() {
        let dir = scratch("new");
        let p = dir.join("shot.png");
        write_private_bytes(&p, b"pngbytes").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"pngbytes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overwrite_tightens_existing_loose_file() {
        let dir = scratch("over");
        let p = dir.join("shot.png");
        std::fs::write(&p, b"loose").unwrap(); // umask 默认（一般 0644）
        write_private_bytes(&p, b"tight").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fresh_dir_is_0700_existing_dir_untouched() {
        let base = scratch("dir");
        let fresh = base.join("a/b");
        ensure_private_dir(&fresh).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
            // 已存在目录不改权限：手动放宽到 0755 后再 ensure 应保持 0755。
            std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755)).unwrap();
            ensure_private_dir(&fresh).unwrap();
            let mode2 = std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode2, 0o755);
        }
        std::fs::remove_dir_all(&base).unwrap();
    }
}
