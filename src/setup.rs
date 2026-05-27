//! 环境初始化 — 小白模式
//!
//! `loading-chip init` 扫描本地环境中的可用后端工具，
//! 检测路径、版本，并写入 `~/.config/loading-chip/config.yaml`。
//!
//! 检测范围：
//! - probe-rs（PATH + ~/.cargo/bin/）
//! - OpenOCD（PATH）
//! - pyOCD（PATH + PYOCD_PATH + 常见 venv 路径）
//! - arm-none-eabi-gdb / xtensa-*-elf-gdb / riscv32-esp-elf-gdb（PATH + 常见路径）
//! - 已连接的调试探针（probe-rs list 输出解析）

use std::process::Command;

use crate::config::{
    BackendDetection, BackendInfo, GdbDetection, ProbeInfo, UserConfig, Versions,
};

/// 运行 init，返回生成的配置
pub fn run_init(force: bool, output: Option<&str>) -> std::io::Result<UserConfig> {
    let default_path = UserConfig::default_path().to_string_lossy().to_string();
    let path: &str = output.unwrap_or(&default_path);

    // 检查是否已存在
    if !force && std::path::Path::new(&path).exists() {
        eprintln!("⚠️  配置文件已存在: {}", path);
        eprintln!("   使用 --force 重新检测覆盖");
        let existing = UserConfig::load()
            .unwrap_or_else(UserConfig::empty);
        return Ok(existing);
    }

    eprintln!("🔍 正在检测本地环境...");

    let probe_rs = detect_probe_rs();
    let openocd = detect_openocd();
    let pyocd = detect_pyocd();
    let gdb = detect_gdb();
    let probes = detect_probes();

    let config = UserConfig {
        backends: BackendDetection {
            probe_rs,
            openocd,
            pyocd,
            gdb,
        },
        probes,
        versions: Versions {
            loading_chip: env!("CARGO_PKG_VERSION").to_string(),
        },
    };

    // 保存
    if let Some(out) = output {
        config.save_to(out)?;
    } else {
        config.save()?;
    }

    print_summary(&config, path);
    Ok(config)
}

impl UserConfig {
    fn empty() -> Self {
        Self {
            backends: BackendDetection {
                probe_rs: None,
                openocd: None,
                pyocd: None,
                gdb: GdbDetection {
                    arm: None,
                    xtensa_esp32: None,
                    xtensa_esp32s2: None,
                    xtensa_esp32s3: None,
                    riscv_esp: None,
                },
            },
            probes: Vec::new(),
            versions: Versions {
                loading_chip: env!("CARGO_PKG_VERSION").to_string(),
            },
        }
    }
}

// ============================================================================
// 后端检测
// ============================================================================

fn detect_probe_rs() -> Option<BackendInfo> {
    let path = resolve_binary("probe-rs")?;
    let version = run_version(&path, &["--version"])?;
    Some(BackendInfo { path, version })
}

fn detect_openocd() -> Option<BackendInfo> {
    let path = resolve_binary("openocd")?;
    let version = run_version(&path, &["--version"])?;
    Some(BackendInfo { path, version })
}

fn detect_pyocd() -> Option<BackendInfo> {
    // 检测顺序：PYOCD_PATH → PATH → ~/.venvs/pyocd/bin/pyocd
    let path = resolve_pyocd_path().or_else(|| resolve_binary("pyocd"))?;
    // pyOCD --version 需要一段时间
    let version = run_version(&path, &["--version"])?;
    Some(BackendInfo { path, version })
}

/// 检测 GDB 系列
fn detect_gdb() -> GdbDetection {
    GdbDetection {
        arm: detect_gdb_one("arm-none-eabi-gdb"),
        xtensa_esp32: detect_gdb_one("xtensa-esp32-elf-gdb"),
        xtensa_esp32s2: detect_gdb_one("xtensa-esp32s2-elf-gdb"),
        xtensa_esp32s3: detect_gdb_one("xtensa-esp32s3-elf-gdb"),
        riscv_esp: detect_gdb_one("riscv32-esp-elf-gdb"),
    }
}

fn detect_gdb_one(name: &str) -> Option<BackendInfo> {
    let path = resolve_binary(name)?;
    let version = run_version(&path, &["--version"])?;
    Some(BackendInfo { path, version })
}

// ============================================================================
// 探针检测
// ============================================================================

/// 解析 probe-rs list 输出提取探针信息
fn detect_probes() -> Vec<ProbeInfo> {
    let binary = resolve_binary("probe-rs").unwrap_or_else(|| "probe-rs".into());
    let output = match Command::new(&binary).arg("list").output() {
        Ok(out) => out,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    parse_probe_list(&text)
}

fn parse_probe_list(text: &str) -> Vec<ProbeInfo> {
    let mut probes = Vec::new();
    for line in text.lines() {
        // probe-rs list 输出: "[0]: STLink -- 1234:5678:ABC (StLink)"
        // 或: "[0]: ESP JTAG -- 303a:1001:... (EspJtag)"
        if !line.starts_with('[') {
            continue;
        }
        // 去掉前缀 "[N]: "
        let rest = match line.split("]: ").nth(1) {
            Some(r) => r,
            None => continue,
        };
        let parts: Vec<&str> = rest.splitn(2, " -- ").collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].trim().to_string();
        let detail = parts[1].trim();

        // 提取 serial（中间部分）和 type（括号部分）
        let serial = detail.split_whitespace().next().unwrap_or("?").to_string();
        let probe_type = detail
            .rsplit_once('(')
            .and_then(|(_, t)| t.trim_end_matches(')').split_whitespace().next())
            .unwrap_or("?")
            .to_string();

        probes.push(ProbeInfo {
            name,
            serial,
            probe_type,
        });
    }
    probes
}

