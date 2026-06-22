# loading-chip 代码审查 & 架构问题报告

> 生成日期: 2026-06-22
> 审查范围: 全仓库 + 未提交变更 (`Cargo.toml`, `Cargo.lock`, `src/tui/app.rs`)

---

## 问题 #1 [P0 - 构建阻塞]: `fff-search` 使用 nightly 预发布版本，无法从 crates.io 解析

### 位置

`Cargo.toml:33`

```toml
fff-search = "0.9.4-nightly.8092cfa"
```

### 问题描述

`fff-search` 的 `0.9.4-nightly.8092cfa` 版本在 crates.io 上不存在（该 crate 最新稳定版为 `0.9.3`）。Cargo 无法解析此版本号，**任何 `cargo build` 都会直接失败**。

此外，nightly 版本的 API 无稳定性保证——即使改为 git 依赖，后续该 crate 的作者可能变更加入 `FilePickerOptions`、`SharedFilePicker::read()` 等方法签名，导致编译错误。

### 修改建议

**方案 A (推荐)**: 使用 crates.io 稳定版：

```toml
# Cargo.toml
fff-search = "0.9.3"
```

然后确认 `0.9.3` 是否包含当前代码中使用的 API（`SharedFilePicker::read()`、`FilePicker::new_with_shared_state`、`SharedQueryTracker::read()`、`FrecencyTracker::open` 等）。如果 API 不兼容，需要适配。

**方案 B**: 固定到 git commit：

```toml
# Cargo.toml
fff-search = { git = "https://github.com/nicholasxuu/fff-search", rev = "8092cfa" }
```

**方案 C** (如果上述方案均不可行): 回退到原先的递归文件扫描方案，保留 `scan_for_firmware` 作为后备路径。在 `fff-search` 发布稳定版后再集成：

```rust
// src/tui/app.rs
pub fn search_elf_files(&mut self) {
    self.elf_files.clear();
    self.elf_file_idx = 0;

    // 优先尝试 fff-search（feature-gated）
    #[cfg(feature = "fff-search")]
    if self.try_fff_search() {
        return;
    }

    // fallback: 传统递归扫描
    if let Ok(cwd) = std::env::current_dir() {
        self.scan_for_firmware(&cwd, 0, 5);
    }
    self.elf_files.sort_by_key(|p| p.len());
}
```

---

## 问题 #2 [P0 - 资源泄漏]: `std::process::exit` 跳过析构函数，导致子进程残留和磁盘泄漏

### 位置

`src/lib.rs` — 5 处调用：

| 行号 | 上下文 | 函数 |
|------|--------|------|
| 238 | JSON 输出错误后退出 | `run_headless` |
| 243 | 烧录成功/失败后退出 | `run_headless` |
| 257 | 缺少必填参数后退出 | `run_headless` |
| 287 | FlashConfig 构建失败 | `run_cli_mode` |
| 302 | 烧录成功/失败后退出 | `run_cli_mode` |

典型代码：

```rust
// src/lib.rs:238-243
let result = do_flash(&config);
println!("{}", serde_json::to_string_pretty(&result).unwrap());
std::process::exit(if result.success { 0 } else { 1 });  // ← 跳过析构
```

### 问题分析

`std::process::exit` 立即终止进程，**不执行任何析构函数**。后果：

1. **子进程泄露**：如果 API 模式下启动了 tokio runtime（`src/lib.rs:176-178`），`_api_shutdown` 的 Drop 不会被调用，后台线程和 tokio runtime 不会优雅关闭
2. **LMDB 资源泄露**：`FffSearchState` 持有的 LMDB 环境不会被正确关闭，可能损坏数据库
3. **终端状态丢失**：TUI 的 `LeaveAlternateScreen` / `disable_raw_mode` 不会被调用

### 修改建议

改为返回 `Result`，由 `main` 统一处理退出码：

```rust
// src/lib.rs
pub fn run() -> io::Result<()> {
    let cli = cli::Cli::parse();
    let registry = board::BoardRegistry::load().map_err(io::Error::other)?;
    let state = Arc::new(AppState::new(registry));

    let exit_code = match cli.command {
        // ... 各分支返回 ExitCode
    };

    if exit_code != 0 {
        std::process::exit(exit_code); // 仅在此处调用，此时所有资源已释放
    }
    Ok(())
}
```

具体地，`run_headless` 应返回 `io::Result<i32>`，将 `process::exit` 替换为退出码传播：

```rust
fn run_headless(/* ... */) -> io::Result<i32> {
    // ... 构建 config ...
    let config = match FlashConfig::from_registry(/* ... */) {
        Ok(cfg) => cfg,
        Err(err) => {
            let result = flash::FlashResult { success: false, message: err, /* ... */ };
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            return Ok(1); // ← 返回退出码，让 main 统一 exit
        }
    };
    let result = do_flash(&config);
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    Ok(if result.success { 0 } else { 1 })
}
```

