# loading-chip 🔥

嵌入式芯片烧录/调试 TUI 工具 — 支持 probe-rs / OpenOCD / pyOCD / GDB 四种后端，提供终端交互界面和命令行无头模式。

## 系统依赖

### 运行时依赖（至少安装一个）

| 工具 | 用途 | 安装方式 |
|------|------|---------|
| **probe-rs** (推荐) | Rust 原生烧录/调试，零配置，速度最快 | `cargo install probe-rs-tools` 或项目内置 library API |
| **OpenOCD** ≥ 0.12 | 开源调试器，芯片支持广泛 | `pacman -S openocd` / `apt install openocd` |
| **pyOCD** | Python CMSIS-Pack 生态烧录工具 | `pip install pyocd` |
| **arm-none-eabi-gdb** | GDB 通用烧录（需配合 GDB Server） | `pacman -S arm-none-eabi-gdb` |

### 可选（ESP32 Xtensa / RISC-V 芯片）

| 工具 | 用途 |
|------|------|
| `xtensa-esp32-elf-gdb` | ESP32 / ESP32-S3 的 Xtensa GDB |
| `riscv32-esp-elf-gdb` | ESP32-C3 的 RISC-V GDB |
| `openocd-esp32` | Espressif 分支 OpenOCD（推荐用于 ESP32 系列） |

### 系统配置

```bash
# Linux: 添加 udev 规则以便访问调试探针
echo 'SUBSYSTEM=="usb", ATTRS{idVendor}=="1366", MODE="0666"' | sudo tee /etc/udev/rules.d/99-jlink.rules
echo 'SUBSYSTEM=="usb", ATTRS{idVendor}=="0483", MODE="0666"' | sudo tee /etc/udev/rules.d/99-stlink.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

## 安装

### 从源码编译

```bash
git clone <repo-url> loading-chip
cd loading-chip

# 默认启用 probe-rs（debug feature）
cargo build --release

# 不启用 probe-rs
cargo build --release --no-default-features
```

编译产物：`target/release/loading-chip`

### 初始化环境

```bash
# 检测本地可用的后端工具 + 调试探针，生成 ~/.config/loading-chip/config.yaml
loading-chip init
```

## 项目架构

```
src/
├── lib.rs              # 入口：解析 CLI → 分发到 TUI/Headless/CLI/Debug/Init/Detect
├── cli.rs              # clap 命令行参数定义
├── main.rs             # fn main() → loading_chip::run()
│
├── board.rs            # 板子注册表 — 从 boards.yaml 加载，管理芯片↔后端映射
├── boards.yaml         # 12 块板子配置（STM32/ESP32/RP2040/nRF52/GD32/AT32/MSPM0）
├── presets.rs          # TUI 下拉菜单的预设数据（后端/接口/芯片列表）
│
├── backend/
│   ├── mod.rs          # Backend trait + FlashBackend 枚举 + do_flash() 调度 + FlashConfig 工厂
│   ├── gdb.rs          # arm-none-eabi-gdb 批量模式烧录
│   ├── openocd.rs      # OpenOCD 直接烧录（interface + target 分离模式）
│   ├── probe_rs.rs     # probe-rs CLI 下载
│   ├── pyocd.rs        # pyOCD flash 子命令
│   └── mappings.rs     # 芯片名/接口名/错误特征映射表
│
├── tui/
│   ├── mod.rs          # TUI 主循环、烧录 ↔ 调试切换、auto-detect 自动填充
│   ├── app.rs          # 应用状态机：参数选择、FlashConfig 构建
│   ├── ui.rs           # 烧录表单渲染：品牌栏、模式切换、下拉菜单、暗色主题
│   ├── events.rs       # 键盘事件分发（Tab/Enter/F5/F12/↑↓）
│   └── debug_ui.rs     # RTT 监视器：三行布局（工具栏 + 输出 + 快捷键）
│
├── debug/
│   ├── mod.rs          # 调试模块入口
│   ├── rtt.rs          # RTT 客户端（probe-rs 库 API / OpenOCD telnet / pyOCD telnet）
│   └── session.rs      # DebugSession 状态管理
│
├── app/
│   └── state.rs        # AppState — TUI/CLI/Headless 共享状态

├── chip_detect.rs      # probe-rs 芯片自动检测
├── config.rs           # 用户配置文件读写（~/.config/loading-chip/config.yaml）
├── setup.rs            # loading-chip init 环境检测
```

## 数据流

```
CLI 参数
  ├── 无参数          → TUI 交互模式
  ├── run              → TUI / --headless JSON / 全参数 CLI
  ├── debug -e <ELF>   → RTT 实时监视器（TUI）
  ├── detect           → probe-rs 芯片检测
  └── init             → 环境检测 → 生成配置文件

TUI 模式:
  用户选择芯片/接口/后端 → FlashConfig::from_registry() → do_flash()
    ↕ F5 切换
  RTT 监视器 → ProbeRsRtt (probe-rs 库 API) → 实时读取芯片 RAM 中的 RTT 缓冲区

Headless 模式:
  全参数 CLI → state.flash() → do_flash() → JSON stdout

CLI 模式:
  全参数 CLI → state.flash() → do_flash() → 文本输出
