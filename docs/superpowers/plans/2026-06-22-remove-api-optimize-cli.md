# Remove API Layer & Optimize CLI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the axum-based HTTP REST + WebSocket API layer and its supporting infrastructure from `loading-chip`.

**Architecture:** Delete `src/api/` (10 files), drop 3 external crates, strip API-only fields from `AppState`, remove the global RTT broadcast channel from `debug/rtt.rs`, and remove `--api`/`--api-addr` CLI flags. All existing TUI and CLI functionality is preserved unchanged.

**Tech Stack:** Rust 2024 edition, cargo, tokio (kept at full features), crossbeam-channel, ratatui/crossterm

## Global Constraints

- `tokio` kept at `features = ["full"]`
- TUI behavior, flash logic, board registry, chip detection unchanged
- `debug` feature unchanged
- All existing tests must pass after each task

---

### Task 1: Delete API files and drop HTTP dependencies

**Files:**
- Delete: `src/api.rs`, `src/api/server.rs`, `src/api/routes.rs`, `src/api/routes/board.rs`, `src/api/routes/debug.rs`, `src/api/routes/detect.rs`, `src/api/routes/flash.rs`, `src/api/routes/rtt.rs`, `src/api/routes/status.rs`
- Delete: `src/api/` directory (after removing all files)
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: none (first task, no dependencies)
- Produces: project no longer depends on `axum`, `tower-http`, `futures-util`; `src/api` module no longer exists

- [ ] **Step 1: Delete all API source files**

```bash
rm src/api/routes/board.rs
rm src/api/routes/debug.rs
rm src/api/routes/detect.rs
rm src/api/routes/flash.rs
rm src/api/routes/rtt.rs
rm src/api/routes/status.rs
rm src/api/routes.rs
rm src/api/server.rs
rm src/api.rs
rmdir src/api/routes
rmdir src/api
```

- [ ] **Step 2: Remove HTTP dependencies from Cargo.toml**

**Edit `Cargo.toml`** — remove these 3 lines:

```diff
-axum = { version = "0.8", features = ["ws"] }
-tower-http = { version = "0.6", features = ["cors"] }
-futures-util = "0.3"
```

After removal, the dependencies section should look like:

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
ratatui = "0.29"
crossterm = "0.28"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
tokio = { version = "1", features = ["full"] }
probe-rs = { version = "0.31", optional = true }
crossbeam-channel = "0.5"
nix = { version = "0.30", features = ["signal", "process"] }
shellexpand = "3"
dirs = "6"
object = { version = "0.38", default-features = false, features = ["read_core", "elf"] }
```

- [ ] **Step 3: Build to confirm the missing API module is now the only error**

```bash
cargo build 2>&1 | head -20
```

Expected: errors about `src/lib.rs` referencing `api` module — this is expected and will be fixed in Task 3.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor: remove API files and HTTP dependencies

Delete src/api/ directory (10 files: axum REST + WebSocket server).
Remove axum, tower-http, futures-util from Cargo.toml.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Remove API flags from CLI

**Files:**
- Modify: `src/cli.rs`

**Interfaces:**
- Consumes: none (CLI changes are independent of other cleanup)
- Produces: `Commands::Run` no longer has `api` or `api_addr` fields

- [ ] **Step 1: Remove `api` and `api_addr` fields from `Commands::Run`**

**Edit `src/cli.rs`** — remove lines 57–63:

```diff
         /// 超时时间（秒），默认 60。0 表示无超时
         #[arg(long, default_value = "60", value_name = "秒")]
         timeout: u64,
-
-        /// 启动 REST API + WebSocket 服务（可与 TUI 同时使用）
-        #[arg(long)]
-        api: bool,
-
-        /// API 监听地址（host:port 或 Unix socket 路径）
-        #[arg(long, default_value = "127.0.0.1:9876")]
-        api_addr: String,
     },
