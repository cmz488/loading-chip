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

use crate::board::BoardRegistry;
use crate::chip_detect::DetectedChip;
use self::app::App;
use self::debug_ui::DebugAppState;
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
    detected_chips: Vec<DetectedChip>,
    registry: Arc<BoardRegistry>,
) -> io::Result<(TuiExit, Option<App>)> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend_t = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend_t)?;

    let mut app = resume_app.unwrap_or_else(move || {
        let mut app = App::new(gdb_port, pyocd_path, timeout_secs, registry);
        // Auto-fill from detection
        if let Some(detected) = detected_chips.first() {
            let board_id = detected
                .board_id
                .clone()
                .unwrap_or_else(|| detected.chip_name.clone());
            // Try to resolve; if it fails, use the raw chip name
            if app.registry.resolve(&board_id, "probe-rs").is_ok() {
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
            app.detected_chips = detected_chips;
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
            let elf = app.debug_elf.clone();
            let target = app.debug_target.clone();
            let backend = app.debug_backend.clone();
            let interface = app.debug_interface.clone();
            let port = app.debug_port;
            let gdb = app.debug_gdb.clone();
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
                    app.status = format!("正在启动 {} ...", app.backend_label());
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

/// 启动调试模式 TUI
///
/// 自动启动 GDB Server（probe-rs / openocd / pyocd），
/// 然后启动 GDB MI 客户端连接上去，显示 dap-ui 风格调试面板。
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
    gdb: String,
) -> io::Result<TuiExit> {
    use crate::backend::GdbServerProcess;
    use crate::debug::{GdbConfig, GdbMi, DebugSession};
    use crossterm::event::{Event, KeyEventKind};
    use std::time::Duration;

    // ---- 1. 确定 GDB 二进制路径 ----
    // 先检查用户指定，没有则自动搜索
    // resolve_gdb_binary 返回 (路径, 是否为首选候选)
    // 首选候选 = 目标架构匹配的 GDB；回退候选 = 架构不匹配（如 arm 读 xtensa）
    let gdb_opt = if gdb.is_empty() {
        crate::backend::mappings::resolve_gdb_binary(&target)
    } else if std::path::Path::new(&gdb).exists() {
        Some((gdb, true)) // 用户显式指定，认为用户知道自己在做什么
    } else {
        None
    };

    // ---- 2. 启动 GDB Server ----
    let gdb_server_result = GdbServerProcess::spawn(
        &backend,
        &target,
        if interface.is_empty() { None } else { Some(&interface) },
        port,
    );

    // 等待 GDB Server 就绪
    let mut gdb_server: Option<GdbServerProcess> = None;
    match gdb_server_result {
        Some(Ok(mut server)) => {
            eprintln!("⏳ 等待 GDB Server 在端口 {} 就绪...", server.port);
            if let Err(e) = server.wait_ready() {
                eprintln!("❌ GDB Server 启动失败: {}", e);
                return Ok(TuiExit::Flashed);
            }
            eprintln!("✅ GDB Server 已就绪");
            gdb_server = Some(server);
        }
        Some(Err(e)) => {
            eprintln!("❌ 启动 GDB Server 失败: {}", e);
            return Ok(TuiExit::Flashed);
        }
        None => {
            // gdb 后端：不启动 GDB Server
        }
    }

    // ---- 3. 检测 GDB 客户端可用性，决定是否进入 TUI ----
    // - 用户手动指定了 GDB 路径 → 信任用户，直接进 TUI
    // - 自动检测到首选 GDB（架构匹配）→ 进 TUI
    // - 自动检测只找到回退 GDB（架构不匹配）→ 输出连接指引，不进 TUI
    // - 完全找不到任何 GDB → 输出连接指引
    let gdb_binary = match gdb_opt {
        Some((path, true)) => path,
        Some((_path, false)) => {
            // 只找到回退 GDB（如 arm-none-eabi-gdb 读 Xtensa ELF）
            eprintln!("✅ GDB Server 已就绪 (localhost:{})", port);
            eprintln!();
            eprintln!("   未找到原生 GDB 客户端（仅找到架构不匹配的回退 GDB）。");
            eprintln!("   请手动连接架构匹配的 GDB：");
            eprintln!("     $ {} {} -ex 'target remote :{}'",
                crate::backend::mappings::gdb_binary_candidates(&target)[0],
                elf,
                port,
            );
            eprintln!();
            eprintln!("   按 Enter 返回，或 Ctrl+C 退出");
            let mut _input = String::new();
            std::io::stdin().read_line(&mut _input)?;
            return Ok(TuiExit::Flashed);
        }
        None => {
            eprintln!("✅ GDB Server 已就绪 (localhost:{})", port);
            eprintln!();
            eprintln!("   系统中未找到任何可用的 GDB 客户端。请安装后重试：");
            eprintln!("     $ {} {} -ex 'target remote :{}'",
                crate::backend::mappings::gdb_binary_candidates(&target)[0],
                elf,
                port,
            );
            eprintln!();
            eprintln!("   按 Enter 返回，或 Ctrl+C 退出");
            let mut _input = String::new();
            std::io::stdin().read_line(&mut _input)?;
            return Ok(TuiExit::Flashed);
        }
    };

    // ---- 4. 启动 GDB MI 客户端，进入 TUI 调试 ----
    let session = DebugSession::new(target.clone());
    let mut state = DebugAppState::new(session);

    let (tx, rx) = crossbeam_channel::unbounded();
    let config = GdbConfig {
        gdb_binary: gdb_binary.clone(),
        elf_path: elf.clone(),
        remote: format!("localhost:{}", port),
    };

    let mut gdb_mi = match GdbMi::spawn(config, tx) {
        Ok(g) => {
            state.session.status = format!("GDB ({}) 已启动", gdb_binary);
            g
        }
        Err(e) => {
            eprintln!("❌ 启动 GDB 失败: {}", e);
            eprintln!("   但 GDB Server 仍在 localhost:{} 上运行", port);
            return Ok(TuiExit::Flashed);
        }
    };

    // 先连远程目标，再加载 ELF 符号（避免架构不匹配的卡死）
    gdb_mi.send_command(&format!("target-select remote :{}", port));
    gdb_mi.load_elf();

    // ---- 5. TTY 检查 ----
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("❌ TUI 调试需要终端环境");
        eprintln!("   GDB Server 仍在 localhost:{} 上运行", port);
        return Ok(TuiExit::Flashed);
    }

    // ---- 6. 终端初始化 ----
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend_t = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend_t)?;

    // ---- 7. 主循环 ----
    loop {
        terminal.draw(|f| {
            debug_ui::render(f, &state, f.area());
        })?;

        if state.should_quit {
            break;
        }

        // 非阻塞读取 GDB 响应
        while let Ok(record) = rx.try_recv() {
            state.session.handle_record(&record);

            if record.is_stopped() {
                gdb_mi.send_command("stack-list-frames");
                gdb_mi.send_command("stack-list-variables --all-values");

                // 更新监视表达式
                let watches: Vec<String> = state
                    .session
                    .watches
                    .iter()
                    .filter_map(|(expr, _)| {
                        if expr.starts_with("输入") {
                            None
                        } else {
                            Some(expr.clone())
                        }
                    })
                    .collect();
                for w in &watches {
                    let cmd = format!("data-evaluate-expression \"{}\"", w);
                    gdb_mi.send_command(&cmd);
                }
            }

            if record.is_ok() {
                state.session.update_breakpoints_from_response(&record);
                state.session.update_frames_from_response(&record);
                state.session.update_variables_from_response(&record);
            }
        }

        if gdb_mi.is_stopped() {
            state.session.status = "GDB 会话已结束".into();
            state.session.terminated = true;
        }

        // 事件处理（带超时）
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let exit_to_flash = handle_debug_key(&mut state, &mut gdb_mi, key.code);
                    if exit_to_flash {
                        break;
                    }
                }
            }
        }
    }

    // ---- 8. 清理 ----
    gdb_mi.shutdown();
    drop(gdb_server); // kill GDB Server before cleaning terminal

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(TuiExit::Flashed)
}

