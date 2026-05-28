//! loading-chip 🔥 — 嵌入式芯片烧录/调试 TUI 工具
//!
//! 通过 TUI 界面收集烧录参数，自动调用 arm-none-eabi-gdb / OpenOCD / probe-rs 完成固件烧录。
//! 同时提供命令行模式和 dap-ui 风格调试界面。
//!
//! ## 用法
//! ```text
//! loading-chip run              → 启动 TUI 交互模式
//! loading-chip run --headless   → 无头模式，JSON 输出（供 IDE 调用）
//! loading-chip debug -e <ELF>   → 调试模式，dap-ui 界面
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
//! swd, jtag, stlink, jlink, cmsis-dap, daplink
//!
//! ## 支持的目标芯片
//! stm32f1, stm32f4, stm32h7, stm32g0, esp32, esp32s3, esp32c3,
//! rp2040, nrf52, gd32, at32

mod api;
mod app;
mod backend;
mod board;
mod chip_detect;
mod cli;
mod config;
mod debug;
mod flash;
mod presets;
mod setup;
mod tui;

use std::io;
use std::sync::Arc;

use clap::Parser;
use flash::{do_flash, FlashBackend, FlashConfig};

/// 程序入口 — 解析 CLI 参数并分发到对应模式
pub fn run() -> io::Result<()> {
    let cli = cli::Cli::parse();

    // 加载板子配置
    let registry = board::BoardRegistry::load()
        .map_err(io::Error::other)?;
    eprintln!("📋 已加载 {} 块板子配置", registry.len());

    match cli.command {
        None => run_tui_default(Arc::new(registry))?,
        Some(cli::Commands::Run {
            backend,
            interface,
            target,
            elf,
            gdb_port,
            pyocd_path,
            headless,
            timeout,
            api,
            api_addr,
        }) => run_flash(Arc::new(registry), backend, interface, target, elf, gdb_port, pyocd_path, headless, timeout, api, api_addr)?,
        Some(cli::Commands::Debug {
            elf,
            target,
            backend,
            interface,
            port,
            gdb,
        }) => run_debug(elf, target, backend, interface.unwrap_or_default(), port, gdb.unwrap_or_default())?,
        Some(cli::Commands::Init { force, output }) => {
            setup::run_init(force, output.as_deref())?;
        }
    }

    Ok(())
}

// ============================================================================
// 烧录模式
// ============================================================================

/// 循环运行 TUI（烧录 ↔ 调试互相切换，直到用户退出）
/// 保留用户选择的状态，从调试返回后不丢失之前的设置
fn run_tui_loop(
    gdb_port: String,
    pyocd_path: String,
    timeout_secs: u64,
    detected_chips: Vec<chip_detect::DetectedChip>,
    registry: Arc<board::BoardRegistry>,
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
            detected_chips.clone(),
            registry.clone(),
        )?;

        match exit {
            tui::TuiExit::Quit => break,
            tui::TuiExit::Flashed => {
                // 烧录完成 → 保留状态继续
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
                // 进调试前保存当前 TUI 状态
                saved_app = new_app;

                // 切换到调试 TUI
                match tui::run_debug(elf, target, backend, interface, port, gdb)? {
                    tui::TuiExit::Quit => break,
                    tui::TuiExit::Flashed | tui::TuiExit::DebugRequested { .. } => {
                        // 回到烧录模式，saved_app 里保留了之前的设置
                    }
                }
            }
        }
    }
    Ok(())
}

fn run_tui_default(registry: Arc<board::BoardRegistry>) -> io::Result<()> {
    let detected = chip_detect::detect_chips();
    if !detected.is_empty() {
        eprintln!("检测到 {} 个设备:", detected.len());
        for d in &detected {
            eprintln!("  - {} (芯片: {})", d.probe_name, d.chip_name);
        }
    }
    run_tui_loop("3333".to_string(), String::new(), 60, detected, registry)
}

