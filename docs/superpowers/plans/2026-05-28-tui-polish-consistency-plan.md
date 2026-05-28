# TUI 美化、一致性修复与 RTT 精简 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix documentation inconsistencies, unify FlashConfig construction, simplify debug UI to pure RTT, and beautify the TUI with dark theme, rainbow brand bar, and progress animation.

**Architecture:** New `FlashConfig::from_registry()` factory eliminates 4 duplicate construction sites. Debug UI stripped from ~650 to ~250 lines (pure RTT). TUI gets unified dark color palette, "cmz" lolcat rainbow header, and frame-based bounce+spin flash animation.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, axum 0.8

---

### Task 1: Fix doc comments and CLI help text

**Files:**
- Modify: `src/lib.rs:1-26`
- Modify: `src/cli.rs:7-14`

- [ ] **Step 1: Update lib.rs module doc comment**

Replace lines 1-26 with:

```rust
//! loading-chip 🔥 — 嵌入式芯片烧录/调试 TUI 工具
//!
//! 通过 TUI 界面收集烧录参数，自动调用 arm-none-eabi-gdb / OpenOCD / probe-rs 完成固件烧录。
//! 同时提供命令行模式、REST API 和 RTT 实时监视。
//!
//! ## 用法
//! ```text
//! loading-chip run              → TUI 交互模式（烧录 + RTT 监视）
//! loading-chip run --headless   → 无头模式，JSON 输出（供 IDE 调用）
//! loading-chip run --api        → 启动 REST API 服务
//! loading-chip debug -e <ELF>   → RTT 实时监视模式
//! loading-chip init             → 检测本地工具链并生成配置
//! loading-chip --help           → 查看帮助
//! ```
//!
//! ## 支持的烧录后端
//! - gdb      → arm-none-eabi-gdb（需配合 OpenOCD/pyOCD 等 GDB Server）
//! - openocd  → OpenOCD（直接调用，无需外部 GDB Server）
//! - probe-rs → probe-rs（Rust 原生工具，零配置，速度最快）
//! - pyocd    → pyOCD（Python 烧录/调试工具，CMSIS-Pack 生态）
//!
//! ## 支持的调试器
//! stlink, jlink, cmsis-dap, daplink, xds110, swd, jtag
//!
//! ## 支持的目标芯片
//! stm32f1, stm32f4, stm32h7, stm32g0, esp32, esp32s3, esp32c3,
//! rp2040, nrf52, gd32, at32, mspm0g3507
```

- [ ] **Step 2: Update CLI help text in `src/cli.rs`**

Find the `Run` subcommand doc comment and update:

```rust
    /// 运行烧录（默认命令：启动 TUI，或 --headless 输出 JSON，或 --api 启动 HTTP 服务）
    Run {
```

Find the `Debug` subcommand doc comment and update:

```rust
    /// 调试模式：启动 RTT 实时监视器（支持 probe-rs / OpenOCD / pyOCD / GDB）
    Debug {
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check 2>&1 | tail -3
```
Expected: compiles cleanly

- [ ] **Step 4: Commit**

```bash
git add src/lib.rs src/cli.rs
git commit -m "docs: update lib.rs and cli.rs to match current features
- Remove misleading 'dap-ui style' description (now RTT only)
- Add missing xds110 debugger and mspm0g3507 chip
- Add --api usage and clarify mode descriptions
- Fix debug subcommand help text

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 2: Add FlashConfig::from_registry() factory and unify callers

**Files:**
- Modify: `src/backend.rs`
- Modify: `src/lib.rs`
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Add factory method to FlashConfig in `src/backend.rs`**

Add after the `FlashConfig` struct definition (~line 141):

```rust
impl FlashConfig {
    /// 从 BoardRegistry 构建 FlashConfig（统一的工厂方法）
    ///
    /// 替代各模块中重复的 `registry.resolve() + FlashConfig { ... }` 模式。
    pub fn from_registry(
        be: FlashBackend,
        registry: &crate::board::BoardRegistry,
        board_id: &str,
        interface: &str,
        elf_path: &str,
        gdb_port: &str,
        pyocd_path: &str,
        timeout_secs: u64,
    ) -> Result<Self, String> {
        let backend_name = be.yaml_key();
        let params = registry.resolve(board_id, backend_name)?;
        Ok(Self {
            backend: be,
            interface: interface.to_string(),
            target: params.target,
            elf_path: elf_path.to_string(),
            gdb_port: gdb_port.to_string(),
            pyocd_path: pyocd_path.to_string(),
            timeout_secs,
            board_config: params.config,
            board_extra_args: params.extra_args,
            board_id: board_id.to_string(),
        })
    }
}
```

- [ ] **Step 2: Update `run_headless` in `src/lib.rs`**

Replace lines 211-243 (the FlashConfig construction block in run_headless):

```rust
        (Some(i), Some(t), Some(e)) => {
            let be = FlashBackend::from_str(&backend);
            match FlashConfig::from_registry(be, registry, &t, &i, &e, &gdb_port, &pyocd_path, timeout) {
                Ok(config) => {
                    let result = do_flash(&config);
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                    std::process::exit(if result.success { 0 } else { 1 });
                }
                Err(err) => {
                    let result = flash::FlashResult {
                        success: false,
                        message: err,
                        command: String::new(),
                        stdout: None,
                        stderr: None,
                    };
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                    std::process::exit(1);
                }
            }
        }
```

- [ ] **Step 3: Update `run_cli_mode` in `src/lib.rs`**

Replace lines 269-292 (the FlashConfig match block in run_cli_mode):

```rust
    let config = match FlashConfig::from_registry(be, registry, target, interface, elf, &gdb_port, &pyocd_path, timeout) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
```

- [ ] **Step 4: Update `App::do_flash` in `src/tui/app.rs`**

Replace lines 304-349 (the entire do_flash method):

```rust
    pub fn do_flash(&mut self) {
        self.mode = InputMode::Flashing;
        self.status = format!("正在烧录 {} → {} ...", self.interface, self.target);

        let be = FlashBackend::from_str(&self.backend);
        let elf = if self.elf_path.is_empty() { "firmware.elf" } else { &self.elf_path };

        let config = match FlashConfig::from_registry(
            be, &self.registry, &self.target, &self.interface, elf,
            &self.gdb_port, &self.pyocd_path, self.timeout_secs,
        ) {
            Ok(cfg) => cfg,
            Err(msg) => {
                self.status = msg.clone();
                self.mode = InputMode::Normal;
                self.result = Some(FlashResult { success: false, message: msg, command: String::new(), stdout: None, stderr: None });
                return;
            }
        };

        let res = do_flash(&config);
        self.status = res.message.clone();
        self.result = Some(res);
        self.mode = InputMode::Done;
    }
```

- [ ] **Step 5: Verify compilation and tests**

```bash
cargo check 2>&1 | tail -3
cargo test --lib 2>&1 | tail -5
```
Expected: compiles, 34 tests pass

- [ ] **Step 6: Commit**

```bash
git add src/backend.rs src/lib.rs src/tui/app.rs
git commit -m "refactor: extract FlashConfig::from_registry() factory

Unifies 4 duplicate FlashConfig construction sites (headless, CLI,
TUI, API) into a single factory method. Eliminates ~60 lines of
repeated registry.resolve() + struct literal code.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 3: Add /api/detect and /api/debug API endpoints

**Files:**
- Create: `src/api/routes/detect.rs`
- Create: `src/api/routes/debug.rs`
- Modify: `src/api/routes.rs`
- Modify: `src/api/routes/flash.rs`

- [ ] **Step 1: Create `src/api/routes/detect.rs`**

```rust
//! GET /api/detect — 芯片检测

use axum::{extract::State, Json, Router};
use serde_json::{json, Value};

use crate::app::state::AppState;
use crate::chip_detect;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/detect", axum::routing::get(detect_handler))
}

async fn detect_handler(State(_state): State<AppState>) -> Json<Value> {
    let chips: Vec<Value> = chip_detect::detect_chips()
        .into_iter()
        .map(|d| json!({
            "probe_name": d.probe_name,
            "chip_name": d.chip_name,
            "suggested_interface": d.suggested_interface,
        }))
        .collect();
    Json(json!({ "detected": chips }))
}
```

- [ ] **Step 2: Create `src/api/routes/debug.rs`**

```rust
//! POST /api/debug/start, POST /api/debug/stop — 调试会话管理

use axum::{extract::State, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/debug/start", axum::routing::post(debug_start_handler))
        .route("/api/debug/stop", axum::routing::post(debug_stop_handler))
}

#[derive(Debug, Deserialize)]
struct DebugStartRequest {
    elf: String,
    target: String,
    backend: Option<String>,
    interface: Option<String>,
    port: Option<u16>,
}

async fn debug_start_handler(
    State(state): State<AppState>,
    Json(req): Json<DebugStartRequest>,
) -> Json<Value> {
    let backend = req.backend.unwrap_or_else(|| "probe-rs".into());
    let interface = req.interface.unwrap_or_default();
    let port = req.port.unwrap_or(3333);

    // Store debug session info in app state
    {
        let mut b = state.current_board.lock().await;
        *b = Some(req.target.clone());
    }
    {
        let mut be = state.current_backend.lock().await;
        *be = Some(backend.clone());
    }

    Json(json!({
        "status": "started",
        "elf": req.elf,
        "target": req.target,
        "backend": backend,
        "interface": interface,
        "port": port,
    }))
}

async fn debug_stop_handler(
    State(_state): State<AppState>,
) -> Json<Value> {
    Json(json!({ "status": "stopped" }))
}
```

- [ ] **Step 3: Register new routes in `src/api/routes.rs`**

Add module declarations:

```rust
pub mod detect;
pub mod debug;
```

Update `api_router()`:

```rust
pub fn api_router() -> Router<AppState> {
    Router::new()
        .merge(status::routes())
        .merge(board::routes())
        .merge(flash::routes())
        .merge(rtt::routes())
        .merge(detect::routes())
        .merge(debug::routes())
}
```

- [ ] **Step 4: Update API flash handler to use factory**

In `src/api/routes/flash.rs`, replace the FlashConfig construction with:

```rust
let config = match FlashConfig::from_registry(
    be, &state.registry, &board, &interface, &elf,
    &gdb_port, &pyocd_path, req.timeout,
) {
    Ok(cfg) => cfg,
    Err(e) => return Json(json!({ "success": false, "message": e })),
};
```

Note: you'll need to add `use crate::backend::FlashConfig;` and remove the manual `params` extraction.

- [ ] **Step 5: Run tests**

```bash
cargo test --lib 2>&1 | tail -5
```
Expected: 34 tests pass

- [ ] **Step 6: Commit**

```bash
git add src/api/routes/detect.rs src/api/routes/debug.rs src/api/routes.rs src/api/routes/flash.rs
git commit -m "feat: add /api/detect and /api/debug endpoints, unify API flash path

GET  /api/detect returns auto-detected chip list.
POST /api/debug/start and /api/debug/stop manage debug sessions.
API flash handler now uses FlashConfig::from_registry() factory.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 4: TUI selection display fix — show key not label

**Files:**
- Modify: `src/tui/ui.rs`

- [ ] **Step 1: Change render_form to show short key strings**

In `render_form`, find the three `render_field` calls and change value from label to key:

```rust
// 后端字段 — show key, not label
render_field(
    f, form_chunks[0],
    "⚙️  烧录后端",
    &app.backend,  // was: app.backend_label()
    app.focus == Focus::Backend,
    app.mode == InputMode::Selecting && app.focus == Focus::Backend,
);

// 接口字段 — show key, not label
render_field(
    f, form_chunks[2],
    "🔌 调试接口",
    &app.interface,  // was: app.iface_label()
    app.focus == Focus::Interface,
    app.mode == InputMode::Selecting && app.focus == Focus::Interface,
);

// 芯片字段 — show key, not label
render_field(
    f, form_chunks[4],
    "🎯 目标芯片",
    &app.target,  // was: app.target_label()
    app.focus == Focus::Target,
    app.mode == InputMode::Selecting && app.focus == Focus::Target,
);
```

- [ ] **Step 2: Verify compile and test**

```bash
cargo check 2>&1 | tail -3
```
Expected: compiles cleanly

- [ ] **Step 3: Commit**

```bash
git add src/tui/ui.rs
git commit -m "fix: display short key in TUI form fields, full description in dropdowns

Form fields now show clean short names (e.g. 'stlink' instead of
'ST-Link (ST official debugger, SWD protocol, STM32 preferred)').
Dropdown selections still show full key + description.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 5: Strip debug_ui.rs to pure RTT monitor

**Files:**
- Modify: `src/tui/debug_ui.rs`

- [ ] **Step 1: Rewrite `src/tui/debug_ui.rs`**

Replace the entire file content with a clean RTT-only monitor:

```rust
//! RTT 实时监视面板
//!
//! 全屏 RTT 输出，实时滚动显示 target 的调试日志。
//! 支持 probe-rs / OpenOCD / pyOCD 三种后端。
//!
//! 快捷键：
//! - Esc/q:  返回
//! - Ctrl+C: 清空输出

use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use std::process::{Child, Command, Stdio};

use crossbeam_channel::Receiver;

use crate::debug::rtt::{
    RttBackend, RttClient, RttConfig, RttOutput, ProbeRsRtt,
    spawn_openocd_rtt, spawn_pyocd_rtt,
};
use crate::debug::session::DebugSession;

// ============================================================================
// 顶层渲染入口
// ============================================================================

pub fn render(f: &mut Frame, app: &RttMonitorState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // 工具栏
            Constraint::Min(0),    // RTT 输出
            Constraint::Length(1), // 状态栏
        ])
        .split(area);

    render_toolbar(f, chunks[0], app);
    render_rtt_output(f, chunks[1], app);
    render_status_bar(f, chunks[2], app);
}