```

- [ ] **Step 2: Verify CLI compiles in isolation**

```bash
cargo build 2>&1 | grep -c "cli.rs"
```

Expected: 0 errors from `cli.rs` (any remaining errors will be from `lib.rs` referencing `api` — fixed in Task 3).

- [ ] **Step 3: Commit**

```bash
git add src/cli.rs
git commit -m "refactor: remove --api and --api-addr CLI flags

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Clean up lib.rs dispatch

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: API module deleted (Task 1), CLI flags removed (Task 2)
- Produces: `run_flash()` no longer takes `api`/`api_addr` params; no API spawning logic

- [ ] **Step 1: Remove `api` and `api_addr` from `Commands::Run` destructure**

**Edit `src/lib.rs`** — in `run()`, lines 65–79. Remove the two fields from the destructure:

```diff
         Some(cli::Commands::Run {
             backend,
             interface,
             target,
             elf,
             gdb_port,
             pyocd_path,
             headless,
             timeout,
-            api,
-            api_addr,
         }) => run_flash(
-            state, backend, interface, target, elf, gdb_port, pyocd_path, headless, timeout, api,
-            api_addr,
+            state, backend, interface, target, elf, gdb_port, pyocd_path, headless, timeout,
         )?,
```

- [ ] **Step 2: Update `run_flash()` signature**

**Edit `src/lib.rs`** — lines 175–188. Remove `api: bool` and `api_addr: String` parameters:

```diff
 #[allow(clippy::too_many_arguments)]
 fn run_flash(
     state: Arc<AppState>,
     backend: String,
     interface: Option<String>,
     target: Option<String>,
     elf: Option<String>,
     gdb_port: String,
     pyocd_path: String,
     headless: bool,
     timeout: u64,
-    api: bool,
-    api_addr: String,
 ) -> io::Result<i32> {
```

- [ ] **Step 3: Remove the API spawning block**

**Edit `src/lib.rs`** — remove the `_api_shutdown` block (lines 189–200):

```diff
 ) -> io::Result<i32> {
-    // 启动 API 服务（与 TUI/Headless 共享同一个 AppState）
-    let _api_shutdown = if api {
-        match api::spawn_server((*state).clone(), api_addr.clone()) {
-            Ok(shutdown) => Some(shutdown),
-            Err(e) => {
-                eprintln!("❌ API 启动失败: {}", e);
-                return Ok(1);
-            }
-        }
-    } else {
-        None
-    };
-
     if headless {
```

- [ ] **Step 4: Replace the headless+API idle path with a simple headless guard**

**Edit `src/lib.rs`** — lines 202–219. Replace:

```rust
    if headless {
        if let Some(_shutdown) = _api_shutdown {
            eprintln!("🟢 API 运行中（按 Ctrl+C 退出）...");
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .map_err(io::Error::other)?;
            rt.block_on(async {
                tokio::signal::ctrl_c().await.ok();
                eprintln!("\n正在关闭...");
            });
            return Ok(0);
        } else {
            return run_headless(
                &state, backend, interface, target, elf, gdb_port, pyocd_path, timeout,
            );
        }
    }
```

With:

```rust
    if headless {
        return run_headless(
            &state, backend, interface, target, elf, gdb_port, pyocd_path, timeout,
        );
    }
```

- [ ] **Step 5: Build and verify compilation**

```bash
cargo build 2>&1
```

Expected: compilation succeeds. If there are remaining `api::` references, the compiler will point to them.

- [ ] **Step 6: Run `cargo run -- help` to verify CLI works**

```bash
cargo run -- run --help
```

Expected: help output with no `--api` or `--api-addr` flags.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs
git commit -m "refactor: remove API spawning from lib.rs dispatch

Remove api/api_addr params from run_flash(), delete API server
spawning and headless+API idle-mode code paths.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Clean up AppState

**Files:**
- Modify: `src/app/state.rs`
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: API module deleted (Task 1)
- Produces: `AppState` has 6 fields (was 8); no `broadcast` or `RttClient` imports

- [ ] **Step 1: Update module doc comment in `src/app.rs`**

**Edit `src/app.rs`** — replace the current comment:

```rust
//! 应用状态模块 — axum 共享状态
//!
//! `AppState` 持有板子注册表、烧录状态、RTT 广播器等，
//! 通过 axum 的 `State` extractor 注入到各路由处理器。

pub mod state;
```

