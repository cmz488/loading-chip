//! 调试会话状态
//!
//! 统一管理调试目标的状态：断点、调用栈、变量、监视表达式。
//! 作为 GDB MI 客户端与 TUI 之间的桥梁。

use super::protocol::{self, MiRecord, StopReason};

/// 断点信息
#[derive(Debug, Clone)]
pub struct Breakpoint {
    pub number: String,
    pub enabled: bool,
    pub addr: String,
    pub func: String,
    pub file: String,
    pub line: String,
    pub hit_count: u32,
}

/// 调用栈帧
#[derive(Debug, Clone)]
pub struct StackFrame {
    pub level: u32,
    pub addr: String,
    pub func: String,
    pub file: String,
    pub line: String,
}

/// 变量
#[derive(Debug, Clone)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub vtype: String,
}

use super::rtt::RttOutput;

/// 调试会话状态
#[derive(Debug, Clone)]
pub struct DebugSession {
    /// 目标芯片名称
    pub target: String,
    /// 程序是否正在运行
    pub running: bool,
    /// 程序是否已终止
    pub terminated: bool,
    /// 当前停止原因
    pub stop_reason: Option<StopReason>,
    /// 断点列表
    pub breakpoints: Vec<Breakpoint>,
    /// 调用栈
    pub frames: Vec<StackFrame>,
    /// 当前变量
    pub variables: Vec<Variable>,
    /// 监视表达式
    pub watches: Vec<(String, Option<String>)>,
    /// 控制台输出（最近 200 条）
    pub console: Vec<String>,
    /// 状态消息
    pub status: String,
    /// GDB 上次响应是否失败
    pub last_error: Option<String>,
    /// 当前帧编号（用于显示变量）
    pub selected_frame: u32,
    /// RTT 输出缓冲（最近 500 条）
    pub rtt_output: Vec<RttOutput>,
    /// RTT 是否已启用
    pub rtt_enabled: bool,
}

impl DebugSession {
    pub fn new(target: String) -> Self {
        Self {
            target,
            running: true,
            terminated: false,
            stop_reason: None,
            breakpoints: Vec::new(),
            frames: Vec::new(),
            variables: Vec::new(),
            watches: vec![(
                "输入监视表达式 (Ctrl+W)".into(),
                None,
            )],
            console: Vec::new(),
            status: "调试会话就绪".into(),
            last_error: None,
            selected_frame: 0,
            rtt_output: Vec::new(),
            rtt_enabled: false,
        }
    }

    /// 处理收到的 MI 记录，更新调试状态
    pub fn handle_record(&mut self, record: &MiRecord) {
        match record {
            MiRecord::Result { class, .. } => {
                match class {
                    protocol::ResultClass::Error => {
                        self.last_error = record.field("msg").map(|s| s.to_string());
                    }
                    protocol::ResultClass::Done => {
                        self.last_error = None;
                    }
                    _ => {}
                }
            }
            MiRecord::Exec { class, fields: _, .. } => {
                match class {
                    protocol::ExecClass::Running => {
                        self.running = true;
                        self.stop_reason = None;
                    }
                    protocol::ExecClass::Stopped => {
                        self.running = false;
                        self.stop_reason = record.stop_reason();
                    }
                }
            }
            MiRecord::Console(text) => {
                self.console.push(text.clone());
                if self.console.len() > 200 {
                    self.console.remove(0);
                }
            }
            _ => {}
        }
    }

    /// 从 break-list 结果中更新断点列表
    pub fn update_breakpoints_from_response(&mut self, record: &MiRecord) {
        if let MiRecord::Result { class, fields, .. } = record {
            if *class != protocol::ResultClass::Done {
                return;
            }
            // 断点数据在 "BreakpointTable" 字段中
            // 格式: {nr_rows="2",nr_cols="6",hdr=[...],body=[...]}
            if let Some(table) = fields
                .iter()
                .find(|(k, _)| k == "BreakpointTable")
                .map(|(_, v)| v.clone())
            {
                self.breakpoints = parse_breakpoint_table(&table);
            }
        }
    }

    /// 从 stack-list-frames 结果中更新调用栈
    pub fn update_frames_from_response(&mut self, record: &MiRecord) {
        if !record.is_ok() {
            return;
        }
        if let Some(stack) = record
            .find_field("stack")
        {
            self.frames = parse_stack_frames(&stack);
        }
    }

    /// 从 stack-list-variables 结果中更新变量
    pub fn update_variables_from_response(&mut self, record: &MiRecord) {
        if !record.is_ok() {
            return;
        }
        if let Some(vars) = record
            .find_field("variables")
        {
            self.variables = parse_variables(&vars);
        }
    }

    /// 更新监视表达式的值
    pub fn update_watch_value(&mut self, expr: &str, value: String) {
        for (w_expr, w_val) in &mut self.watches {
            if w_expr == expr {
                *w_val = Some(value);
                return;
            }
        }
    }

    /// 添加监视表达式
    pub fn add_watch(&mut self, expr: String) {
        self.watches.push((expr, None));
    }