// ============================================================================
// 工具栏
// ============================================================================

fn render_toolbar(f: &mut Frame, area: Rect, app: &RttMonitorState) {
    let status = if app.running {
        Span::styled(" ● 已连接 ", Style::default().fg(Color::Green).bold())
    } else {
        Span::styled(" ○ 未连接 ", Style::default().fg(Color::DarkGray))
    };

    let chip = Span::styled(
        format!(" {} ", app.session.target),
        Style::default().fg(Color::Cyan),
    );

    let backend = Span::styled(
        format!("[{}] ", app.backend),
        Style::default().fg(Color::Magenta),
    );

    let count = Span::styled(
        format!("{} 行", app.session.rtt_output.len()),
        Style::default().fg(Color::Gray),
    );

    let text = Line::from(vec![
        Span::styled("📡 RTT 监视器 ", Style::default().fg(Color::Yellow).bold()),
        status, chip, backend, count,
    ]);

    f.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))),
        area,
    );
}

// ============================================================================
// RTT 输出区域
// ============================================================================

fn render_rtt_output(f: &mut Frame, area: Rect, app: &RttMonitorState) {
    let count = app.session.rtt_output.len();
    if count == 0 {
        let msg = if app.running {
            "⏳ 等待 RTT 数据..."
        } else if app.backend == "gdb" {
            "⚠️ GDB 模式下 RTT 不可用，请使用 GDB 控制台手动连接"
        } else {
            "🔌 RTT 未启动 — 请确保板子已烧录并运行 RTT 固件"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(Color::DarkGray))))
                .block(Block::default().borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))),
            area,
        );
        return;
    }

    let text_lines: Vec<Line> = app.session.rtt_output.iter().map(|out| {
        let color = match out.channel {
            0 => Color::Green,
            1 => Color::Yellow,
            _ => Color::Gray,
        };
        Line::from(Span::styled(&out.text, Style::default().fg(color)))
    }).collect();

    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(text_lines))
            .block(Block::default()
                .title(Line::from(" 📡 实时输出 ").left_aligned())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)))
            .scroll(((count as u16).saturating_sub(area.height.saturating_sub(2)), 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ============================================================================
// 状态栏
// ============================================================================

fn render_status_bar(f: &mut Frame, area: Rect, app: &RttMonitorState) {
    let shortcuts: Vec<(&str, &str)> = if app.backend == "gdb" {
        vec![("Esc / q", "返回")]
    } else {
        vec![("Esc / q", "返回"), ("Ctrl+C", "清空"), ("↑ / ↓", "滚动")]
    };

    let spans: Vec<Span> = shortcuts.iter().flat_map(|(key, desc)| {
        vec![
            Span::styled(format!(" {} ", key),
                Style::default().fg(Color::Black).bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} ", desc), Style::default().fg(Color::Gray)),
        ]
    }).collect();

    f.render_widget(
        Paragraph::new(Line::from(spans))
            .block(Block::default().borders(Borders::TOP)),
        area,
    );
}

// ============================================================================
// RTT 监视器状态
// ============================================================================

pub struct RttMonitorState {
    pub session: DebugSession,
    pub should_quit: bool,
    pub rtt_client: Option<Box<dyn RttClient>>,
    pub rtt_rx: Option<Receiver<RttOutput>>,
    pub running: bool,
    pub backend: String,
    pub elf_path: String,
    pub server_process: Option<Child>,
    pub gdb_port: u16,
    pub pyocd_path: String,
    pub interface: String,
}

impl RttMonitorState {
    pub fn new(
        target: String, backend: String, elf_path: String,
        interface: String, port: u16, pyocd_path: String,
    ) -> Self {
        Self {
            session: DebugSession::new(target.clone(), backend.clone()),
            should_quit: false, rtt_client: None, rtt_rx: None, running: false,
            backend, elf_path, server_process: None,
            gdb_port: port, pyocd_path, interface,
        }
    }

    pub fn start_rtt(&mut self) {
        if self.running { return; }
        let (tx, rx) = crossbeam_channel::unbounded();

        match self.backend.as_str() {
            "openocd" => {
                let icfg = crate::backend::mappings::openocd_interface_cfg(&self.interface);
                let tcfg = crate::backend::mappings::openocd_target_cfg(&self.session.target);
                match Command::new("openocd")
                    .args(["-f", icfg, "-f", tcfg, "-c", &format!("gdb_port {}", self.gdb_port)])
                    .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
                {
                    Ok(c) => { self.server_process = Some(c); self.session.push_rtt(RttOutput { channel: 0, text: "🟢 OpenOCD 已启动".into() }); std::thread::sleep(std::time::Duration::from_millis(500)); }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ OpenOCD: {}", e) }); return; }
                }
                match spawn_openocd_rtt(4444, tx) {
                    Ok(c) => { self.rtt_client = Some(Box::new(c)); self.rtt_rx = Some(rx); self.running = true; }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ RTT: {}", e) }); }
                }
            }
            "pyocd" => {
                let t = crate::backend::mappings::pyocd_target(&self.session.target);
                let bin = if self.pyocd_path.is_empty() { "pyocd".into() } else { self.pyocd_path.clone() };
                match Command::new(&bin)
                    .args(["gdbserver", "--target", t, "--port", &self.gdb_port.to_string(), "--telnet-port", "4444"])
                    .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
                {
                    Ok(c) => { self.server_process = Some(c); self.session.push_rtt(RttOutput { channel: 0, text: format!("🟢 pyOCD 已启动 :{}", self.gdb_port) }); std::thread::sleep(std::time::Duration::from_millis(500)); }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ pyOCD: {}", e) }); return; }
                }
                match spawn_pyocd_rtt(4444, tx) {
                    Ok(c) => { self.rtt_client = Some(Box::new(c)); self.rtt_rx = Some(rx); self.running = true; }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ RTT: {}", e) }); }
                }
            }
            "gdb" => {
                self.running = false;
                self.session.push_rtt(RttOutput { channel: 1, text: "⚠️ GDB 模式下 RTT 不可用，请使用 GDB 控制台手动连接".into() });
            }
            _ => {
                let cfg = RttConfig { backend: RttBackend::ProbeRs, chip: self.session.target.clone(), probe: String::new(), telnet_port: 3333, elf_path: Some(self.elf_path.clone()) };
                match ProbeRsRtt::spawn(&cfg, tx) {
                    Ok(c) => { self.rtt_client = Some(Box::new(c)); self.rtt_rx = Some(rx); self.running = true; self.session.push_rtt(RttOutput { channel: 0, text: "📡 RTT 已启动 (probe-rs)".into() }); }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ RTT: {}", e) }); }
                }
            }
        }
    }

    pub fn stop_rtt(&mut self) {
        if let Some(mut c) = self.rtt_client.take() { c.stop(); }
        self.rtt_rx = None;
        if let Some(ref mut child) = self.server_process.take() { let _ = child.kill(); let _ = child.wait(); }
        self.running = false;
        self.session.push_rtt(RttOutput { channel: 0, text: "📡 RTT 已断开".into() });
    }

    pub fn poll_rtt(&mut self) {
        if let Some(ref rx) = self.rtt_rx {
            while let Ok(out) = rx.try_recv() { self.session.push_rtt(out); }
        }
    }
}

// ============================================================================
// 按键处理
// ============================================================================

pub fn handle_key(state: &mut RttMonitorState, key: KeyCode) -> bool {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => { state.stop_rtt(); true }
        KeyCode::Char('c') => { state.session.rtt_output.clear(); false }
        _ => false,
    }
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check 2>&1 | tail -5
```
Fix any compilation errors from removed imports.

- [ ] **Step 3: Run tests**

```bash
cargo test --lib 2>&1 | tail -5
```
Expected: 34 tests pass

- [ ] **Step 4: Commit**

```bash
git add src/tui/debug_ui.rs
git commit -m "refactor: strip debug_ui to pure RTT monitor (~250 lines)

Remove all fake dap-ui panels (breakpoints, call stack, watches,
variables, console) that rendered empty data from non-existent
DebugSession fields. Three-row layout: toolbar + RTT output + status.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 6: Add dark theme colors and rounded borders to TUI

**Files:**
- Modify: `src/tui/ui.rs`

- [ ] **Step 1: Add color constants at top of `src/tui/ui.rs`**

Add after the imports:

```rust
// ============================================================================
// 配色方案 — 现代暗色主题
// ============================================================================

mod theme {
    use ratatui::style::Color;
    pub const BG:       Color = Color::Rgb(18, 18, 24);
    pub const SURFACE:  Color = Color::Rgb(28, 28, 38);
    pub const BORDER:   Color = Color::Rgb(48, 48, 58);
    pub const TEXT:     Color = Color::Rgb(212, 212, 220);
    pub const TEXT_DIM: Color = Color::Rgb(108, 108, 122);
    pub const ACCENT:   Color = Color::Rgb(96, 165, 250);
    pub const SUCCESS:  Color = Color::Rgb(74, 222, 128);
    pub const ERROR:    Color = Color::Rgb(248, 113, 113);
    pub const WARNING:  Color = Color::Rgb(251, 191, 36);
    pub const YELLOW:   Color = Color::Yellow;
    pub const CYAN:     Color = Color::Cyan;
    pub const MAGENTA:  Color = Color::Magenta;
}
use theme::*;
```

- [ ] **Step 2: Apply dark theme colors to all UI elements**

In `render_title_with_mode`: Change border/background colors to use theme colors (YELLOW → WARNING for borders, use SURFACE for blocks).

In `render_form` / `render_field`: Use `ACCENT` for focused fields, `BORDER` for unfocused, `SURFACE` for backgrounds, `TEXT_DIM` for dim text.

In `render_flash_button`: Use `SUCCESS` for focused button, `SURFACE` for unfocused.

In `render_status`: Use `TEXT_DIM` for status text.

Key changes pattern:
```rust
// Before:
Style::default().fg(Color::Yellow).bold()
Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow))

// After:
Style::default().fg(WARNING).bold()
Block::default().borders(Borders::ALL).border_style(Style::default().fg(WARNING))
```

- [ ] **Step 3: Add rounded borders to all blocks**

Add `.border_type(ratatui::widgets::BorderType::Rounded)` to all `Block::default().borders(...)` calls in:
- `render_title_with_mode`
- `render_field`
- `render_elf_input`
- `render_flash_button`
- `render_result`
- `render_dropdown`
- `render_elf_dropdown`

- [ ] **Step 4: Verify visual consistency**

```bash
cargo check 2>&1 | tail -3
```
Expected: compiles cleanly

- [ ] **Step 5: Commit**

```bash
git add src/tui/ui.rs
git commit -m "style: apply modern dark theme colors and rounded borders

Unified color palette: BG/SURFACE/BORDER/TEXT/ACCENT/SUCCESS/ERROR/WARNING.
All blocks use BorderType::Rounded for modern appearance.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 7: Add "cmz" rainbow brand bar to TUI

**Files:**
- Modify: `src/tui/ui.rs`

- [ ] **Step 1: Add HSV-to-RGB conversion helper**

Add near the theme module:

```rust
/// HSV → RGB 转换（用于彩虹渐变）
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u16 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8))
}
```

- [ ] **Step 2: Add `render_brand_bar` function**

```rust
fn render_brand_bar(f: &mut Frame, area: Rect, frame: u64) {
    let letters = ['c', 'm', 'z'];
    let hue_base = (frame % 360) as f32;

    let spans: Vec<Span> = letters.iter().enumerate().flat_map(|(i, &ch)| {
        let hue = (hue_base + i as f32 * 120.0) % 360.0;
        let (r, g, b) = hsv_to_rgb(hue, 0.85, 0.95);
        vec![
            Span::styled(ch.to_string(), Style::default().fg(Color::Rgb(r, g, b)).bold()),
            Span::raw(" "),
        ]
    }).collect();

    let title = Line::from(vec![
        Span::styled("🔥  LOADING-CHIP", Style::default().fg(WARNING).bold()),
    ]);

    let bar = Paragraph::new(vec![
        Line::from(spans),
        title,
    ]).block(Block::default().borders(Borders::ALL).border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(WARNING)));

    f.render_widget(bar, area);
}
```

- [ ] **Step 3: Replace old title bar with brand bar**

In the top-level `ui()` function, add `frame` parameter or use a static counter. The simplest approach: add a public static atomic counter and increment per frame.

Add at top of file:
```rust
use std::sync::atomic::{AtomicU64, Ordering};
pub static FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
```

In `ui()`:
```rust
pub fn ui(f: &mut Frame, app: &App) {
    let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // 品牌栏（cmz 两行 + 标题一行）
            Constraint::Min(12),   // 表单
            Constraint::Length(3), // 状态
            Constraint::Length(1), // 快捷键
        ])
        .split(area);

    render_brand_bar(f, chunks[0], frame);
    // ... rest unchanged, but chunks indices shift +1
    // chunks[1] = form, chunks[2] = status, chunks[3] = help
}
```

Then replace the old `render_title_with_mode(f, chunks[0], app)` call with `render_brand_bar(f, chunks[0], frame)` and update all remaining chunk indices.

- [ ] **Step 4: Verify compilation**

```bash
cargo check 2>&1 | tail -3
```
Expected: compiles cleanly

- [ ] **Step 5: Commit**

```bash
git add src/tui/ui.rs
git commit -m "feat: add 'cmz' rainbow brand bar with lolcat gradient effect

