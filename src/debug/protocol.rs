//! GDB MI 协议解析
//!
//! GDB Machine Interface (MI) 是 GDB 的机器可读接口，
//! 输出为结构化记录，每条记录的第一行以 `(token?)` 开头。
//!
//! ## 记录类型
//! - `^done` — 同步命令成功
//! - `^error` — 同步命令失败
//! - `*stopped` — 异步：执行停止（断点/单步完成）
//! - `*running` — 异步：执行继续
//! - `~"...'` — 控制台输出流
//! - `&"...'` — 日志流

/// MI 记录
#[derive(Debug, Clone)]
pub enum MiRecord {
    /// 同步结果：命令执行完成（done / error）
    Result {
        #[allow(dead_code)]
        token: Option<u32>,
        class: ResultClass,
        fields: Vec<(String, String)>,
        #[allow(dead_code)]
        raw: String,
    },
    /// 异步执行状态：*running / *stopped
    Exec {
        class: ExecClass,
        fields: Vec<(String, String)>,
        #[allow(dead_code)]
        raw: String,
    },
    /// 控制台输出：~"..."
    Console(String),
    /// 日志输出：&"..."
    Log(String),
    /// 未识别的其他行
    Other(String),
}

/// 同步结果类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultClass {
    Done,
    Error,
    Running,
    Connected,
    Exit,
}

impl ResultClass {
    pub fn from_str(s: &str) -> Self {
        match s {
            "done" => Self::Done,
            "error" => Self::Error,
            "running" => Self::Running,
            "connected" => Self::Connected,
            "exit" => Self::Exit,
            _ => Self::Done,
        }
    }
}

/// 异步执行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecClass {
    Stopped,
    Running,
}

/// 停止原因
#[derive(Debug, Clone)]
pub enum StopReason {
    BreakpointHit { bkpt_no: String },
    EndSteppingRange,
    FunctionFinished,
    SignalReceived { name: String },
    ExitedNormally,
    Exited { code: i32 },
    Unknown(String),
}

// ============================================================================
// MI 解析器
// ============================================================================

/// 解析一行 MI 输出
pub fn parse_line(line: &str) -> Option<MiRecord> {
    let line = line.trim();
    if line.is_empty() || line == "(gdb)" {
        return None;
    }

    // 跳过 token 前缀，找到 ^/*/~ /& 标记
    let prefix = line.find(|c: char| ['^', '*', '~', '&', '='].contains(&c))?;
    let stripped = &line[..prefix]; // token 部分
    let rec = &line[prefix..]; // 记录部分
    let _ = stripped; // token 在 parse_result 中处理

    let bytes = rec.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    match bytes[0] {
        b'^' => parse_result(line),
        b'*' => parse_exec(line),
        b'~' => parse_console(line),
        b'&' => parse_log(line),
        b'=' => parse_notify(line),
        _ => Some(MiRecord::Other(line.to_string())),
    }
}

fn parse_result(line: &str) -> Option<MiRecord> {
    // 格式: ^done[,field=value,...]
    // 或带 token: 123^done[,field=value,...]
    let (token, rest) = parse_token(line);

    let content = rest.strip_prefix('^')?;
    let (class_str, fields) = split_class_and_fields(content);

    Some(MiRecord::Result {
        token,
        class: ResultClass::from_str(class_str),
        fields,
        raw: line.to_string(),
    })
}

fn parse_exec(line: &str) -> Option<MiRecord> {
    let content = line.strip_prefix('*')?;
    let (class_str, fields) = split_class_and_fields(content);

    let class = match class_str {
        "stopped" => ExecClass::Stopped,
        "running" => ExecClass::Running,
        _ => return Some(MiRecord::Other(line.to_string())),
    };

    Some(MiRecord::Exec {
        class,
        fields,
        raw: line.to_string(),
    })
}

fn parse_console(line: &str) -> Option<MiRecord> {
    let text = extract_quoted(line.strip_prefix('~')?);
    Some(MiRecord::Console(text.unwrap_or_default()))
}

fn parse_log(line: &str) -> Option<MiRecord> {
    let text = extract_quoted(line.strip_prefix('&')?);
    Some(MiRecord::Log(text.unwrap_or_default()))
}

fn parse_notify(line: &str) -> Option<MiRecord> {
    // 通知记录: =classname[,field=value]
    Some(MiRecord::Other(line.to_string()))
}

// ============================================================================
// 解析辅助函数
// ============================================================================

/// 解析 token 前缀，如 "123^done" → (Some(123), "done")
fn parse_token(line: &str) -> (Option<u32>, &str) {
    if let Some(digit_end) = line.find(|c: char| !c.is_ascii_digit()) {
        if let Ok(n) = line[..digit_end].parse::<u32>() {
            return (Some(n), &line[digit_end..]);
        }
    }
    (None, line)
}