同样 `run_cli_mode` 返回 `io::Result<i32>`。

---

## 问题 #3 [P0 - 资源泄漏]: `thread::park()` 作为永久等待 — 无优雅关闭机制

### 位置

`src/lib.rs:185-187`

```rust
if _api_shutdown.is_some() {
    eprintln!("🟢 API 运行中（按 Ctrl+C 退出）...");
    std::thread::park();
}
```

### 问题分析

`std::thread::park()` 等待一个永远不会到来的 `unpark()`。Ctrl+C 发送 SIGINT，但：
- 如果未注册 SIGINT handler，进程被直接杀死——同样跳过析构
- 即使注册了 handler，`park()` 也不会被唤醒

### 修改建议

```rust
// 替换为:
if _api_shutdown.is_some() {
    eprintln!("🟢 API 运行中（按 Ctrl+C 退出）...");
    // 等待 Ctrl+C 信号
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            tokio::signal::ctrl_c().await.ok();
            eprintln!("\n正在关闭...");
        });
    // shutdown_tx 在此处 drop，触发 graceful shutdown
}
```

或者如果不想引入 tokio（但项目已有 tokio 依赖），使用 `signal_hook` crate：

```rust
let (tx, rx) = std::sync::mpsc::channel();
signal_hook::flag::register(signal_hook::consts::SIGINT, tx.clone())?;
signal_hook::flag::register(signal_hook::consts::SIGTERM, tx)?;
rx.recv().ok();
eprintln!("\n正在关闭...");
```

---

## 问题 #4 [P1 - 维护负担]: 烧录逻辑在三处独立实现中重复

### 位置

| 函数 | 文件:行号 | 代码量 |
|------|-----------|--------|
| `AppState::flash` | `src/app/state.rs:72-136` | 65 行 |
| `App::do_flash` | `src/tui/app.rs:399-426` | 28 行 |
| `run_headless` | `src/lib.rs:205-260` | 56 行 |
| `run_cli_mode` | `src/lib.rs:263-303` | 41 行 |

### 核心重复逻辑

四处的核心逻辑完全一致：

```
1. FlashBackend::from_str(&backend)
2. FlashConfig::from_registry(be, registry, target, interface, elf, ...)
3. 错误处理 → 构建 FlashResult { success: false }
4. do_flash(&config)
5. 输出结果
```

唯独差异在于**输出格式**：
- `AppState::flash`: 返回 `FlashResult`（供 API handler 序列化为 JSON）
- `App::do_flash`: 写入 `self.status` / `self.result`（供 TUI 渲染）
- `run_headless`: `println!(serde_json::to_string_pretty(...))`（供 IDE 解析）
- `run_cli_mode`: `println!` + `eprintln!`（供终端用户阅读）

### 修改建议

将烧录流程收敛为单一函数 + 结果回调：

```rust
// src/app/state.rs
impl AppState {
    /// 统一烧录入口 — 所有模式共用
    ///
    /// `on_result` 在烧录完成时调用（无论成功或失败），
    /// 各模式在回调中处理自身的输出格式。
    pub async fn flash_with<F>(
        &self,
        backend: &str,
        board_id: &str,
        interface: &str,
        elf_path: &str,
        gdb_port: &str,
        pyocd_path: &str,
        timeout_secs: u64,
        on_result: F,
    ) -> FlashResult
    where
        F: FnOnce(&FlashResult),
    {
        let be = FlashBackend::from_str(backend);
        // 更新状态锁...
        *self.run_state.lock().await = RunState::Flashing;
        // ...

        let config = match FlashConfig::from_registry(
            be, &self.registry, board_id, interface, elf_path,
            gdb_port, pyocd_path, timeout_secs,
        ) {
            Ok(cfg) => cfg,
            Err(msg) => {
                let result = FlashResult { success: false, message: msg, /* ... */ };
                *self.last_result.lock().await = Some(result.clone());
                *self.run_state.lock().await = RunState::FlashDone;
                on_result(&result);
                return result;
            }
        };

        let result = do_flash(&config);
        *self.last_result.lock().await = Some(result.clone());
        *self.run_state.lock().await = RunState::FlashDone;
        on_result(&result);
        result
    }
}
```

各调用方变为：

