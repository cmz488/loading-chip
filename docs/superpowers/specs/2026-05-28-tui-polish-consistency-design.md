# TUI 美化、一致性修复与 RTT 精简设计

Date: 2026-05-28
Status: approved

## Overview

四项改进统一实施：

1. **消除描述与实际功能不一致** — 修复文档注释、统一 FlashConfig 构建、补充 API 端点、统一芯片检测入口
2. **TUI 选择显示修复** — 表单字段显示短 key，下拉菜单显示完整描述
3. **Debug UI 精简** — 删除虚假 dap-ui 面板，改为纯 RTT 全屏监视器
4. **TUI 美化** — 现代暗色配色、"cmz" lolcat 彩虹品牌栏、跳动+旋转烧录动画

---

## Section 1: 消除不一致 + 统一 Flash 路径

### 1a. 文档注释修复 (`src/lib.rs`)

- 删除"dap-ui 风格调试界面"的误导性描述
- 补充缺失的调试器 `xds110`
- 补充缺失的芯片 `mspm0g3507`
- 补充 `run --api` 用法说明

### 1b. FlashConfig::from_registry() 工厂 (`src/backend.rs`)

当前 4 个位置有 `FlashConfig` 构建逻辑的重复代码:
- `run_headless` (lib.rs)
- `run_cli_mode` (lib.rs)
- `App::do_flash` (tui/app.rs)
- `api/routes/flash.rs` POST handler

新增统一工厂方法：

```rust
impl FlashConfig {
    pub fn from_registry(
        be: FlashBackend,
        registry: &BoardRegistry,
        board_id: &str,
        interface: &str,
        elf_path: &str,
        gdb_port: &str,
        pyocd_path: &str,
        timeout_secs: u64,
    ) -> Result<Self, String> {
        let backend_name = be.yaml_key();
        let params = registry.resolve(board_id, backend_name)?;
        Ok(Self {
            backend: be,
            interface: interface.to_string(),
            target: params.target,
            elf_path: elf_path.to_string(),
            gdb_port: gdb_port.to_string(),
            pyocd_path: pyocd_path.to_string(),
            timeout_secs,
            board_config: params.config,
            board_extra_args: params.extra_args,
            board_id: board_id.to_string(),
        })
    }
}
```

所有 4 条路径改为调用此工厂，消除重复代码。

