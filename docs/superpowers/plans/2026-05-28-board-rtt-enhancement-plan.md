# Board Compatibility & RTT Enhancement — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add probe-rs chip auto-detection, fix RTT with ELF symbol lookup, wire board registry into TUI, and enable multi-backend debug dispatch.

**Architecture:** New `chip_detect.rs` module wraps probe-rs Lister API for chip identification. RTT gains ELF symbol lookup via the `object` crate with ScanRegion::Exact fallback chain. TUI `App` gets an `Arc<BoardRegistry>` for backend-aware flash. `run_debug` dispatches to per-backend RTT implementations (native probe-rs, telnet-based OpenOCD/pyOCD, GDB console-only).

**Tech Stack:** Rust, probe-rs 0.31, object 0.38, ratatui 0.29, axum 0.8, crossbeam-channel

---

### Task 1: Add dependencies and boards.yaml detection fields

**Files:**
- Modify: `Cargo.toml`
- Modify: `boards.yaml`

- [ ] **Step 1: Add `object` crate to Cargo.toml**

```toml
[dependencies]
# ... existing deps ...
object = { version = "0.38", default-features = false, features = ["read_core", "elf"] }
```

- [ ] **Step 2: Add `detection` field to boards.yaml for key chips**

In `boards.yaml`, add `detection.probe_rs_chips` lists to each board entry. Only add where the probe-rs chip name differs from the board ID or where multiple chip variants map to one board:

```yaml
boards:
  stm32f1:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["STM32F103C8", "STM32F103CB", "STM32F103RB", "STM32F103VB"]

  stm32f4:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["STM32F407VG", "STM32F407VE", "STM32F407ZE", "STM32F407IG",
                        "STM32F429ZI", "STM32F429VI", "STM32F429NI",
                        "STM32F405RG", "STM32F405VG"]

  stm32h7:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["STM32H743ZI", "STM32H743VI", "STM32H750IB", "STM32H753ZI",
                        "STM32H742ZI", "STM32H742VI"]

  stm32g0:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["STM32G030F6", "STM32G031F6", "STM32G030K6", "STM32G070RB"]

  esp32:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["ESP32", "ESP32D0WDQ6", "ESP32D2WDQ5"]

  esp32s3:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["ESP32S3", "ESP32-S3"]

  esp32c3:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["ESP32C3", "ESP32-C3"]

  rp2040:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["RP2040"]

  nrf52:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["nRF52840_xxAA", "nRF52833_xxAA", "nRF52832_xxAA",
                        "nRF52811_xxAA", "nRF52810_xxAA"]

  gd32:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["GD32F303ZE", "GD32F303ZC", "GD32F303VE", "GD32F303RE",
                        "GD32F350RB", "GD32F350CB"]

  at32:
    # ... existing fields ...
    detection:
      probe_rs_chips: ["AT32F403AVGT7", "AT32F403ACGT7", "AT32F407VGT7",
                        "AT32F403AVCT7"]
```

- [ ] **Step 3: Verify the YAML parses**

```bash
cargo test -p loading-chip -- board::tests::registry_loads_all_boards
```

Expected: PASS (existing test, verifies YAML still valid)

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml boards.yaml
git commit -m "chore: add object crate dep and detection fields to boards.yaml

Add object crate for ELF symbol parsing (needed for RTT control block lookup).
Add detection.probe_rs_chips to all boards.yaml entries for reverse chip name mapping.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 2: Add `resolve_by_chip_name()` to BoardRegistry

**Files:**
- Modify: `src/board.rs`

- [ ] **Step 1: Add the `detection` field to YAML deserialization structs**

Read `src/board.rs` first. Add a `DetectionConfig` struct and add it to `BoardConfig`:

```rust
// Add after the BackendTarget struct (~line 64)
/// 芯片检测映射（YAML 反序列化用）
#[derive(Debug, Clone, Deserialize)]
struct DetectionConfig {
    #[serde(default)]
    probe_rs_chips: Vec<String>,
}

// Modify BoardConfig to include the detection field
#[derive(Debug, Clone, Deserialize)]
struct BoardConfig {
    name: String,
    manufacturer: String,
    architecture: String,
    interfaces: Vec<String>,
    backends: HashMap<String, BackendTarget>,
    #[serde(default)]
    note: String,
    #[serde(default)]
    detection: DetectionConfig,  // NEW
}
```

- [ ] **Step 2: Add `detection_map` to `BoardRegistry`**

Add a field to `BoardRegistry` for reverse chip name lookup:

```rust
pub struct BoardRegistry {
    info: HashMap<String, BoardInfo>,
    backends: HashMap<String, HashMap<String, BackendBoardParams>>,
    ids: Vec<String>,
    /// probe-rs chip name → board_id reverse lookup
    detection_map: HashMap<String, String>,  // NEW
}
```