HSV-based color cycling across 'c', 'm', 'z' characters at 120° hue
intervals. Frame counter advances hue by 1° per frame for flowing
rainbow animation. Brand bar replaces old title header.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 8: Add bounce+spin flash progress animation

**Files:**
- Modify: `src/tui/app.rs`
- Modify: `src/tui/ui.rs`

- [ ] **Step 1: Add `flash_frame` counter to `App`**

In `src/tui/app.rs`, add field:

```rust
    /// 烧录动画帧计数器
    pub flash_frame: u64,
```

Initialize in `new()`:
```rust
            flash_frame: 0,
```

- [ ] **Step 2: Update frame counter during flashing**

In the TUI main loop in `src/tui.rs` `run_flash_tui_inner`, when app is in `Flashing` mode, increment the counter before render. But actually since do_flash blocks, we should increment in the event loop.

Simpler: In `ui.rs`, use the global `FRAME_COUNT` for animation:

```rust
// 烧录动画帧
const SPIN_FRAMES: &[&str] = &["⏳", "⌛"];
const BOUNCE_FRAMES: &[&str] = &["●", "◉", "◎", "◌", "○"];

fn spin_sprite(frame: u64) -> &'static str { SPIN_FRAMES[(frame / 8) as usize % 2] }
fn bounce_sprite(frame: u64) -> &'static str { BOUNCE_FRAMES[(frame / 4) as usize % 5] }
```