```rust
// TUI (src/tui/app.rs)
pub fn do_flash(&mut self) {
    self.mode = InputMode::Flashing;
    let app_state = self.state.clone();
    // TUI 主循环是同步的，需要 block_on
    let rt = tokio::runtime::Handle::current();
    let _ = rt.block_on(app_state.flash_with(
        &self.backend, &self.target, &self.interface, &self.elf_path,
        &self.gdb_port, &self.pyocd_path, self.timeout_secs,
        |result| {
            // 回调中更新 UI 状态（需要小心跨线程）
        },
    ));
}

// Headless (src/lib.rs)
let exit_code = rt.block_on(state.flash_with(
    &backend, &target, &interface, &elf, &gdb_port, &pyocd_path, timeout,
    |result| {
        println!("{}", serde_json::to_string_pretty(result).unwrap());
    },
));
```

---

## 问题 #5 [P1 - 磁盘泄漏]: LMDB 临时目录从不清理

### 位置

`src/tui/app.rs:356-360`

```rust
let tmp = std::env::temp_dir().join("loading-chip-fff");
if let Err(e) = std::fs::create_dir_all(&tmp) {
    self.status = format!("FFF 搜索初始化失败: 无法创建临时目录 ({})", e);
    return;
}
```

### 问题分析

- `/tmp/loading-chip-fff/` 在每次应用启动时创建，但 **从未被删除**
- 每次运行留下 ~MB 级别的 LMDB 数据文件
- LMDB 使用 `mmap`，如果进程崩溃，数据库可能处于不一致状态，影响下次启动
- Linux 的 `/tmp` 在重启时清理，但 `XDG_RUNTIME_DIR` 或 macOS 的临时目录不会

### 修改建议

使用 `tempfile::TempDir`（项目中已有 `tempfile` 依赖吗？如果没有，添加它）：

```rust
// Cargo.toml
tempfile = "3"

// src/tui/app.rs
use tempfile::TempDir;

pub(crate) struct FffSearchState {
    shared_picker: fff_search::SharedFilePicker,
    shared_frecency: fff_search::SharedFrecency,
    shared_query_tracker: fff_search::SharedQueryTracker,
    /// 持有 TempDir 确保 LMDB 数据在 drop 时自动清理
    _temp_dir: TempDir,
}

fn init_fff_search(&mut self) {
    // ...
    let temp_dir = match tempfile::TempDir::with_prefix("loading-chip-fff") {
        Ok(d) => d,
        Err(e) => {
            self.status = format!("FFF 搜索初始化失败: 无法创建临时目录 ({})", e);
            return;
        }
    };

    let frecency = fff_search::dbs::frecency::FrecencyTracker::open(
        temp_dir.path().join("frecency")
    ).map_err(/* ... */)?;
    // ...

    self.fff_search_state = Some(FffSearchState {
        shared_picker,
        shared_frecency,
        shared_query_tracker,
        _temp_dir: temp_dir, // TempDir::drop() 自动删除目录
    });
}
```

`TempDir` 在 `FffSearchState` 被 drop 时自动清理全部 LMDB 文件——无需手动调用 `remove_dir_all`。

---

## 问题 #6 [P1 - 运行时无感知]: API 服务器启动失败被静默吞掉

### 位置

`src/api/server.rs:53-88` — `spawn_server`

```rust
pub fn spawn_server(
    state: AppState,
    addr: String,
) -> tokio::sync::oneshot::Sender<()> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");

        rt.block_on(async {
            let app = routes::api_router().with_state(state.clone());

            let listener = match TcpListener::bind(&addr).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("❌ API 绑定失败: {}", e);
                    return; // ← 线程静默退出，调用者无感知！
                }
            };
            // ...
        });
    });

    shutdown_tx // ← 只返回 shutdown sender，无启动状态
}
```

### 问题分析

`spawn_server` 返回给调用者的是一个 `oneshot::Sender<()>`。调用者拿到这个 sender 后认为"服务器已就绪"，但实际上：

1. `TcpListener::bind` 可能因端口被占用而失败——后台线程静默退出，`eprintln!` 可能被 TUI 覆盖
2. 调用者调用 `_api_shutdown.send(())` 时，receiver 已经随线程 drop 了——send 返回 `Err`
3. 但这个 `Err` 在 `lib.rs:176` 的 `let _api_shutdown = ...` 中被丢弃（`let _` 不触发 `#[must_use]` 警告）

### 修改建议

**方案 A**：使用 `oneshot` 双向通信，启动完成后通知调用者：

```rust
pub fn spawn_server(
    state: AppState,
    addr: String,
) -> Result<tokio::sync::oneshot::Sender<()>, String> {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");

        rt.block_on(async {
            let listener = match TcpListener::bind(&addr).await {
                Ok(l) => l,
                Err(e) => {
                    let _ = started_tx.send(Err(format!("绑定 {}: {}", addr, e)));
                    return;
                }
            };

            // 绑定成功，通知调用者
            let _ = started_tx.send(Ok(()));

            let app = routes::api_router().with_state(state);
            axum::serve(listener, app)
                .with_graceful_shutdown(async { let _ = shutdown_rx.await; })
                .await
                .unwrap_or_else(|e| eprintln!("⚠️ API 服务器: {}", e));
        });
    });

    // 等待启动确认（最多 3 秒）
    match started_rx.blocking_recv() {
        Ok(Ok(())) => Ok(shutdown_tx),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("API 服务器线程意外退出".into()),
    }
}
```