With:

```rust
//! 应用状态模块 — TUI / CLI / Headless 共享状态
//!
//! `AppState` 持有板子注册表、烧录状态等，
//! 通过 `Arc` 在多处共享。

pub mod state;
```

- [ ] **Step 2: Update doc comment in `src/app/state.rs`**

**Edit `src/app/state.rs`** — replace lines 1–3:

```diff
-//! 应用共享状态 — TUI / API / Headless 统一接口
+//! 应用共享状态 — TUI / CLI / Headless 统一接口
 //!
-//! `AppState` 是全局单例，所有模式共享同一个实例。
-//! axum 要求 handler 共享的状态实现 `Clone`，因此字段用 `Arc` 包裹。
+//! `AppState` 是全局单例，所有模式共享同一个实例。
+//! 字段用 `Arc` 包裹以支持多模式共享。
```

- [ ] **Step 3: Remove `RttClient` import**

**Edit `src/app/state.rs`** — remove line 9:

```diff
 use crate::backend::{do_flash, FlashBackend, FlashConfig, FlashResult};
 use crate::board::BoardRegistry;
 use crate::chip_detect::DetectedChip;
-use crate::debug::rtt::{RttClient, RttOutput};
 use serde::{Deserialize, Serialize};
 use std::sync::Arc;
 use tokio::sync::{broadcast, Mutex};
```

- [ ] **Step 4: Remove `rtt_tx` and `rtt_session` fields from `AppState`**

**Edit `src/app/state.rs`** — in the struct definition (lines 31–44):

```diff
 #[derive(Clone)]
 pub struct AppState {
     /// 板子注册表（所有模式共享）
     pub registry: Arc<BoardRegistry>,
     /// 芯片检测结果缓存
     pub detected_chips: Arc<Mutex<Vec<DetectedChip>>>,
     pub run_state: Arc<Mutex<RunState>>,
     pub current_board: Arc<Mutex<Option<String>>>,
     pub current_backend: Arc<Mutex<Option<String>>>,
     pub last_result: Arc<Mutex<Option<FlashResult>>>,
-    /// RTT 广播器（TUI / API 共享）
-    pub rtt_tx: broadcast::Sender<RttOutput>,
-    /// 活跃的 RTT 会话（API 模式管理）
-    pub rtt_session: Arc<Mutex<Option<Box<dyn RttClient>>>>,
 }
```

- [ ] **Step 5: Simplify `AppState::new()`**

**Edit `src/app/state.rs`** — in the `new()` method (lines 48–61):

```diff
 impl AppState {
     pub fn new(registry: BoardRegistry) -> Self {
-        let (rtt_tx, _) = broadcast::channel(256);
         Self {
             registry: Arc::new(registry),
             detected_chips: Arc::new(Mutex::new(Vec::new())),
             run_state: Arc::new(Mutex::new(RunState::Idle)),
             current_board: Arc::new(Mutex::new(None)),
             current_backend: Arc::new(Mutex::new(None)),
             last_result: Arc::new(Mutex::new(None)),
-            rtt_tx,
-            rtt_session: Arc::new(Mutex::new(None)),
         }
     }
```

- [ ] **Step 6: Remove `broadcast` from tokio import**

**Edit `src/app/state.rs`** — line 12:

```diff
-use tokio::sync::{broadcast, Mutex};
+use tokio::sync::Mutex;
```

- [ ] **Step 7: Build and verify**

```bash
cargo build 2>&1
```

Expected: compilation succeeds. No remaining references to `rtt_tx`, `rtt_session`, or `RttOutput` in `AppState`.

- [ ] **Step 8: Commit**

