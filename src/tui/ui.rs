//! TUI 渲染模块
//!
//! 使用 ratatui 绘制烧录工具的所有界面元素。

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::presets;

use super::app::{App, Focus, InputMode};
use std::sync::atomic::{AtomicU64, Ordering};

static FRAME: AtomicU64 = AtomicU64::new(0);

// ============================================================================
// 配色方案 — 现代暗色主题
// ============================================================================

#[allow(dead_code)]
mod theme {
    use ratatui::style::Color;
    pub const BG:       Color = Color::Rgb(18, 18, 24);
    pub const SURFACE:  Color = Color::Rgb(28, 28, 38);
    pub const BORDER:   Color = Color::Rgb(48, 48, 58);
    pub const TEXT:     Color = Color::Rgb(212, 212, 220);
    pub const TEXT_DIM: Color = Color::Rgb(108, 108, 122);
    pub const ACCENT:   Color = Color::Rgb(96, 165, 250);
    pub const SUCCESS:  Color = Color::Rgb(74, 222, 128);
    pub const ERROR:    Color = Color::Rgb(248, 113, 113);
    pub const WARNING:  Color = Color::Rgb(251, 191, 36);
    pub const CYAN:     Color = Color::Cyan;
    pub const MAGENTA:  Color = Color::Magenta;
}
use theme::*;

// 烧录动画帧
const SPIN_FRAMES: &[&str] = &["\u{23f3}", "\u{231b}"];
const BOUNCE_FRAMES: &[&str] = &["\u{25cf}", "\u{25c9}", "\u{25ce}", "\u{25cc}", "\u{25cb}"];

/// HSV → RGB 转换（用于彩虹渐变）
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u16 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8)
}

// ============================================================================
// 顶层渲染入口
// ============================================================================

/// 主渲染函数，每帧调用
pub fn ui(f: &mut Frame, app: &App) {
    let area = f.area();

    // 主布局：品牌栏 / 模式切换 / 表单 / 状态 / 快捷键
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // 品牌栏 (cmz + 标题)
            Constraint::Length(1), // 模式切换 (烧录/调试)
            Constraint::Min(12),   // 表单
            Constraint::Length(3), // 状态
            Constraint::Length(1), // 快捷键
        ])
        .split(area);

    render_brand_bar(f, chunks[0]);
    render_mode_switch(f, chunks[1], app);

    match app.mode {
        InputMode::Done => render_result(f, chunks[2], app),
        _ => {
            render_form(f, chunks[2], app);
            // 下拉选择时渲染弹出层
            if app.mode == InputMode::Selecting {
                render_dropdown(f, area, app);
            } else if app.mode == InputMode::SelectingElf {
                render_elf_dropdown(f, area, app);
            }
        }
    }

    render_status(f, chunks[3], app);
    render_help(f, chunks[4], app);
}

// ============================================================================
// 品牌栏 — "cmz" 彩虹渐变 + 标题
// ============================================================================

fn render_brand_bar(f: &mut Frame, area: Rect) {
    let frame = FRAME.fetch_add(1, Ordering::Relaxed);
    let hue_base = (frame % 360) as f32;
    let letters = ['c', 'm', 'z'];

    let span_lines: Vec<Span> = letters.iter().enumerate().flat_map(|(i, &ch)| {
        let hue = (hue_base + i as f32 * 120.0) % 360.0;
        let (r, g, b) = hsv_to_rgb(hue, 0.85, 0.95);
        vec![
            Span::styled(ch.to_string(), Style::default().fg(Color::Rgb(r, g, b)).bold()),
            Span::raw(" "),
        ]
    }).collect();

    let brand_line = Line::from(span_lines);
    let title_line = Line::from(Span::styled("🔥  LOADING-CHIP", Style::default().fg(WARNING).bold()));

    f.render_widget(
        Paragraph::new(vec![brand_line, title_line])
            .block(Block::default().borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(WARNING))),
        area,
    );
}

// ============================================================================
// 模式切换 — 烧录 ↔ 调试
// ============================================================================