调用方（`src/lib.rs:176`）：

```rust
let _api_shutdown = if api {
    match api::spawn_server((*state).clone(), api_addr.clone()) {
        Ok(shutdown) => Some(shutdown),
        Err(e) => {
            eprintln!("❌ API 启动失败: {}", e);
            None
        }
    }
} else {
    None
};
```

---

## 问题 #7 [P1 - 静默错误]: `FlashBackend::from_str` 对非法输入静默回退

### 位置

`src/backend.rs:69-76`

```rust
pub fn from_str(s: &str) -> Self {
    match s {
        "openocd" => Self::OpenOcd,
        "probe-rs" => Self::ProbeRs,
        "pyocd" => Self::PyOcd,
        _ => Self::Gdb,  // ← 任何拼写错误都静默回退到 GDB
    }
}
```

### 影响范围

所有调用处都依赖此行为：
- `src/app/state.rs:82` — API 烧录 handler
- `src/tui/app.rs:403` — TUI 烧录
- `src/lib.rs:217` — headless 模式
- `src/lib.rs:273` — CLI 模式
- `src/tui/debug_ui.rs` — RTT 后端选择（有独立的 `RttBackend::from_str`，同样静默回退）

### 问题分析

如果用户在 TUI 中输入 `"probe_rs"`（下划线而非连字符），或 API 客户端发送 `"backend": "gdb-server"`：
- 静默回退到 GDB 后端
- GDB 工具链可能未安装（用户本意是用 probe-rs）
- 错误信息显示"无法启动 arm-none-eabi-gdb"，用户不知道为什么用了 GDB

### 修改建议

```rust
// 改为返回 Result
pub fn from_str(s: &str) -> Result<Self, String> {
    match s.to_lowercase().as_str() {
        "gdb" => Ok(Self::Gdb),
        "openocd" => Ok(Self::OpenOcd),
        "probe-rs" | "probe_rs" => Ok(Self::ProbeRs),  // 容错常见拼写
        "pyocd" | "pyocd-gdbserver" => Ok(Self::PyOcd),
        other => Err(format!(
            "未知后端 '{}'，可选: gdb, openocd, probe-rs, pyocd",
            other
        )),
    }
}
```

然后在所有调用处传播错误，例如 `src/lib.rs:267`：

```rust
let be = FlashBackend::from_str(&backend).map_err(|e| {
    io::Error::new(io::ErrorKind::InvalidInput, e)
})?;
```

同时需要在 `src/cli.rs` 的 clap 参数上添加 `value_parser`：

```rust
#[arg(short = 'b', long, default_value = "gdb", 
      value_parser = ["gdb", "openocd", "probe-rs", "pyocd"])]
backend: String,
```

这确保无效输入在 CLI 解析层就被拦截，而非传播到业务逻辑层。

---

## 问题 #8 [P2 - 可维护性]: God Object — `App` 结构体承载过多职责

### 位置

`src/tui/app.rs:50-101`

`App` 包含 28 个字段，横跨 5 个不同关注域：

| 关注域 | 字段 |
|--------|------|
| 烧录参数 | `backend`, `interface`, `target`, `elf_path`, `gdb_port`, `pyocd_path`, `timeout_secs` |
| 选择索引 | `backend_idx`, `iface_idx`, `target_idx`, `elf_file_idx` |
| UI 状态 | `focus`, `mode`, `list_state`, `status` |
| 调试参数 | `debug_elf`, `debug_target`, `debug_backend`, `debug_interface`, `debug_port`, `debug_gdb` |
| 运行时 | `result`, `switch_to_debug`, `should_quit`, `state`, `detected_chips`, `fff_search_state` |

### 问题表现

`select_prev` / `select_next` / `confirm_selection` 中反复出现相同的三段式 match：

```rust
// src/tui/app.rs:166-204 — 相同模式重复 3 次
match self.focus {
    Focus::Backend => { self.backend_idx = ... }
    Focus::Interface => { self.iface_idx = ... }
    Focus::Target => { self.target_idx = ... }
    _ => {}
}
```

添加新字段（如 "速度" 下拉）需要修改 5+ 个函数。

### 修改建议

将 App 拆分为子结构体，并引入泛型选择列表抽象：

