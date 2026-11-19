//! 错误码化 re-export（2026-11-20 下沉）：实现已移至 `hawksnap_core::errcode`
//! ——crates 层（platform-mac record/windowcap、hawksnap-ocr models）的用户可见
//! 错误串同样码化，需要共享哨兵构造器。app 层调用点（`crate::errcode::…`）不动。
//!
//! 注意重导出的是**函数项**而非模块：`pub use hawksnap_core::errcode;`（模块整体）
//! 会在本模块里造出一个 `errcode` 子模块（`crate::errcode::errcode::coded` 才通），
//! `crate::errcode::coded` 反而落空——CI E0432 实测教训。
pub use hawksnap_core::errcode::{coded, coded_kv};
