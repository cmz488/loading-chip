//! GDB MI 客户端 — 进程管理与命令通信
//!
//! 启动 arm-none-eabi-gdb 进程，使用 MI3 解释器模式，
//! 通过 stdin/stdout 发送命令、接收响应。
//!
//! 异步设计：后台线程持续读取 GDB 输出，将解析后的 MI 记录
//! 通过 `crossbeam-channel` 发送给 UI 线程。

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use super::protocol::{self, MiRecord};

/// GDB MI 客户端
pub struct GdbMi {
    /// GDB 子进程
    child: Option<Child>,
    /// 当前 token（命令序号）
    token: Arc<AtomicU32>,
    /// 后台读取线程
    thread: Option<JoinHandle<()>>,
    /// 是否已停止
    stopped: Arc<AtomicBool>,
    /// ELF 文件路径（留空则不预加载）
    elf_path: String,
}

/// GDB MI 配置
pub struct GdbConfig {
    /// 可执行文件路径（默认 arm-none-eabi-gdb）
    pub gdb_binary: String,
    /// ELF 文件路径（启动时不传入命令行，连接远程后再加载）
    pub elf_path: String,
    /// 远程连接地址（如 localhost:3333），留空则本地加载
    #[allow(dead_code)]
    pub remote: String,
}

impl Default for GdbConfig {
    fn default() -> Self {
        Self {
            gdb_binary: "arm-none-eabi-gdb".into(),
            elf_path: String::new(),
            remote: String::new(),
        }
    }
}

impl GdbMi {
    /// 启动 GDB 进程并返回客户端
    pub fn spawn(
        config: GdbConfig,
        sender: crossbeam_channel::Sender<MiRecord>,
    ) -> std::io::Result<Self> {
        let mut cmd = Command::new(&config.gdb_binary);
        cmd.args(["-q", "--interpreter=mi3"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 注意：ELF 文件不在命令行传入！
        // arm-none-eabi-gdb 读 Xtensa ELF 会卡死在 "Reading symbols"
        // 改为先连远程目标，再用 file-exec-and-symbols 加载符号

        let mut child = cmd.spawn()?;
        let _stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");

        let token = Arc::new(AtomicU32::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped_clone = Arc::clone(&stopped);
        let sender_stdout = sender.clone();

        // 后台线程读取 GDB 输出
        let handle = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();

            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        if let Some(record) = protocol::parse_line(&line) {
                            // 检测 GDB 是否退出
                            if matches!(
                                &record,
                                MiRecord::Result {
                                    class: protocol::ResultClass::Exit,
                                    ..
                                }
                            ) {
                                stopped_clone.store(true, Ordering::SeqCst);
                            }
                            // 发送给 UI
                            if sender_stdout.send(record).is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // 同时读取 stderr 避免阻塞
        let stderr_sender = sender;
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = stderr_sender.send(MiRecord::Log(line));
            }
        });

        let elf_path = config.elf_path;

        Ok(Self {
            child: Some(child),
            token,
            thread: Some(handle),
            stopped,
            elf_path,
        })
    }

    /// 发送一条 MI 命令
    pub fn send_command(&mut self, cmd: &str) -> u32 {
        let token = self.token.fetch_add(1, Ordering::SeqCst);
        let line = protocol::build_command(token, cmd);
        if let Some(ref mut child) = self.child {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(line.as_bytes());
                let _ = stdin.flush();
            }
        }
        token
    }

    /// 加载 ELF 符号文件（连接到远程目标后调用）
    /// 发送 `file-exec-and-symbols <path>` 命令
    /// 避免在 GDB 启动命令行传入与架构不匹配的 ELF
    pub fn load_elf(&mut self) -> Option<u32> {
        if self.elf_path.is_empty() {
            return None;
        }
        let cmd = format!("file-exec-and-symbols {}", self.elf_path);
        Some(self.send_command(&cmd))
    }

    /// 发送 Ctrl+C 中断信号（用于暂停执行）
    pub fn interrupt(&self) {
        // 通过 kill 发送 SIGINT 给子进程
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            if let Some(ref child) = self.child {
                let _ = kill(Pid::from_raw(child.id() as i32), Signal::SIGINT);
            }
        }
        #[cfg(not(unix))]
        {
            // Windows: 创建 Ctrl-C 事件到进程组
            if let Some(ref child) = self.child {
                let _ = unsafe {
                    windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent(
                        0, // CTRL_C_EVENT
                        child.id(),
                    )
                };
            }
        }
    }

    /// 是否已停止
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// 关闭 GDB 并等待后台线程退出
    pub fn shutdown(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(mut child) = self.child.take() {
            // 发送 quit 命令优雅退出
            if let Some(ref mut stdin) = child.stdin {
                let _ = writeln!(stdin, "-gdb-exit");
                let _ = stdin.flush();
            }
            let _ = child.wait();
        }
        if let Some(handle) = self.thread.take() {
            // 不阻塞 — 线程会在 GDB 退出时自动结束
            let _ = handle.join();
        }
    }
}

impl Drop for GdbMi {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_gdb_detects_missing_binary() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let result = GdbMi::spawn(
            GdbConfig {
                gdb_binary: "nonexistent-gdb".into(),
                ..Default::default()
            },
            tx,
        );
        assert!(result.is_err());
    }
}