```rust
// src/tui/selectable_list.rs (新文件)

/// 通用选择列表——管理 (key, label) 对 + 当前索引
pub struct SelectableList {
    items: Vec<(String, String)>, // (key, 显示名)
    idx: usize,
}

impl SelectableList {
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self { /* ... */ }
    pub fn selected_key(&self) -> &str { &self.items[self.idx].0 }
    pub fn select_prev(&mut self) { self.idx = self.idx.checked_sub(1).unwrap_or(self.items.len() - 1); }
    pub fn select_next(&mut self) { self.idx = (self.idx + 1) % self.items.len(); }
}
```

然后 App 变更为：

```rust
// src/tui/app.rs
pub struct App {
    // --- 参数选择（统一抽象）---
    pub backend: SelectableList,
    pub interface: SelectableList,
    pub target: SelectableList,
    pub files: SelectableList,  // ELF/BIN 文件列表

    // --- 烧录参数（从选择列表中派生，缓存以减少 clone）---
    pub elf_path: String,
    pub gdb_port: String,
    pub pyocd_path: String,
    pub timeout_secs: u64,

    // --- UI 状态 ---
    pub focus: Focus,
    pub mode: InputMode,

    // --- 调试参数 ---
    pub debug: DebugParams,

    // --- 运行时 ---
    pub status: String,
    pub result: Option<FlashResult>,
    pub switch_to_debug: bool,
    pub should_quit: bool,
    pub state: Arc<AppState>,
    pub detected_chips: Vec<DetectedChip>,
    pub fff_search_state: Option<FffSearchState>,
}

/// 调试相关参数独立管理
#[derive(Default)]
pub struct DebugParams {
    pub elf: String,
    pub target: String,
    pub backend: String,
    pub interface: String,
    pub port: u16,
    pub gdb: String,
}
```

### 收益

- `select_prev` / `select_next` 退化为一行：`self.active_list().select_prev()`
- 添加新选择字段只需在 `App` 中加一个 `SelectableList` 字段，在 `Focus` 枚举中加变体
- `DebugParams` 可以通过 `sync_from(&self)` 方法从烧录参数同步

---

## 问题 #9 [P2 - 架构耦合]: RTT 后端启动逻辑与 TUI 状态深度耦合

### 位置

`src/tui/debug_ui.rs:214-265` — `start_rtt`

```rust
pub fn start_rtt(&mut self) {
    if self.running { return; }
    let (tx, rx) = crossbeam_channel::unbounded();

    match self.backend.as_str() {
        "openocd" => {
            // 1. 映射接口/目标到配置文件路径
            let icfg = crate::backend::mappings::openocd_interface_cfg(&self.interface);
            let tcfg = crate::backend::mappings::openocd_target_cfg(&self.session.target);
            // 2. spawn 子进程
            match Command::new("openocd").args([...]).spawn() {
                Ok(c) => {
                    self.server_process = Some(c);          // ← UI 状态
                    self.session.push_rtt(RttOutput { ... }); // ← 日志
                    std::thread::sleep(Duration::from_millis(500)); // ← 硬编码等待
                }
                Err(e) => { self.session.push_rtt(...); return; }
            }
            // 3. 连接 telnet RTT
            match spawn_openocd_rtt(4444, tx) {
                Ok(c) => { self.rtt_client = Some(Box::new(c)); /* ... */ }
                Err(e) => { self.session.push_rtt(...); }
            }
        }
        "pyocd" => { /* 类似的 spawn + telnet 逻辑 */ }
        "gdb" => { /* 特殊处理 */ }
        _ => { /* probe-rs 直接 RTT */ }
    }
}
```

### 问题分析

1. **进程生命周期与 UI 状态耦合**：`self.server_process` 挂在 `RttMonitorState` 上，该结构体还管理 `session`、`rtt_client`、`rtt_rx`
2. **API 模式重复实现**：`src/api/routes/debug.rs:54-70` 重新实现了同样的后端开关逻辑
3. **硬编码端口**：OpenOCD telnet 端口写死为 `4444`，无法配置
4. **硬编码等待**：`thread::sleep(500ms)` 不可靠——如果 OpenOCD 启动慢于 500ms，telnet 连接会失败

### 修改建议

提取 `RttBackendManager` trait + 各后端实现：

```rust
// src/debug/backend_manager.rs (新文件)

/// RTT 后端生命周期管理器
pub trait RttBackendManager: Send {
    /// 启动后端服务（spawn 子进程等）
    fn start(&mut self, config: &RttManagerConfig) -> Result<(), String>;
    /// 创建 RTT 客户端连接
    fn connect_rtt(&self, tx: Sender<RttOutput>) -> Result<Box<dyn RttClient>, String>;
    /// 停止后端（kill 子进程）
    fn stop(&mut self);
}

// 各后端实现:
// - OpenOcdManager: start() 启动 openocd 进程, connect_rtt() 连接 telnet
// - PyOcdManager:   同上
// - ProbeRsManager: start() 为空, connect_rtt() 直接 probe-rs API
// - GdbManager:     start() 为空, connect_rtt() 返回错误

/// 将 RTT 客户端连接到 broadcast 通道的胶水函数
pub fn rtt_client_to_broadcast(
    mut client: Box<dyn RttClient>,
    tx: broadcast::Sender<RttOutput>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // 轮询 client 并将数据通过 broadcast tx 发送
    })
}
```