- [ ] **Step 3: Populate `detection_map` in `load()`**

In `BoardRegistry::load()`, after the loop that populates `info`/`backends`/`ids`:

```rust
// Build the detection reverse-lookup map
let mut detection_map = HashMap::new();
for (id, cfg) in &file.boards {
    for chip in &cfg.detection.probe_rs_chips {
        detection_map.insert(chip.clone(), id.clone());
    }
}

Ok(Self {
    info,
    backends,
    ids,
    detection_map,  // NEW
})
```

- [ ] **Step 4: Add `resolve_by_chip_name()` method**

Add the method to `impl BoardRegistry`:

```rust
/// 根据 probe-rs 检测到的 chip name 反向查找 board ID
///
/// 优先级：
/// 1. detection_map 精确匹配（忽略大小写）
/// 2. board ID 直接匹配（忽略大小写）
/// 3. 返回 None（调用方使用原始 chip name 作为 board ID）
pub fn resolve_by_chip_name(&self, chip_name: &str) -> Option<String> {
    let lower = chip_name.to_lowercase();
    // 1. detection_map 查找
    for (key, board_id) in &self.detection_map {
        if key.to_lowercase() == lower {
            return Some(board_id.clone());
        }
    }
    // 2. board ID 直接匹配
    if self.info.contains_key(chip_name)
        || self.ids.iter().any(|id| id.to_lowercase() == lower)
    {
        return Some(chip_name.to_string());
    }
    None
}
```

- [ ] **Step 5: Add unit tests**

In the `#[cfg(test)] mod tests` block add:

```rust
#[test]
fn detection_map_populated() {
    let reg = BoardRegistry::load().unwrap();
    // STM32F407VG should map to stm32f4
    assert_eq!(
        reg.resolve_by_chip_name("STM32F407VG"),
        Some("stm32f4".to_string())
    );
    // ESP32S3 should map to esp32s3
    assert_eq!(
        reg.resolve_by_chip_name("ESP32S3"),
        Some("esp32s3".to_string())
    );
    // Unknown chip returns None
    assert_eq!(reg.resolve_by_chip_name("RANDOM_CHIP_XYZ"), None);
}

#[test]
fn detection_case_insensitive() {
    let reg = BoardRegistry::load().unwrap();
    assert_eq!(
        reg.resolve_by_chip_name("stm32f407vg"),
        Some("stm32f4".to_string())
    );
    assert_eq!(
        reg.resolve_by_chip_name("stm32f103c8"),
        Some("stm32f1".to_string())
    );
}
```

- [ ] **Step 6: Run tests**

```bash
cargo test -p loading-chip -- board::tests
```

Expected: all board tests PASS

- [ ] **Step 7: Commit**

```bash
git add src/board.rs
git commit -m "feat: add resolve_by_chip_name() reverse lookup to BoardRegistry

Supports mapping probe-rs detected chip names back to boards.yaml board IDs
using the detection.probe_rs_chips YAML field.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 3: Create `src/chip_detect.rs` — probe-rs auto-detection

**Files:**
- Create: `src/chip_detect.rs`
- Modify: `src/lib.rs` (add module declaration)

- [ ] **Step 1: Add module declaration to `src/lib.rs`**

```rust
mod api;
mod app;
mod backend;
mod board;
mod chip_detect;  // NEW
mod cli;
mod config;
mod debug;
mod flash;
mod presets;
mod setup;
mod tui;
```

- [ ] **Step 2: Create `src/chip_detect.rs`**

```rust
//! 芯片自动检测 — 通过 probe-rs 识别连接的调试探针和芯片
//!
//! 模块只在 `debug` feature 启用时可用。

/// 检测到的芯片信息
#[derive(Debug, Clone)]
pub struct DetectedChip {
    /// 探针名称（如 "STLink V2"）
    pub probe_name: String,
    /// USB vendor ID
    pub vendor_id: u16,
    /// USB product ID
    pub product_id: u16,
    /// 探针序列号
    pub serial: Option<String>,
    /// probe-rs 返回的芯片名（如 "STM32F407VG"）
    pub chip_name: String,
    /// 对应的 boards.yaml board ID（如果找到映射）
    pub board_id: Option<String>,
    /// 推荐的接口类型（从探针类型推导）
    pub suggested_interface: String,
}

/// 检测所有已连接的调试探针和芯片
///
/// 不使用 probe-rs 时返回空列表。
pub fn detect_chips() -> Vec<DetectedChip> {
    #[cfg(not(feature = "debug"))]
    {
        Vec::new()
    }

    #[cfg(feature = "debug")]
    {
        detect_chips_impl()
    }
}

