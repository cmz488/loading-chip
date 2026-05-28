//! 板子配置系统
//!
//! 从 `boards.yaml` 加载板子定义，提供：
//! - 板子 → 后端目标字符串映射（替代硬编码的 mappings.rs）
//! - 后端兼容性校验（板子不支持某后端时提前报错）
//! - 可扩展：用户自行在 YAML 中添加板子
//!
//! 架构：
//! ```
//! boards.yaml → BoardRegistry::load() → BoardConfig
//!                                          ├── BackendTarget（per-backend 参数）
//!                                          └── BoardInfo（名称/架构/接口）
//! ```
//!
//! 使用：
//! ```text
//! loading-chip run -t stm32f4 -b pyocd    → 检查 stm32f4.backends.pyocd 存在
//! loading-chip run -t esp32s3 -b pyocd     → 报错：pyOCD 不支持 esp32s3
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ============================================================================
// YAML 结构
// ============================================================================

/// YAML 顶层结构
#[derive(Debug, Clone, Deserialize)]
struct BoardsFile {
    boards: HashMap<String, BoardConfig>,
}

/// 单个板子配置（YAML 反序列化用）
#[derive(Debug, Clone, Deserialize)]
struct BoardConfig {
    /// 可读名称
    name: String,
    /// 制造商
    manufacturer: String,
    /// 架构（arm / xtensa / riscv）
    architecture: String,
    /// 推荐的调试接口列表
    interfaces: Vec<String>,
    /// 各后端的目标参数（key = 后端名如 "probe-rs"）
    backends: HashMap<String, BackendTarget>,
    /// 可选备注
    #[serde(default)]
    note: String,
    /// 芯片检测映射（probe-rs 探测用）
    #[serde(default)]
    detection: DetectionConfig,
}

/// 芯片检测映射（YAML 反序列化用）
#[derive(Debug, Clone, Deserialize, Default)]
struct DetectionConfig {
    #[serde(default)]
    probe_rs_chips: Vec<String>,
}

/// 单个后端的目标参数（YAML 反序列化用）
#[derive(Debug, Clone, Deserialize)]
struct BackendTarget {
    /// 该后端对应的芯片/板子标识符
    target: String,
    /// 可选的额外配置（如 OpenOCD config 路径）
    #[serde(default)]
    config: String,
    /// 可选的额外命令行参数
    #[serde(default)]
    extra_args: Vec<String>,
}

// ============================================================================
// 公开类型
// ============================================================================

/// 板子架构
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Architecture {
    Arm,
    Xtensa,
    RiscV,
    #[serde(untagged)]
    Other(String),
}

impl std::fmt::Display for Architecture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arm => write!(f, "ARM"),
            Self::Xtensa => write!(f, "Xtensa"),
            Self::RiscV => write!(f, "RISC-V"),
            Self::Other(s) => write!(f, "{}", s),
        }
    }
}

/// 板子信息（用于 TUI 展示） — 字段预留给 TUI 使用
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct BoardInfo {
    /// 板子 ID（YAML key）
    pub id: String,
    /// 人类可读名称
    pub name: String,
    /// 制造商
    pub manufacturer: String,
    /// 架构
    pub architecture: Architecture,
    /// 支持的调试接口
    pub interfaces: Vec<String>,
    /// 是否支持指定后端（用于 TUI 筛选）
    pub supported_backends: Vec<String>,
    /// 备注
    pub note: String,
}

/// 后端适配结果 — 包含目标名称和额外参数
#[derive(Debug, Clone)]
pub struct BackendBoardParams {
    /// 目标芯片/板子标识符（传给后端的 --target / --chip 参数）
    pub target: String,
    /// 额外配置（如 OpenOCD config 文件路径）
    pub config: String,
    /// 额外命令行参数
    pub extra_args: Vec<String>,
}

// ============================================================================
// 板子注册表
// ============================================================================

/// 板子注册表 — 从 `boards.yaml` 加载，全局单例
///
/// # 使用
/// ```ignore
/// let registry = BoardRegistry::load()?;
/// let params = registry.resolve("stm32f4", "pyocd")?;
/// // params.target = "stm32f407vg"
/// ```
#[derive(Debug, Clone)]
pub struct BoardRegistry {
    /// board_id → BoardInfo
    info: HashMap<String, BoardInfo>,
    /// board_id → (backend_name → BackendBoardParams)
    backends: HashMap<String, HashMap<String, BackendBoardParams>>,
    /// 所有板子 ID 列表（保持 YAML 顺序）
    ids: Vec<String>,
    /// probe-rs chip name → board_id reverse lookup
    detection_map: HashMap<String, String>,
}

