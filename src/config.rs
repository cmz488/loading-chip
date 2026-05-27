//! 用户配置
//!
//! 通过 `loading-chip init` 自动检测并生成 `~/.config/loading-chip/config.yaml`。
//! 记录本地可用的后端工具路径、版本、以及检测到的调试探针。

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// 用户配置文件完整结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    /// 各后端检测结果
    pub backends: BackendDetection,
    /// 检测到的调试探针列表
    pub probes: Vec<ProbeInfo>,
    /// 工具版本信息
    pub versions: Versions,
}

/// 所有后端的检测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendDetection {
    pub probe_rs: Option<BackendInfo>,
    pub openocd: Option<BackendInfo>,
    pub pyocd: Option<BackendInfo>,
    pub gdb: GdbDetection,
}

/// 单个后端信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendInfo {
    /// 可执行文件完整路径
    pub path: String,
    /// 版本字符串
    pub version: String,
}

/// GDB 检测结果（每种架构一个）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GdbDetection {
    pub arm: Option<BackendInfo>,
    pub xtensa_esp32: Option<BackendInfo>,
    pub xtensa_esp32s2: Option<BackendInfo>,
    pub xtensa_esp32s3: Option<BackendInfo>,
    pub riscv_esp: Option<BackendInfo>,
}

/// 调试探针信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeInfo {
    pub name: String,
    pub serial: String,
    pub probe_type: String,
}

/// 工具版本
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Versions {
    pub loading_chip: String,
}

impl UserConfig {
    /// 默认配置路径 ~/.config/loading-chip/config.yaml
    pub fn default_path() -> PathBuf {
        let mut path = dirs_config_dir();
        path.push("loading-chip");
        std::fs::create_dir_all(&path).ok();
        path.push("config.yaml");
        path
    }

    /// 从默认位置加载配置
    pub fn load() -> Option<Self> {
        let path = Self::default_path();
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        serde_yaml::from_str(&content).ok()
    }

    /// 保存到默认位置
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_yaml::to_string(self)
            .map_err(std::io::Error::other)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// 保存到指定路径
    pub fn save_to(&self, path: &str) -> std::io::Result<()> {
        let p = std::path::Path::new(path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_yaml::to_string(self)
            .map_err(std::io::Error::other)?;
        std::fs::write(p, content)?;
        Ok(())
    }
}

/// 获取平台标准配置目录
/// - Linux:   ~/.config/loading-chip/config.yaml  (XDG_CONFIG_HOME)
/// - macOS:   ~/Library/Application Support/loading-chip/config.yaml
/// - Windows: C:\Users\<user>\AppData\Roaming\loading-chip\config.yaml
fn dirs_config_dir() -> PathBuf {
    // 优先使用 XDG_CONFIG_HOME（Linux 标准）
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(dir)
    } else if let Some(dir) = dirs::config_dir() {
        // dirs::config_dir() 自动处理各平台
        dir
    } else {
        // 终极回退
        PathBuf::from(".config")
    }
}