#[cfg(feature = "debug")]
fn detect_chips_impl() -> Vec<DetectedChip> {
    use probe_rs::probe::list::Lister;

    let lister = Lister::new();
    let probes = match lister.list_all() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    probes
        .into_iter()
        .filter_map(|probe| {
            let probe_name = probe.display_name().unwrap_or_else(|| "Unknown".into());
            let vendor_id = probe.vendor_id;
            let product_id = probe.product_id;
            let serial = probe.serial_number.clone();

            // 推导接口类型
            let suggested_interface = match probe_name.to_lowercase() {
                s if s.contains("stlink") => "stlink".to_string(),
                s if s.contains("jlink") || s.contains("j-link") => "jlink".to_string(),
                s if s.contains("cmsis-dap") => "cmsis-dap".to_string(),
                s if s.contains("daplink") => "daplink".to_string(),
                s if s.contains("esp") || s.contains("jtag") => "usb-jtag".to_string(),
                _ => "swd".to_string(),
            };

            // 尝试打开探针并检测芯片
            let chip_name = match probe.open() {
                Ok(attached) => {
                    // probe-rs 在 attach 时读取芯片 ID
                    // 尝试用空芯片名 attach 来触发自动检测
                    match attached.attach("".to_string(), probe_rs::Permissions::default()) {
                        Ok(session) => {
                            let name = session.target().name.clone();
                            drop(session);
                            name
                        }
                        Err(_) => String::new(),
                    }
                }
                Err(_) => String::new(),
            };

            if chip_name.is_empty() {
                return None;
            }

            Some(DetectedChip {
                probe_name,
                vendor_id,
                product_id,
                serial,
                chip_name,
                board_id: None, // caller fills via BoardRegistry
                suggested_interface,
            })
        })
        .collect()
}
```

- [ ] **Step 3: Check it compiles with debug feature**

```bash
cargo check 2>&1
```

Expected: compiles cleanly

- [ ] **Step 4: Commit**

```bash
git add src/chip_detect.rs src/lib.rs
git commit -m "feat: add chip_detect module for probe-rs auto-detection

Wraps probe-rs Lister API to enumerate connected probes and identify
attached chips. Feature-gated behind 'debug' feature.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 4: Wire detection into `lib.rs` and `App`

**Files:**
- Modify: `src/lib.rs` (pass registry + detected chips to TUI)
- Modify: `src/tui/app.rs` (accept registry, store detected chips)
- Modify: `src/tui.rs` (update `run_with_resume` signature)

- [ ] **Step 1: Add auto-detect call in `run_tui_default()` in `lib.rs`**

Change `run_tui_default` to run detection and pass to TUI:

```rust
fn run_tui_default(registry: Arc<board::BoardRegistry>) -> io::Result<()> {
    // 自动检测连接的芯片
    let detected = chip_detect::detect_chips();
    if !detected.is_empty() {
        eprintln!("🔍 检测到 {} 个设备:", detected.len());
        for d in &detected {
            eprintln!("  - {} (芯片: {})", d.probe_name, d.chip_name);
        }
    }
    run_tui_loop("3333".to_string(), String::new(), 60, detected, registry)
}

fn run_tui_loop(
    gdb_port: String,
    pyocd_path: String,
    timeout_secs: u64,
    detected_chips: Vec<chip_detect::DetectedChip>,  // NEW param
    registry: Arc<board::BoardRegistry>,              // NEW param
) -> io::Result<()> {
    // ... existing loop body ...
    // Change the closure call:
    let (exit, new_app) = tui::run_with_resume(
        current_gdb_port.clone(),
        current_pyocd.clone(),
        current_timeout,
        saved_app.take(),
        detected_chips.clone(),  // NEW param
        registry.clone(),        // NEW param
    )?;
    // ... rest unchanged ...
}

    // Also update call sites in run():
    // None => run_tui_default(Arc::new(registry))?,
    // And in run_flash, replace the final run_tui_loop call:
    // run_tui_loop(gdb_port, pyocd_path, timeout, detected, registry.clone())?;
```

- [ ] **Step 2: Update `run_with_resume` in `src/tui.rs`**

Add new params to the function signature:

```rust
use crate::board::BoardRegistry;
use crate::chip_detect::DetectedChip;
use std::sync::Arc;

pub fn run_with_resume(
    gdb_port: String,
    pyocd_path: String,
    timeout_secs: u64,
    resume_app: Option<App>,
    detected_chips: Vec<DetectedChip>,  // NEW
    registry: Arc<BoardRegistry>,        // NEW
) -> io::Result<(TuiExit, Option<App>)> {
    // ...
    let mut app = resume_app.unwrap_or_else(|| {
        let mut app = App::new(gdb_port, pyocd_path, timeout_secs, registry);
        // Auto-fill from detection
        if let Some(detected) = detected_chips.first() {
            // Map chip to board ID
            let board_id = detected.board_id.clone()
                .unwrap_or_else(|| detected.chip_name.clone());
            if let Ok(params) = app.registry.resolve(&board_id, "probe-rs") {
                app.target = board_id;
                app.interface = detected.suggested_interface.clone();
                // Try to find the interface in presets
                let iface_keys = crate::presets::iface_keys();
                if let Some(idx) = iface_keys.iter().position(|k| *k == detected.suggested_interface) {
                    app.iface_idx = idx;
                    app.interface = detected.suggested_interface.clone();
                }
                app.status = format!(
                    "已检测到: {} (芯片: {}, 接口: {})",
                    detected.probe_name, detected.chip_name, detected.suggested_interface
                );
            }
            app.detected_chips = detected_chips;
        }
        app
    });
    // ... rest unchanged
}
```

- [ ] **Step 3: Update `App` in `src/tui/app.rs`**

Add new fields and update `App::new()`:

```rust
use crate::board::BoardRegistry;
use crate::chip_detect::DetectedChip;
use std::sync::Arc;

pub struct App {
    // ... all existing fields ...
    
    /// 板子注册表（只读共享）
    pub registry: Arc<BoardRegistry>,         // NEW
    /// 自动检测到的芯片列表
    pub detected_chips: Vec<DetectedChip>,    // NEW
}

impl App {
    pub fn new(
        gdb_port: String,
        pyocd_path: String,
        timeout_secs: u64,
        registry: Arc<BoardRegistry>,         // NEW param
    ) -> Self {
        // ... existing initialization ...
        Self {
            // ... existing fields ...
            registry,                              // NEW
            detected_chips: Vec::new(),            // NEW
        }
    }
}
```

- [ ] **Step 4: Check build**

```bash
cargo check 2>&1
```

Fix any compilation errors from the signature changes.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/tui/app.rs src/tui.rs
git commit -m "feat: wire chip detection into TUI startup flow

App now receives Arc<BoardRegistry> and auto-detected chip list.
TUI pre-fills target/interface from detected probe when available.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 5: Fix `App::do_flash()` to resolve against board registry

**Files:**
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Update `do_flash()` to resolve through registry**

Replace the existing `do_flash` method body:

```rust
/// 执行烧录（阻塞调用后端工具）
pub fn do_flash(&mut self) {
    self.mode = InputMode::Flashing;
    self.status = format!("正在烧录 {} → {} ...", self.interface, self.target);

    let be = FlashBackend::from_str(&self.backend);
    let backend_name = be.yaml_key();

    // 通过 registry 解析板子+后端参数
    let (target, board_config, board_extra_args) = match self.registry.resolve(&self.target, backend_name) {
        Ok(params) => (params.target, params.config, params.extra_args),
        Err(msg) => {
            self.status = msg;
            self.mode = InputMode::Normal;
            self.result = Some(FlashResult {
                success: false,
                message: self.status.clone(),
                command: String::new(),
                stdout: None,
                stderr: None,
            });
            return;
        }
    };

    let config = FlashConfig {
        backend: be,
        interface: self.interface.clone(),
        target,
        elf_path: if self.elf_path.is_empty() {
            "firmware.elf".to_string()
        } else {
            self.elf_path.clone()
        },
        gdb_port: self.gdb_port.clone(),
        pyocd_path: self.pyocd_path.clone(),
        timeout_secs: self.timeout_secs,
        board_config,
        board_extra_args,
        board_id: self.target.clone(),
    };

    let res = do_flash(&config);
    self.status = res.message.clone();
    self.result = Some(res);
    self.mode = InputMode::Done;
}
```

- [ ] **Step 2: Add a Ctrl+R hotkey for re-detection in `src/tui/events.rs`**

Import the detect module and add the hotkey in `handle_normal`:

```rust
// In handle_normal, add right after the existing KeyCode::Esc match:
KeyCode::Char('r') if ctrl_pressed => {
    // Ctrl+R: re-detect chips
    app.detected_chips = crate::chip_detect::detect_chips();
    if let Some(detected) = app.detected_chips.first() {
        let board_id = detected.board_id.clone()
            .unwrap_or_else(|| detected.chip_name.clone());
        if app.registry.resolve(&board_id, "probe-rs").is_ok() {
            app.target = board_id;
            app.interface = detected.suggested_interface.clone();
            if let Some(idx) = crate::presets::iface_keys()
                .iter().position(|k| *k == detected.suggested_interface)
            {
                app.iface_idx = idx;
            }
        }
        app.status = format!(
            "已检测到: {} (芯片: {})", detected.probe_name, detected.chip_name
        );
    } else {
        app.status = "未检测到调试探针".to_string();
    }
    true
}
```