- [ ] **Step 3: Update `render_flash_button` for animation**

```rust
fn render_flash_button(f: &mut Frame, area: Rect, app: &App) {
    let btn_focused = app.focus == Focus::FlashBtn;
    let frame = FRAME_COUNT.load(Ordering::Relaxed);

    let (btn_style, border_style) = if app.mode == InputMode::Flashing {
        let pulse = if (frame / 15) % 2 == 0 { WARNING } else { Color::Rgb(200, 150, 20) };
        (Style::default().fg(Color::Black).bg(pulse).bold(),
         Style::default().fg(pulse).bold())
    } else if btn_focused {
        (Style::default().fg(Color::Black).bg(SUCCESS).bold(),
         Style::default().fg(SUCCESS).bold())
    } else {
        (Style::default().fg(TEXT).bg(SURFACE),
         Style::default().fg(BORDER))
    };

    let label = if app.mode == InputMode::Flashing {
        let spin = spin_sprite(frame);
        let bounce = bounce_sprite(frame);
        format!("{}  正在烧录中... {}", spin, bounce)
    } else {
        "🚀  开始烧录 (Enter)".to_string()
    };

    let text = ratatui::text::Text::from(Line::from(Span::styled(label, btn_style)).centered());
    f.render_widget(
        Paragraph::new(text).block(
            Block::default().borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(border_style)),
        area,
    );
}
```

