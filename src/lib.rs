//! loading-chip 🔥 — 嵌入式芯片烧录/调试 TUI 工具
//!
//! 通过 TUI 界面收集烧录参数，自动调用 arm-none-eabi-gdb / OpenOCD / probe-rs 完成固件烧录。
//! 同时提供命令行模式、无头模式和 RTT 实时监视。
//!
//! ## 用法
//! ```text
//! loading-chip run              → TUI 交互模式（烧录 + RTT 监视）
//! loading-chip run --headless   → 无头模式，JSON 输出（供 IDE 调用）
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

mod app;
mod backend;
mod board;
mod chip_detect;
mod cli;
mod config;
mod debug;
mod presets;
mod setup;
mod tui;

use std::io;
use std::sync::Arc;

use app::state::AppState;
use backend::FlashResult;
use chip_detect::run_detect;
use clap::Parser;

/// 程序入口 — 解析 CLI 参数并分发到对应模式
pub fn run() -> io::Result<()> {
    let cli = cli::Cli::parse();

    // 加载板子配置（全局单例，TUI/CLI/Headless 共享）
    let registry = board::BoardRegistry::load().map_err(io::Error::other)?;
    let state = Arc::new(AppState::new(registry));

    let exit_code = match cli.command {
        None => {
            run_tui_default(state)?;
            0
        }
        Some(cli::Commands::Run {
            backend,
            interface,
            target,
            elf,
            gdb_port,
            pyocd_path,
            headless,
            timeout,
        }) => run_flash(
            state, backend, interface, target, elf, gdb_port, pyocd_path, headless, timeout,
        )?,
        Some(cli::Commands::Debug {
            elf,
            target,
            backend,
            interface,
            port,
            gdb,
        }) => {
            run_debug(
                elf,
                target,
                backend,
                interface.unwrap_or_default(),
                port,
                gdb.unwrap_or_default(),
            )?;
            0
        }
        Some(cli::Commands::Init { force, output }) => {
            setup::run_init(force, output.as_deref())?;
            0_i32
        }
        Some(cli::Commands::Detect {}) => {
            run_detect()?;
            0_i32
        }
    };

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

// ============================================================================
// 烧录模式
// ============================================================================

/// 循环运行 TUI（烧录 ↔ 调试互相切换，直到用户退出）
fn run_tui_loop(
    gdb_port: String,
    pyocd_path: String,
    timeout_secs: u64,
    state: Arc<AppState>,
) -> io::Result<()> {
    let current_gdb_port = gdb_port;
    let current_pyocd = pyocd_path;
    let current_timeout = timeout_secs;
    let mut saved_app: Option<tui::app::App> = None;

    loop {
        let (exit, new_app) = tui::run_with_resume(
            current_gdb_port.clone(),
            current_pyocd.clone(),
            current_timeout,
            saved_app.take(),
            state.clone(),
            None,
        )?;

        match exit {
            tui::TuiExit::Quit => break,
            tui::TuiExit::Flashed => {
                saved_app = new_app;
            }
            tui::TuiExit::DebugRequested {
                elf,
                target,
                backend,
                interface,
                port,
                gdb,
            } => {
                saved_app = new_app;
                match tui::run_debug(elf, target, backend, interface, port, gdb)? {
                    tui::TuiExit::Quit => break,
                    tui::TuiExit::Flashed | tui::TuiExit::DebugRequested { .. } => {}
                }
            }
        }
    }
    Ok(())
}

fn run_tui_default(state: Arc<AppState>) -> io::Result<()> {
    let detected = chip_detect::detect_chips();
    if !detected.is_empty() {
        eprintln!("检测到 {} 个设备:", detected.len());
        for d in &detected {
            eprintln!("  - {} (芯片: {})", d.probe_name, d.chip_name);
        }
    }

    // Pass detected chips into first TUI launch for auto-fill
    let gdb_port = "3333".to_string();
    let pyocd_path = String::new();
    let timeout = 60u64;
    let mut saved_app: Option<tui::app::App> = None;

    loop {
        let initial_detected = if saved_app.is_none() { Some(detected.clone()) } else { None };
        let (exit, new_app) = tui::run_with_resume(
            gdb_port.clone(),
            pyocd_path.clone(),
            timeout,
            saved_app.take(),
            state.clone(),
            initial_detected,
        )?;

        match exit {
            tui::TuiExit::Quit => break,
            tui::TuiExit::Flashed => {
                saved_app = new_app;
            }
            tui::TuiExit::DebugRequested {
                elf,
                target,
                backend,
                interface,
                port,
                gdb,
            } => {
                saved_app = new_app;
                match tui::run_debug(elf, target, backend, interface, port, gdb)? {
                    tui::TuiExit::Quit => break,
                    tui::TuiExit::Flashed | tui::TuiExit::DebugRequested { .. } => {}
                }
            }
        }
    }
    Ok(())
}

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
) -> io::Result<i32> {

    if headless {
        return run_headless(
            &state, backend, interface, target, elf, gdb_port, pyocd_path, timeout,
        );
    }

    // 命令行模式：全参数 → 跳过 TUI
    if let (Some(i), Some(t), Some(e)) = (&interface, &target, &elf) {
        return run_cli_mode(&state, backend, i, t, e, gdb_port, pyocd_path, timeout);
    }

    // TUI 模式
    run_tui_loop(gdb_port, pyocd_path, timeout, state)?;
    Ok(0)
}

#[allow(clippy::too_many_arguments)]
fn run_headless(
    state: &AppState,
    backend: String,
    interface: Option<String>,
    target: Option<String>,
    elf: Option<String>,
    gdb_port: String,
    pyocd_path: String,
    timeout: u64,
) -> io::Result<i32> {
    let (i, t, e) = match (interface, target, elf) {
        (Some(i), Some(t), Some(e)) => (i, t, e),
        _ => {
            println!(
                "{}",
                serde_json::to_string_pretty(&FlashResult {
                    success: false,
                    message: "无头模式需要提供 -i, -t, -e 全部参数".to_string(),
                    command: String::new(),
                    stdout: None,
                    stderr: None,
                })
                .unwrap()
            );
            return Ok(1);
        }
    };

    let result = state.flash(&backend, &t, &i, &e, &gdb_port, &pyocd_path, timeout);
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    Ok(if result.success { 0 } else { 1 })
}

#[allow(clippy::too_many_arguments)]
fn run_cli_mode(
    state: &AppState,
    backend: String,
    interface: &str,
    target: &str,
    elf: &str,
    gdb_port: String,
    pyocd_path: String,
    timeout: u64,
) -> io::Result<i32> {
    let result = state.flash(&backend, target, interface, elf, &gdb_port, &pyocd_path, timeout);

    println!("{}", result.message);
    if let Some(ref stdout) = result.stdout
        && !stdout.trim().is_empty()
    {
        println!("\n--- 输出 ---\n{}", stdout);
    }
    if let Some(ref stderr) = result.stderr
        && !stderr.trim().is_empty()
    {
        eprintln!("\n--- 错误 ---\n{}", stderr);
    }
    Ok(if result.success { 0 } else { 1 })
}

// ============================================================================
// 调试模式
// ============================================================================

fn run_debug(
    elf: String,
    target: String,
    backend: String,
    interface: String,
    port: u16,
    gdb: String,
) -> io::Result<()> {
    match tui::run_debug(elf, target, backend, interface, port, gdb)? {
        tui::TuiExit::Quit => {}
        tui::TuiExit::Flashed | tui::TuiExit::DebugRequested { .. } => {}
    }
    Ok(())
}