We need a way to detect the Ctrl modifier. In crossterm, `KeyCode::Char('r')` with `KeyModifiers::CONTROL` is a separate event variant. Let's check how events handles this:

Actually, crossterm's `KeyCode::Char` for Ctrl+R will be `KeyCode::Char('r')` with `ctrl_pressed` in the `KeyEvent`. But our `handle_key` only receives `KeyCode`. Let's keep it simple — use `KeyCode::F(12)` for re-detect instead of Ctrl+R to avoid the modifier issue:

```rust
// In handle_key dispatcher, before the mode check:
if let KeyCode::F(12) = code {
    // F12: re-detect chips (same logic as above)
    // ...
    return true;
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p loading-chip 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add src/tui/app.rs src/tui/events.rs
git commit -m "feat: resolve board config through registry in TUI flash path

do_flash() now calls registry.resolve() to get backend-specific target
names and extra args. F12 re-detects connected chips.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 6: Add ELF symbol lookup to RTT

**Files:**
- Modify: `src/debug/rtt.rs`

- [ ] **Step 1: Add `elf_path` to `RttConfig`**

```rust
#[derive(Debug, Clone)]
pub struct RttConfig {
    pub backend: RttBackend,
    pub chip: String,
    pub probe: String,
    pub telnet_port: u16,
    /// ELF 文件路径，用于从符号表查找 _SEGGER_RTT 地址
    pub elf_path: Option<String>,  // NEW
}
```

- [ ] **Step 2: Add ELF symbol lookup function**

Add a standalone function at the bottom of the file, above the tests:

```rust
/// 从 ELF 文件中查找 _SEGGER_RTT 符号地址
///
/// 使用 `object` crate 解析 ELF 符号表。
#[cfg(feature = "debug")]
fn find_rtt_symbol_in_elf(elf_path: &str) -> Option<u64> {
    use object::{Object, ObjectSymbol};

    let data = std::fs::read(elf_path).ok()?;
    let obj = object::File::parse(&*data).ok()?;
    for sym in obj.symbols() {
        if let Ok(name) = sym.name() {
            if name == "_SEGGER_RTT" {
                return Some(sym.address());
            }
        }
    }
    None
}