```bash
git add src/app/state.rs src/app.rs
git commit -m "refactor: remove API-only fields from AppState

Remove rtt_tx (broadcast channel), rtt_session (RTT client handle),
and RttClient import. These existed only for the HTTP API server.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Remove RTT global broadcast infrastructure

**Files:**
- Modify: `src/debug/rtt.rs`

**Interfaces:**
- Consumes: API module deleted (Task 1), AppState broadcast removed (Task 4)
- Produces: `RttConfig` no longer has `broadcast` field; no `GLOBAL_BROADCAST`; `probe_rs_rtt_loop` only sends to crossbeam channel

- [ ] **Step 1: Remove `GLOBAL_BROADCAST` static and `set_global_broadcast()` function**

**Edit `src/debug/rtt.rs`** — remove lines 18–25:

```diff
-/// 全局 RTT broadcast — TUI 和 API 共享
-/// API 服务启动时设置，TUI RTT 循环自动向此通道发布数据
-static GLOBAL_BROADCAST: OnceLock<tokio::sync::broadcast::Sender<RttOutput>> = OnceLock::new();
-
-/// 设置全局 RTT broadcast 发送端（由 API server 调用）
-pub fn set_global_broadcast(tx: tokio::sync::broadcast::Sender<RttOutput>) {
-    let _ = GLOBAL_BROADCAST.set(tx);
-}
-
 /// RTT 输出记录
 #[derive(Debug, Clone)]
 pub struct RttOutput {
```

- [ ] **Step 2: Remove `OnceLock` import if no longer used**

Check if `OnceLock` is used elsewhere in `rtt.rs`. The current imports (line 12) include:

```rust
use std::sync::{Arc, OnceLock};
```

After removing `GLOBAL_BROADCAST`, `OnceLock` is no longer used. Remove it:

```diff
-use std::sync::{Arc, OnceLock};
+use std::sync::Arc;
```

- [ ] **Step 3: Remove `broadcast` field from `RttConfig`**

**Edit `src/debug/rtt.rs`** — lines 44–53. Remove the `broadcast` field:

```diff
 #[derive(Clone)]
 pub struct RttConfig {
     pub backend: RttBackend,
     pub chip: String,
     pub probe: String,
     pub telnet_port: u16,
     pub elf_path: Option<String>,
-    /// 可选的 broadcast sender — RTT 数据同时发布到此通道（供 API WebSocket 订阅）
-    pub broadcast: Option<tokio::sync::broadcast::Sender<RttOutput>>,
 }
```

- [ ] **Step 4: Remove `broadcast_tx` parameter from `probe_rs_rtt_loop()` and update its caller**

**Edit 1: Update function signature** (`src/debug/rtt.rs`, lines 136–143):

```diff
 fn probe_rs_rtt_loop(
     chip: &str,
     probe_desc: &str,
     running: &AtomicBool,
     sender: &Sender<RttOutput>,
     elf_path: Option<&str>,
-    broadcast_tx: Option<tokio::sync::broadcast::Sender<RttOutput>>,
 ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
```

**Edit 2: Update caller in `ProbeRsRtt::spawn()`** (lines 96–101):

```diff
-            let broadcast = config.broadcast.clone();
-
             let handle = thread::Builder::new()
                 .name("probe-rs-rtt".into())
                 .spawn(move || {
-                    if let Err(e) = probe_rs_rtt_loop(&chip, &probe_desc, &running_clone, &sender, elf_path.as_deref(), broadcast) {
+                    if let Err(e) = probe_rs_rtt_loop(&chip, &probe_desc, &running_clone, &sender, elf_path.as_deref()) {
                         let _ = sender.send(RttOutput { channel: 1, text: format!("RTT 错误: {}", e) });
```

- [ ] **Step 5: Remove broadcast logic from the polling loop**

**Edit `src/debug/rtt.rs`** — lines 256–276. Remove the `broadcast` clone and the broadcast send block:

```diff
     let mut line_bufs: Vec<String> = vec![String::new(); num_channels];
     let mut buf = vec![0u8; 4096];
-    let broadcast = broadcast_tx.clone();

     while running.load(Ordering::SeqCst) {
         for i in 0..num_channels {
             if let Some(ch) = rtt.up_channel(i) {
                 match ch.read(&mut core, &mut buf) {
                     Ok(count) if count > 0 => {
                         line_bufs[i].push_str(&String::from_utf8_lossy(&buf[..count]));
                         while let Some(nl) = line_bufs[i].find('\n') {
                             let line = line_bufs[i][..nl].trim_end_matches('\r').to_string();
                             line_bufs[i].drain(..=nl);
                             if !line.is_empty() {
                                 let out = RttOutput { channel: i as u8, text: line };
-                                // 发送到 TUI（crossbeam channel）
-                                if sender.send(out.clone()).is_err() { return Ok(()); }
-                                // 发布到 API broadcast（优先 config 传入，回退到全局）
-                                if let Some(ref tx) = broadcast {
-                                    let _ = tx.send(out.clone());
-                                } else if let Some(tx) = GLOBAL_BROADCAST.get() {
-                                    let _ = tx.send(out);
-                                }
+                                if sender.send(out).is_err() { return Ok(()); }
                             }
                         }
                     }
                     _ => {}
                 }
             }
         }
```

- [ ] **Step 6: Remove `broadcast: None` from `create_rtt_client()`**

**Edit `src/debug/rtt.rs`** — lines 529–536. Remove the `broadcast` field from the `RttConfig` construction:

```diff
             let cfg = RttConfig {
                 backend: RttBackend::ProbeRs,
                 chip: target.to_string(),
                 probe: String::new(),
                 telnet_port: gdb_port,
                 elf_path: if elf_path.is_empty() { None } else { Some(elf_path.to_string()) },
-                broadcast: None,
             };
```

- [ ] **Step 7: Update `create_rtt_client` doc comment**

**Edit `src/debug/rtt.rs`** — lines 472–478. Remove "API" from the doc comment:

```diff
 /// 根据后端类型创建 RTT 客户端和可选的子进程句柄
 ///
-/// TUI 和 API 模式共享此工厂函数，避免后端 spawn 逻辑重复。
+/// TUI 模式的 RTT 客户端工厂函数。
 ///
 /// # Returns
 /// - `Ok((client, child))` — RTT 客户端 + OpenOCD/pyOCD 子进程（probe-rs 时为 None）
 /// - `Err(msg)` — 启动失败的原因
```

- [ ] **Step 8: Build and verify**

```bash
cargo build 2>&1
```

Expected: compilation succeeds. No references to `GLOBAL_BROADCAST`, `set_global_broadcast`, or `broadcast` in RTT code.

- [ ] **Step 9: Run tests**

```bash
cargo test -p loading-chip 2>&1
```

Expected: all tests pass (the existing RTT tests in `debug/rtt.rs` test `RttOutput`, `RttChannel` — none test the broadcast path).

- [ ] **Step 10: Commit**

```bash
git add src/debug/rtt.rs
git commit -m "refactor: remove RTT global broadcast infrastructure

Remove GLOBAL_BROADCAST static, set_global_broadcast(),
broadcast field from RttConfig, and broadcast_tx parameter
from probe_rs_rtt_loop. The TUI uses crossbeam_channel directly
and never reads from the tokio broadcast channel.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Full build and verification

**Files:**
- None (verification only)

**Interfaces:**
- Consumes: all previous tasks completed
- Produces: verified clean build, all tests passing

- [ ] **Step 1: Clean build (debug)**

```bash
cargo clean
cargo build 2>&1
```

Expected: clean compile, zero warnings about unused imports/dead code from API-related items.

- [ ] **Step 2: Release build**

```bash
cargo build --release 2>&1
```

Expected: clean release compile.

- [ ] **Step 3: Run all tests**

```bash
cargo test 2>&1
```

Expected: all tests pass.

- [ ] **Step 4: Verify CLI help output**

```bash
cargo run -- --help
cargo run -- run --help
cargo run -- detect --help
cargo run -- init --help
cargo run -- debug --help
```

Expected: `run --help` shows no `--api` or `--api-addr` flags. All subcommands parse correctly.

- [ ] **Step 5: Verify `detect` subcommand works (if hardware available, else skip)**

```bash
cargo run -- detect
```

Expected: runs without panicking, either lists detected probes or exits cleanly.

- [ ] **Step 6: Commit (if any final cleanup needed)**

```bash
git status
```

If clean, no commit needed. If there are any remaining tweaks from build warnings, fix and commit.
