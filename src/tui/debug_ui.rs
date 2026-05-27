//! 调试 TUI 面板 — dap-ui 风格布局
//!
//! 六面板布局，参考 nvim-dap-ui：
//! - 左上: 断点列表
//! - 左中: 调用栈
//! - 左下: 监视表达式
//! - 右侧: 变量 / 控制台
//!
//! 快捷键：
//! - F5/Enter:  继续执行
//! - F6/F10:    单步步过
//! - F7/F11:    单步步入
//! - F8/S-F11:  步出函数
//! - F12:       暂停
//! - Ctrl+B:    添加断点
//! - Ctrl+W:    添加监视
//! - Esc/q:     返回

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::debug::session::DebugSession;

/// 调试 UI 渲染入口
pub fn render(f: &mut Frame, app: &DebugAppState, area: Rect) {
    // 布局：工具栏 + 主区域（左右分栏）+ 状态栏
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // 工具栏
            Constraint::Min(10),   // 主体
            Constraint::Length(1), // 状态栏
        ])
        .split(area);

    render_toolbar(f, chunks[0], app);
    render_main(f, chunks[1], app);
    render_status_bar(f, chunks[2], app);
}

// ============================================================================
// 工具栏
// ============================================================================

fn render_toolbar(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let mut spans = vec![
        Span::styled("🔥 DEBUG  ", Style::default().fg(Color::Yellow).bold()),
        Span::styled(
            format!("[{}]  ", app.session.target.clone()),
            Style::default().fg(Color::Cyan),
        ),
    ];

    if let Some(ref stop) = app.session.stop_reason {
        spans.push(Span::styled(
            format!("◼ 停止: {:?}  ", stop),
            Style::default().fg(Color::Red).bold(),
        ));
    } else if app.session.running {
        spans.push(Span::styled(
            "▸ 运行中  ",
            Style::default().fg(Color::Green).bold(),
        ));
    } else {
        spans.push(Span::styled(
            "◼ 已停止  ",
            Style::default().fg(Color::Yellow).bold(),
        ));
    }

    if let Some(ref err) = app.session.last_error {
        spans.push(Span::styled(
            format!("⚠ {}", err),
            Style::default().fg(Color::Red),
        ));
    }

    let line = Line::from(spans);
    f.render_widget(
        Paragraph::new(line).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        area,
    );
}

// ============================================================================
// 主体布局
// ============================================================================

fn render_main(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35), // 左栏
            Constraint::Percentage(65), // 右栏
        ])
        .split(area);

    render_left_panel(f, cols[0], app);
    render_right_panel(f, cols[1], app);
}

// ---- 左栏：断点 + 调用栈 + 监视 ----

fn render_left_panel(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30), // 断点
            Constraint::Percentage(35), // 调用栈
            Constraint::Percentage(35), // 监视
        ])
        .split(area);

    render_breakpoints(f, chunks[0], app);
    render_call_stack(f, chunks[1], app);
    render_watches(f, chunks[2], app);
}

