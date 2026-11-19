//! 错误码化 re-export（2026-11-20 下沉）：实现已移至 `hawksnap_core::errcode`
//! ——crates 层（platform-mac record/windowcap、hawksnap-ocr models）的用户可见
//! 错误串同样码化，需要共享哨兵构造器。app 层调用点（`crate::errcode::…`）不动。
pub use hawksnap_core::errcode;
