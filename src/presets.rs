//! 嵌入式芯片烧录预设数据
//!
//! 提供调试接口（interface）和目标芯片（target）的预设列表，
//! 使用通俗易懂的嵌入式俗语描述。

/// 烧录后端预设
/// 每个元素: (key, 描述)
pub const BACKENDS: &[(&str, &str)] = &[
    (
        "gdb",
        "arm-none-eabi-gdb（GDB 通用烧录，需 OpenOCD/pyOCD 等 GDB Server）",
    ),
    ("openocd", "OpenOCD（开源调试器，芯片支持广泛，社区活跃）"),
    (
        "probe-rs",
        "probe-rs（Rust 原生烧录工具，无需外部 GDB Server）",
    ),
    (
        "pyocd",
        "pyOCD（Python 烧录/调试工具，CMSIS-Pack 生态，支持自定义脚本）",
    ),
];

/// 调试器/调试接口预设
/// 每个元素: (key, 描述)
pub const INTERFACES: &[(&str, &str)] = &[
    ("stlink", "ST-Link（ST 官方调试器，SWD 协议，STM32 首选）"),
    ("jlink", "J-Link（SEGGER 专业调试器，JTAG/SWD 双协议）"),
    ("cmsis-dap", "CMSIS-DAP（ARM 标准调试固件，SWD 协议）"),
    ("daplink", "DAP-Link（开源 CMSIS-DAP 实现，SWD 协议）"),
    ("xds110", "XDS110（TI 官方调试器，SWD 协议，MSPM0/MSP430 首选）"),
    ("swd", "SWD 协议 — 默认使用 CMSIS-DAP 探针（如不确定探针型号选此项）"),
    ("jtag", "JTAG 协议 — 默认使用 J-Link 探针（如不确定探针型号选此项）"),
];

/// 目标芯片预设
/// 每个元素: (key, 描述)
pub const TARGETS: &[(&str, &str)] = &[
    ("stm32f1", "STM32F103 (Cortex-M3, 72MHz, 国产平替多)"),
    ("stm32f4", "STM32F407/429 (Cortex-M4, 168/180MHz, 带 FPU/DSP)"),
    ("stm32h7", "STM32H743/750 (Cortex-M7, 480MHz)"),
    ("stm32g0", "STM32G0 (Cortex-M0+, 低功耗入门)"),
    ("esp32", "ESP32 (Xtensa 双核, WiFi/BLE, 需外接调试器)"),
    ("esp32s3", "ESP32-S3 (Xtensa 双核, 内置 USB-JTAG)"),
    ("esp32c3", "ESP32-C3 (RISC-V 单核, 内置 USB-JTAG)"),
    ("rp2040", "RP2040 (Cortex-M0+ 双核, 133MHz, 树莓派芯片)"),
    ("nrf52", "nRF52840 (Cortex-M4, BLE 5.0, Nordic 低功耗)"),
    ("gd32", "GD32F303/350 (Cortex-M4)"),
    ("at32", "AT32F403A/407 (Cortex-M4, 雅特力)"),
    ("mspm0g3507", "MSPM0G3507 (Cortex-M0+, 80MHz, TI 混合信号 MCU)"),
];

/// 获取所有后端 key 列表
pub fn backend_keys() -> Vec<String> {
    BACKENDS.iter().map(|(k, _)| k.to_string()).collect()
}

/// 获取所有接口 key 列表（供下拉菜单渲染）
pub fn iface_keys() -> Vec<String> {
    INTERFACES.iter().map(|(k, _)| k.to_string()).collect()
}

/// 获取所有芯片 key 列表
pub fn target_keys() -> Vec<String> {
    TARGETS.iter().map(|(k, _)| k.to_string()).collect()
}
