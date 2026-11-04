//! 单实例守卫（Task 13）：temp 目录锁文件 + flock 独占。
//!
//! `File::try_lock`（Rust 1.89+ std API，macOS 实现为 `flock(LOCK_EX | LOCK_NB)`）：
//! flock 挂在 open file description 上，`try_clone` 出的 fd 共享同一把锁，最后一个
//! fd 关闭（含进程退出）才释放。守卫由调用方 `std::mem::forget` 泄漏成 'static，
//! 锁因此覆盖进程全生命周期。

use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};

/// 本进程是否为主实例（拿到 flock 的那个）。第二实例拿不到锁 → 恒 false。
///
/// 用途：退出钩子要清抓帧缓存，但**第二实例也会走到退出钩子**（拿不到锁后
/// `app.exit(0)` → `RunEvent::Exit`）——无差别清空会删掉正在运行的主实例的
/// 当前帧。故清空前先问「我是主实例吗」。
static PRIMARY: AtomicBool = AtomicBool::new(false);

/// 标记本进程为主实例（setup 拿到锁后调用）。
pub fn mark_primary() {
    PRIMARY.store(true, Ordering::SeqCst);
}

/// 本进程是否为主实例。
pub fn is_primary() -> bool {
    PRIMARY.load(Ordering::SeqCst)
}

/// 持锁守卫：持有的 fd 不关闭则锁不释放。进程内由 setup 泄漏，永不 drop。
pub struct LockGuard {
    _file: File,
}

/// 尝试独占锁定 `path`；已被其他实例持有时 Err（含「另一实例正在运行」语义）。
pub fn acquire_lock_at(path: &std::path::Path) -> Result<LockGuard, String> {
    let file = File::create(path).map_err(|e| e.to_string())?;
    file.try_lock()
        .map_err(|e| format!("另一实例正在运行: {e}"))?;
    // 克隆 fd 交给守卫：原始 file 在本函数返回时 drop（关闭其中一个 fd 不影响
    // 共享的 flock），守卫存活期间锁持续有效。
    let _file = file.try_clone().map_err(|e| e.to_string())?;
    Ok(LockGuard { _file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_releasable() {
        let dir = std::env::temp_dir().join(format!("hs-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.lock");
        let g1 = acquire_lock_at(&path).expect("first acquire");
        assert!(acquire_lock_at(&path).is_err(), "second acquire must fail");
        drop(g1);
        assert!(acquire_lock_at(&path).is_ok(), "reacquire after drop");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