fn render_breakpoints(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let items: Vec<ListItem> = if app.session.breakpoints.is_empty() {
        vec![ListItem::new(
            Span::styled("  无断点 · Ctrl+B 添加", Style::default().fg(Color::DarkGray)),
        )]
    } else {
        app.session
            .breakpoints
            .iter()
            .map(|bp| {
                let icon = if bp.enabled { "●" } else { "○" };
                let loc = format!("{}:{}", bp.file, bp.line);
                let text = format!(
                    " {} {}  {}  {}()",
                    icon, bp.number, loc, bp.func
                );
                let style = if bp.enabled {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                ListItem::new(Span::styled(text, style))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .title(Line::from(" 🔴 断点 ").left_aligned())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red)),
    );
    f.render_widget(list, area);
}

fn render_call_stack(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let items: Vec<ListItem> = if app.session.frames.is_empty() {
        vec![ListItem::new(
            Span::styled("  (等待停止...)", Style::default().fg(Color::DarkGray)),
        )]
    } else {
        app.session
            .frames
            .iter()
            .enumerate()
            .map(|(i, frame)| {
                let prefix = if i == app.session.selected_frame as usize {
                    "▸"
                } else {
                    " "
                };
                let text = format!(
                    " {} #{} {}()  at {}:{}",
                    prefix, frame.level, frame.func, frame.file, frame.line
                );
                let style = if i == app.session.selected_frame as usize {
                    Style::default().fg(Color::Yellow).bold()
                } else {
                    Style::default().fg(Color::Gray)
                };
                ListItem::new(Span::styled(text, style))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .title(Line::from(" 📞 调用栈 ").left_aligned())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(list, area);
}

fn render_watches(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let items: Vec<ListItem> = app
        .session
        .watches
        .iter()
        .map(|(expr, val)| {
            let text = match val {
                Some(v) => format!("  {} = {}", expr, v),
                None => format!("  {} = ?", expr),
            };
            let style = if val.is_some() {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(Span::styled(text, style))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(Line::from(" 👁 监视 ").left_aligned())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta)),
    );
    f.render_widget(list, area);
}

// ---- 右栏：变量 + 控制台 ----

fn render_right_panel(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(35), // 变量
            Constraint::Percentage(30), // RTT 输出
            Constraint::Percentage(35), // 控制台
        ])
        .split(area);

    render_variables(f, chunks[0], app);
    render_rtt_console(f, chunks[1], app);
    render_console(f, chunks[2], app);
}

fn render_variables(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let items: Vec<ListItem> = if app.session.variables.is_empty() {
        vec![ListItem::new(
            Span::styled("  (停止执行后显示变量)", Style::default().fg(Color::DarkGray)),
        )]
    } else {
        app.session
            .variables
            .iter()
            .map(|var| {
                let text = format!(
                    "  {}: {} = {}",
                    var.name, var.vtype, var.value
                );
                ListItem::new(Span::styled(text, Style::default().fg(Color::White)))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .title(Line::from(" 📦 变量 ").left_aligned())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green)),
    );
    f.render_widget(list, area);
}


/// 渲染 RTT 实时输出面板
fn render_rtt_console(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let title = if app.session.rtt_enabled {
        " 📡 RTT 实时输出 "
    } else {
        " 📡 RTT (Ctrl+R 启动) "
    };

    let mut text_lines: Vec<Line> = app
        .session
        .rtt_output
        .iter()
        .rev()
        .take(30)
        .rev()
        .map(|out| {
            let color = match out.channel {
                0 => Color::Green,
                1 => Color::Yellow,
                _ => Color::Gray,
            };
            Line::from(Span::styled(out.text.clone(), Style::default().fg(color)))
        })
        .collect();

    if text_lines.is_empty() {
        text_lines.push(Line::from(Span::styled(
            "  RTT 未启动 · 按 Ctrl+R 启动 probe-rs rtt 实时日志",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let text = Text::from(text_lines);
    f.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .title(Line::from(title).left_aligned())
                    .borders(Borders::ALL)
                    .border_style(if app.session.rtt_enabled {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    }),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}
fn render_console(f: &mut Frame, area: Rect, app: &DebugAppState) {
    let mut text_lines: Vec<Line> = app
        .session
        .console
        .iter()
        .rev()
        .take(30)
        .rev()
        .flat_map(|line| {
            line.lines().map(|l| {
                let style = if l.starts_with('>') {
                    Style::default().fg(Color::Cyan)
                } else if l.starts_with('*') {
                    Style::default().fg(Color::Yellow)
                } else if l.starts_with('^') {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Gray)
                };
                Line::from(Span::styled(l.to_string(), style))
            })
        })
        .collect();

    // 空控制台提示
    if text_lines.is_empty() {
        text_lines.push(Line::from(Span::styled(
            "  GDB MI 控制台 · 命令输出在此显示",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let text = Text::from(text_lines);
    f.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .title(Line::from(" 📝 控制台 ").left_aligned())
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ============================================================================
// 状态栏
// ============================================================================

fn render_status_bar(f: &mut Frame, area: Rect, _app: &DebugAppState) {
    let shortcuts = [
        ("F5", "继续"),
        ("F6", "步过"),
        ("F7", "步入"),
        ("F8", "步出"),
        ("F9", "断点"),
        ("F12", "暂停"),
        ("Ctrl+R", "RTT"),
        ("Esc", "退出"),
    ];

    let spans: Vec<Span> = shortcuts
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(
                    format!(" {} ", key),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} ", desc),
                    Style::default().fg(Color::Gray),
                ),
            ]
        })
        .collect();

    f.render_widget(
        Paragraph::new(Line::from(spans))
            .block(Block::default().borders(Borders::TOP)),
        area,
    );
}

// ============================================================================
// 调试 UI 状态
// ============================================================================

/// 调试 TUI 的应用状态（区别于烧录 TUI 的 App）
#[allow(dead_code)]
pub struct DebugAppState {
    pub session: DebugSession,
    pub should_quit: bool,
    /// 用户正在输入的命令（控制台输入模式）
    pub input_buffer: String,
    pub input_mode: bool,
    /// 添加断点的输入 buffer
    pub bp_input: String,
    pub bp_input_mode: bool,
    /// 添加监视的输入 buffer
    pub watch_input: String,
    pub watch_input_mode: bool,
}

impl DebugAppState {
    pub fn new(session: DebugSession) -> Self {
        Self {
            session,
            should_quit: false,
            input_buffer: String::new(),
            input_mode: false,
            bp_input: String::new(),
            bp_input_mode: false,
            watch_input: String::new(),
            watch_input_mode: false,
        }
    }
}