/// 分割 class 和 fields: "done,reason="breakpoint-hit",...")
fn split_class_and_fields(content: &str) -> (&str, Vec<(String, String)>) {
    if let Some(comma) = content.find(',') {
        let class = &content[..comma];
        let fields_str = &content[comma + 1..];
        (class, parse_fields(fields_str))
    } else {
        (content, Vec::new())
    }
}

/// 解析 CSV 风格的 key=value 字段（公开版本）
pub fn parse_fields_mi(s: &str) -> Vec<(String, String)> {
    parse_fields(s)
}

/// 解析 CSV 风格的 key=value 字段
fn parse_fields(s: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let mut i = 0;
    let bytes = s.as_bytes();

    while i < bytes.len() {
        // 跳过空白
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        // 找 key
        let eq = match s[i..].find('=') {
            Some(pos) => i + pos,
            None => break,
        };
        let key = s[i..eq].to_string();

        i = eq + 1;

        // 解析 value
        let value = if i < bytes.len() && bytes[i] == b'"' {
            // 双引号字符串
            let (val, next) = parse_quoted_field(&s[i..]);
            i = if next > 0 { i + next } else { bytes.len() };
            val.unwrap_or_default()
        } else if i < bytes.len() && bytes[i] == b'{' {
            // 列表: {a,b,c}
            let (val, next) = parse_braced_list(&s[i..]);
            i = if next > 0 { i + next } else { bytes.len() };
            val.unwrap_or_default()
        } else {
            // 纯 token
            let end = s[i..]
                .find(',')
                .unwrap_or(s.len() - i);
            let val = s[i..i + end].to_string();
            i += end;
            val
        };

        fields.push((key, value));

        // 跳过逗号
        if i < bytes.len() && bytes[i] == b',' {
            i += 1;
        }
    }
    fields
}

/// 解析双引号包裹的值
fn parse_quoted_field(s: &str) -> (Option<String>, usize) {
    if !s.starts_with('"') {
        return (None, 0);
    }
    let mut result = String::new();
    let bytes = s.as_bytes();
    let mut i = 1; // 跳过开头的 "

    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                return (Some(result), i + 1);
            }
            b'\\' if i + 1 < bytes.len() => {
                i += 1;
                match bytes[i] {
                    b'n' => result.push('\n'),
                    b't' => result.push('\t'),
                    b'r' => result.push('\r'),
                    b'\\' => result.push('\\'),
                    b'"' => result.push('"'),
                    c => result.push(c as char),
                }
            }
            c => result.push(c as char),
        }
        i += 1;
    }
    (Some(result), s.len())
}

/// 解析 {a,b,c...} 列表
fn parse_braced_list(s: &str) -> (Option<String>, usize) {
    if !s.starts_with('{') {
        return (None, 0);
    }
    let mut depth = 0;
    for (i, &b) in s.as_bytes().iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let content = &s[1..i];
                    return (Some(format!("{{{}}}", content)), i + 1);
                }
            }
            _ => {}
        }
    }
    (Some(s[1..].to_string()), s.len())
}

/// 从 MI 双引号字符串中提取内容
fn extract_quoted(s: &str) -> Option<String> {
    parse_quoted_field(s).0
}

// ============================================================================
// 字段提取辅助
// ============================================================================