TUI 端则简化为：

```rust
// src/tui/debug_ui.rs
pub fn start_rtt(&mut self) {
    if self.running { return; }
    let manager = make_backend_manager(&self.backend, &self.session.target, &self.interface);
    match manager.start() {
        Ok(()) => {
            self.backend_manager = Some(manager);
            let (tx, rx) = crossbeam_channel::unbounded();
            match self.backend_manager.as_ref().unwrap().connect_rtt(tx) {
                Ok(client) => {
                    self.rtt_client = Some(client);
                    self.rtt_rx = Some(rx);
                    self.running = true;
                }
                Err(e) => { /* ... */ }
            }
        }
        Err(e) => { /* ... */ }
    }
}
```

API 端复用同一个 `make_backend_manager`：

```rust
// src/api/routes/debug.rs
async fn debug_start_handler(/* ... */) -> Json<Value> {
    let mut manager = make_backend_manager(&backend, &target, &interface);
    manager.start().map_err(|e| json!({ "error": e }))?;
    let client = manager.connect_rtt(tx).map_err(/* ... */)?;
    // backend_manager 需要保存在某处以支持 stop
}
```

---

## 问题 #10 [P2 - 性能 & 正确性]: `search_elf_files` 每个扩展名独立执行全量模糊搜索

### 位置

`src/tui/app.rs:306-332`

```rust
for ext in FIRMWARE_EXTS {           // 6 次迭代
    let query = query_parser.parse(ext);       // 用 ".elf" 字符串做模糊搜索
    let results = picker.fuzzy_search(
        &query, qt,
        fff_search::FuzzySearchOptions {
            pagination: fff_search::PaginationArgs {
                offset: 0,
                limit: 100,                     // 每次搜索最多 100 条
            },
            ..Default::default()
        },
    );
    for item in &results.items {
        // ...
        if FIRMWARE_EXTS.iter().any(|e| rel_path.ends_with(e)) {  // 再确认扩展名
            self.elf_files.push(abs);
        }
    }
}
```

### 问题分析

1. **6 次独立全量搜索**：`fff-search` 的 `fuzzy_search` 是 O(n) 数据库扫描，6 个扩展名 = 6 次扫描。在一个 10 万文件的仓库中，这是 6× 的浪费
2. **每扩展名限制 100 条**：如果项目有 150 个 `.bin` 文件，只能看到前 100 个。其他 50 个被静默截断
3. **重复检查**：第 327 行 `FIRMWARE_EXTS.iter().any(|e| rel_path.ends_with(e))` 是对模糊搜索结果做精确扩展名回检。这是因为 `fff-search` 的模糊查询 `.bin` 可能匹配 `binary_data.json`——这一步证明了"单次模糊搜索 + 精确过滤"更好
4. **`query_parser.parse(".elf")` 的语义不明**：将扩展名作为"查询"传给模糊搜索器，实际上是利用 fff-search 的文件名匹配能力找到了以 `.elf` 结尾的文件，但这绕过了 fff-search 的 `pattern` 类型查询

### 修改建议

使用单次 `*` 通配符搜索 + 客户端过滤：

```rust
pub fn search_elf_files(&mut self) {
    self.elf_files.clear();
    self.elf_file_idx = 0;

    if self.fff_search_state.is_none() {
        self.init_fff_search();
    }

    let Some(ref state) = self.fff_search_state else { return };

    let Ok(picker_guard) = state.shared_picker.read() else { return };
    let Some(picker) = picker_guard.as_ref() else { return };

    // 单次搜索：用通配符匹配所有扩展名（如果 fff-search 支持 OR 查询则更好）
    // 方案 A: 搜索 "*" 然后在客户端按扩展名过滤
    let query = fff_search::QueryParser::default().parse("*");
    let results = picker.fuzzy_search(
        &query,
        None,
        fff_search::FuzzySearchOptions {
            pagination: fff_search::PaginationArgs {
                offset: 0,
                limit: 500, // 一次获取足够多
            },
            ..Default::default()
        },
    );

    let ext_set: std::collections::HashSet<&str> = FIRMWARE_EXTS.iter().copied().collect();
    for item in &results.items {
        let rel_path = item.relative_path(picker);
        if ext_set.iter().any(|e| rel_path.ends_with(e)) {
            // 直接使用 relative_path 或 canonicalize
            if let Ok(abs) = std::path::absolute(&rel_path) {
                self.elf_files.push(abs.to_string_lossy().to_string());
            }
        }
    }

    self.elf_files.sort_by_key(|p| p.len());
}
```