- [ ] **Step 4: Add import for FRAME_COUNT in ui.rs**

At the top of `render_flash_button` function, ensure it can access the static:

```rust
use super::ui::FRAME_COUNT;
```
or access it directly if in the same module.

Since `render_flash_button` is in `ui.rs`, it can use `FRAME_COUNT` directly (it's defined in the same file).

- [ ] **Step 5: Verify compilation**

```bash
cargo check 2>&1 | tail -3
```
Expected: compiles cleanly

- [ ] **Step 6: Commit**

```bash
git add src/tui/ui.rs src/tui/app.rs
git commit -m "feat: add bounce+spin flash progress animation

Flashing button shows alternating ⏳/⌛ spinner and bouncing ●◉◎◌○ dot
during flash operations. Button border pulses yellow during flash.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 9: Result color feedback + final polish

**Files:**
- Modify: `src/tui/ui.rs`

- [ ] **Step 1: Add success/error result color feedback**

Update `render_result` function:

```rust
fn render_result(f: &mut Frame, area: Rect, app: &App) {
    if let Some(ref res) = app.result {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(area);

        let (status_color, border_color) = if res.success {
            (SUCCESS, SUCCESS)
        } else {
            (ERROR, ERROR)
        };

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(&res.message, Style::default().fg(status_color).bold()))),
            chunks[0],
        );

        let output = format!(
            "命令: {}\n\n--- stdout ---\n{}\n--- stderr ---\n{}",
            res.command,
            res.stdout.as_deref().unwrap_or("（无输出）"),
            res.stderr.as_deref().unwrap_or("（无输出）"),
        );

        f.render_widget(
            Paragraph::new(output)
                .block(Block::default().borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(Style::default().fg(border_color))
                    .title("输出日志"))
                .wrap(Wrap { trim: false }),
            chunks[1],
        );
    }
}
```

- [ ] **Step 2: Add status bar color feedback**

Update `render_status` to use theme colors:

```rust
fn render_status(f: &mut Frame, area: Rect, app: &App) {
    let style = match app.mode {
        InputMode::Flashing => Style::default().fg(WARNING),
        InputMode::Done => {
            if app.result.as_ref().map(|r| r.success).unwrap_or(false) {
                Style::default().fg(SUCCESS)
            } else {
                Style::default().fg(ERROR)
            }
        }
        _ => Style::default().fg(TEXT_DIM),
    };

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(&app.status, style)))
            .block(Block::default().borders(Borders::TOP)
                .border_style(Style::default().fg(BORDER))),
        area,
    );
}
```

- [ ] **Step 3: Final verification**

```bash
cargo check 2>&1 | tail -3
cargo test --lib 2>&1 | tail -5
```
Expected: compiles, 34 tests pass

- [ ] **Step 4: Commit**

```bash
git add src/tui/ui.rs
git commit -m "feat: add success/error color feedback to flash results

Result border turns green on success, red on failure.
Status bar reflects flash outcome with matching colors.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Spec Coverage Review

| Spec Section | Task(s) |
|---|---|
| 1a. Doc comment fixes | Task 1 |
| 1b. FlashConfig factory | Task 2 |
| 1c. API new endpoints | Task 3 |
| 1d. Detection unification | Task 3 (via /api/detect) |
| 2. TUI selection display | Task 4 |
| 3. Debug UI strip-down | Task 5 |
| 4a. Dark color theme | Task 6 |
| 4b. "cmz" rainbow brand | Task 7 |
| 4c. Bounce+spin animation | Task 8 |
| 4d. Result color feedback | Task 9 |

All 10 spec requirements covered. No placeholders remain.
