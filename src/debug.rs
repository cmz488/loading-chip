//! 调试模块 — GDB MI 客户端 + 调试会话状态
//!
//! 通过 GDB Machine Interface (MI) 协议与 arm-none-eabi-gdb 通信，
//! 管理断点、调用栈、变量、监视表达式等调试状态。
//!
//! 本模块处于活跃开发中，部分组件尚未集成到 TUI。

#![allow(dead_code, unused_imports)]

pub mod gdb_mi;
pub mod protocol;
pub mod rtt;
pub mod session;

pub use gdb_mi::{GdbConfig, GdbMi};
pub use protocol::MiRecord;
pub use session::DebugSession;