如果 fff-search 提供了文件名过滤 API（如 `file_picker::FilePicker::list_files(&self, pattern: &str) -> Vec<PathBuf>`），应优先使用：

```rust
// 更优方案：使用 fff-search 的精确扩展名过滤（如果支持）
let results = picker.list_by_extensions(&[".elf", ".out", ".bin", ".hex", ".axf", ".ihx"]);
for path in &results {
    self.elf_files.push(path.to_string_lossy().to_string());
}
```

---

## 问题 #11 [P2 - 正确性]: `search_elf_files` 路径拼接在 TUI 运行期间工作目录变化时失效

### 位置

`src/tui/app.rs:292-294, 373-374`

初始化时（`init_fff_search`）：

```rust
let cwd = std::env::current_dir()?;  // 读取一次
let options = fff_search::FilePickerOptions {
    base_path: cwd.to_string_lossy().to_string(),  // 固化为 base_path
    // ...
};
```

搜索时（`search_elf_files`）：

```rust
let cwd = match std::env::current_dir() {  // 重新读取 current_dir
    Ok(d) => d,
    Err(_) => return,
};
// ...
let full_path = cwd.join(&rel_path);  // 将 base_path-相对路径 与 cwd 拼接
```

### 问题分析

`fff-search` 的 `FilePicker` 索引的是 `base_path`（即初始化时的 CWD）。`relative_path()` 返回的是相对于此 `base_path` 的路径。如果用户在 TUI 运行期间通过终端执行了 `cd ..`，那么 `current_dir()` 变成父目录，`cwd.join(rel_path)` 会产生错误路径。

在 TUI 场景中此场景不太可能发生（TUI 占满终端），但在 `cargo run -- run --api` 模式下，TUI 和 HTTP API 共存，用户在另一终端 `cd` 是有可能的。

### 修改建议

在初始化时缓存 `base_path`，搜索时直接使用它：

```rust
pub(crate) struct FffSearchState {
    shared_picker: fff_search::SharedFilePicker,
    shared_frecency: fff_search::SharedFrecency,
    shared_query_tracker: fff_search::SharedQueryTracker,
    _temp_dir: TempDir,
    /// 缓存 base_path，避免搜索时 current_dir 已变化
    base_path: PathBuf,
}

fn init_fff_search(&mut self) {
    let cwd = std::env::current_dir()?;
    // ...创建 shared state...
    self.fff_search_state = Some(FffSearchState {
        shared_picker,
        shared_frecency,
        shared_query_tracker,
        _temp_dir: temp_dir,
        base_path: cwd,
    });
}

pub fn search_elf_files(&mut self) {
    // ...
    let base = &state.base_path;  // 使用缓存的 base_path
    for item in &results.items {
        let rel_path = item.relative_path(picker);
        let full_path = base.join(&rel_path);
        // ...
    }
}
```

---

## 问题 #12 [P2 - 体验]: `wait_for_scan` 硬编码 10 秒超时且无进度反馈

### 位置

`src/tui/app.rs:389`

```rust
shared_picker.wait_for_scan(std::time::Duration::from_secs(10));
```

### 问题分析

- 用户首次按 Enter 搜索固件时，TUI 冻结 10 秒无响应
- 如果项目非常大（如 Android AOSP 级别），10 秒可能不足以完成索引，导致搜索结果为空
- 没有降级策略：超时后是否回退到传统扫描？用户不知道
- 没有进度条或任何 UI 反馈

### 修改建议

```rust
// 改为非阻塞轮询 + 进度提示
let start = std::time::Instant::now();
let scan_timeout = std::time::Duration::from_secs(30);

loop {
    if shared_picker.is_scan_complete() {
        break;
    }
    if start.elapsed() > scan_timeout {
        self.status = "FFF 索引超时，使用部分结果".to_string();
        break;
    }
    // 允许用户在等待期间看到状态更新
    // （如果 TUI 事件循环能做，就渲染一帧）
    self.status = format!(
        "正在索引文件... {:.1}s",
        start.elapsed().as_secs_f32()
    );
    std::thread::sleep(std::time::Duration::from_millis(100));
}
```

更激进的做法：在 `init_fff_search` 中**不等待**扫描完成，直接将 `FffSearchState` 标记为 "索引中"。搜索时检查是否索引完成，如果未完成则提示用户稍后重试：

