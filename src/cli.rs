//! CLI 参数解析
//!
//! 使用 clap derive 定义命令行接口，支持烧录和调试两种模式。

use clap::Parser;

/// 嵌入式芯片烧录/调试工具
#[derive(Parser, Debug)]
#[command(
    name = "loading-chip",
    version,
    about = "嵌入式芯片烧录/调试 TUI 工具",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Commands {
    /// 运行烧录（默认命令：启动 TUI，或 --headless 输出 JSON，或 --api 启动 HTTP 服务）
    Run {
        /// 烧录后端: gdb, openocd, probe-rs, pyocd（默认 gdb）
        #[arg(short = 'b', long, default_value = "gdb", value_name = "后端",
              value_parser = ["gdb", "openocd", "probe-rs", "pyocd"])]
        backend: String,

        /// 调试接口: swd, jtag, stlink, jlink, cmsis-dap, daplink
        #[arg(short = 'i', long, value_name = "接口")]
        interface: Option<String>,

        /// 目标芯片: stm32f1, stm32f4, stm32h7, esp32, rp2040, nrf52, gd32, at32
        #[arg(short = 't', long, value_name = "芯片")]
        target: Option<String>,

        /// ELF 固件文件路径
        #[arg(short = 'e', long, value_name = "文件")]
        elf: Option<String>,

        /// GDB 远程端口（默认 3333）
        #[arg(short = 'p', long, default_value = "3333", value_name = "端口")]
        gdb_port: String,

        /// pyOCD 可执行文件路径（如安装在 venv 中：<venv>/bin/pyocd）
        #[arg(long, default_value = "", value_name = "路径")]
        pyocd_path: String,

        /// 无头模式：跳过 TUI，输出 JSON 结果（供 IDE 调用）
        #[arg(long)]
        headless: bool,

        /// 超时时间（秒），默认 60。0 表示无超时
        #[arg(long, default_value = "60", value_name = "秒")]
        timeout: u64,
    },

    /// 初始化环境：检测本地可用的后端工具并生成用户配置文件
    Init {
        /// 强制重新检测，覆盖已有配置
        #[arg(long)]
        force: bool,

        /// 输出路径（默认 ~/.config/loading-chip/config.yaml）
        #[arg(long, value_name = "路径")]
        output: Option<String>,
    },

    /// 调试模式：启动 RTT 实时监视器（支持 probe-rs / OpenOCD / pyOCD / GDB）
    Debug {
        /// ELF 固件文件路径（必填）
        #[arg(short = 'e', long, value_name = "文件")]
        elf: String,

        /// 目标芯片: stm32f1, stm32f4, stm32h7, esp32, rp2040, nrf52, gd32, at32
        #[arg(short = 't', long, default_value = "stm32f4", value_name = "芯片")]
        target: String,

        /// 烧录后端: probe-rs, openocd, gdb, pyocd（默认 probe-rs）
        #[arg(short = 'b', long, default_value = "probe-rs", value_name = "后端",
              value_parser = ["gdb", "openocd", "probe-rs", "pyocd"])]
        backend: String,

        /// 调试接口: swd, jtag, stlink, jlink, cmsis-dap, daplink
        #[arg(short = 'i', long, value_name = "接口")]
        interface: Option<String>,

        /// GDB Server 端口（默认 3333）
        #[arg(short = 'p', long, default_value = "3333", value_name = "端口")]
        port: u16,

        /// GDB 可执行文件路径（留空则根据目标架构自动选择 GDB）
        #[arg(short = 'g', long, value_name = "GDB路径")]
        gdb: Option<String>,
    },
    Detect {},
}
