//! TUI 键盘事件处理
//!
//! 根据当前输入模式分发按键，更新应用状态。

use crossterm::event::KeyCode;

use super::app::{App, Focus, InputMode};

// ============================================================================
// 顶层分发
// ============================================================================

/// 处理键盘事件，返回 false 表示应执行烧录（由主循环接管）
pub fn handle_key(app: &mut App, code: KeyCode) -> bool {
    // 全局快捷键：在任何模式下都生效
    if let KeyCode::F(5) = code {
        if app.mode != InputMode::EditingElf && app.mode != InputMode::Flashing {
            app.request_debug();
        }
        return true;
    }

    // F12: 重新检测芯片
    if let KeyCode::F(12) = code
        && app.mode != InputMode::Flashing
        && app.mode != InputMode::EditingElf
    {
        app.detected_chips = crate::chip_detect::detect_chips();
        if let Some(detected) = app.detected_chips.first() {
            let board_id = detected.board_id.clone()
                .unwrap_or_else(|| detected.chip_name.clone());
            if app.registry.resolve(&board_id, "probe-rs").is_ok() {
                app.target = board_id;
            } else {
                app.target = detected.chip_name.clone();
            }
            app.interface = detected.suggested_interface.clone();
            let iface_keys = crate::presets::iface_keys();
            if let Some(idx) = iface_keys.iter().position(|k| *k == detected.suggested_interface) {
                app.iface_idx = idx;
            } else {
                app.iface_idx = 0;
            }
            app.status = format!(
                "已检测到: {} (芯片: {})", detected.probe_name, detected.chip_name
            );
        } else {
            app.status = "未检测到调试探针".to_string();
        }
        return true;
    }

    match app.mode {
        InputMode::Normal => handle_normal(app, code),
        InputMode::Selecting => handle_selecting(app, code),
        InputMode::SelectingElf => handle_selecting_elf(app, code),
        InputMode::EditingElf => handle_editing_elf(app, code),
        InputMode::Done => handle_done(app, code),
        InputMode::Flashing => false,
    }
}

// ============================================================================
// Normal 模式
// ============================================================================

fn handle_normal(app: &mut App, code: KeyCode) -> bool {
    match code {
        // 正向切换焦点（Tab）
        KeyCode::Tab => {
            app.focus = match app.focus {
                Focus::ModeSwitch => Focus::Backend,
                Focus::Backend => Focus::Interface,
                Focus::Interface => Focus::Target,
                Focus::Target => Focus::ElfPath,
                Focus::ElfPath => Focus::FlashBtn,
                Focus::FlashBtn => Focus::ModeSwitch,
            };
            true
        }
        // 反向切换焦点（Shift+Tab）
        KeyCode::BackTab => {
            app.focus = match app.focus {
                Focus::ModeSwitch => Focus::FlashBtn,
                Focus::Backend => Focus::ModeSwitch,
                Focus::Interface => Focus::Backend,
                Focus::Target => Focus::Interface,
                Focus::ElfPath => Focus::Target,
                Focus::FlashBtn => Focus::ElfPath,
            };
            true
        }
        // Enter：根据焦点执行不同操作
        KeyCode::Enter => match app.focus {
            Focus::ModeSwitch => {
                // 模式切换按钮按 Enter 进入调试模式
                app.request_debug();
                true
            }
            Focus::Backend | Focus::Interface | Focus::Target => {
                // 打开下拉选择
                app.mode = InputMode::Selecting;
                app.list_state.select(Some(app.current_idx()));
                app.status = "请用 ↑↓ 选择后按 Enter 确认".to_string();
                true
            }
            Focus::ElfPath => {
                // 先搜索当前目录的 .elf 文件
                app.search_elf_files();
                if app.elf_files.is_empty() {
                    // 未找到 > 回退到手动输入
                    app.mode = InputMode::EditingElf;
                    app.status = "未找到 .elf 文件，请手动输入路径".to_string();
                } else {
                    // 找到文件 > 弹出选择列表
                    app.mode = InputMode::SelectingElf;
                    app.list_state.select(Some(0));
                    app.status = format!(
                        "找到 {} 个 .elf 文件，↑↓ 选择，Enter 确认，e 手动输入",
                        app.elf_files.len()
                    );
                }
                true
            }
            Focus::FlashBtn => {
                if app.elf_path.is_empty() {
                    app.status = "⚠️  请先填写 ELF 文件路径！".to_string();
                    true
                } else {
                    app.status = "正在启动烧录...".to_string();
                    false // 通知主循环执行烧录
                }
            }
        },
        // Esc / q：退出
        KeyCode::Esc | KeyCode::Char('q') => {
            app.should_quit = true;
            true
        }
        _ => true,
    }
}

