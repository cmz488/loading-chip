//! TUI 模块
//!
//! 提供终端交互界面的渲染和事件处理。
//! 通过 `run_tui()` 启动主循环，通过 `run_debug()` 启动调试界面。
//! 支持在烧录 TUI 和调试 TUI 之间通过 F5 / Shift+F5 切换。

pub mod app;
pub mod events;
pub mod ui;
pub mod debug_ui;

use std::io::{self, stdout, IsTerminal};
use std::sync::Arc;

use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::app::state::AppState;
use self::app::App;
use self::events::handle_key;
use self::ui::ui;

/// TUI 退出/切换信号
#[derive(Debug)]
pub enum TuiExit {
    /// 用户退出
    Quit,
    /// 烧录完成
    Flashed,
    /// 请求切换到调试模式（携带参数）
    DebugRequested {
        elf: String,
        target: String,
        backend: String,
        interface: String,
        port: u16,
        gdb: String,
    },
}

/// 启动统一 TUI 主循环（可恢复状态）
///
/// 管理烧录 ↔ 调试切换，自动保持用户的选择状态。
/// 第一次调用的 `resume_app` 传 None，之后传上次返回的 App。
pub fn run_with_resume(
    gdb_port: String,
    pyocd_path: String,
    timeout_secs: u64,
    resume_app: Option<App>,
    state: Arc<AppState>,
) -> io::Result<(TuiExit, Option<App>)> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend_t = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend_t)?;

    let mut app = resume_app.unwrap_or_else(move || {
        let mut app = App::new(gdb_port, pyocd_path, timeout_secs, state);
        // Auto-fill from detection
        if let Some(detected) = app.detected_chips.first() {
            let board_id = detected
                .board_id
                .clone()
                .unwrap_or_else(|| detected.chip_name.clone());
            // Try to resolve; if it fails, use the raw chip name
            if app.state.registry.resolve(&board_id, "probe-rs").is_ok() {
                app.target = board_id;
            } else {
                app.target = detected.chip_name.clone();
            }
            // Set interface from detection
            app.interface = detected.suggested_interface.clone();
            let iface_keys = crate::presets::iface_keys();
            if let Some(idx) = iface_keys
                .iter()
                .position(|k| *k == detected.suggested_interface)
            {
                app.iface_idx = idx;
            }
            app.status = format!(
                "已检测到: {} (芯片: {})",
                detected.probe_name, detected.chip_name
            );
            // detection results cached in AppState
        }
        app
    });
    let result = run_flash_tui_inner(&mut terminal, &mut app);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result.map(|exit| (exit, Some(app)))
}

/// 烧录 TUI 子循环（内部实现，复用 App 状态）
fn run_flash_tui_inner(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> io::Result<TuiExit> {

    loop {
        // 每一帧渲染
        terminal.draw(|f| ui(f, app))?;

        // 检查模式切换标志
        if app.switch_to_debug {
            app.switch_to_debug = false;
            let elf = app.debug.elf.clone();
            let target = app.debug.target.clone();
            let backend = app.debug.backend.clone();
            let interface = app.debug.interface.clone();
            let port = app.debug.port;
            let gdb = app.debug.gdb.clone();
            return Ok(TuiExit::DebugRequested {
                elf,
                target,
                backend,
                interface,
                port,
                gdb,
            });
        }

        // 检查退出
        if app.should_quit {
            return Ok(TuiExit::Quit);
        }

        // 等待按键
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                let should_continue = handle_key(app, key.code);
                if !should_continue {
                    // handle_key 返回 false → 执行烧录
                    // 先渲染一帧"烧录中"界面，避免看起来像卡死
                    app.mode = app::InputMode::Flashing;
                    app.status = format!("正在启动 {} ...", app.backend);
                    terminal.draw(|f| ui(f, app))?;

                    // 执行烧录（阻塞，大固件可能需要几十秒）
                    app.do_flash();

                    // 重绘结果
                    terminal.draw(|f| ui(f, app))?;
                }
            }
        }
    }
}

/// 启动 RTT 监视器 TUI
///
/// 根据后端类型启动对应的 GDB Server + RTT 采集：
/// - probe-rs: 直接使用 CLI-based probe-rs rtt
/// - openocd:  启动 OpenOCD GDB server 子进程，通过 telnet 读取 RTT
/// - pyocd:    启动 pyOCD gdbserver 子进程，通过 telnet 读取 RTT
/// - gdb:      RTT 不可用，显示提示信息
///
/// 参数说明：
/// - `elf`:     ELF 固件路径
/// - `target`:  目标芯片 key（如 "esp32s3"）
/// - `backend`: 后端类型（probe-rs / openocd / pyocd / gdb）
/// - `interface`: 调试接口（如 "swd"、"stlink"）
/// - `port`:    GDB Server 端口（默认 3333）
/// - `gdb`:     GDB 二进制路径（留空自动检测）
pub fn run_debug(
    elf: String,
    target: String,
    backend: String,
    interface: String,
    port: u16,
    _gdb: String, // GDB 路径在 RTT-only 模式下暂不直接使用
) -> io::Result<TuiExit> {
    use std::time::Duration;

    // TTY 检查
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("❌ RTT 监视器需要终端环境");
        return Ok(TuiExit::Flashed);
    }

    // 终端初始化
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend_t = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend_t)?;

    // 解析 pyocd 路径环境变量
    let pyocd_path = std::env::var("PYOCD_PATH").unwrap_or_default();

    let mut state = debug_ui::RttMonitorState::new(
        target.clone(),
        backend.clone(),
        elf.clone(),
        interface.clone(),
        port,
        pyocd_path,
    );

    // 启动 RTT（GDB 后端跳过）
    if backend.as_str() != "gdb" {
        state.start_rtt();
    }

    // 主循环
    loop {
        terminal.draw(|f| {
            debug_ui::render(f, &state, f.area());
        })?;

        if state.should_quit {
            break;
        }

        if state.running {
            state.poll_rtt();
        }

        if event::poll(Duration::from_millis(10))?
            && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press {
                    let exit = debug_ui::handle_key(&mut state, key.code);
                    if exit {
                        break;
                    }
                }
    }

    state.stop_rtt();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(TuiExit::Flashed)
}
