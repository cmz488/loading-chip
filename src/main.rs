//! loading-chip 🔥 — 嵌入式芯片烧录/调试 TUI 工具
//!
//! 用法见 `lib.rs` 文档注释。

use std::io;

fn main() -> io::Result<()> {
    loading_chip::run()
}