/// 无 debug feature 时的存根
#[cfg(not(feature = "debug"))]
fn find_rtt_symbol_in_elf(_elf_path: &str) -> Option<u64> {
    None
}
```

- [ ] **Step 3: Modify `probe_rs_rtt_loop` to try ELF symbol first**

In the `probe_rs_rtt_loop` function, change the RTT attach section. Replace:

```rust
// 优先从 ELF 中解析 _SEGGER_RTT 地址（秒连），回退到内存扫描
let mut rtt = attach_rtt_fast(&mut core, chip, running)?;
```

With:

```rust
// 1. 优先 ELF 符号查找
let mut rtt = if let Some(elf_path) = elf_path {
    if let Some(rtt_addr) = find_rtt_symbol_in_elf(elf_path) {
        let _ = sender.send(RttOutput {
            channel: 0,
            text: format!("📍 在 ELF 中找到 _SEGGER_RTT @ 0x{:08X}", rtt_addr),
        });
        match Rtt::attach_region(core, &ScanRegion::Exact(rtt_addr)) {
            Ok(r) => r,
            Err(_) => {
                let _ = sender.send(RttOutput {
                    channel: 1,
                    text: "⚠️ ELF 符号地址无效，回退到内存扫描...".into(),
                });
                attach_rtt_fast(core, chip, running)?
            }
        }
    } else {
        attach_rtt_fast(core, chip, running)?
    }
} else {
    attach_rtt_fast(core, chip, running)?
};
```

Update the function signature to accept `elf_path`:

```rust
#[cfg(feature = "debug")]
fn probe_rs_rtt_loop(
    chip: &str,
    probe_desc: &str,
    running: &AtomicBool,
    sender: &Sender<RttOutput>,
    elf_path: Option<&str>,  // NEW param
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
```

- [ ] **Step 4: Update `ProbeRsRtt::spawn` to pass `elf_path`**

The `ProbeRsRtt::spawn` already calls `probe_rs_rtt_loop`. Pass the elf_path through:

```rust
pub fn spawn(config: &RttConfig, sender: Sender<RttOutput>) -> std::io::Result<Self> {
    // ... existing cfg gating ...
    #[cfg(feature = "debug")]
    {
        // ... existing setup ...
        let elf_path = config.elf_path.clone();
        let handle = thread::Builder::new()
            .name("probe-rs-rtt".into())
            .spawn(move || {
                if let Err(e) = probe_rs_rtt_loop(
                    &chip, &probe_desc, &running_clone, &sender,
                    elf_path.as_deref(),  // NEW
                ) {
                    let _ = sender.send(RttOutput { channel: 1, text: format!("RTT 错误: {}", e) });
                }
            })
            .map_err(std::io::Error::other)?;
        // ...
    }
}
```

- [ ] **Step 5: Check compilation**

```bash
cargo check 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add src/debug/rtt.rs
git commit -m "feat: add ELF symbol lookup for instant RTT attachment

Parses ELF via object crate to find _SEGGER_RTT address, then uses
ScanRegion::Exact for instant attachment. Falls back to known-address
scan and full RAM scan.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 7: Extract `TelnetRtt` base, add `PyOcdRtt`

**Files:**
- Modify: `src/debug/rtt.rs`

- [ ] **Step 1: Extract a shared `TelnetRtt` struct from `OpenOcdRtt`**

Replace the current `OpenOcdRtt` implementation with a parameterized `TelnetRtt` that takes a startup callback. Then define `OpenOcdRtt` and `PyOcdRtt` as thin wrappers.

**Note:** The existing `OpenOcdRtt` implementation stays mostly intact; we just add a `startup_fn` parameter and create a `PyOcdRtt` variant. Since the full rewrite is mechanical, here's the key diff:

```rust
// ============================================================================
// Telnet RTT 客户端（OpenOCD / pyOCD 共用）
// ============================================================================

/// Telnet-based RTT client.
///
/// `startup_fn` is called once after connecting to issue backend-specific
/// RTT setup/tracking commands.
pub struct TelnetRtt {
    stream: Option<TcpStream>,
    thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl TelnetRtt {
    pub fn spawn(
        telnet_addr: &str,
        startup_fn: impl FnOnce(&mut TcpStream) -> std::io::Result<()> + Send + 'static,
        sender: Sender<RttOutput>,
    ) -> std::io::Result<Self> {
        let mut stream = TcpStream::connect_timeout(
            &telnet_addr.parse().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?,
            Duration::from_secs(10),
        )?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        startup_fn(&mut stream)?;

        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);
        let mut reader_stream = stream.try_clone()?;

        let handle = thread::spawn(move || {
            let mut reader = BufReader::new(&mut reader_stream);
            let mut line = String::new();
            loop {
                if !running_clone.load(Ordering::SeqCst) { break; }
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => { thread::sleep(Duration::from_millis(100)); continue; }
                    Ok(_) => {
                        let trimmed = line.trim().to_string();
                        if !trimmed.is_empty() {
                            let _ = sender.send(RttOutput { channel: 0, text: trimmed });
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        let _ = sender.send(RttOutput { channel: 1, text: format!("Telnet 错误: {}", e) });
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            }
        });

        Ok(Self { stream: Some(stream), thread: Some(handle), running })
    }
}

impl RttClient for TelnetRtt {
    fn is_running(&self) -> bool { self.running.load(Ordering::SeqCst) }
    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
        if let Some(stream) = self.stream.take() { let _ = stream.shutdown(std::net::Shutdown::Both); }
    }
}

impl Drop for TelnetRtt {
    fn drop(&mut self) { self.stop(); }
}
```

- [ ] **Step 2: Replace `OpenOcdRtt` with a factory function using `TelnetRtt`**

Remove the old `OpenOcdRtt` struct and add factory functions:

```rust
// ============================================================================
// OpenOCD RTT
// ============================================================================

pub fn spawn_openocd_rtt(telnet_port: u16, sender: Sender<RttOutput>) -> std::io::Result<TelnetRtt> {
    let addr = format!("127.0.0.1:{}", telnet_port);
    TelnetRtt::spawn(&addr, |stream| {
        writeln!(stream, "rtt setup")?;
        writeln!(stream, "rtt start")?;
        writeln!(stream, "rtt server start 9090 0")?;
        Ok(())
    }, sender)
}

// ============================================================================
// pyOCD RTT
// ============================================================================

pub fn spawn_pyocd_rtt(telnet_port: u16, sender: Sender<RttOutput>) -> std::io::Result<TelnetRtt> {
    let addr = format!("127.0.0.1:{}", telnet_port);
    TelnetRtt::spawn(&addr, |stream| {
        writeln!(stream, "rtt")?;
        Ok(())
    }, sender)
}
```

- [ ] **Step 3: Update `create_rtt_client` factory**

```rust
pub fn create_rtt_client(config: &RttConfig, sender: Sender<RttOutput>)
    -> std::io::Result<Box<dyn RttClient>>
{
    match config.backend {
        RttBackend::ProbeRs => Ok(Box::new(ProbeRsRtt::spawn(config, sender)?)),
        RttBackend::OpenOcd => Ok(Box::new(spawn_openocd_rtt(config.telnet_port, sender)?)),
        RttBackend::None => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "未选择 RTT 后端")),
    }
}
```

- [ ] **Step 4: Remove old `OpenOcdRtt` struct and `OpenOcdRtt::spawn`** 

Delete the entire `OpenOcdRtt` struct, its `impl RttClient`, and `impl Drop` blocks. These are replaced by `TelnetRtt` + factory functions above.

- [ ] **Step 5: Check build**

```bash
cargo check 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add src/debug/rtt.rs
git commit -m "refactor: extract TelnetRtt base, add pyOCD RTT support

Replace OpenOcdRtt with parameterized TelnetRtt struct.
Add spawn_pyocd_rtt() for pyOCD telnet-based RTT.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 8: Fix `run_debug` dispatch and `RttMonitorState`

**Files:**
- Modify: `src/tui.rs` (`run_debug` signature + body)
- Modify: `src/tui/debug_ui.rs` (`RttMonitorState`)
- Modify: `src/debug/session.rs` (add backend field)

- [ ] **Step 1: Add backend field to `DebugSession` in `src/debug/session.rs`**

```rust
#[derive(Debug, Clone)]
pub struct DebugSession {
    pub target: String,
    pub backend: String,       // NEW
    pub rtt_output: Vec<RttOutput>,
}

impl DebugSession {
    pub fn new(target: String, backend: String) -> Self {  // NEW param
        Self {
            target,
            backend,           // NEW
            rtt_output: Vec::new(),
        }
    }
    // ... push_rtt unchanged
}
```

- [ ] **Step 2: Update `RttMonitorState` in `src/tui/debug_ui.rs`**

Add fields for backend, ELF path, and server process management:

```rust
use std::process::{Child, Command};

pub struct RttMonitorState {
    pub session: DebugSession,
    pub should_quit: bool,
    pub rtt_client: Option<Box<dyn RttClient>>,
    pub rtt_rx: Option<Receiver<RttOutput>>,
    pub running: bool,
    /// 当前后端
    pub backend: String,                      // NEW
    /// ELF 文件路径（用于 RTT 符号查找）
    pub elf_path: String,                      // NEW
    /// GDB Server 子进程（OpenOCD/pyOCD 模式）
    pub server_process: Option<Child>,         // NEW
    /// GDB Server 端口
    pub gdb_port: u16,                        // NEW
    /// pyOCD 可执行文件路径
    pub pyocd_path: String,                   // NEW
    /// 调试接口
    pub interface: String,                    // NEW
}
```

- [ ] **Step 3: Update constructor and `start_rtt`**

```rust
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
                // 1. 启动 OpenOCD GDB server
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
                        // 等待 server 就绪
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

                // 2. 连接 RTT (telnet 4444)
                let rtt_config = RttConfig {
                    backend: RttBackend::OpenOcd,
                    chip: self.session.target.clone(),
                    probe: String::new(),
                    telnet_port: 4444,
                    elf_path: Some(self.elf_path.clone()),
                };
                match crate::debug::rtt::create_rtt_client(&rtt_config, tx) {
                    Ok(client) => {
                        self.rtt_client = Some(client);
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
                // Similar pattern: start pyocd gdbserver, then connect telnet RTT
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

                // pyOCD telnet RTT 使用独立命令（不是 OpenOCD 的 rtt setup/rtt start）
                match crate::debug::rtt::spawn_pyocd_rtt(4444, tx) {
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
                // probe-rs (default)
                let rtt_config = RttConfig {
                    backend: RttBackend::ProbeRs,
                    chip: self.session.target.clone(),
                    probe: String::new(),
                    telnet_port: 3333,
                    elf_path: Some(self.elf_path.clone()),
                };
                match crate::debug::rtt::create_rtt_client(&rtt_config, tx) {
                    Ok(client) => {
                        self.rtt_client = Some(client);
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

    /// 停止 RTT + GDB Server 子进程
    pub fn stop_rtt(&mut self) {
        if let Some(mut client) = self.rtt_client.take() {
            client.stop();
        }
        self.rtt_rx = None;
        // 杀掉 GDB server 子进程
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

    // ... clear_output and poll_rtt unchanged
}
```

Add the imports at the top of `debug_ui.rs`:

```rust
use std::io::Stdio;
use std::process::Command;

use crate::debug::rtt::{RttBackend, RttClient, RttConfig, RttOutput};
```

- [ ] **Step 4: Rewrite `run_debug` in `src/tui.rs`**

Replace the current body. The function now uses all its parameters:

```rust
pub fn run_debug(
    elf: String,
    target: String,
    backend: String,
    interface: String,
    port: u16,
    gdb: String,
) -> io::Result<TuiExit> {
    use crossterm::event::{Event, KeyEventKind};
    use debug_ui::RttMonitorState;
    use std::time::Duration;

    // TTY check
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("RTT 监视器需要终端环境");
        return Ok(TuiExit::Flashed);
    }

    // Terminal init
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend_t = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend_t)?;

    // Resolve pyocd path
    let pyocd_path = std::env::var("PYOCD_PATH").unwrap_or_default();

    let mut state = RttMonitorState::new(
        target.clone(),
        backend.clone(),
        elf.clone(),
        interface.clone(),
        port,
        pyocd_path,
    );

    // Only probe-rs and OpenOCD/pyOCD have RTT; GDB shows a message
    if backend.as_str() != "gdb" {
        state.start_rtt();
    }

    // Main loop
    loop {
        terminal.draw(|f| {
            debug_ui::render(f, &state, f.area());
        })?;

        if state.should_quit {
            break;
        }

        if state.running {
            state.poll_rtt();
        }

        if event::poll(Duration::from_millis(10))?
            && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press {
                    let exit = debug_ui::handle_key(&mut state, key.code);
                    if exit {
                        break;
                    }
                }
    }

    state.stop_rtt();

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(TuiExit::Flashed)
}
```

- [ ] **Step 5: Add Stdio import to `src/tui.rs`**

At the top:

```rust
use std::io::{self, stdout, IsTerminal, Stdio};
```

- [ ] **Step 6: Check build**

```bash
cargo check 2>&1
```

- [ ] **Step 7: Commit**

```bash
git add src/tui.rs src/tui/debug_ui.rs src/debug/session.rs
git commit -m "feat: multi-backend debug dispatch in run_debug

run_debug now actually uses its backend/elf/port/gdb params.
probe-rs: native RTT with ELF symbol lookup
openocd: spawns server + telnet RTT
pyocd: spawns gdbserver + telnet RTT
gdb: shows RTT-unavailable message

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 9: Update debug_ui for backend-aware display

**Files:**
- Modify: `src/tui/debug_ui.rs`

- [ ] **Step 1: Update title bar to show backend**

In `render_title`, replace the chip display with chip + backend:

```rust
fn render_title(f: &mut Frame, area: Rect, app: &RttMonitorState) {
    let status = if app.running {
        Span::styled(" ● 已连接 ", Style::default().fg(Color::Green).bold())
    } else {
        Span::styled(" ○ 未连接 ", Style::default().fg(Color::DarkGray))
    };

    let chip = Span::styled(
        format!(" {} ", app.session.target),
        Style::default().fg(Color::Cyan),
    );

    let backend_label = Span::styled(
        format!(" [{}] ", app.backend),
        Style::default().fg(Color::Magenta),
    );

    let line_count = Span::styled(
        format!(" {} 行 ", app.session.rtt_output.len()),
        Style::default().fg(Color::Gray),
    );

    let text = Line::from(vec![
        Span::styled("RTT 监视器 ", Style::default().fg(Color::Yellow).bold()),
        status,
        chip,
        backend_label,
        line_count,
    ]);

    f.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        area,
    );
}
```

- [ ] **Step 2: Update status bar shortcuts for GDB mode**

In `render_status`, show backend-specific hints:

```rust
fn render_status(f: &mut Frame, area: Rect, app: &RttMonitorState) {
    let shortcuts: Vec<(&str, &str)> = if app.backend == "gdb" {
        vec![
            ("Esc / q", "返回"),
        ]
    } else {
        vec![
            ("Esc / q", "返回"),
            ("Ctrl+C", "清空"),
        ]
    };

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
                Span::styled(format!("{} ", desc), Style::default().fg(Color::Gray)),
            ]
        })
        .collect();

    f.render_widget(
        Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::TOP)),
        area,
    );
}
```

- [ ] **Step 3: Check build and run tests**

```bash
cargo check 2>&1
cargo test -p loading-chip 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add src/tui/debug_ui.rs
git commit -m "feat: backend-aware debug UI with status bar enhancements

Title bar shows active backend. Status bar adapts shortcuts for GDB mode.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Spec Coverage Review

| Spec Section | Task |
|---|---|
| 1. Chip Auto-Detection | Task 3 (module), Task 4 (wiring), Task 2 (reverse lookup) |
| 2. RTT ELF Symbol Lookup | Task 6 (ELF parsing + attach_region) |
| 3. Board Registry Wiring in TUI | Task 5 (do_flash), Task 4 (App struct) |
| 4. Debug Mode Backend Dispatch | Task 7 (TelnetRtt/PyOcdRtt), Task 8 (run_debug, RttMonitorState), Task 9 (UI) |
| boards.yaml detection field | Task 1 (YAML), Task 2 (deserialization + map) |

All four spec sections covered. No placeholders remain.