fn render_mode_switch(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let flash_style = Style::default().fg(Color::Black).bg(SUCCESS).bold();
    let debug_style = if app.focus == Focus::ModeSwitch {
        Style::default().fg(Color::Black).bg(ACCENT).bold()
    } else {
        Style::default().fg(TEXT_DIM)
    };

    let flash_text = Paragraph::new(Line::from(Span::styled(" 🔥 烧录 (F5→调试) ", flash_style)).centered())
        .block(Block::default().borders(Borders::ALL).border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(SUCCESS)));
    let debug_text = Paragraph::new(Line::from(Span::styled(" 🐛 调试 (F5) ", debug_style)).centered())
        .block(Block::default().borders(Borders::ALL).border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(if app.focus == Focus::ModeSwitch { ACCENT } else { BORDER })));

    f.render_widget(flash_text, chunks[0]);
    f.render_widget(debug_text, chunks[1]);
}

// ============================================================================
// 表单区域
// ============================================================================

fn render_form(f: &mut Frame, area: Rect, app: &App) {
    // 固件路径编辑模式下给输入框更多空间
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
            Constraint::Length(elf_height), // 固件路径
            Constraint::Length(1),          // 间距
            Constraint::Length(3),          // 烧录按钮
        ])
        .split(area);

    // 后端字段
    render_field(
        f,
        form_chunks[0],
        "⚙️  烧录后端",
        &app.backend,
        app.focus == Focus::Backend,
        app.mode == InputMode::Selecting && app.focus == Focus::Backend,
    );

    // 接口字段
    render_field(
        f,
        form_chunks[2],
        "🔌 调试接口",
        &app.interface,
        app.focus == Focus::Interface,
        app.mode == InputMode::Selecting && app.focus == Focus::Interface,
    );

    // 芯片字段
    render_field(
        f,
        form_chunks[4],
        "🎯 目标芯片",
        &app.target,
        app.focus == Focus::Target,
        app.mode == InputMode::Selecting && app.focus == Focus::Target,
    );

    // 固件路径字段 — 根据不同模式渲染
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
            "📁 固件文件（从搜索结果选择）",
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
            "📁 固件文件",
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
        Style::default().fg(ACCENT).bold()
    } else {
        Style::default().fg(TEXT_DIM)
    };

    let cursor = if active && focused { " ▌" } else { "" };

    let text = Text::from(vec![
        Line::from(Span::styled(
            label,
            Style::default().fg(TEXT).bold(),
        )),
        Line::from(Span::styled(
            format!("{}{}", value, cursor),
            Style::default().fg(if focused { TEXT } else { TEXT }),
        )),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(border_style)
        .title_bottom(if focused {
            Line::from(" ◀ 已选中 ▶ ")
                .centered()
                .style(Style::default().fg(ACCENT))
        } else {
            Line::from("")
        });

    f.render_widget(Paragraph::new(text).block(block), area);
}

/// 渲染 固件路径编辑输入框（带高亮光标和提示）
fn render_elf_input(f: &mut Frame, area: Rect, app: &App) {
    // 输入框使用醒目的绿色/亮色边框
    let border_style = Style::default().fg(SUCCESS).bold();

    // 构建显示内容
    let display_text = if app.elf_path.is_empty() {
        // 空输入时显示占位提示
        Span::styled(
            "📝 在此输入 固件文件路径... ｜",
            Style::default().fg(TEXT_DIM),
        )
    } else {
        // 已输入文字 + 闪烁光标
        Span::styled(
            format!("📝 {}｜", app.elf_path),
            Style::default().fg(TEXT).bg(Color::Rgb(32, 48, 32)),
        )
    };

    let text = Text::from(vec![
        Line::from(Span::styled(
            "📁 固件文件",
            Style::default().fg(TEXT).bold(),
        )),
        Line::from(""),
        Line::from(display_text),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Thick)
        .border_style(border_style)
        .title_top(Line::from(" ✏️ 正在编辑 ").style(Style::default().fg(SUCCESS).bold()))
        .title_bottom(
            Line::from(" Enter: 确认  |  Esc: 取消  |  Tab: 下一项  |  Backspace: 删除 ")
                .style(Style::default().fg(TEXT_DIM)),
        );

    f.render_widget(Paragraph::new(text).block(block), area);
}

/// 渲染烧录按钮（带动画）
fn render_flash_button(f: &mut Frame, area: Rect, app: &App) {
    let btn_focused = app.focus == Focus::FlashBtn;
    let frame = FRAME.load(Ordering::Relaxed);

    let (btn_style, border_style) = if app.mode == InputMode::Flashing {
        // 脉冲高亮
        let pulse = if (frame / 15) % 2 == 0 { WARNING } else { Color::Rgb(200, 150, 20) };
        (Style::default().fg(Color::Black).bg(pulse).bold(),
         Style::default().fg(pulse).bold())
    } else if btn_focused {
        (Style::default().fg(Color::Black).bg(SUCCESS).bold(),
         Style::default().fg(SUCCESS).bold())
    } else {
        (Style::default().fg(TEXT).bg(SURFACE),
         Style::default().fg(BORDER))
    };

    let label = if app.mode == InputMode::Flashing {
        let spin = SPIN_FRAMES[(frame / 8) as usize % 2];
        let bounce = BOUNCE_FRAMES[(frame / 4) as usize % 5];
        format!("{}  正在烧录中... {}", spin, bounce)
    } else {
        "🚀  开始烧录 (Enter)".to_string()
    };

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(label, btn_style)).centered()).block(
            Block::default().borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(border_style)),
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

    // 计算弹出位置（品牌栏4 + 模式切换1 + 字段高）
    let popup_y = match app.focus {
        Focus::Backend => 8,
        Focus::Interface => 12,
        Focus::Target => 16,
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
                Style::default().fg(SURFACE).bg(ACCENT)
            } else {
                Style::default().fg(TEXT).bg(TEXT_DIM)
            };
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    let list_widget = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    let mut list_state = app.list_state.clone();
    list_state.select(Some(current_idx));
    f.render_stateful_widget(list_widget, popup_area, &mut list_state);
}