```rust
pub(crate) struct FffSearchState {
    // ... 原有字段 ...
    scan_ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

fn init_fff_search(&mut self) {
    let ready = Arc::new(AtomicBool::new(false));
    let ready_clone = ready.clone();

    // 在后台线程等待扫描完成
    let picker = shared_picker.clone();
    std::thread::spawn(move || {
        picker.wait_for_scan(Duration::from_secs(120));
        ready_clone.store(true, Ordering::SeqCst);
    });

    self.fff_search_state = Some(FffSearchState {
        // ...
        scan_ready: ready,
    });
    self.status = "FFF 搜索引擎已启动，文件索引中...".to_string();
}

pub fn search_elf_files(&mut self) {
    if let Some(ref state) = self.fff_search_state {
        if !state.scan_ready.load(Ordering::SeqCst) {
            self.status = "文件索引尚未完成，请稍后重试".to_string();
            return;
        }
    }
    // ... 执行搜索 ...
}
```

---

## 问题 #13 [P2 - 架构]: 整个代码库只有一个测试

### 位置

唯一测试位于 `src/backend.rs:369-374`：

```rust
#[test]
fn backend_from_str() {
    assert_eq!(FlashBackend::from_str("gdb"), FlashBackend::Gdb);
    // ...
}
```

### 缺失的关键测试

| 模块 | 应测内容 | 风险 |
|------|----------|------|
| `src/board.rs` | YAML 解析、`resolve` 错误路径、`resolve_by_chip_name` 大小写匹配 | boards.yaml 格式变化导致崩溃 |
| `src/backend.rs` | `do_flash` 的文件不存在路径、超时路径、`detect_fatal_error` | 烧录失败时无正确修复 |
| `src/debug/protocol.rs` | RTT 数据包解析（68 个符号，0 测试） | RTT 解析错误导致数据错位 |
| `src/app/state.rs` | `flash` 方法的全部三条路径（成功/配置错误/烧录失败） | 状态机卡死 |
| `src/tui/app.rs` | `search_elf_files` 空目录、无匹配文件、多匹配 | UI 崩溃 |

### 修改建议

优先添加以下测试：

```rust
// tests/board_registry.rs (集成测试)

#[test]
fn resolve_valid_board_with_backend() {
    let yaml = r#"
stm32f407vet6:
  name: "STM32F407VET6"
  manufacturer: "ST"
  architecture: arm
  interfaces: [swd, stlink]
  backends:
    probe-rs:
      target: "STM32F407VGTx"
    openocd:
      target: "stm32f4x.cfg"
"#;
    let registry = BoardRegistry::from_yaml(yaml).unwrap();
    let params = registry.resolve("stm32f407vet6", "probe-rs").unwrap();
    assert_eq!(params.target, "STM32F407VGTx");
}

#[test]
fn resolve_unsupported_backend_returns_error() {
    let registry = BoardRegistry::from_yaml(/* ... */).unwrap();
    assert!(registry.resolve("stm32f407vet6", "pyocd").is_err());
}

#[test]
fn resolve_unknown_board_returns_error() {
    let registry = BoardRegistry::from_yaml(/* ... */).unwrap();
    assert!(registry.resolve("nonexistent", "probe-rs").is_err());
}
```

```rust
// tests/flash_config.rs (集成测试)
#[test]
fn from_registry_missing_interface_returns_error() {
    // ...
}
```

---

## 附录：优先级汇总表

| 优先级 | # | 问题 | 影响范围 |
|--------|---|------|----------|
| **P0** | 1 | `fff-search` nightly 版本号无法从 crates.io 解析 | 构建完全阻塞 |
| **P0** | 2 | `process::exit` 跳过析构 — 子进程泄露、LMDB 损坏 | 所有退出路径 |
| **P0** | 3 | `thread::park()` 无优雅关闭 | API 模式 |
| **P1** | 4 | 烧录逻辑三处重复 | 维护负担、bug 修复需改三处 |
| **P1** | 5 | LMDB 临时目录从不清理 | 磁盘累积 |
| **P1** | 6 | API 服务器启动失败静默吞掉 | 运行时故障无感知 |
| **P1** | 7 | `from_str` 静默回退 | 用户输入错误无提示 |
| **P2** | 8 | God Object `App` | 可维护性下降 |
| **P2** | 9 | RTT 后端与 UI 耦合 | 难以复用、重复实现 |
| **P2** | 10 | 6 次全量模糊搜索 | 性能、不完整结果 |
| **P2** | 11 | 工作目录变化导致路径错误 | 特定场景下崩溃 |
| **P2** | 12 | `wait_for_scan` 硬编码 10 秒 | 用户体验差 |
| **P2** | 13 | 整个代码库只有一个测试 | 回归风险 |

---

**报告结束** — 每个问题已给出具体位置、代码和可操作的修改建议。
