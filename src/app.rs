//! 应用状态模块 — TUI / CLI / Headless 共享状态
//!
//! `AppState` 持有板子注册表、烧录状态等，
//! 通过 `Arc` 在多处共享。

pub mod state;