impl MiRecord {
    /// 从 Result/Exec 记录中查找字段值
    pub fn field(&self, key: &str) -> Option<&str> {
        let fields = match self {
            MiRecord::Result { fields, .. } => fields,
            MiRecord::Exec { fields, .. } => fields,
            _ => return None,
        };
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// 从 Result/Exec 记录中查找字段值（owned 版本）
    pub fn find_field(&self, key: &str) -> Option<String> {
        self.field(key).map(|s| s.to_string())
    }

    /// 判断 Result 是否成功
    pub fn is_ok(&self) -> bool {
        matches!(
            self,
            MiRecord::Result {
                class: ResultClass::Done,
                ..
            }
        )
    }

    /// 判断是否是停止事件
    pub fn is_stopped(&self) -> bool {
        matches!(
            self,
            MiRecord::Exec {
                class: ExecClass::Stopped,
                ..
            }
        )
    }

    /// 提取停止原因
    pub fn stop_reason(&self) -> Option<StopReason> {
        let reason = self.field("reason")?;
        match reason {
            "breakpoint-hit" => Some(StopReason::BreakpointHit {
                bkpt_no: self.field("bkptno").unwrap_or("?").to_string(),
            }),
            "end-stepping-range" => Some(StopReason::EndSteppingRange),
            "function-finished" => Some(StopReason::FunctionFinished),
            "signal-received" => Some(StopReason::SignalReceived {
                name: self.field("signal-name").unwrap_or("?").to_string(),
            }),
            "exited-normally" => Some(StopReason::ExitedNormally),
            "exited" => {
                let code = self
                    .field("exit-code")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                Some(StopReason::Exited { code })
            }
            _ => Some(StopReason::Unknown(reason.to_string())),
        }
    }
}

// ============================================================================
// MI 命令构建
// ============================================================================

/// 构建 MI 命令行（自动添加递增 token）
pub fn build_command(token: u32, cmd: &str) -> String {
    format!("{}-{}\n", token, cmd)
}

/// 常用 MI 命令（不含 token）
pub mod commands {
    /// 目标选择：连接远程 GDB Server
    pub const TARGET_SELECT_REMOTE: &str = "target-select remote :3333";
    /// 设置断点
    pub fn break_insert(location: &str) -> String {
        format!("break-insert {}", location)
    }
    /// 删除断点
    pub fn break_delete(ids: &[u32]) -> String {
        let ids_str: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
        format!("break-delete {}", ids_str.join(" "))
    }
    /// 列出所有断点
    pub const BREAK_LIST: &str = "break-list";
    /// 运行程序
    pub const EXEC_RUN: &str = "exec-run";
    /// 继续执行
    pub const EXEC_CONTINUE: &str = "exec-continue";
    /// 单步步入
    pub const EXEC_STEP: &str = "exec-step";
    /// 单步步过
    pub const EXEC_NEXT: &str = "exec-next";
    /// 跳出函数
    pub const EXEC_FINISH: &str = "exec-finish";
    /// 暂停
    pub const EXEC_INTERRUPT: &str = "exec-interrupt";
    /// 获取调用栈
    pub const STACK_LIST_FRAMES: &str = "stack-list-frames";
    /// 获取局部变量
    pub const STACK_LIST_VARIABLES_ALL: &str = "stack-list-variables --all-values";
    /// 获取指定帧的局部变量
    pub fn stack_list_variables(thread: u32, frame: u32) -> String {
        format!("stack-list-variables --thread {} --frame {} --all-values", thread, frame)
    }
    /// 计算表达式
    pub fn data_evaluate_expression(expr: &str) -> String {
        format!("data-evaluate-expression \"{}\"", expr)
    }
    /// 设置变量
    pub fn var_create(_name: &str, expr: &str) -> String {
        format!("var-create - * \"{}\"", expr)
    }
    /// 更新变量对象
    pub const VAR_UPDATE_ALL: &str = "-var-update --all-values *";
    /// 读取寄存器
    pub const DATA_LIST_REGISTER_NAMES: &str = "data-list-register-names";
    /// 读取指定寄存器值
    pub fn data_list_register_values(fmt: &str, regs: &[&str]) -> String {
        let regs_str = regs.join(" ");
        format!("data-list-register-values {} {}", fmt, regs_str)
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_done() {
        let rec = parse_line(r#"^done,bkpt={number="1",file="main.c"}"#).unwrap();
        assert!(rec.is_ok());
        assert_eq!(rec.field("bkpt").unwrap(), r#"{number="1",file="main.c"}"#);
    }

    #[test]
    fn parse_stopped_breakpoint() {
        let rec = parse_line(r#"*stopped,reason="breakpoint-hit",bkptno="2",thread-id="1""#).unwrap();
        assert!(rec.is_stopped());
    }

    #[test]
    fn parse_console() {
        if let Some(MiRecord::Console(text)) = parse_line(r#"~"hello\n""#) {
            assert_eq!(text, "hello\n");
        } else {
            panic!("Expected Console");
        }
    }

    #[test]
    fn parse_error() {
        if let Some(MiRecord::Result { class, .. }) = parse_line(r#"^error,msg="timeout""#) {
            assert_eq!(class, ResultClass::Error);
        } else {
            panic!("Expected error result");
        }
    }

    #[test]
    fn parse_running() {
        if let Some(MiRecord::Exec { class, .. }) = parse_line(r#"*running,thread-id="1""#) {
            assert_eq!(class, ExecClass::Running);
        } else {
            panic!("Expected running");
        }
    }

    #[test]
    fn parse_with_token() {
        let rec = parse_line(r#"42^done"#).unwrap();
        if let MiRecord::Result { token, class, .. } = rec {
            assert_eq!(token, Some(42));
            assert_eq!(class, ResultClass::Done);
        } else {
            panic!("Expected result with token");
        }
    }
}
