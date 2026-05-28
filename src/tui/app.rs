//! TUI 应用状态机
//!
//! 管理烧录参数、UI 焦点、输入模式和运行状态。

use ratatui::widgets::ListState;
use std::sync::Arc;

use crate::board::BoardRegistry;
use crate::chip_detect::DetectedChip;
use crate::flash::{do_flash, FlashBackend, FlashConfig, FlashResult};
use crate::presets;

/// 当前聚焦的输入字段
#[derive(PartialEq, Clone, Copy)]
pub enum Focus {
    ModeSwitch,
    Backend,
    Interface,
    Target,
    ElfPath,
    FlashBtn,
}

/// TUI 输入模式
#[derive(PartialEq)]
pub enum InputMode {
    Normal,       // 键盘导航
    Selecting,    // 下拉选择中（接口/芯片）
    SelectingElf, // ELF 文件下拉选择中
    EditingElf,   // 编辑 ELF 路径
    Flashing,     // 正在烧录
    Done,         // 烧录完成
}

/// TUI 应用状态
pub struct App {
    // --- 烧录参数 ---
    pub backend: String,
    pub interface: String,
    pub target: String,
    pub elf_path: String,
    pub gdb_port: String,
    /// pyOCD 可执行文件路径（留空则自动检测）
    pub pyocd_path: String,
    /// 超时时间（秒）
    pub timeout_secs: u64,

    // --- 可选列表索引 ---
    pub backend_idx: usize,
    pub iface_idx: usize,
    pub target_idx: usize,
    pub elf_file_idx: usize,

    // --- ELF 文件搜索结果 ---
    pub elf_files: Vec<String>,

    // --- UI 状态 ---
    pub focus: Focus,
    pub mode: InputMode,
    pub list_state: ListState,

    // --- 运行结果 ---
    pub status: String,
    pub result: Option<FlashResult>,

    // --- 调试模式参数 ---
    pub debug_elf: String,
    pub debug_target: String,
    pub debug_backend: String,
    pub debug_interface: String,
    pub debug_port: u16,
    pub debug_gdb: String,

    // --- 模式切换标志 ---
    pub switch_to_debug: bool,

    // --- 退出标志 ---
    pub should_quit: bool,

    /// 板子注册表（只读共享）
    pub registry: Arc<BoardRegistry>,
    /// 自动检测到的芯片列表
    pub detected_chips: Vec<DetectedChip>,
}

