//! TUI 渲染模块
//!
//! 使用 ratatui 绘制烧录工具的所有界面元素。

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::presets;

use super::app::{App, Focus, InputMode};

// ============================================================================
// 顶层渲染入口
// ============================================================================

/// 主渲染函数，每帧调用
pub fn ui(f: &mut Frame, app: &App) {
    let area = f.area();

    // 主布局：标题 / 表单 / 状态 / 快捷键
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // 标题 + 模式切换
            Constraint::Min(12),   // 表单
            Constraint::Length(3), // 状态
            Constraint::Length(1), // 快捷键
        ])
        .split(area);

    render_title_with_mode(f, chunks[0], app);

    match app.mode {
        InputMode::Done => render_result(f, chunks[1], app),
        _ => {
            render_form(f, chunks[1], app);
            // 下拉选择时渲染弹出层
            if app.mode == InputMode::Selecting {
                render_dropdown(f, area, app);
            } else if app.mode == InputMode::SelectingElf {
                render_elf_dropdown(f, area, app);
            }
        }
    }

    render_status(f, chunks[2], app);
    render_help(f, chunks[3], app);
}

// ============================================================================
// 标题栏 + 模式切换按钮
// ============================================================================

fn render_title_with_mode(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // 左侧标题
    let title_bar = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let title_text = Paragraph::new("🔥  LOADING-CHIP")
        .style(Style::default().fg(Color::Yellow).bold())
        .alignment(Alignment::Left)
        .block(title_bar);
    f.render_widget(title_text, chunks[0]);

    // 右侧模式切换按钮
    let mode_bar = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let mode_focused = app.focus == Focus::ModeSwitch;

    // 两个按钮：烧录模式是当前模式
    let flash_style = if mode_focused && app.focus == Focus::ModeSwitch {
        // 左侧（烧录）按钮选中
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .bold()
    } else {
        Style::default().fg(Color::Green).bold()
    };

    let debug_style = if mode_focused {
        // 右侧有焦点，光标在右侧
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mode_text = Line::from(vec![
        Span::styled(" 🔥烧录 ", flash_style),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(" 🐛调试 ", debug_style),
    ]);

    f.render_widget(
        Paragraph::new(mode_text)
            .alignment(Alignment::Center)
            .block(mode_bar),
        chunks[1],
    );
}

// ============================================================================
// 表单区域
// ============================================================================

fn render_form(f: &mut Frame, area: Rect, app: &App) {
    // ELF 编辑模式下给输入框更多空间
    let elf_height = if app.mode == InputMode::EditingElf {
        5
    } else {
        3
    };

    let form_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),          // 后端
            Constraint::Length(1),          // 间距
            Constraint::Length(3),          // 接口
            Constraint::Length(1),          // 间距
            Constraint::Length(3),          // 芯片
            Constraint::Length(1),          // 间距
            Constraint::Length(elf_height), // ELF 路径
            Constraint::Length(1),          // 间距
            Constraint::Length(3),          // 烧录按钮
        ])
        .split(area);

    // 后端字段
    render_field(
        f,
        form_chunks[0],
        "⚙️  烧录后端",
        app.backend_label(),
        app.focus == Focus::Backend,
        app.mode == InputMode::Selecting && app.focus == Focus::Backend,
    );

    // 接口字段
    render_field(
        f,
        form_chunks[2],
        "🔌 调试接口",
        app.iface_label(),
        app.focus == Focus::Interface,
        app.mode == InputMode::Selecting && app.focus == Focus::Interface,
    );

    // 芯片字段
    render_field(
        f,
        form_chunks[4],
        "🎯 目标芯片",
        app.target_label(),
        app.focus == Focus::Target,
        app.mode == InputMode::Selecting && app.focus == Focus::Target,
    );

    // ELF 路径字段 — 根据不同模式渲染
    if app.mode == InputMode::EditingElf {
        render_elf_input(f, form_chunks[6], app);
    } else if app.mode == InputMode::SelectingElf {
        // 下拉选择中：高亮提示
        let preview = app
            .elf_files
            .get(app.elf_file_idx)
            .map(|s| s.as_str())
            .unwrap_or("");
        render_field(
            f,
            form_chunks[6],
            "📁 ELF 文件（从搜索结果选择）",
            preview,
            true,
            true,
        );
    } else {
        let elf_display = if app.elf_path.is_empty() {
            "（按 Enter 输入固件路径）"
        } else {
            &app.elf_path
        };
        render_field(
            f,
            form_chunks[6],
            "📁 ELF 文件",
            elf_display,
            app.focus == Focus::ElfPath,
            false,
        );
    }

    // 烧录按钮
    render_flash_button(f, form_chunks[8], app);
}