// ============================================================================
// 工具函数
// ============================================================================

/// 在 PATH + 常见路径中寻找可执行文件
fn resolve_binary(name: &str) -> Option<String> {
    // 1. which 命令
    if let Ok(out) = Command::new("which").arg(name).output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() && std::path::Path::new(&path).is_file() {
                return Some(path);
            }
        }
    }

    // 2. PATH 逐项搜索
    for dir in std::env::var_os("PATH")
        .unwrap_or_default()
        .to_str()?
        .split(':') {
        let candidate = format!("{}/{}", dir, name);
        if std::path::Path::new(&candidate).is_file() {
            return Some(candidate);
        }
    }

    // 3. ~/.cargo/bin/（probe-rs 常在这里）
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = format!("{}/.cargo/bin/{}", home.to_string_lossy(), name);
        if std::path::Path::new(&candidate).is_file() {
            return Some(candidate);
        }
    }

    // 4. ~/.local/bin/
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = format!("{}/.local/bin/{}", home.to_string_lossy(), name);
        if std::path::Path::new(&candidate).is_file() {
            return Some(candidate);
        }
    }

    None
}

/// 查找 pyOCD 的特定路径（venv）
fn resolve_pyocd_path() -> Option<String> {
    // PYOCD_PATH 环境变量
    if let Ok(p) = std::env::var("PYOCD_PATH") {
        if std::path::Path::new(&p).is_file() {
            return Some(p);
        }
    }

    // 常见 venv 位置
    let candidates = [
        "~/.venvs/pyocd/bin/pyocd",
        "~/.virtualenvs/pyocd/bin/pyocd",
        "~/pyocd/.venv/bin/pyocd",
    ];
    for c in &candidates {
        let expanded = shellexpand::tilde(c).to_string();
        if std::path::Path::new(&expanded).is_file() {
            return Some(expanded);
        }
    }

    None
}

/// 运行命令获取版本字符串（第一行）
fn run_version(path: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(path).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let first_line = text.lines().next().unwrap_or("?").trim().to_string();
    if first_line.is_empty() {
        // 有些工具把版本信息输出到 stderr
        let stderr = String::from_utf8_lossy(&output.stderr);
        let first_line = stderr.lines().next().unwrap_or("?").trim().to_string();
        if first_line.is_empty() {
            return None;
        }
        return Some(first_line);
    }
    Some(first_line)
}

// ============================================================================
// 输出
// ============================================================================

fn print_summary(config: &UserConfig, path: &str) {
    eprintln!();
    eprintln!("╔══════════════════════════════════════╗");
    eprintln!("║     🔥 loading-chip 环境检测报告      ║");
    eprintln!("╚══════════════════════════════════════╝");
    eprintln!();

    print_backend("probe-rs", &config.backends.probe_rs);
    print_backend("OpenOCD", &config.backends.openocd);
    print_backend("pyOCD", &config.backends.pyocd);
    eprintln!();

    eprintln!("  GDB:");
    print_gdb("  arm-none-eabi-gdb", &config.backends.gdb.arm);
    print_gdb("  xtensa-esp32-elf-gdb", &config.backends.gdb.xtensa_esp32);
    print_gdb("  xtensa-esp32s2-elf-gdb", &config.backends.gdb.xtensa_esp32s2);
    print_gdb("  xtensa-esp32s3-elf-gdb", &config.backends.gdb.xtensa_esp32s3);
    print_gdb("  riscv32-esp-elf-gdb", &config.backends.gdb.riscv_esp);
    eprintln!();

    if config.probes.is_empty() {
        eprintln!("  🔌 调试探针: 未检测到");
    } else {
        eprintln!("  🔌 调试探针:");
        for p in &config.probes {
            eprintln!("    - {}  ({}  {})", p.name, p.serial, p.probe_type);
        }
    }
    eprintln!();
    eprintln!("  📄 配置文件已写入: {}", path);
    eprintln!();
}

fn print_backend(name: &str, info: &Option<BackendInfo>) {
    match info {
        Some(i) => {
            eprintln!("  ✅ {}  — {}  ({})", name, i.path, i.version);
        }
        None => {
            eprintln!("  ❌ {}  — 未找到", name);
        }
    }
}

fn print_gdb(name: &str, info: &Option<BackendInfo>) {
    if let Some(i) = info {
        eprintln!(
            "    ✅ {}  — {}  ({})",
            name,
            i.path,
            format_version_short(&i.version)
        );
    }
}

/// 截取 GDB --version 输出中的版本号部分
fn format_version_short(v: &str) -> &str {
    // "GNU gdb (GNU Tools for STM32 14.3.rel1...) 15.2.90..."
    // → 取前 60 字符
    let truncated = if v.len() > 60 { &v[..60] } else { v };
    truncated.trim()
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_probe() {
        let input = "[0]: ESP JTAG -- 303a:1001:24:58:7C:DA:5E:04 (EspJtag)\n";
        let probes = parse_probe_list(input);
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].name, "ESP JTAG");
        assert!(probes[0].serial.contains("303a:1001"));
    }

    #[test]
    fn parse_multiple_probes() {
        let input = "[0]: STLink -- 0483:3748:ABC123 (StLink V2)\n[1]: CMSIS-DAP -- 0d28:0204:5678 (BMP)\n";
        let probes = parse_probe_list(input);
        assert_eq!(probes.len(), 2);
        assert_eq!(probes[0].name, "STLink");
        assert_eq!(probes[1].serial, "0d28:0204:5678");
    }

    #[test]
    fn parse_empty_probe_list() {
        assert!(parse_probe_list("").is_empty());
        assert!(parse_probe_list("No probes found.").is_empty());
    }
}
