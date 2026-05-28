//! RTT 实时监视面板
//!
//! 全屏 RTT 输出，实时滚动显示 target 的调试日志。
//! 支持 probe-rs / OpenOCD / pyOCD 三种后端。
//!
//! 快捷键：
//! - Esc/q:  返回
//! - Ctrl+C: 清空输出

use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use std::process::{Child, Command, Stdio};

use crossbeam_channel::Receiver;

use crate::debug::rtt::{
    RttBackend, RttClient, RttConfig, RttOutput, ProbeRsRtt,
    spawn_openocd_rtt, spawn_pyocd_rtt,
};
use crate::debug::session::DebugSession;

// ============================================================================
// 顶层渲染入口
// ============================================================================

pub fn render(f: &mut Frame, app: &RttMonitorState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // 工具栏
            Constraint::Min(0),    // RTT 输出
            Constraint::Length(1), // 状态栏
        ])
        .split(area);

    render_toolbar(f, chunks[0], app);
    render_rtt_output(f, chunks[1], app);
    render_status_bar(f, chunks[2], app);
}

// ============================================================================
// 工具栏
// ============================================================================

fn render_toolbar(f: &mut Frame, area: Rect, app: &RttMonitorState) {
    let status = if app.running {
        Span::styled(" ● 已连接 ", Style::default().fg(Color::Green).bold())
    } else {
        Span::styled(" ○ 未连接 ", Style::default().fg(TEXT_DIM))
    };

    let chip = Span::styled(
        format!(" {} ", app.session.target),
        Style::default().fg(ACCENT),
    );

    let backend = Span::styled(
        format!("[{}] ", app.backend),
        Style::default().fg(Color::Magenta),
    );

    let count = Span::styled(
        format!(" {} 行 ", app.session.rtt_output.len()),
        Style::default().fg(TEXT_DIM),
    );

    let hint = Span::styled("q 退出 | c 清空 | ↑↓ 滚动", Style::default().fg(BORDER));

    let text = Line::from(vec![
        Span::styled("📡 RTT 监视器 ", Style::default().fg(Color::Yellow).bold()),
        status, chip, backend, count,
        Span::raw(" │ "), hint,
    ]);

    f.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow))),
        area,
    );
}

// ============================================================================
// RTT 输出区域
// ============================================================================

fn render_rtt_output(f: &mut Frame, area: Rect, app: &RttMonitorState) {
    let count = app.session.rtt_output.len();
    if count == 0 {
        let msg = if app.running {
            "⏳ 等待 RTT 数据..."
        } else if app.backend == "gdb" {
            "⚠️ GDB 模式下 RTT 不可用，请使用 GDB 控制台手动连接"
        } else {
            "🔌 RTT 未启动 — 请确保板子已烧录并运行 RTT 固件"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(Color::DarkGray))))
                .block(Block::default().borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))),
            area,
        );
        return;
    }

    let text_lines: Vec<Line> = app.session.rtt_output.iter().map(|out| {
        let color = match out.channel {
            0 => Color::Green,
            1 => Color::Yellow,
            _ => Color::Gray,
        };
        Line::from(Span::styled(&out.text, Style::default().fg(color)))
    }).collect();

    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(text_lines))
            .block(Block::default()
                .title(Line::from(" 📡 实时输出 ").left_aligned())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)))
            .scroll(((count as u16).saturating_sub(area.height.saturating_sub(2)), 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ============================================================================
// 快捷键栏
// ============================================================================

fn render_status_bar(f: &mut Frame, area: Rect, app: &RttMonitorState) {
    let running = app.running;
    let is_gdb = app.backend == "gdb";

    // 左侧：运行状态
    let status_span = if running {
        Span::styled(" ● 采集 ", Style::default().fg(Color::Green).bold())
    } else if is_gdb {
        Span::styled(" ○ GDB模式 ", Style::default().fg(Color::Yellow))
    } else {
        Span::styled(" ○ 待机 ", Style::default().fg(TEXT_DIM))
    };

    // 中间：快捷键
    let mut shortcuts: Vec<(&str, &str)> = vec![
        ("q/Esc", "返回"),
    ];
    if !is_gdb {
        shortcuts.push(("c", "清空"));
        shortcuts.push(("↑↓", "滚动"));
    }

    let key_spans: Vec<Span> = shortcuts.iter().flat_map(|(key, desc)| {
        vec![
            Span::styled(format!(" {} ", key),
                Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}  ", desc), Style::default().fg(TEXT_DIM)),
        ]
    }).collect();

    let mut all_spans = vec![status_span, Span::styled(" │ ", Style::default().fg(BORDER))];
    all_spans.extend(key_spans);

    f.render_widget(
        Paragraph::new(Line::from(all_spans))
            .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(BORDER))),
        area,
    );
}