#[allow(dead_code)]
impl BoardRegistry {
    /// 从 YAML 文件加载板子配置
    ///
    /// 搜索顺序：
    /// 1. `LOADING_CHIP_BOARDS` 环境变量指定的路径
    /// 2. 用户配置目录 `~/.config/loading-chip/boards.yaml`
    /// 3. 可执行文件同目录的 `boards.yaml`
    /// 4. 当前目录的 `boards.yaml`
    /// 5. 项目的 `boards.yaml`（嵌入，cargo run 场景）
    pub fn load() -> Result<Self, String> {
        let content = Self::read_yaml()?;
        let file: BoardsFile =
            serde_yaml::from_str(&content).map_err(|e| format!("boards.yaml 解析失败: {}", e))?;

        let mut info = HashMap::new();
        let mut backends = HashMap::new();
        let mut ids = Vec::new();

        for (id, cfg) in &file.boards {
            let board_info = BoardInfo {
                id: id.clone(),
                name: cfg.name.clone(),
                manufacturer: cfg.manufacturer.clone(),
                architecture: parse_arch(&cfg.architecture),
                interfaces: cfg.interfaces.clone(),
                supported_backends: cfg.backends.keys().cloned().collect(),
                note: cfg.note.clone(),
            };

            let mut be_map = HashMap::new();
            for (be_name, be_target) in &cfg.backends {
                be_map.insert(
                    be_name.clone(),
                    BackendBoardParams {
                        target: be_target.target.clone(),
                        config: be_target.config.clone(),
                        extra_args: be_target.extra_args.clone(),
                    },
                );
            }

            ids.push(id.clone());
            info.insert(id.clone(), board_info);
            backends.insert(id.clone(), be_map);
        }

        // Build the detection reverse-lookup map
        let mut detection_map = HashMap::new();
        for (id, cfg) in &file.boards {
            for chip in &cfg.detection.probe_rs_chips {
                detection_map.insert(chip.to_lowercase(), id.clone());
            }
        }

        Ok(Self {
            info,
            backends,
            ids,
            detection_map,
        })
    }

    /// 读取 YAML 文件内容
    fn read_yaml() -> Result<String, String> {
        // 1. 环境变量
        if let Ok(p) = std::env::var("LOADING_CHIP_BOARDS")
            && Path::new(&p).is_file() {
                return std::fs::read_to_string(&p).map_err(|e| format!("读取 {}: {}", p, e));
            }

        // 2. 用户配置目录: ~/.config/loading-chip/boards.yaml
        if let Some(dir) = dirs::config_dir() {
            let p = dir.join("loading-chip").join("boards.yaml");
            if p.is_file() {
                return std::fs::read_to_string(&p).map_err(|e| format!("读取 {:?}: {}", p, e));
            }
        }

        // 3. 可执行文件同目录
        if let Ok(exe) = std::env::current_exe() {
            let dir = exe.parent().unwrap_or(Path::new("."));
            let p = dir.join("boards.yaml");
            if p.is_file() {
                return std::fs::read_to_string(&p).map_err(|e| format!("读取 {:?}: {}", p, e));
            }
        }

        // 4. 当前目录
        if Path::new("boards.yaml").is_file() {
            return std::fs::read_to_string("boards.yaml")
                .map_err(|e| format!("读取 boards.yaml: {}", e));
        }

        // 5. 嵌入的默认配置（编译时 include_str!）
        Ok(include_str!("../boards.yaml").to_string())
    }

    /// 解析板子 + 后端的适配参数
    ///
    /// # Arguments
    /// * `board_id` — 板子 ID（如 "stm32f4"）
    /// * `backend` — 后端名（如 "pyocd" / "probe-rs"）
    ///
    /// # Returns
    /// 成功返回 `BackendBoardParams`；后端不在板子的 backends 列表中时返回错误
    pub fn resolve(&self, board_id: &str, backend: &str) -> Result<BackendBoardParams, String> {
        // 查找板子
        let board_backends = self
            .backends
            .get(board_id)
            .ok_or_else(|| format!("未知板子: {}（可用: {}）", board_id, self.ids.join(", ")))?;

        // 查找后端
        board_backends.get(backend).cloned().ok_or_else(|| {
            let available: Vec<_> = board_backends.keys().collect();
            let maybe_note = self
                .info
                .get(board_id)
                .map(|b| &b.note)
                .filter(|n| !n.is_empty());

            let mut msg = format!(
                "❌ 后端 \"{}\" 不支持板子 \"{}\"（{} 支持的后端: {})",
                backend,
                board_id,
                board_id,
                available
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );

            if let Some(note) = maybe_note {
                msg.push_str(&format!("\n💡 提示: {}", note));
            }

            msg
        })
    }

    /// 获取板子信息
    pub fn get(&self, id: &str) -> Option<&BoardInfo> {
        self.info.get(id)
    }

    /// 所有板子 ID
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// 板子总数
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// 过滤出支持指定后端的板子
    pub fn filter_by_backend(&self, backend: &str) -> Vec<&BoardInfo> {
        self.info
            .values()
            .filter(|b| b.supported_backends.contains(&backend.to_string()))
            .collect()
    }