#[allow(clippy::too_many_arguments)]
fn run_flash(
    registry: Arc<board::BoardRegistry>,
    backend: String,
    interface: Option<String>,
    target: Option<String>,
    elf: Option<String>,
    gdb_port: String,
    pyocd_path: String,
    headless: bool,
    timeout: u64,
    api: bool,
    api_addr: String,
) -> io::Result<()> {
    let be = FlashBackend::from_str(&backend);

    // 启动 API 服务（axum）
    let _api_shutdown = if api {
        let state = app::state::AppState::new((*registry).clone());
        let shutdown = api::spawn_server(state, api_addr.clone());
        Some(shutdown)
    } else {
        None
    };

    if headless {
        if _api_shutdown.is_some() {
            // 无头模式 + API：阻塞主线程，等待 Ctrl+C
            eprintln!("🟢 API 运行中（按 Ctrl+C 退出）...");
            std::thread::park();
        } else {
            return run_headless(&*registry, be, interface, target, elf, gdb_port, pyocd_path, timeout);
        }
        return Ok(());
    }

    // 命令行模式：全参数 → 跳过 TUI
    if let (Some(i), Some(t), Some(e)) = (&interface, &target, &elf) {
        return run_cli_mode(&*registry, be, i, t, e, gdb_port, pyocd_path, timeout);
    }

    // TUI 模式（支持烧录 ↔ 调试切换）
    let detected = chip_detect::detect_chips();
    run_tui_loop(gdb_port, pyocd_path, timeout, detected, registry)
}

#[allow(clippy::too_many_arguments)]
fn run_headless(
    registry: &board::BoardRegistry,
    be: FlashBackend,
    interface: Option<String>,
    target: Option<String>,
    elf: Option<String>,
    gdb_port: String,
    pyocd_path: String,
    timeout: u64,
) -> io::Result<()> {
    match (interface, target, elf) {
        (Some(i), Some(t), Some(e)) => {
            let backend_name = be.yaml_key();
            let resolved = match registry.resolve(&t, backend_name) {
                Ok(p) => p,
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
            };
            let config = FlashConfig {
                backend: be,
                interface: i,
                target: resolved.target,
                elf_path: e,
                gdb_port,
                pyocd_path,
                timeout_secs: timeout,
                board_config: resolved.config,
                board_extra_args: resolved.extra_args,
                board_id: t.clone(),
            };
            let result = do_flash(&config);
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            std::process::exit(if result.success { 0 } else { 1 });
        }
        _ => {
            let err = flash::FlashResult {
                success: false,
                message: "无头模式需要提供 -i, -t, -e 全部参数".to_string(),
                command: String::new(),
                stdout: None,
                stderr: None,
            };
            println!("{}", serde_json::to_string_pretty(&err).unwrap());
            std::process::exit(1);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_cli_mode(
    registry: &board::BoardRegistry,
    be: FlashBackend,
    interface: &str,
    target: &str,
    elf: &str,
    gdb_port: String,
    pyocd_path: String,
    timeout: u64,
) -> io::Result<()> {
    let backend_name = be.yaml_key();
    let resolved = registry.resolve(target, backend_name);
    let config = match resolved {
        Ok(params) => FlashConfig {
            backend: be,
            interface: interface.to_string(),
            target: params.target,
            elf_path: elf.to_string(),
            gdb_port,
            pyocd_path,
            timeout_secs: timeout,
            board_config: params.config,
            board_extra_args: params.extra_args,
            board_id: target.to_string(),
        },
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    };
    let result = do_flash(&config);
    println!("{}", result.message);
    if let Some(ref stdout) = result.stdout {
        if !stdout.trim().is_empty() {
            println!("\n--- 输出 ---\n{}", stdout);
        }
    }
    if let Some(ref stderr) = result.stderr {
        if !stderr.trim().is_empty() {
            eprintln!("\n--- 错误 ---\n{}", stderr);
        }
    }
    std::process::exit(if result.success { 0 } else { 1 });
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
        tui::TuiExit::Flashed | tui::TuiExit::DebugRequested { .. } => {
            // 从调试模式返回，继续到 run_tui_loop
        }
    }
    Ok(())
}