/// 渲染单个表单字段
fn render_field(f: &mut Frame, area: Rect, label: &str, value: &str, focused: bool, active: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let cursor = if active && focused { " ▌" } else { "" };

    let text = Text::from(vec![
        Line::from(Span::styled(
            label,
            Style::default().fg(Color::White).bold(),
        )),
        Line::from(Span::styled(
            format!("{}{}", value, cursor),
            Style::default().fg(if focused { Color::White } else { Color::Gray }),
        )),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title_bottom(if focused {
            Line::from(" ◀ 已选中 ▶ ")
                .centered()
                .style(Style::default().fg(Color::Cyan))
        } else {
            Line::from("")
        });

    f.render_widget(Paragraph::new(text).block(block), area);
}

/// 渲染 ELF 路径编辑输入框（带高亮光标和提示）
fn render_elf_input(f: &mut Frame, area: Rect, app: &App) {
    // 输入框使用醒目的绿色/亮色边框
    let border_style = Style::default().fg(Color::Green).bold();

    // 构建显示内容
    let display_text = if app.elf_path.is_empty() {
        // 空输入时显示占位提示
        Span::styled(
            "📝 在此输入 ELF 文件路径... ｜",
            Style::default().fg(Color::DarkGray),
        )
    } else {
        // 已输入文字 + 闪烁光标
        Span::styled(
            format!("📝 {}｜", app.elf_path),
            Style::default().fg(Color::White).bg(Color::Rgb(32, 48, 32)),
        )
    };

    let text = Text::from(vec![
        Line::from(Span::styled(
            "📁 ELF 文件",
            Style::default().fg(Color::White).bold(),
        )),
        Line::from(""),
        Line::from(display_text),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .border_type(ratatui::widgets::BorderType::Thick)
        .title_top(Line::from(" ✏️ 正在编辑 ").style(Style::default().fg(Color::Green).bold()))
        .title_bottom(
            Line::from(" Enter: 确认  |  Esc: 取消  |  Tab: 下一项  |  Backspace: 删除 ")
                .style(Style::default().fg(Color::DarkGray)),
        );

    f.render_widget(Paragraph::new(text).block(block), area);
}

/// 渲染烧录按钮
fn render_flash_button(f: &mut Frame, area: Rect, app: &App) {
    let btn_focused = app.focus == Focus::FlashBtn;
    let btn_style = if btn_focused {
        Style::default().fg(Color::Black).bg(Color::Green).bold()
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };

    let label = if app.mode == InputMode::Flashing {
        "⏳  正在烧录中..."
    } else {
        "🚀  开始烧录 (Enter)"
    };

    let text = Text::from(Line::from(Span::styled(label, btn_style)).centered());
    let border_style = if btn_focused {
        Style::default().fg(Color::Green).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };

    f.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style),
        ),
        area,
    );
}

// ============================================================================
// 下拉选择弹出层
// ============================================================================

fn render_dropdown(f: &mut Frame, parent_area: Rect, app: &App) {
    // 确定当前选择的列表
    let list: &[(&str, &str)] = match app.focus {
        Focus::Backend => presets::BACKENDS,
        Focus::Interface => presets::INTERFACES,
        Focus::Target => presets::TARGETS,
        _ => return,
    };

    let current_idx = match app.focus {
        Focus::Backend => app.backend_idx,
        Focus::Interface => app.iface_idx,
        Focus::Target => app.target_idx,
        _ => return,
    };

    // 计算弹出位置（标题3 + 模式切换一行）
    // 布局：标题(3) + 字段高
    let popup_y = match app.focus {
        Focus::Backend => 6,
        Focus::Interface => 10,
        Focus::Target => 14,
        _ => return,
    };
    let popup_area = Rect {
        x: parent_area.x + 2,
        y: parent_area.y + popup_y,
        width: (parent_area.width - 4).min(60),
        height: (list.len() as u16 + 2).min(12),
    };

    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = list
        .iter()
        .enumerate()
        .map(|(i, (key, desc))| {
            let prefix = if i == current_idx { "▶ " } else { "  " };
            let text = format!("{}{}  —  {}", prefix, key, desc);
            let style = if i == current_idx {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            };
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    let list_widget = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    let mut list_state = app.list_state.clone();
    list_state.select(Some(current_idx));
    f.render_stateful_widget(list_widget, popup_area, &mut list_state);
}

/// 渲染 ELF 文件搜索下拉选择
fn render_elf_dropdown(f: &mut Frame, parent_area: Rect, app: &App) {
    if app.elf_files.is_empty() {
        return;
    }

    // 标题3 + 模式切换 + 后端3+1 + 接口3+1 + 芯片3+1
    let popup_y = 15;
    // ELF 输入框高度不影响下拉位置（下拉在表单外部）
    let elf_rows = app.elf_files.len().min(12);
    let popup_area = Rect {
        x: parent_area.x + 2,
        y: parent_area.y + popup_y,
        width: (parent_area.width - 4).min(80),
        height: (elf_rows as u16 + 2).min(12),
    };

    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = app
        .elf_files
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let prefix = if i == app.elf_file_idx { "▶ " } else { "  " };
            // 显示文件名 + 完整路径
            let display = if let Some(name) = std::path::Path::new(path).file_name() {
                format!("{}  ({})", name.to_string_lossy(), path)
            } else {
                path.clone()
            };
            let text = format!("{}{}", prefix, display);
            let style = if i == app.elf_file_idx {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            };
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    let list_widget = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .title(
                    Line::from(" 📁 找到的 ELF 文件 ")
                        .style(Style::default().fg(Color::Green).bold()),
                ),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    let mut list_state = app.list_state.clone();
    list_state.select(Some(app.elf_file_idx));
    f.render_stateful_widget(list_widget, popup_area, &mut list_state);
}

// ============================================================================
// 烧录结果
// ============================================================================

fn render_result(f: &mut Frame, area: Rect, app: &App) {
    if let Some(ref res) = app.result {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(area);

        let status_color = if res.success {
            Color::Green
        } else {
            Color::Red
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &res.message,
                Style::default().fg(status_color).bold(),
            ))),
            chunks[0],
        );

        let output = format!(
            "命令: {}\n\n--- stdout ---\n{}\n--- stderr ---\n{}",
            res.command,
            res.stdout.as_deref().unwrap_or("（无输出）"),
            res.stderr.as_deref().unwrap_or("（无输出）"),
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if res.success {
                Color::Green
            } else {
                Color::Red
            }))
            .title("输出日志");
        f.render_widget(
            Paragraph::new(output)
                .block(block)
                .wrap(Wrap { trim: false }),
            chunks[1],
        );
    }
}

// ============================================================================
// 状态栏 & 快捷键
// ============================================================================

fn render_status(f: &mut Frame, area: Rect, app: &App) {
    let style = match app.mode {
        InputMode::Flashing => Style::default().fg(Color::Yellow),
        InputMode::Done => {
            if app.result.as_ref().map(|r| r.success).unwrap_or(false) {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            }
        }
        _ => Style::default().fg(Color::Gray),
    };

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(&app.status, style)))
            .block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn render_help(f: &mut Frame, area: Rect, app: &App) {
    let help_text = match app.mode {
        InputMode::Normal | InputMode::EditingElf => {
            "F5: 调试模式  |  Tab/Shift+Tab: 切换字段  |  Enter: 选择/确认/开始烧录  |  Esc: 退出"
        }
        InputMode::Selecting | InputMode::SelectingElf => {
            "↑↓: 移动选择  |  Enter: 确认  |  Esc: 取消  |  e/Tab: 手动输入"
        }
        InputMode::Flashing => "⏳ 烧录进行中，请稍候...",
        InputMode::Done => "Enter/r: 重新烧录  |  F5: 调试模式  |  Esc/q: 退出",
    };

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            help_text,
            Style::default().fg(Color::DarkGray),
        ))),
        area,
    );
}
