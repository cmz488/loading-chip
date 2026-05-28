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

use std::process::{Child, Command, Stdio};

use crossbeam_channel::Receiver;

use crate::debug::rtt::{
    RttBackend, RttClient, RttConfig, RttOutput, ProbeRsRtt,
    spawn_openocd_rtt, spawn_pyocd_rtt,
};
use crate::debug::session::DebugSession;

/// 调试 UI 渲染入口
pub fn render(f: &mut Frame, app: &RttMonitorState, area: Rect) {
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

fn render_toolbar(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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

fn render_main(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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

fn render_left_panel(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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

fn render_breakpoints(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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

fn render_call_stack(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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

fn render_watches(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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

fn render_right_panel(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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

fn render_variables(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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
fn render_rtt_console(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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
fn render_console(f: &mut Frame, area: Rect, app: &RttMonitorState) {
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

fn render_status_bar(f: &mut Frame, area: Rect, _app: &RttMonitorState) {
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

// ============================================================================
// RTT 监视器状态（RTT-only UI）
// ============================================================================

/// RTT 监视器状态 — 仅 RTT 输出的精简调试 UI
pub struct RttMonitorState {
    pub session: DebugSession,
    pub should_quit: bool,
    pub rtt_client: Option<Box<dyn RttClient>>,
    pub rtt_rx: Option<Receiver<RttOutput>>,
    pub running: bool,
    pub backend: String,
    pub elf_path: String,
    pub server_process: Option<Child>,
    pub gdb_port: u16,
    pub pyocd_path: String,
    pub interface: String,
}

impl RttMonitorState {
    pub fn new(
        target: String,
        backend: String,
        elf_path: String,
        interface: String,
        port: u16,
        pyocd_path: String,
    ) -> Self {
        Self {
            session: DebugSession::new(target.clone(), backend.clone()),
            should_quit: false,
            rtt_client: None,
            rtt_rx: None,
            running: false,
            backend,
            elf_path,
            server_process: None,
            gdb_port: port,
            pyocd_path,
            interface,
        }
    }

    pub fn start_rtt(&mut self) {
        if self.running {
            return;
        }

        let (tx, rx) = crossbeam_channel::unbounded();

        match self.backend.as_str() {
            "openocd" => {
                let interface_cfg = crate::backend::mappings::openocd_interface_cfg(&self.interface);
                let target_cfg = crate::backend::mappings::openocd_target_cfg(&self.session.target);
                let gdb_port_str = self.gdb_port.to_string();

                match Command::new("openocd")
                    .args(["-f", interface_cfg, "-f", target_cfg,
                           "-c", &format!("gdb_port {}", gdb_port_str)])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    Ok(child) => {
                        self.server_process = Some(child);
                        self.session.push_rtt(RttOutput {
                            channel: 0,
                            text: "🟢 OpenOCD GDB Server 已启动".into(),
                        });
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                    Err(e) => {
                        self.session.push_rtt(RttOutput {
                            channel: 1,
                            text: format!("❌ 无法启动 OpenOCD: {}", e),
                        });
                        return;
                    }
                }

                match spawn_openocd_rtt(4444, tx) {
                    Ok(client) => {
                        self.rtt_client = Some(Box::new(client));
                        self.rtt_rx = Some(rx);
                        self.running = true;
                        self.session.push_rtt(RttOutput {
                            channel: 0,
                            text: "📡 RTT 已启动 (OpenOCD telnet)".into(),
                        });
                    }
                    Err(e) => {
                        self.session.push_rtt(RttOutput {
                            channel: 1,
                            text: format!("❌ RTT 连接失败: {}", e),
                        });
                    }
                }
            }

            "pyocd" => {
                let target = crate::backend::mappings::pyocd_target(&self.session.target);
                let pyocd_bin = if self.pyocd_path.is_empty() {
                    "pyocd".to_string()
                } else {
                    self.pyocd_path.clone()
                };

                match Command::new(&pyocd_bin)
                    .args(["gdbserver", "--target", target,
                           "--port", &self.gdb_port.to_string(),
                           "--telnet-port", "4444"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    Ok(child) => {
                        self.server_process = Some(child);
                        self.session.push_rtt(RttOutput {
                            channel: 0,
                            text: format!("🟢 pyOCD GDB Server 已启动 (端口 {})", self.gdb_port),
                        });
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                    Err(e) => {
                        self.session.push_rtt(RttOutput {
                            channel: 1,
                            text: format!("❌ 无法启动 pyOCD: {}", e),
                        });
                        return;
                    }
                }

                match spawn_pyocd_rtt(4444, tx) {
                    Ok(client) => {
                        self.rtt_client = Some(Box::new(client));
                        self.rtt_rx = Some(rx);
                        self.running = true;
                        self.session.push_rtt(RttOutput {
                            channel: 0,
                            text: "📡 RTT 已启动 (pyOCD telnet)".into(),
                        });
                    }
                    Err(e) => {
                        self.session.push_rtt(RttOutput {
                            channel: 1,
                            text: format!("❌ RTT 连接失败: {}", e),
                        });
                    }
                }
            }

            "gdb" => {
                self.running = false;
                self.session.push_rtt(RttOutput {
                    channel: 1,
                    text: "⚠️ GDB 模式下 RTT 不可用，请使用 GDB 控制台手动连接".into(),
                });
            }

            _ => {
                // probe-rs (default) — uses CLI-based probe-rs rtt
                let rtt_config = RttConfig {
                    backend: RttBackend::ProbeRs,
                    chip: self.session.target.clone(),
                    probe: String::new(),
                    telnet_port: 3333,
                    elf_path: Some(self.elf_path.clone()),
                };
                match ProbeRsRtt::spawn(&rtt_config, tx) {
                    Ok(client) => {
                        self.rtt_client = Some(Box::new(client));
                        self.rtt_rx = Some(rx);
                        self.running = true;
                        self.session.push_rtt(RttOutput {
                            channel: 0,
                            text: "📡 RTT 已启动 (probe-rs)".into(),
                        });
                    }
                    Err(e) => {
                        self.session.push_rtt(RttOutput {
                            channel: 1,
                            text: format!("❌ RTT 启动失败: {}", e),
                        });
                    }
                }
            }
        }
    }

    pub fn stop_rtt(&mut self) {
        if let Some(mut client) = self.rtt_client.take() {
            client.stop();
        }
        self.rtt_rx = None;
        if let Some(ref mut child) = self.server_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.running = false;
        self.session.push_rtt(RttOutput {
            channel: 0,
            text: "📡 RTT 已断开".into(),
        });
    }

    /// 非阻塞轮询 RTT 输出
    pub fn poll_rtt(&mut self) {
        if let Some(ref rx) = self.rtt_rx {
            while let Ok(output) = rx.try_recv() {
                self.session.push_rtt(output);
            }
        }
    }
}

/// RTT 监视器按键处理
/// 返回 true 表示请求退出
pub fn handle_key(state: &mut RttMonitorState, key: crossterm::event::KeyCode) -> bool {
    match key {
        crossterm::event::KeyCode::Char('q') | crossterm::event::KeyCode::Esc => {
            state.should_quit = true;
            true
        }
        // Ctrl+C: 清空输出
        crossterm::event::KeyCode::Char('c') => {
            state.session.rtt_output.clear();
            false
        }
        _ => false,
    }
}