impl App {
    /// 创建应用初始状态
    pub fn new(
        gdb_port: String,
        pyocd_path: String,
        timeout_secs: u64,
        registry: Arc<BoardRegistry>,
    ) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            backend: presets::BACKENDS[0].0.to_string(),
            interface: presets::INTERFACES[0].0.to_string(),
            target: presets::TARGETS[0].0.to_string(),
            elf_path: String::new(),
            gdb_port,
            pyocd_path,
            timeout_secs,
            backend_idx: 0,
            iface_idx: 0,
            target_idx: 0,
            elf_file_idx: 0,
            elf_files: Vec::new(),
            focus: Focus::FlashBtn,
            mode: InputMode::Normal,
            list_state,
            status: "就绪 — F5: 调试模式  |  Tab: 切换字段  |  ↑↓: 选择  |  Enter: 确认".to_string(),
            result: None,
            debug_elf: String::new(),
            debug_target: String::new(),
            debug_backend: "probe-rs".to_string(),
            debug_interface: String::new(),
            debug_port: 3333,
            debug_gdb: String::new(),
            switch_to_debug: false,
            should_quit: false,
            registry,
            detected_chips: Vec::new(),
        }
    }

    /// 获取当前接口的中文描述
    pub fn iface_label(&self) -> &str {
        presets::iface_label(&self.interface)
    }

    /// 获取当前芯片的中文描述
    pub fn target_label(&self) -> &str {
        presets::target_label(&self.target)
    }

    /// 获取当前后端的中文描述
    pub fn backend_label(&self) -> &str {
        presets::backend_label(&self.backend)
    }

    /// 获取当前焦点对应的选项数量
    pub fn option_count(&self) -> usize {
        match self.focus {
            Focus::Backend => presets::BACKENDS.len(),
            Focus::Interface => presets::INTERFACES.len(),
            Focus::Target => presets::TARGETS.len(),
            _ => 0,
        }
    }

    /// 获取当前焦点对应的当前索引
    pub fn current_idx(&self) -> usize {
        match self.focus {
            Focus::Backend => self.backend_idx,
            Focus::Interface => self.iface_idx,
            Focus::Target => self.target_idx,
            _ => 0,
        }
    }

    /// 在当前焦点列表中上移选择
    pub fn select_prev(&mut self) {
        let count = self.option_count();
        if count == 0 {
            return;
        }
        match self.focus {
            Focus::Backend => {
                self.backend_idx = self.backend_idx.checked_sub(1).unwrap_or(count - 1);
            }
            Focus::Interface => {
                self.iface_idx = self.iface_idx.checked_sub(1).unwrap_or(count - 1);
            }
            Focus::Target => {
                self.target_idx = self.target_idx.checked_sub(1).unwrap_or(count - 1);
            }
            _ => {}
        }
        self.list_state.select(Some(self.current_idx()));
    }

    /// 在当前焦点列表中下移选择
    pub fn select_next(&mut self) {
        let count = self.option_count();
        if count == 0 {
            return;
        }
        match self.focus {
            Focus::Backend => {
                self.backend_idx = (self.backend_idx + 1) % count;
            }
            Focus::Interface => {
                self.iface_idx = (self.iface_idx + 1) % count;
            }
            Focus::Target => {
                self.target_idx = (self.target_idx + 1) % count;
            }
            _ => {}
        }
        self.list_state.select(Some(self.current_idx()));
    }

    /// 保存当前烧录参数到调试参数字段（切换到调试模式时用）
    pub fn sync_to_debug(&mut self) {
        self.debug_elf = self.elf_path.clone();
        self.debug_target = self.target.clone();
        self.debug_backend = self.backend.clone();
        self.debug_interface = self.interface.clone();
        // gdb 留空（由调试模式自动检测）
    }

    /// 切换到调试模式（设置标志，由主循环处理）
    pub fn request_debug(&mut self) {
        self.sync_to_debug();
        self.switch_to_debug = true;
    }

    /// 确认当前下拉选择，将选中值写入 app 字段
    pub fn confirm_selection(&mut self) {
        match self.focus {
            Focus::Backend => {
                let keys = presets::backend_keys();
                if let Some(k) = keys.get(self.backend_idx) {
                    self.backend = k.clone();
                }
            }
            Focus::Interface => {
                let keys = presets::iface_keys();
                if let Some(k) = keys.get(self.iface_idx) {
                    self.interface = k.clone();
                }
            }
            Focus::Target => {
                let keys = presets::target_keys();
                if let Some(k) = keys.get(self.target_idx) {
                    self.target = k.clone();
                }
            }
            _ => {}
        }
        self.mode = InputMode::Normal;
        self.status = format!(
            "后端: {}, 接口: {}, 芯片: {}",
            self.backend_label(),
            self.iface_label(),
            self.target_label()
        );
    }

    /// 确认 ELF 文件选择（从搜索结果中选择）
    pub fn confirm_elf_selection(&mut self) {
        if let Some(path) = self.elf_files.get(self.elf_file_idx) {
            self.elf_path = path.clone();
        }
        self.mode = InputMode::Normal;
        self.status = format!("ELF 路径: {}", self.elf_path);
    }

    /// 搜索当前目录及子目录中的 .elf 文件
    /// 若找到则填充 self.elf_files 列表（按路径长度排序）
    /// 支持 PlatformIO（.pio/build/...）、CMake、ESP-IDF 等常见构建目录
    pub fn search_elf_files(&mut self) {
        self.elf_files.clear();
        self.elf_file_idx = 0;

        // 从当前工作目录搜索，最大深度 5 层
        // PlatformIO 路径: .pio/build/<env>/firmware.elf 需要 3 层
        // ESP-IDF: build/<project>.elf 需要 2 层
        // 再给一些嵌套项目余量
        if let Ok(cwd) = std::env::current_dir() {
            self.scan_for_elf(&cwd, 0, 5);
        }

        // 短路径优先显示
        self.elf_files.sort_by_key(|p| p.len());
    }

    /// 递归扫描目录中的 .elf 文件
    fn scan_for_elf(&mut self, dir: &std::path::Path, depth: usize, max_depth: usize) {
        if depth > max_depth {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.is_dir() {
                    // 只跳过确定不含 .elf 的巨型目录
                    if name == "node_modules" || name == ".git" {
                        continue;
                    }
                    self.scan_for_elf(&path, depth + 1, max_depth);
                } else if path.is_file() && name.ends_with(".elf") {
                    if let Ok(abs) = path.canonicalize() {
                        self.elf_files.push(abs.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    /// 执行烧录（阻塞调用后端工具）
    pub fn do_flash(&mut self) {
        self.mode = InputMode::Flashing;
        self.status = format!("正在烧录 {} → {} ...", self.interface, self.target);

        let config = FlashConfig {
            backend: FlashBackend::from_str(&self.backend),
            interface: self.interface.clone(),
            target: self.target.clone(),
            elf_path: if self.elf_path.is_empty() {
                "firmware.elf".to_string()
            } else {
                self.elf_path.clone()
            },
            gdb_port: self.gdb_port.clone(),
            pyocd_path: self.pyocd_path.clone(),
            timeout_secs: self.timeout_secs,
            board_config: String::new(),
            board_extra_args: vec![],
            board_id: String::new(),
        };

        let res = do_flash(&config);
        self.status = res.message.clone();
        self.result = Some(res);
        self.mode = InputMode::Done;
    }
}
