//! 芯片/接口/错误特征映射表
//!
//! 所有后端共享的映射数据集中管理，避免重复硬编码。

// ============================================================================
// OpenOCD 映射
// ============================================================================

/// 接口 key → OpenOCD interface 配置文件
pub fn openocd_interface_cfg(interface: &str) -> &'static str {
    match interface {
        "stlink" => "interface/stlink.cfg",
        "jlink" => "interface/jlink.cfg",
        "cmsis-dap" | "daplink" => "interface/cmsis-dap.cfg",
        "swd" => "interface/cmsis-dap.cfg",
        "jtag" => "interface/jlink.cfg",
        _ => "interface/cmsis-dap.cfg",
    }
}

/// 芯片 key → OpenOCD target/board 配置文件
pub fn openocd_target_cfg(target: &str) -> &'static str {
    match target {
        "stm32f1" => "target/stm32f1x.cfg",
        "stm32f4" => "target/stm32f4x.cfg",
        "stm32h7" => "target/stm32h7x.cfg",
        "stm32g0" => "target/stm32g0x.cfg",
        "esp32" => "board/esp32-wrover-kit-3.3v.cfg",
        "esp32s3" => "board/esp32s3-builtin.cfg",
        "esp32c3" => "board/esp32c3-builtin.cfg",
        "rp2040" => "target/rp2040.cfg",
        "nrf52" => "target/nrf52.cfg",
        "gd32" => "target/stm32f1x.cfg",
        "at32" => "target/stm32f4x.cfg",
        _ => "target/stm32f1x.cfg",
    }
}

// ============================================================================
// probe-rs 映射
// ============================================================================

/// 芯片 key → probe-rs chip name
pub fn probe_rs_chip(target: &str) -> &str {
    match target {
        "stm32f1" => "STM32F103C8",
        "stm32f4" => "STM32F407VG",
        "stm32h7" => "STM32H743ZI",
        "stm32g0" => "STM32G030F6",
        "esp32" => "ESP32",
        "esp32s3" => "ESP32S3",
        "esp32c3" => "ESP32C3",
        "rp2040" => "RP2040",
        "nrf52" => "nRF52840_xxAA",
        "gd32" => "GD32F303ZE",
        "at32" => "AT32F403AVGT7",
        _ => target,
    }
}

// ============================================================================
// GDB 二进制文件映射
// ============================================================================

/// 芯片 key → 默认 GDB 可执行文件
/// 不同架构需要不同的 GDB，ARM 用 arm-none-eabi-gdb，Xtensa 用 xtensa-*-elf-gdb
/// 返回列表，优先级从高到低，前面的找不到时会尝试后面的
pub fn gdb_binary_candidates(target: &str) -> &[&str] {
    match target {
        "stm32f1" | "stm32f4" | "stm32h7" | "stm32g0"
        | "rp2040" | "nrf52" | "gd32" | "at32" => {
            &["arm-none-eabi-gdb"]
        }
        "esp32" => &["xtensa-esp32-elf-gdb", "arm-none-eabi-gdb"],
        "esp32s2" => &["xtensa-esp32s2-elf-gdb", "arm-none-eabi-gdb"],
        "esp32s3" => &["xtensa-esp32s3-elf-gdb", "arm-none-eabi-gdb"],
        "esp32c3" | "esp32c2" | "esp32c6" | "esp32h2" => {
            &["riscv32-esp-elf-gdb", "arm-none-eabi-gdb"]
        }
        _ => &["arm-none-eabi-gdb"],
    }
}

/// 解析实际可用的 GDB 二进制路径
/// 返回 `(二进制路径, 是否为首选)`，`is_preferred=false` 表示是回退候选
/// 回退的 GDB（如 arm-none-eabi-gdb 读 Xtensa ELF）会导致符号加载失败
pub fn resolve_gdb_binary(target: &str) -> Option<(String, bool)> {
    let candidates = gdb_binary_candidates(target);
    for (i, candidate) in candidates.iter().enumerate() {
        let is_preferred = i == 0;
        // 1. 检查 PATH
        for dir in std::env::var_os("PATH")
            .unwrap_or_default()
            .to_str()
            .unwrap_or("")
            .split(':') {
            let path = format!("{}/{}", dir, candidate);
            if std::path::Path::new(&path).is_file() {
                return Some((path, is_preferred));
            }
        }
        // 2. 检查 ~/.cargo/bin/
        if let Some(home) = std::env::var_os("HOME") {
            let path = format!("{}/.cargo/bin/{}", home.to_string_lossy(), candidate);
            if std::path::Path::new(&path).is_file() {
                return Some((path, is_preferred));
            }
        }
        // 3. which 命令
        if let Ok(out) = std::process::Command::new("which").arg(candidate).output() {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some((path, is_preferred));
                }
            }
        }
    }
    None
}