// 本地颜色常量（与 ui.rs theme 模块保持一致）
const ACCENT:   Color = Color::Rgb(96, 165, 250);
const BORDER:   Color = Color::Rgb(48, 48, 58);
const TEXT_DIM: Color = Color::Rgb(108, 108, 122);

// ============================================================================
// RTT 监视器状态
// ============================================================================

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
        target: String, backend: String, elf_path: String,
        interface: String, port: u16, pyocd_path: String,
    ) -> Self {
        Self {
            session: DebugSession::new(target.clone(), backend.clone()),
            should_quit: false, rtt_client: None, rtt_rx: None, running: false,
            backend, elf_path, server_process: None,
            gdb_port: port, pyocd_path, interface,
        }
    }

    pub fn start_rtt(&mut self) {
        if self.running { return; }
        let (tx, rx) = crossbeam_channel::unbounded();

        match self.backend.as_str() {
            "openocd" => {
                let icfg = crate::backend::mappings::openocd_interface_cfg(&self.interface);
                let tcfg = crate::backend::mappings::openocd_target_cfg(&self.session.target);
                match Command::new("openocd")
                    .args(["-f", icfg, "-f", tcfg, "-c", &format!("gdb_port {}", self.gdb_port)])
                    .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
                {
                    Ok(c) => { self.server_process = Some(c); self.session.push_rtt(RttOutput { channel: 0, text: "🟢 OpenOCD 已启动".into() }); std::thread::sleep(std::time::Duration::from_millis(500)); }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ OpenOCD: {}", e) }); return; }
                }
                match spawn_openocd_rtt(4444, tx) {
                    Ok(c) => { self.rtt_client = Some(Box::new(c)); self.rtt_rx = Some(rx); self.running = true; }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ RTT: {}", e) }); }
                }
            }
            "pyocd" => {
                let t = crate::backend::mappings::pyocd_target(&self.session.target);
                let bin = if self.pyocd_path.is_empty() { "pyocd".into() } else { self.pyocd_path.clone() };
                match Command::new(&bin)
                    .args(["gdbserver", "--target", t, "--port", &self.gdb_port.to_string(), "--telnet-port", "4444"])
                    .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
                {
                    Ok(c) => { self.server_process = Some(c); self.session.push_rtt(RttOutput { channel: 0, text: format!("🟢 pyOCD 已启动 :{}", self.gdb_port) }); std::thread::sleep(std::time::Duration::from_millis(500)); }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ pyOCD: {}", e) }); return; }
                }
                match spawn_pyocd_rtt(4444, tx) {
                    Ok(c) => { self.rtt_client = Some(Box::new(c)); self.rtt_rx = Some(rx); self.running = true; }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ RTT: {}", e) }); }
                }
            }
            "gdb" => {
                self.running = false;
                self.session.push_rtt(RttOutput { channel: 1, text: "⚠️ GDB 模式下 RTT 不可用，请使用 GDB 控制台手动连接".into() });
            }
            _ => {
                let cfg = RttConfig {
                    backend: RttBackend::ProbeRs, chip: self.session.target.clone(),
                    probe: String::new(), telnet_port: 3333, elf_path: Some(self.elf_path.clone()),
                    broadcast: None,
                };
                match ProbeRsRtt::spawn(&cfg, tx) {
                    Ok(c) => { self.rtt_client = Some(Box::new(c)); self.rtt_rx = Some(rx); self.running = true; self.session.push_rtt(RttOutput { channel: 0, text: "📡 RTT 已启动 (probe-rs)".into() }); }
                    Err(e) => { self.session.push_rtt(RttOutput { channel: 1, text: format!("❌ RTT: {}", e) }); }
                }
            }
        }
    }

    pub fn stop_rtt(&mut self) {
        if let Some(mut c) = self.rtt_client.take() { c.stop(); }
        self.rtt_rx = None;
        if let Some(ref mut child) = self.server_process.take() { let _ = child.kill(); let _ = child.wait(); }
        self.running = false;
        self.session.push_rtt(RttOutput { channel: 0, text: "📡 RTT 已断开".into() });
    }

    pub fn poll_rtt(&mut self) {
        if let Some(ref rx) = self.rtt_rx {
            while let Ok(out) = rx.try_recv() { self.session.push_rtt(out); }
        }
    }
}

// ============================================================================
// 按键处理
// ============================================================================

pub fn handle_key(state: &mut RttMonitorState, key: KeyCode) -> bool {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => { state.stop_rtt(); true }
        KeyCode::Char('c') => { state.session.rtt_output.clear(); false }
        _ => false,
    }
}