// ============================================================================
// Selecting 模式（下拉选择）
// ============================================================================

fn handle_selecting(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.select_prev();
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.select_next();
            true
        }
        KeyCode::Enter => {
            app.confirm_selection();
            true
        }
        KeyCode::Esc => {
            app.mode = InputMode::Normal;
            app.status = "已取消选择".to_string();
            true
        }
        _ => true,
    }
}

// ============================================================================
// Editing ELF 模式
// ============================================================================

fn handle_editing_elf(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Enter => {
            app.mode = InputMode::Normal;
            if app.elf_path.is_empty() {
                app.status = "⚠️  ELF 路径为空，请重新填写".to_string();
            } else {
                app.status = format!("ELF 路径: {}", app.elf_path);
            }
            true
        }
        KeyCode::Esc => {
            app.mode = InputMode::Normal;
            app.status = "已取消编辑".to_string();
            true
        }
        // Tab / Shift+Tab：保存当前输入并切换到其他字段
        KeyCode::Tab => {
            app.mode = InputMode::Normal;
            app.focus = Focus::FlashBtn;
            app.status = format!("ELF 路径: {}", app.elf_path);
            true
        }
        KeyCode::BackTab => {
            app.mode = InputMode::Normal;
            app.focus = Focus::Target;
            app.status = format!("ELF 路径: {}", app.elf_path);
            true
        }
        KeyCode::Backspace => {
            app.elf_path.pop();
            true
        }
        KeyCode::Char(c) => {
            app.elf_path.push(c);
            true
        }
        _ => true,
    }
}

// ============================================================================
// SelectingElf 模式（ELF 文件下拉选择）
// ============================================================================

fn handle_selecting_elf(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Up | KeyCode::Char('k') => {
            if !app.elf_files.is_empty() {
                app.elf_file_idx = app
                    .elf_file_idx
                    .checked_sub(1)
                    .unwrap_or(app.elf_files.len() - 1);
                app.list_state.select(Some(app.elf_file_idx));
            }
            true
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !app.elf_files.is_empty() {
                app.elf_file_idx = (app.elf_file_idx + 1) % app.elf_files.len();
                app.list_state.select(Some(app.elf_file_idx));
            }
            true
        }
        KeyCode::Enter => {
            app.confirm_elf_selection();
            true
        }
        // e / Tab：切换到手动输入模式
        KeyCode::Char('e') | KeyCode::Tab => {
            app.mode = InputMode::EditingElf;
            app.status = "手动输入 ELF 文件路径".to_string();
            true
        }
        KeyCode::Esc => {
            app.mode = InputMode::Normal;
            app.status = "已取消 ELF 文件选择".to_string();
            true
        }
        _ => true,
    }
}

// ============================================================================
// Done 模式（烧录完成后）
// ============================================================================

fn handle_done(app: &mut App, code: KeyCode) -> bool {
    match code {
        // 重新烧录
        KeyCode::Enter | KeyCode::Char('r') => {
            app.mode = InputMode::Normal;
            app.result = None;
            app.status = "准备重新烧录...".to_string();
            true
        }
        // 退出
        KeyCode::Esc | KeyCode::Char('q') => {
            app.should_quit = true;
            true
        }
        _ => true,
    }
}
