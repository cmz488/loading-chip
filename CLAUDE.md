# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test

```bash
cargo build                  # Debug build
cargo build --release        # Release build
cargo test                   # Run all tests
cargo test -p loading-chip   # Run tests for the main crate
cargo run -- run             # Launch TUI (default mode)
cargo run -- run --headless  # Headless mode with JSON output
cargo run -- init            # Detect local toolchain and generate config
cargo run -- debug -e <ELF>  # Launch debug/RTT monitor
```

The `debug` feature gates the `probe-rs` dependency. It's on by default. To build without it:
```bash
cargo build --no-default-features
```

## Architecture

This is an embedded chip flashing/debugging TUI tool supporting four backends (GDB, OpenOCD, probe-rs, pyOCD) across ARM, Xtensa, and RISC-V targets.

### Entry & Dispatch (`src/main.rs` → `src/lib.rs`)

`main.rs` calls `loading_chip::run()`. `lib.rs` parses CLI args via clap (`src/cli.rs`) and dispatches to one of:
- **TUI mode** — `run_tui_loop()` loops between flash TUI and debug TUI, preserving app state across transitions
- **Headless mode** — direct flash, JSON output to stdout (for IDE integration)
- **CLI mode** — all params supplied on command line, skip TUI
- **Init mode** — detects local tools and writes `~/.config/loading-chip/config.yaml`
- **Detect mode** — lists detected probes and chips

### Backend Abstraction (`src/backend.rs`)

The `Backend` trait defines the interface (`binary()`, `build_args()`, `resolve_binary()`). Four unit structs implement it: `GdbBackend`, `OpenOcdBackend`, `ProbeRsBackend`, `PyOcdBackend`.

Dispatch uses `FlashBackend` enum + `make_backend()` returning `&'static dyn Backend` — zero-cost compared to `Box<dyn>`.

Chip/interface name mappings between the shorthand keys (e.g. `"stm32f4"`) and backend-specific target names are centralized in `src/backend/mappings.rs`.

`do_flash()` spawns the backend process, waits with timeout, and scans stdout/stderr for known fatal error patterns.

### Board Registry (`src/board.rs`)

`BoardRegistry` loads from `boards.yaml` (searched in env var, user config dir, exe dir, CWD, embedded default). Each board defines per-backend target names and extra args. `registry.resolve(board_id, backend)` validates the backend is supported for that board before returning the target string.

### TUI (`src/tui.rs`)

ratatui-based terminal UI with two sub-modes:

- **Flash TUI** (`app.rs`, `ui.rs`, `events.rs`) — form-based: select backend/interface/target/ELF, press Enter to flash. Supports dropdown selection, ELF file scanning, and manual path editing.
- **Debug TUI** (`debug_ui.rs`) — RTT real-time monitor showing target serial output via probe-rs. Polls RTT data on a 10ms event loop.

Mode switching: F5 in flash mode syncs current params to debug params, sets `switch_to_debug` flag. The outer `run_tui_loop()` in `lib.rs` handles the transition and preserves flash TUI state via `saved_app`.

### RTT (`src/debug/rtt.rs`)

Two RTT client implementations: `ProbeRsRtt` (probe-rs library reading RAM RTT buffers directly) and `OpenOcdRtt` (telnet-based). Both run on background threads sending `RttOutput` via `crossbeam_channel`. ESP32 chips use known-memory-address scanning for faster RTT control block detection.

### User Config (`src/config.rs`, `src/setup.rs`)

`loading-chip init` detects installed tools (probe-rs, OpenOCD, pyOCD, various GDB binaries) and connected debug probes, writing results to `~/.config/loading-chip/config.yaml`.

### Presets (`src/presets.rs`)

Hardcoded lists of backends, interfaces, and targets with Chinese descriptions — used by the TUI dropdown menus. These are separate from `boards.yaml`; the YAML is authoritative for backend compatibility, presets are for UI display.