    /// 移除监视表达式
    pub fn remove_watch(&mut self, index: usize) {
        if index < self.watches.len() {
            self.watches.remove(index);
        }
    }

    /// 在当前源文件位置添加断点
    pub fn current_location(&self) -> Option<(String, String)> {
        self.frames.first().map(|f| (f.file.clone(), f.line.clone()))
    }

    /// 推送 RTT 输出到缓冲区
    pub fn push_rtt(&mut self, output: RttOutput) {
        self.rtt_output.push(output);
        if self.rtt_output.len() > 500 {
            self.rtt_output.remove(0);
        }
    }
}

// ============================================================================
// 解析辅助函数
// ============================================================================

/// 解析 BreakpointTable body 为断点列表
/// 输入格式: body=[bkpt={...},bkpt={...}]
fn parse_breakpoint_table(table: &str) -> Vec<Breakpoint> {
    let mut bps = Vec::new();
    // 简单状态机：找到所有 bkpt={...} 块
    let mut remaining = table;
    while let Some(start) = remaining.find("bkpt={") {
        remaining = &remaining[start + 5..]; // 跳过 "bkpt="
        if let Some(block) = extract_brace_block(remaining) {
            if let Some(bp) = parse_breakpoint_block(&block) {
                bps.push(bp);
            }
            remaining = &remaining[block.len()..];
        } else {
            break;
        }
    }
    bps
}

/// 从 {key="val",key="val",...} 中提取键值对
fn extract_brace_block(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let mut depth = 0;
    for (i, &b) in s.as_bytes()[start..].iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// 解析单个断点块
fn parse_breakpoint_block(block: &str) -> Option<Breakpoint> {
    let inner = block.trim_start_matches('{').trim_end_matches('}');
    let fields: Vec<(String, String)> = protocol::parse_fields_mi(inner);

    let get = |key: &str| -> String {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };

    Some(Breakpoint {
        number: get("number"),
        enabled: get("enabled") != "n",
        addr: get("addr"),
        func: get("func"),
        file: get("file"),
        line: get("line"),
        hit_count: get("times").parse().unwrap_or(0),
    })
}

/// 解析 stack=[...] 为调用栈帧列表
fn parse_stack_frames(stack: &str) -> Vec<StackFrame> {
    let mut frames = Vec::new();
    let mut remaining = stack;
    while let Some(start) = remaining.find("frame={") {
        remaining = &remaining[start + 6..];
        if let Some(block) = extract_brace_block(remaining) {
            if let Some(frame) = parse_frame_block(&block) {
                frames.push(frame);
            }
            remaining = &remaining[block.len()..];
        } else {
            break;
        }
    }
    frames
}

/// 解析单个栈帧块
fn parse_frame_block(block: &str) -> Option<StackFrame> {
    let inner = block.trim_start_matches('{').trim_end_matches('}');
    let fields: Vec<(String, String)> = protocol::parse_fields_mi(inner);

    let get = |key: &str| -> String {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };

    Some(StackFrame {
        level: get("level").parse().unwrap_or(0),
        addr: get("addr"),
        func: get("func"),
        file: get("file"),
        line: get("line"),
    })
}

/// 解析 variables=[...] 为变量列表
fn parse_variables(vars: &str) -> Vec<Variable> {
    let mut result = Vec::new();
    let mut remaining = vars;
    while let Some(start) = remaining.find("name=") {
        // 找当前变量的结束位置
        let rest = &remaining[start..];
        let (var, consumed) = parse_single_variable(rest);
        if let Some(v) = var {
            result.push(v);
        }
        remaining = &remaining[start + consumed..];
    }
    result
}

/// 解析单个变量 "name=\"x\",value=\"42\",type=\"int\""
fn parse_single_variable(s: &str) -> (Option<Variable>, usize) {
    let mut name = String::new();
    let mut value = String::new();
    let mut vtype = String::new();
    let mut pos = 0;

    // 解析 name="..."
    if let Some(val) = extract_key_value(s, "name") {
        name = val;
        pos = s.find(&format!("name=\"{}\"", name)).unwrap_or(0) + name.len() + 7;
    }

    // 解析 value="..."
    if let Some(val) = extract_key_value(&s[pos..], "value") {
        value = val;
    }

    // 解析 type="..."
    if let Some(val) = extract_key_value(s, "type") {
        vtype = val;
    }

    if name.is_empty() {
        return (None, pos);
    }

    (
        Some(Variable {
            name,
            value,
            vtype,
        }),
        pos,
    )
}

/// 从 MI 字段字符串中提取 key="val"
fn extract_key_value(s: &str, key: &str) -> Option<String> {
    let search = format!("{}=\"", key);
    let start = s.find(&search)?;
    let start = start + search.len();
    let mut result = String::new();
    let bytes = s.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some(result),
            b'\\' if i + 1 < bytes.len() => {
                i += 1;
                result.push(bytes[i] as char);
            }
            c => result.push(c as char),
        }
        i += 1;
    }
    Some(result)
}