    /// 根据 probe-rs 检测到的 chip name 反向查找 board ID
    ///
    /// 优先级：
    /// 1. detection_map 精确匹配（忽略大小写）
    /// 2. board ID 直接匹配（忽略大小写）
    /// 3. 返回 None（调用方使用原始 chip name 作为 board ID）
    pub fn resolve_by_chip_name(&self, chip_name: &str) -> Option<String> {
        let lower = chip_name.to_lowercase();
        // 1. detection_map 查找 (O(1) hash lookup)
        if let Some(board_id) = self.detection_map.get(&lower) {
            return Some(board_id.clone());
        }
        // 2. board ID 直接匹配（小写比较）
        if self.ids.iter().any(|id| id.to_lowercase() == lower) {
            return Some(chip_name.to_string());
        }
        None
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

fn parse_arch(s: &str) -> Architecture {
    match s.to_lowercase().as_str() {
        "arm" => Architecture::Arm,
        "xtensa" => Architecture::Xtensa,
        "riscv" | "risc-v" => Architecture::RiscV,
        other => Architecture::Other(other.to_string()),
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_arch_standard() {
        assert_eq!(parse_arch("arm"), Architecture::Arm);
        assert_eq!(parse_arch("ARM"), Architecture::Arm);
        assert_eq!(parse_arch("xtensa"), Architecture::Xtensa);
        assert_eq!(parse_arch("riscv"), Architecture::RiscV);
    }

    #[test]
    fn registry_loads_all_boards() {
        let reg = BoardRegistry::load().expect("boards.yaml should parse");
        assert!(
            reg.len() >= 11,
            "expected at least 11 boards, got {}",
            reg.len()
        );
    }

    #[test]
    fn registry_resolve_supported() {
        let reg = BoardRegistry::load().unwrap();
        let p = reg.resolve("stm32f4", "pyocd").unwrap();
        assert_eq!(p.target, "stm32f407vg");

        let p = reg.resolve("stm32f4", "probe-rs").unwrap();
        assert_eq!(p.target, "STM32F407VG");

        let p = reg.resolve("esp32s3", "probe-rs").unwrap();
        assert_eq!(p.target, "ESP32S3");
    }

    #[test]
    fn registry_resolve_unsupported() {
        let reg = BoardRegistry::load().unwrap();
        let err = reg.resolve("esp32s3", "pyocd").unwrap_err();
        assert!(err.contains("不支持"), "expected '不支持', got: {}", err);
        assert!(err.contains("probe-rs"), "should list supported backends");
        assert!(err.contains("pyOCD"), "should mention pyOCD note");
    }

    #[test]
    fn registry_resolve_unknown_board() {
        let reg = BoardRegistry::load().unwrap();
        let err = reg.resolve("nonexistent", "gdb").unwrap_err();
        assert!(err.contains("未知板子"));
        assert!(err.contains("stm32f1"));
    }

    #[test]
    fn registry_filter_by_backend() {
        let reg = BoardRegistry::load().unwrap();
        let pyocd_boards = reg.filter_by_backend("pyocd");
        // ESP32 系列不应支持 pyOCD
        let esp_ids: Vec<_> = pyocd_boards.iter().map(|b| &b.id).collect();
        assert!(!esp_ids.contains(&&"esp32".to_string()));
        assert!(!esp_ids.contains(&&"esp32s3".to_string()));
        // STM32 系列应支持
        assert!(esp_ids.contains(&&"stm32f4".to_string()));
        assert!(esp_ids.contains(&&"stm32f1".to_string()));
    }

    #[test]
    fn detection_map_populated() {
        let reg = BoardRegistry::load().unwrap();
        eprintln!("detection_map len: {}", reg.detection_map.len());
        for (k, v) in &reg.detection_map {
            eprintln!("  {} -> {}", k, v);
        }
        // STM32F407VG should map to stm32f4
        assert_eq!(
            reg.resolve_by_chip_name("STM32F407VG"),
            Some("stm32f4".to_string())
        );
        // ESP32S3 should map to esp32s3
        assert_eq!(
            reg.resolve_by_chip_name("ESP32S3"),
            Some("esp32s3".to_string())
        );
        // Unknown chip returns None
        assert_eq!(reg.resolve_by_chip_name("RANDOM_CHIP_XYZ"), None);
    }

    #[test]
    fn detection_case_insensitive() {
        let reg = BoardRegistry::load().unwrap();
        assert_eq!(
            reg.resolve_by_chip_name("stm32f407vg"),
            Some("stm32f4".to_string())
        );
        assert_eq!(
            reg.resolve_by_chip_name("stm32f103c8"),
            Some("stm32f1".to_string())
        );
    }

    #[test]
    fn board_info_has_backends() {
        let reg = BoardRegistry::load().unwrap();
        let stm32 = reg.get("stm32f4").unwrap();
        assert!(stm32.supported_backends.contains(&"pyocd".to_string()));
        assert!(stm32.supported_backends.contains(&"probe-rs".to_string()));

        let esp32 = reg.get("esp32s3").unwrap();
        assert!(!esp32.supported_backends.contains(&"pyocd".to_string()));
        assert!(esp32.supported_backends.contains(&"probe-rs".to_string()));
    }
}