### 1c. API 新增端点

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/detect` | 运行 `chip_detect::detect_chips()`，返回检测结果 |
| POST | `/api/debug/start` | 启动 RTT 调试会话（background task） |
| POST | `/api/debug/stop` | 停止当前调试会话 |

### 1d. 芯片检测统一

Headless 模式不再跳过芯片检测。所有模式（TUI/Headless/API）启动时运行 `detect_chips()`，结果:
- TUI 模式: 自动填写表单字段
- Headless 模式: 检测结果作为 JSON 输出的一部分
- API 模式: 通过 `/api/detect` 获取

---

## Section 2: TUI 选择显示修复

### 问题

当前 `render_form` 用 `app.backend_label()` / `app.iface_label()` / `app.target_label()` 渲染字段值，返回超长描述文本。

### 修复

- 表单字段: 直接显示 `app.backend` / `app.interface` / `app.target`（短 key）
- 下拉菜单: 保持现有 `key  —  description` 格式
- presets.rs 的 `*_label()` 函数保留给下拉菜单使用

### 代码变更

`src/tui/ui.rs` `render_form`: `render_field` 的 value 参数从 `app.backend_label()` 改为 `&app.backend`。接口和芯片字段同理。

---

## Section 3: Debug UI 精简为纯 RTT 监视器

### 删除

- `render_breakpoints` — DebugSession 无 breakpoints 数据
- `render_call_stack` — 无 frames 数据
- `render_watches` — 无 watches 数据
- `render_variables` — 无 variables 数据
- `render_console` — 无 console 数据
- `render_left_panel` / `render_right_panel` / `render_main` — 面板布局

### 保留

三行布局:

```
┌─ 工具栏 ──────────────────────────────────────┐
│ 🔥 RTT 监视器  ● 已连接  stm32f4  [probe-rs] │
├─ RTT 输出 ────────────────────────────────────┤
│  [0] 程序启动...     (绿色 = channel 0)        │
│  [1] ⚠ 传感器异常   (黄色 = channel 1)        │
│  ...（自动滚底，实时刷新）                     │
├─ 状态栏 ──────────────────────────────────────┤
│ Esc/q 返回  Ctrl+C 清空  ↑↓ 滚动              │
└───────────────────────────────────────────────┘
```

`debug_ui.rs` 从 ~650 行减少到 ~250 行。

---

## Section 4: TUI 美化

### 4a. 现代暗色配色

```rust
const BG:       Color = Rgb(18, 18, 24);    // 深蓝黑底
const SURFACE:  Color = Rgb(28, 28, 38);    // 卡片面板
const BORDER:   Color = Rgb(48, 48, 58);    // 边框
const TEXT:     Color = Rgb(212, 212, 220);  // 主文字
const TEXT_DIM: Color = Rgb(108, 108, 122);  // 次要文字
const ACCENT:   Color = Rgb(96, 165, 250);   // 蓝色强调
const SUCCESS:  Color = Rgb(74, 222, 128);   // 绿色成功
const ERROR:    Color = Rgb(248, 113, 113);   // 红色失败
const WARNING:  Color = Rgb(251, 191, 36);   // 黄色警告
```

所有边框统一 `BorderType::Rounded`。

### 4b. "cmz" 彩虹品牌栏

HSV 色彩空间，色调在 0°-360° 之间均匀分布在 "c", "m", "z" 三个字符上。每帧色调 +1°，产生 lolcat 风格的缓慢流动彩虹效果:

```
 hue: 0°       120°      240°
       ↓         ↓         ↓
  ┌─────────────────────────────────────────┐
  │  c         m         z    🔥 LOADING-CHIP │
  └─────────────────────────────────────────┘
   Red     Green      Blue
```

### 4c. 跳动 + 旋转烧录动画

烧录过程中，闪烁指示器 + 跳动点:

```
帧周期 800ms:
  "⏳ 正在烧录... ●"   →   "⌛ 正在烧录... ○"   →   "⏳ 正在烧录... ◌"   →   "⌛ 正在烧录... ○"
```

动画通过帧计数器 `app.flash_frame`（每 100ms +1）驱动。

### 4d. 结果色彩反馈

- 成功: 结果区域绿色边框 + 绿色粗体消息
- 失败: 结果区域红色边框 + 红色粗体消息

---

## Files Changed

| File | Change |
|------|--------|
| `src/lib.rs` | 文档注释更新 + 统一调用 `FlashConfig::from_registry` |
| `src/cli.rs` | 帮助文本准确性修复 |
| `src/backend.rs` | 新增 `FlashConfig::from_registry()` 工厂 |
| `src/tui/ui.rs` | 暗色配色、cmz 品牌栏、跳动动画、key 显示、圆角边框 |
| `src/tui/app.rs` | 添加 `flash_frame` 动画计数器 |
| `src/tui/events.rs` | 无改动 |
| `src/tui/debug_ui.rs` | 删除 dap-ui 面板，精简为 ~250 行纯 RTT |
| `src/presets.rs` | 无改动 |
| `src/api/routes.rs` | 注册 detect + debug 新路由 |
| `src/api/routes/detect.rs` | **新文件** — GET /api/detect |
| `src/api/routes/debug.rs` | **新文件** — POST /api/debug/start, /api/debug/stop |
| `src/api/server.rs` | 新路由注册 |

## Testing

- 单元测试: `cargo test --lib`（34 pass），验证 FlashConfig::from_registry 不破坏现有行为
- 手动验证: TUI 选择显示短 key、下拉显示完整描述
- 手动验证: 品牌栏颜色流动效果
- 手动验证: 烧录动画跳动旋转
- 手动验证: RTT 监视器只有 RTT 输出 + 工具栏 + 状态栏
