# Remove API/HTTP Layer, Optimize CLI — Design

**Date**: 2026-06-22
**Status**: Approved

## Goal

Remove the axum-based HTTP REST + WebSocket API layer from `loading-chip`. Keep and optimize the command-line implementation: TUI mode, CLI mode, and headless JSON mode.

## Motivation

The API layer (`src/api/`) is unused in practice — all real workflows use the TUI or CLI. Removing it eliminates 3 heavy dependencies (`axum`, `tower-http`, `futures-util`) and ~50 lines of broadcast/session infrastructure that existed solely to serve HTTP clients.

## Design

### 1. Deletions (10 files)

```
src/api.rs
src/api/server.rs
src/api/routes.rs
src/api/routes/board.rs
src/api/routes/debug.rs
src/api/routes/detect.rs
src/api/routes/flash.rs
src/api/routes/rtt.rs
src/api/routes/status.rs
```

### 2. Dependency Changes (`Cargo.toml`)

Remove 3 crates:

- `axum` (HTTP framework)
- `tower-http` (CORS middleware)
- `futures-util` (WebSocket stream splitting)

Keep `tokio` at `features = ["full"]` — still needed for `tokio::sync::Mutex` in `AppState` and the tokio runtime used by `state.flash().await` in CLI/headless paths.

### 3. AppState Cleanup (`src/app/state.rs`, `src/app.rs`)

Remove from `AppState`:

| Field | Why |
|-------|-----|
| `rtt_tx: broadcast::Sender<RttOutput>` | Only used by WebSocket `/api/rtt` handler and `api/server.rs` |
| `rtt_session: Arc<Mutex<Option<Box<dyn RttClient>>>>` | Only used by `POST /api/debug/start\|stop` |

Remove `RttClient` import — no longer needed.

`AppState::new()` simplifies: no `broadcast::channel(256)` call, no `rtt_session` init.

Update doc comment in `src/app.rs` — remove "axum" reference.

### 4. RTT Broadcast Infrastructure (`src/debug/rtt.rs`)

Remove three items:

1. **`GLOBAL_BROADCAST` static + `set_global_broadcast()` function** — set by `api/server.rs`, read by `ProbeRsRtt` polling loop. No other callers.

2. **`broadcast_tx` field in `RttConfig`** — optional broadcast sender carried to the probe-rs polling thread.

3. **Broadcast logic in the probe-rs polling loop** — after sending to `crossbeam_channel` (used by TUI), the loop also sent to `GLOBAL_BROADCAST`. That code block is removed.

**Rationale**: The TUI (`src/tui/debug_ui.rs`) creates its own `crossbeam_channel::(tx, rx)` pair and passes `tx` to `create_rtt_client()`. It never reads from the tokio broadcast channel. The broadcast channel existed only for the API WebSocket handler.

### 5. CLI Changes (`src/cli.rs`)

Remove from `Commands::Run`:

- `--api` flag
- `--api-addr` flag

### 6. Dispatch Changes (`src/lib.rs`)

In `run_flash()`:

1. Remove `api: bool` and `api_addr: String` parameters
2. Remove the `if api { api::spawn_server(...) }` block (~10 lines)
3. Remove the headless+API idle path: `if headless && _api_shutdown.is_some() { ... wait for Ctrl+C ... }` block (~10 lines)

In `run()`:
- Drop `api` and `api_addr` from the `Commands::Run` destructure

### Resulting Architecture

```
                    ┌──────────────────────────────┐
                    │  AppState                     │
                    │  tokio::sync::Mutex fields     │
                    │  (no broadcast, no session)    │
                    └──────────┬───────────────────┘
                               │
                  ┌────────────┴────────────┐
                  ▼                         ▼
           ┌──────────────┐          ┌──────────────┐
           │  TUI (ratatui)│          │  CLI/Headless │
           │  crossbeam RTT│          │  JSON output  │
           └──────────────┘          └──────────────┘
```

## Files Changed Summary

| File | Action |
|------|--------|
| `src/api.rs` | Delete |
| `src/api/server.rs` | Delete |
| `src/api/routes.rs` | Delete |
| `src/api/routes/board.rs` | Delete |
| `src/api/routes/debug.rs` | Delete |
| `src/api/routes/detect.rs` | Delete |
| `src/api/routes/flash.rs` | Delete |
| `src/api/routes/rtt.rs` | Delete |
| `src/api/routes/status.rs` | Delete |
| `Cargo.toml` | Remove 3 dependencies |
| `src/cli.rs` | Remove `--api` and `--api-addr` flags |
| `src/lib.rs` | Remove API spawning, clean up `run_flash()` |
| `src/app/state.rs` | Remove `rtt_tx`, `rtt_session`, simplify `new()` |
| `src/app.rs` | Update doc comment |
| `src/debug/rtt.rs` | Remove `GLOBAL_BROADCAST`, `set_global_broadcast()`, `broadcast_tx`, broadcast logic |

## Non-Goals

- Not changing TUI behavior or appearance
- Not changing flash/backend logic
- Not changing board registry or chip detection
- Not changing the `tokio` feature set

## Verification

```bash
cargo build                  # Must compile without errors
cargo build --release        # Release build
cargo test                   # All tests pass
cargo run -- run             # TUI launches
cargo run -- run -b probe-rs -i swd -t stm32f4 -e test.elf  # CLI mode
cargo run -- run --headless -b probe-rs -i swd -t stm32f4 -e test.elf  # Headless
cargo run -- detect          # Chip detection
cargo run -- init            # Environment setup
cargo run -- debug -e test.elf -t stm32f4  # Debug/RTT mode
```