```

## 用户接口
### 使用probe-rs检测板子是否连接
```bash
loading-chip detect
```

### TUI 交互模式

```bash
# 启动 TUI
loading-chip run
```

**快捷键:**

| 按键 | 功能 |
|------|------|
| `Tab` / `Shift+Tab` | 切换焦点字段 |
| `↑` / `↓` | 下拉菜单中移动选择 |
| `Enter` | 确认选择 / 开始烧录 / 搜索固件 |
| `F5` | 切换到 RTT 监视器 |
| `F12` | 重新检测调试探针 |
| `Esc` / `q` | 退出 |

**固件文件搜索:** 自动递归扫描当前目录（深度 5 层），支持 `.elf` / `.out` / `.bin` / `.hex` / `.axf` / `.ihx`

**自动检测:** 启动时运行 probe-rs 芯片检测，自动填入接口类型；检测到芯片型号时自动匹配

### 命令行模式

```bash
# 全参数烧录（跳过 TUI）
loading-chip run -b probe-rs -i jlink -t stm32f4 -e ./firmware.elf

# 调试模式 — 启动 RTT 监视器
loading-chip debug -e ./firmware.elf -t mspm0g3507 -b probe-rs -i jlink
```

### 无头模式（IDE 集成）

```bash
# JSON 输出，供 IDE / CI 解析
loading-chip run --headless -b probe-rs -i jlink -t stm32f4 -e ./firmware.elf
# → {"success":true,"message":"✅ 烧录成功！...","command":"...","stdout":null,"stderr":null}
```

## 支持的硬件

### 调试器/探针

stlink, jlink, cmsis-dap, daplink, xds110, swd, jtag

### 目标芯片

| 系列 | 芯片 | 架构 | 验证状态 |
|------|------|------|---------|
| STM32F1 | STM32F103C8 | Cortex-M3 | — |
| STM32F4 | STM32F407/429 | Cortex-M4 | — |
| STM32H7 | STM32H743/750 | Cortex-M7 | — |
| STM32G0 | STM32G030 | Cortex-M0+ | — |
| ESP32 | ESP32 | Xtensa | — |
| ESP32-S3 | ESP32-S3 | Xtensa | — |
| ESP32-C3 | ESP32-C3 | RISC-V | — |
| RP2040 | RP2040 | Cortex-M0+ | — |
| nRF52 | nRF52840 | Cortex-M4 | — |
| GD32 | GD32F303/350 | Cortex-M4 | — |
| AT32 | AT32F403A/407 | Cortex-M4 | — |
| MSPM0G3507 | MSPM0G3507 | Cortex-M0+ | ✅ probe-rs + J-Link 实测通过 |

### 烧录后端

| 后端 | 烧录 | RTT | 说明 |
|------|------|-----|------|
| **probe-rs** | ✅ | ✅ (库 API) | Rust 原生，零配置，速度最快。RTT 通过 ELF 符号 `_SEGGER_RTT` 定位 |
| **OpenOCD** | ✅ | ✅ (telnet) | 芯片支持最广，需外部安装 |
| **pyOCD** | ✅ | ✅ (telnet) | CMSIS-Pack 生态，仅 ARM Cortex-M |
| **GDB** | ✅ | ❌ | 需配合外部 GDB Server，不支持 RTT |

## 板子配置

用户可通过 `boards.yaml` 自定义板子配置。搜索路径优先级：

1. `LOADING_CHIP_BOARDS` 环境变量
2. `~/.config/loading-chip/boards.yaml`
3. 可执行文件同目录
4. 当前目录
5. 内置默认配置（编译嵌入）

YAML 格式：

```yaml
boards:
  my-board:
    name: "My Custom Board"
    manufacturer: "Vendor"
    architecture: arm
    interfaces: [swd, jlink]
    backends:
      probe-rs: { target: "STM32F407VG" }
      openocd: { target: "stm32f4x", config: "target/stm32f4x.cfg" }
      gdb: { target: "stm32f4" }
    note: "自定义板子"
    detection:
      probe_rs_chips: ["STM32F407VG", "STM32F407VE"]
```

## 功能特性

- **自动芯片检测** — 启动时通过 probe-rs 识别连接的探针和芯片
- **固件文件搜索** — 递归扫描 6 种固件格式 (.elf/.out/.bin/.hex/.axf/.ihx)
- **RTT 实时监视** — 直接读取芯片 RAM 中的 RTT 缓冲区（probe-rs 库 API），ELf 符号定位，行缓冲按行输出
- **多后端 RTT** — probe-rs（库 API）/ OpenOCD（telnet）/ pyOCD（telnet）
- **芯片自适应** — 选择芯片后自动匹配推荐接口类型
- **统一烧录接口** — `AppState::flash()` 为 CLI/Headless 模式提供统一烧录入口
- **烧录动画** — 跳动 + 旋转进度指示
- **暗色主题** — 统一 11 色调色板 + 圆角边框

## 测试

```bash
cargo test --lib             # 34 个单元测试
cargo run -- run             # 启动 TUI 手动测试
cargo run -- run --headless -b probe-rs -i jlink -t <chip> -e <elf>  # 无头模式测试
```

## License

MIT
