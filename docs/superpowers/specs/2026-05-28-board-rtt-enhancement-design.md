# Board Compatibility & RTT Enhancement Design

Date: 2026-05-28
Status: approved

## Overview

Three interconnected improvements to loading-chip:

1. **Auto-detect connected chips** via probe-rs instead of requiring manual selection
2. **Fix RTT** by using ELF symbol lookup for instant attachment
3. **Wire up debug mode** to actually use the selected backend

## Section 1: Chip Auto-Detection (`src/chip_detect.rs`)

New module, feature-gated behind `debug`.

### Flow

```
probe-rs Lister::list_all()
        |
        v
For each probe: open -> attach -> read chip info
        |
        v
DetectedChip { probe_name, vendor_id, product_id, serial, chip_name }
        |
        v
BoardRegistry::resolve_by_chip_name(chip_name) -> board_id
        |
        v
TUI auto-fills: target = board_id, interface = detected probe type
```

### Key Decisions

- Returns `Vec<DetectedChip>` — handles multiple connected probes
- Maps probe-rs chip names back to `boards.yaml` IDs via reverse lookup; falls back to raw chip name as ID
- When no probe found, TUI falls back to manual selection (current behavior)
- Runs on TUI startup and on Ctrl+R (re-detect hotkey)
- `boards.yaml` gains optional `detection` field for chips where probe-rs name doesn't match board ID:
  ```yaml
  stm32f4:
    detection:
      probe_rs_chips: ["STM32F407VG", "STM32F407VE", "STM32F407ZE", "STM32F405RG"]
  ```
- Reverse lookup is prefix-based: if probe-rs reports `STM32F407VG`, it matches `stm32f4` because the detection list contains it. If no match found, the raw chip name (e.g. `STM32F407VG`) becomes the board ID directly — probe-rs already knows how to flash it.

### Struct

```rust
pub struct DetectedChip {
    pub probe_name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub serial: Option<String>,
    pub chip_name: String,       // probe-rs target name
    pub board_id: Option<String>, // mapped boards.yaml ID
}
```

## Section 2: RTT ELF Symbol Lookup (`src/debug/rtt.rs`)

### Flow

```
ELF file path --> probe_rs::rtt::Rtt::attach_elf(core, elf_path)
                         |
               .---------'---------.
               v                   v
          Success:            Fallback:
     instant RTT attach    known addresses
     (no scanning)         (ESP32 hardcoded)
                                 |
                                 v
                            Final fallback:
                         Rtt::attach(core)
                         (full RAM scan)
```

### Changes

- `RttConfig` gains `elf_path: Option<String>`
- `ProbeRsRtt::spawn` passes ELF path through to the RTT loop
- Try `Rtt::attach_elf()` first — probe-rs natively parses ELF symbol table for `_SEGGER_RTT`
- Keep existing known-address scan as first fallback, full RAM scan as last resort
- `OpenOcdRtt` receives ELF path for potential OpenOCD symbol-based RTT setup

## Section 3: Board Registry Wiring in TUI (`src/tui/app.rs`)

### Problem

`App::do_flash()` creates `FlashConfig` with empty `board_config`, `board_extra_args`, `board_id` — YAML board config never consulted in TUI mode.

### Fix

- `App::new()` gains `Arc<BoardRegistry>` parameter
- `do_flash()` resolves the selected board against the registry before building `FlashConfig`
- Resolution failure shows error in TUI status bar instead of crashing

```
App::do_flash()
    |
    +-- registry.resolve(&self.target, backend_name)
    |       |
    |       +-- Ok(params) -> FlashConfig { board_config, board_extra_args, board_id }
    |       +-- Err(msg)   -> self.status = msg, return
    |
    +-- do_flash(&config)
```

- Auto-detection integration: when detection finds a chip, `App` stores mapped board ID and resolves against registry to pre-fill backend/interface suggestions

## Section 4: Debug Mode Backend Dispatch (`src/tui.rs`, `src/tui/debug_ui.rs`)

### Problem

`run_debug` ignores all parameters (`_elf`, `_backend`, `_interface`, `_port`, `_gdb`) and hardcodes probe-rs RTT.

### Fix: Backend Dispatch

```
run_debug(elf, target, backend, interface, port, gdb)
    |
    +-- probe-rs  ->  ProbeRsRtt (native, ELF symbol lookup)
    |
    +-- openocd   ->  start OpenOCD GDB server on :port
    |                 +-- OpenOcdRtt (telnet :4444)
    |
    +-- pyocd     ->  start pyOCD GDB server on :port
    |                 +-- PyOcdRtt (telnet :4444, new struct)
    |
    +-- gdb       ->  connect GDB client to :port
                      (no RTT; show GDB console instead)
```

### `RttMonitorState` Changes

- Gains `backend: String` field
- `start_rtt()` dispatches to correct RTT client
- For OpenOCD/pyocd: spawns GDB server subprocess first, then connects RTT via telnet
- `stop_rtt()` kills GDB server subprocess if spawned

### `PyOcdRtt` — New Struct

Near-identical to `OpenOcdRtt` (both use telnet-based RTT). Shares a parameterized `TelnetRtt` struct parameterized by server launch command to avoid duplication.

### `debug_ui.rs` Changes

- Status bar shows current backend name
- GDB backend: shows "RTT unavailable — GDB mode" instead of RTT output

## Files Changed

| File | Change |
|------|--------|
| `src/chip_detect.rs` | **New** — probe-rs auto-detection |
| `src/debug/rtt.rs` | ELF symbol lookup, `PyOcdRtt`, `TelnetRtt` base |
| `src/debug/session.rs` | Add backend field |
| `src/tui/app.rs` | `Arc<BoardRegistry>`, auto-detect integration, registry-aware flash |
| `src/tui/debug_ui.rs` | Backend dispatch, GDB fallback UI |
| `src/tui.rs` | `run_debug` actually uses its params |
| `src/lib.rs` | Pass registry to TUI, detection call on startup |
| `src/backend.rs` | `FlashConfig` used more fully (no struct change needed) |
| `boards.yaml` | Add `detection` mapping field |

## Error Handling

- **No probe found:** TUI shows "未检测到调试探针" status, falls back to manual selection
- **Detection chip unknown to boards.yaml:** Uses raw chip name as board ID, treats it as "unvalidated but usable" — user can still flash
- **RTT ELF symbol not found:** Falls back through known-address scan -> full scan chain; user sees "扫描 RTT 控制块..." status
- **Backend binary not found:** TUI status shows which tool is missing with install hint
- **Registry resolve failure in TUI:** Status bar error, does not crash

## Testing

- Unit tests for `parse_probe_list` (already exists, extend for chip name mapping)
- Unit tests for board registry reverse-lookup
- Manual verification: connect real hardware with probe-rs, confirm auto-detection fills TUI fields
- Manual verification: flash firmware with RTT support, confirm RTT monitor shows output instantly (no scanning delay)