// ============================================================================
// pyOCD 映射
// ============================================================================

/// 芯片 key → pyOCD target name（CMSIS-Pack 格式，全小写）
pub fn pyocd_target(target: &str) -> &str {
    match target {
        "stm32f1" => "stm32f103c8",
        "stm32f4" => "stm32f407vg",
        "stm32h7" => "stm32h743zi",
        "stm32g0" => "stm32g030f6",
        "esp32" => "esp32",
        "esp32s3" => "esp32s3",
        "esp32c3" => "esp32c3",
        "rp2040" => "rp2040",
        "nrf52" => "nrf52840",
        "gd32" => "gd32f303ze",
        "at32" => "at32f403avgt7",
        _ => target,
    }
}

// ============================================================================
// 错误特征检测（多后端通用 + 各后端特有）
// ============================================================================

/// 致命错误特征模式表
/// 每个元素: (匹配字符串, 中文说明)
pub static FATAL_ERROR_PATTERNS: &[(&str, &str)] = &[
    // ---- 连接错误（通用） ----
    ("could not connect", "无法连接到调试目标"),
    ("Connection timed out", "连接超时"),
    ("连接超时", "连接超时"),
    ("Connection refused", "连接被拒绝"),
    ("拒绝连接", "连接被拒绝"),
    ("Error: Failed to attach", "无法连接到调试探针"),
    ("USB device not found", "未找到 USB 调试探针"),
    ("No probes found", "未检测到调试探针"),
    ("Error attaching to the chip", "无法连接目标芯片"),
    // ---- OpenOCD 接口不匹配 ----
    (
        "unable to find a matching",
        "调试器接口配置不匹配：请确认选择的接口类型与实际硬件一致",
    ),
    // ---- 文件/格式错误（通用） ----
    ("No such file or directory", "ELF 文件路径无效"),
    ("没有那个文件或目录", "ELF 文件路径无效"),
    ("No executable file specified", "未指定可执行文件"),
    ("not in executable format", "文件不是合法的可执行格式"),
    // ---- OpenOCD 特有 ----
    ("Error: open failed", "无法打开接口配置文件"),
    ("Error: Translation", "OpenOCD 配置翻译错误"),
    ("invalid command name", "OpenOCD 命令无效"),
    ("target not halted", "目标芯片未暂停"),
    // ---- probe-rs 特有 ----
    ("The firmware on the probe is outdated", "调试探针固件过旧"),
    ("IO error while using", "调试探针通信出错"),
    // ---- pyOCD 特有 ----
    ("No target connected", "未检测到目标芯片连接"),
    ("No debug probe detected", "未检测到调试探针"),
    ("Failed to connect to target", "连接目标芯片失败"),
    ("Target type not recognized", "pyOCD 不支持此芯片型号"),
    ("Cannot open", "无法打开文件或设备"),
    ("Permission denied", "权限不足，请检查 udev 规则或使用 sudo"),
    ("Waiting for a debug probe", "未检测到调试探针（探针不在线或芯片不受支持）"),
    // ---- GDB 特有 ----
    ("Remote communication error", "远程通信错误"),
    (
        "\"monitor\" command not supported",
        "目标不支持 monitor 命令",
    ),
];

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openocd_mappings() {
        assert_eq!(openocd_interface_cfg("stlink"), "interface/stlink.cfg");
        assert_eq!(openocd_interface_cfg("jlink"), "interface/jlink.cfg");
        assert_eq!(
            openocd_interface_cfg("cmsis-dap"),
            "interface/cmsis-dap.cfg"
        );
        assert_eq!(openocd_target_cfg("stm32f4"), "target/stm32f4x.cfg");
        assert_eq!(openocd_target_cfg("rp2040"), "target/rp2040.cfg");
        assert_eq!(openocd_target_cfg("esp32s3"), "board/esp32s3-builtin.cfg");
    }

    #[test]
    fn probe_rs_mappings() {
        assert_eq!(probe_rs_chip("stm32f4"), "STM32F407VG");
        assert_eq!(probe_rs_chip("rp2040"), "RP2040");
        assert_eq!(probe_rs_chip("esp32s3"), "ESP32S3");
    }

    #[test]
    fn pyocd_mappings() {
        assert_eq!(pyocd_target("stm32f4"), "stm32f407vg");
        assert_eq!(pyocd_target("stm32f1"), "stm32f103c8");
        assert_eq!(pyocd_target("rp2040"), "rp2040");
        assert_eq!(pyocd_target("nrf52"), "nrf52840");
        assert_eq!(pyocd_target("esp32"), "esp32");
    }
}