/// 调试模式按键处理
/// 返回 true 表示请求切换回烧录模式
fn handle_debug_key(
    state: &mut DebugAppState,
    gdb: &mut crate::debug::GdbMi,
    code: crossterm::event::KeyCode,
) -> bool {
    use crossterm::event::KeyCode;
    match code {
        // Shift+F5（BackTab）或 Esc 回到烧录模式
        KeyCode::BackTab | KeyCode::Esc => {
            return true; // 回到烧录模式
        }
        KeyCode::Char('q') => {
            state.should_quit = true;
        }
        KeyCode::F(5) | KeyCode::Enter => {
            gdb.send_command("exec-continue");
            state.session.console.push("> exec-continue".into());
        }
        KeyCode::F(6) | KeyCode::F(10) => {
            gdb.send_command("exec-next");
            state.session.console.push("> exec-next".into());
        }
        KeyCode::F(7) | KeyCode::F(11) => {
            gdb.send_command("exec-step");
            state.session.console.push("> exec-step".into());
        }
        KeyCode::F(8) => {
            gdb.send_command("exec-finish");
            state.session.console.push("> exec-finish".into());
        }
        KeyCode::F(12) => {
            gdb.interrupt();
            state.session.console.push("> [SIGINT]".into());
        }
        KeyCode::F(9) => {
            gdb.send_command("break-list");
        }
        _ => {}
    }
    false
}