/// 渲染 固件文件搜索下拉选择
fn render_elf_dropdown(f: &mut Frame, parent_area: Rect, app: &App) {
    if app.elf_files.is_empty() {
        return;
    }

    // 品牌栏4 + 模式切换1 + 后端3+1 + 接口3+1 + 芯片3+1
    let popup_y = 17;
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
                Style::default().fg(SURFACE).bg(ACCENT)
            } else {
                Style::default().fg(TEXT).bg(TEXT_DIM)
            };
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    let list_widget = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(SUCCESS))
                .title(
                    Line::from(" 📁 找到的 固件文件 ")
                        .style(Style::default().fg(SUCCESS).bold()),
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

        let (status_color, border_color) = if res.success {
            (SUCCESS, SUCCESS)
        } else {
            (ERROR, ERROR)
        };

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(&res.message, Style::default().fg(status_color).bold()))),
            chunks[0],
        );

        let output = format!(
            "命令: {}\n\n--- stdout ---\n{}\n--- stderr ---\n{}",
            res.command,
            res.stdout.as_deref().unwrap_or("（无输出）"),
            res.stderr.as_deref().unwrap_or("（无输出）"),
        );

        f.render_widget(
            Paragraph::new(output)
                .block(Block::default().borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(Style::default().fg(border_color))
                    .title("输出日志"))
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
        InputMode::Flashing => Style::default().fg(WARNING),
        InputMode::Done => {
            if app.result.as_ref().map(|r| r.success).unwrap_or(false) {
                Style::default().fg(SUCCESS)
            } else {
                Style::default().fg(ERROR)
            }
        }
        _ => Style::default().fg(TEXT_DIM),
    };

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(&app.status, style)))
            .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(BORDER))),
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
            Style::default().fg(TEXT_DIM),
        ))),
        area,
    );
}
