# Noctrail 渲染架构与技术选型

> 建议文件路径：`docs/rendering-architecture-and-selection.md`
> 建议替代或扩展：`docs/rendering-ecosystem-notes.md`
> 状态：Proposed ADR / Architecture Note
> 最后复核日期：2026-05-17

## 0. 决策摘要

Noctrail 应采用 **GPU-first、RenderPlan-driven、可降级** 的渲染架构。

推荐路线：

- Window/event loop：`winit`
- GPU abstraction：`wgpu`
- 文本 shaping/fallback/rasterization：优先评估 `cosmic-text`
- 初期 wgpu text renderer：优先评估 `glyphon`
- Terminal parser：`vte`
- PTY：`portable-pty`
- Fallback：software/debug path 只作为诊断和保底，不作为长期主渲染路径

核心决策：

```text
TerminalState
  -> immutable snapshot + damage
  -> RenderInput / RenderPlan
  -> Renderer
      -> wgpu surface/device/queue
      -> text shaping/fallback
      -> glyph atlas/cache
      -> cell background/selection/cursor
      -> frame presentation
```

Noctrail 不应把 Hyprland 的 compositor 逻辑塞进 terminal core，也不应模仿 Warp 的产品形态而忽略底层 terminal 正确性。应吸收 Hyprland 的空间感、视觉反馈和配置文化；吸收 Warp 的 block/product 思路但后置；尊重 Nushell/Starship 这类 shell/prompt 生态，不替代它们。

---

## 1. 目标与非目标

### 1.1 目标

渲染系统应满足：

- 跨平台：Windows、macOS、Linux。
- GPU 主路径：默认使用 GPU 渲染文本与 UI。
- 低延迟：输入路径和 frame scheduling 不被视觉效果拖慢。
- 高吞吐：高输出时可以跳帧，但不能破坏 terminal state。
- 正确文本：ASCII、CJK、emoji、combining mark、Nerd Font symbols 有明确策略。
- 多 pane：8+ pane 时 frame time 不随 scrollback 和 pane 数失控。
- 可降级：GPU、字体、透明、blur、动画失败时不影响基础终端可用性。
- 可测试：render fixture、screenshot test、font fallback test、DPI test。

### 1.2 非目标

早期不做：

- 独立 Wayland compositor。
- 全局窗口管理器。
- 复杂 3D scene graph。
- shader-heavy visual demo。
- 把 renderer 写成 terminal parser。
- 把 shell integration 或 block model 写进 renderer。
- 在没有 terminal MVP 前做 Agent UI。

---

## 2. 外部项目定位

### 2.1 Hyprland

Hyprland 是 Wayland compositor，不是 terminal emulator。它的职责是桌面合成、窗口管理、输入输出、workspace 和 layout。

Noctrail 应借鉴：

- dynamic tiling 的空间模型；
- active/inactive visual feedback；
- keyboard-first 操作；
- 视觉效果可配置；
- 配置文化和主题文化。

Noctrail 不应复制：

- compositor responsibilities；
- Wayland server 协议处理；
- output management；
- system-level window management。

### 2.2 Warp

Warp 是 terminal UI product。公开资料能确认其使用图形后端，但内部 UI framework、text engine、scene graph 并没有完整公开。

Noctrail 可借鉴：

- command block 的产品体验；
- command/output 的可复制、可跳转、可折叠；
- prompt 与 shell integration 的用户价值；
- terminal 作为工作流界面，而不是纯字符设备。

Noctrail 不应在早期复制：

- block-first core；
- agent-first workflow；
- 对传统 terminal correctness 的弱化；
- 任何无法从公开实现确认的内部架构假设。

### 2.3 Nushell

Nushell 是 shell，不是 GUI terminal，也不是 compositor。

Noctrail 可借鉴：

- structured data 的后期 lens 思路；
- line editor 与 shell 语义分层；
- 对 terminal UI 的依赖边界。

Noctrail 不应复制：

- shell parser/evaluator；
- shell 内部数据模型；
- 把 terminal emulator 做成 shell。

### 2.4 Starship

Starship 是 prompt generator。它输出 prompt 文本，由 shell 和 terminal 显示。

Noctrail 可借鉴：

- prompt 与 terminal 解耦；
- 上下文收集模块化；
- 配置驱动的 UI 内容。

Noctrail 不应：

- 替换用户 prompt；
- 解析并重写 prompt；
- 假设用户一定使用 Starship。

---

## 3. 推荐技术栈

### 3.1 总览

| 层 | 推荐 | 角色 | 结论 |
|---|---|---|---|
| Window/event loop | `winit` | 跨平台窗口、事件、输入、surface lifecycle | 继续使用 |
| GPU abstraction | `wgpu` | Vulkan/Metal/DX12/WebGPU/GL abstraction | 作为主 GPU path |
| Text shaping/fallback | `cosmic-text` | shaping、font discovery、fallback、layout、rasterization | 优先评估 |
| wgpu text renderer | `glyphon` | 基于 wgpu + cosmic-text 的 2D text renderer | 先用它验证 GPU text path |
| Terminal parser | `vte` | ANSI/VT parser state machine | 继续使用 |
| PTY | `portable-pty` | 跨平台 PTY trait/runtime implementation | 继续使用 |
| Software fallback | 自研最小 fallback 或 debug presenter | 诊断、GPU 不可用保底 | 不作为长期主路径 |

### 3.2 wgpu

选择 `wgpu` 的理由：

- 跨平台统一 API。
- 可映射到 macOS Metal、Windows DX12、Linux Vulkan。
- 避免固定 OpenGL 带来的长期兼容与性能债务。
- 与 Rust 生态中 `glyphon` 等文本渲染库兼容。
- 可以暴露 backend diagnostics，利于 `noctrail doctor gpu`。

Backend 策略：

| 平台 | 首选 | 备选 | 说明 |
|---|---|---|---|
| macOS | Metal | Vulkan portability / GL fallback 视情况 | Metal 是主路径 |
| Windows | DX12 | Vulkan / GL fallback 视驱动 | DX12 是主路径 |
| Linux | Vulkan | GL fallback | Vulkan 是主路径 |
| Web | Browser WebGPU | 暂不作为桌面 MVP 目标 | 后期考虑 |

设计要求：

- renderer init 必须返回可诊断错误。
- GPU backend 名称应进入 `doctor` 输出。
- GPU 初始化失败不能导致 app 无提示黑屏。
- `WGPU_BACKEND` 或 config override 应用于调试。
- surface lost/outdated 必须可恢复。

### 3.3 cosmic-text

选择 `cosmic-text` 的理由：

- 支持 advanced text handling。
- 提供 shaping、font discovery、font fallback、layout、rasterization、editing 抽象。
- 能处理比简单 `char`/`unicode-width` 更复杂的文本情况。
- 适合做 CJK、emoji、Nerd Font fallback 的基础。

注意：

- Terminal grid 仍然是 cell-based。
- `cosmic-text` 不替代 terminal grid。
- Renderer 阶段可以使用 shaping 结果绘制 cell 内文本。
- 对复杂 grapheme/emoji ZWJ/RTL，需要明确支持等级与测试矩阵。

### 3.4 glyphon

选择 `glyphon` 的理由：

- 它是基于 `wgpu`、`cosmic-text`、`etagere` 的 2D text renderer。
- 适合快速验证 GPU text path。
- 自带 text atlas/cache 概念。
- 可降低初期自研 glyph atlas 的风险。

使用策略：

- Phase 3 先用 `glyphon` 完成最小 GPU text renderer。
- 后续根据性能、atlas control、terminal cell 特化需求决定是否自研。
- 不应把 terminal state 结构绑定到 `glyphon` API。
- `RenderInput` 应保持 renderer-agnostic。

### 3.5 vte

选择 `vte` 的理由：

- 它是 parser，而不是完整 terminal emulator。
- Parser 不赋予 escape sequence 语义，语义由 `Perform` 实现提供。
- 这符合 Noctrail 自己维护 `TerminalState` 的设计。

注意：

- `vte` 只解决解析，不解决 terminal correctness。
- CSI/OSC/DEC private mode 的语义要由 `noctrail-term` 明确实现。
- 所有新增 sequence 都要配 fixture。

### 3.6 portable-pty

选择 `portable-pty` 的理由：

- 提供跨平台 PTY API。
- 提供 runtime-selectable trait，适合 Windows 多实现策略。
- 是 WezTerm 生态的一部分，方向与 terminal 项目匹配。

注意：

- `portable-pty` 只是 PTY 边界，不是 runtime event model。
- Noctrail 仍需要自己的 reader thread、bounded channel、lifecycle、backpressure 策略。
- PTY output bytes 不应因为 renderer 慢而被直接丢弃。

---

## 4. 渲染架构

### 4.1 分层

```text
noctrail-term
  TerminalState
  TerminalSnapshot
  DamageSet

noctrail-layout
  LayoutTree
  PaneLayout
  LayoutRect

noctrail-render
  RenderInput
  RenderPlan
  Renderer trait
  GpuRenderer
  TextRenderer
  GlyphAtlas
  SoftwareFallback

noctrail-app
  winit ApplicationHandler
  surface lifecycle
  input routing
  frame scheduling
```

### 4.2 RenderInput

`RenderInput` 是 renderer 的主要输入。它不应拥有 terminal state，只引用或复制必要的 immutable 数据。

```rust
pub struct RenderInput<'a> {
    pub frame_id: u64,
    pub surface_size: PhysicalSize<u32>,
    pub scale_factor: f64,
    pub theme: &'a ResolvedTheme,
    pub panes: &'a [PaneRenderInput<'a>],
}

pub struct PaneRenderInput<'a> {
    pub pane_id: PaneId,
    pub rect_px: RenderRect,
    pub terminal_size: TerminalSize,
    pub snapshot: &'a TerminalSnapshot,
    pub damage: &'a DamageSet,
    pub active: bool,
    pub opacity: f32,
}
```

### 4.3 RenderPlan

`RenderPlan` 是中间层，不应直接等同于 GPU resources。

```rust
pub struct RenderPlan {
    pub clear_color: Color,
    pub panes: Vec<PaneRenderPlan>,
    pub overlays: Vec<OverlayPlan>,
}

pub struct PaneRenderPlan {
    pub pane_id: PaneId,
    pub rect_px: RenderRect,
    pub background: PaneBackground,
    pub rows: Vec<RowPlan>,
    pub cursor: CursorPlan,
    pub selection: SelectionPlan,
    pub border: BorderPlan,
}
```

原则：

- `RenderPlan` 可以由 renderer crate 构造。
- `RenderPlan` 不持有 `wgpu::Buffer`、`wgpu::Texture`、`wgpu::Device`。
- GPU resources 只属于 `GpuRenderer`。
- `RenderPlan` 可用于 software screenshot fixture。

### 4.4 Renderer trait

```rust
pub trait Renderer {
    fn resize(&mut self, size: PhysicalSize<u32>, scale_factor: f64) -> Result<(), RenderError>;
    fn render(&mut self, input: RenderInput<'_>) -> Result<FrameStats, RenderError>;
    fn backend_info(&self) -> BackendInfo;
}
```

`FrameStats` 应包含：

```rust
pub struct FrameStats {
    pub frame_id: u64,
    pub backend: RenderBackendName,
    pub panes: usize,
    pub dirty_rows: usize,
    pub glyphs_prepared: usize,
    pub atlas_uploads: usize,
    pub cpu_prepare_ms: f32,
    pub gpu_submit_ms: f32,
}
```

### 4.5 GpuRenderer 内部结构

```rust
pub struct GpuRenderer {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,

    text: TerminalTextRenderer,
    pane_pipeline: PanePipeline,
    border_pipeline: BorderPipeline,
    overlay_pipeline: OverlayPipeline,

    frame_cache: FrameCache,
    diagnostics: RendererDiagnostics,
}
```

### 4.6 Text renderer

```rust
pub struct TerminalTextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    glyph_atlas: GlyphAtlas,
    row_cache: RowLayoutCache,
}
```

职责：

- resolve font fallback；
- shape text runs；
- rasterize glyphs；
- upload glyphs to atlas；
- produce draw calls for visible rows；
- invalidate cache on font/theme/DPI changes。

不负责：

- parser；
- scrollback mutation；
- selection semantic；
- PTY reading；
- shell integration。

---

## 5. 文本渲染策略

### 5.1 Cell model 与 shaping model

Terminal 是 cell-based，但现代文本不是简单 `char -> cell`。

推荐分层：

```text
Terminal parser
  -> cell content model
     - text: grapheme-ish string
     - width: 0/1/2
     - style
     - wide_continuation
  -> renderer text shaping
     - per row / per style run / per font run
     - fallback font selection
     - glyph placement in cell grid
```

### 5.2 需要支持的文本类型

| 类型 | 支持策略 |
|---|---|
| ASCII | P0，必须稳定 |
| CJK fullwidth | P0，必须正确占 2 cells |
| Combining marks | P0，附着到前一个可见 cell |
| Emoji basic | P1，至少 fallback font 可见 |
| Emoji ZWJ sequence | P1/P2，先列 fixture，逐步提高 |
| Nerd Font symbols | P1，fallback 或用户配置 font |
| RTL/Arabic | P2，明确 limitation 或使用 shaping 支持 |
| Indic scripts | P2，明确 limitation 或使用 shaping 支持 |

### 5.3 宽字符覆盖规则

必须测试：

```text
写入 "中"
移动光标到第二个 cell
写入 "x"
```

要求：

- 不能留下半个 wide char。
- 写入窄字符时，应清理被覆盖 wide glyph 的主 cell 与 continuation cell。
- 写入宽字符时，应清理被覆盖范围内原有 glyph。
- 行尾写入宽字符时行为必须符合 wrap 策略。

### 5.4 Fallback 字体

要求：

- fallback miss 要有 debug log。
- `doctor font` 能输出主字体、fallback 字体、missing glyph 样例。
- CJK、emoji、Nerd Font symbol 分别有 fixture。
- 用户可配置字体优先级。

建议配置：

```toml
[font]
family = "JetBrainsMono Nerd Font"
size = 14.0
fallback = [
  "Noto Sans CJK SC",
  "Noto Color Emoji",
  "Apple Color Emoji",
  "Segoe UI Emoji"
]
```

---

## 6. Frame scheduling

### 6.1 Redraw 原则

Renderer 不应自己决定何时读 PTY。App/runtime 负责事件驱动，renderer 只在收到 render input 时绘制。

```text
RuntimeEvent::Output
  -> drain bounded bytes
  -> terminal.advance()
  -> damage merge
  -> request_redraw()

WindowEvent::RedrawRequested
  -> build RenderInput
  -> renderer.render()
  -> present
```

### 6.2 Event coalescing

可合并：

- 多次 PTY output wakeup；
- 多次 cursor blink；
- 多次 resize；
- 多次 selection drag damage；
- 多个 pane 的 dirty rows。

不可丢弃：

- PTY output bytes；
- input bytes；
- close/kill event；
- resize 的最终状态；
- process exit event。

### 6.3 Budget

建议初始预算：

| 项目 | 初始值 |
|---|---|
| 每帧最大 PTY drain time | 4-6ms |
| 每帧最大 render prepare time | 4-8ms |
| 输入事件优先级 | 高于 bulk output |
| cursor blink interval | 500ms 左右，可配置 |
| idle redraw | 无 damage 不 redraw |

---

## 7. Damage 与缓存

### 7.1 Damage 来源

| 来源 | Damage |
|---|---|
| printable char | 当前行 |
| cursor move | old row + new row |
| clear line | 当前行 |
| clear screen | full pane |
| scroll | visible rows 或 scroll delta |
| resize | full pane |
| font/theme/DPI change | full frame |
| selection change | selection affected rows |
| active pane change | old/new pane border + cursor rows |
| workspace switch | full frame |

### 7.2 缓存层级

```text
TerminalState
  -> dirty rows

Renderer
  -> row layout cache
  -> glyph raster cache
  -> glyph atlas
  -> pane background cache
  -> border geometry cache
```

### 7.3 缓存失效

| 事件 | 失效 |
|---|---|
| text changed | row layout |
| style changed | row layout or draw style |
| font changed | all text cache |
| font size changed | all text cache + atlas |
| DPI changed | all text cache + atlas |
| theme color changed | draw style；必要时 screenshot baseline |
| selection changed | overlay/cache only |
| cursor blink | cursor only |
| pane resize | affected pane layout/cache |

---

## 8. Surface lifecycle

### 8.1 winit lifecycle

要求：

- Window 与 graphics surface 在 `resumed` 后创建。
- `suspended` 时能丢弃或暂停 surface resources。
- `Resized` 时重新 configure surface。
- `ScaleFactorChanged` 时更新 DPI 和 font metrics。
- `RedrawRequested` 中执行 render。
- `AboutToWait` 不应作为主渲染驱动；它只适合调度和检查 wakeup。

### 8.2 Surface error handling

| 错误 | 处理 |
|---|---|
| Lost | recreate/reconfigure surface |
| Outdated | reconfigure surface |
| Timeout | skip current frame |
| OutOfMemory | 进入 fatal diagnostic，避免 silent crash |
| Adapter missing | fallback 或 safe mode |

---

## 9. Fallback 策略

Fallback 不等于长期 software renderer。

### 9.1 GPU fallback

顺序：

1. user-configured backend；
2. platform primary backend；
3. wgpu fallback backend；
4. safe mode clear frame；
5. optional software/debug presenter。

### 9.2 Software fallback 允许做什么

允许：

- debug screenshot fixture；
- safe mode diagnostic；
- CI deterministic rendering；
- GPU 不可用时显示最小可读文本。

不允许：

- 长期作为默认主路径；
- 掩盖 GPU renderer 缺失；
- 让产品目标变成 software terminal。

### 9.3 Safe mode UI

Safe mode 至少显示：

- Noctrail version；
- OS/arch；
- selected backend；
- GPU init error；
- config path；
- font path；
- recommended next command，例如 `noctrail doctor gpu`。

---

## 10. 测试策略

### 10.1 Render fixture

目录建议：

```text
tests/fixtures/render/
  ascii.ntshot
  ansi-16.ntshot
  ansi-256.ntshot
  truecolor.ntshot
  cjk.ntshot
  combining.ntshot
  emoji-basic.ntshot
  emoji-zwj.ntshot
  nerd-font.ntshot
  selection.ntshot
  cursor.ntshot
  underline.ntshot
  resize-dpi-125.ntshot
  pane-border.ntshot
```

### 10.2 Screenshot tests

要求：

- deterministic software/golden path；
- GPU screenshot 可作为 smoke，不强求逐像素完全一致；
- 容忍字体栅格化平台差异；
- 对核心布局、颜色、cursor、selection 做结构化断言。

### 10.3 Performance tests

| 测试 | 目标 |
|---|---|
| 80x24 ASCII redraw | 60 FPS target |
| 8 pane idle | CPU < 5% |
| 8 pane high output | 输入仍可响应 |
| 10k scrollback | 不每帧全量 layout |
| DPI switch | 不 crash，不错位 |
| font fallback stress | 不无限 rasterize |
| atlas eviction | 不闪烁、不泄露内存 |

### 10.4 Diagnostics

`noctrail doctor gpu` 应输出：

```text
Renderer:
  requested backend: auto
  selected backend: Metal / DX12 / Vulkan / GL / Software
  adapter: <name>
  device type: discrete / integrated / cpu / unknown
  surface format: <format>
  present mode: <mode>
  vsync: enabled/disabled
  scale factor: <value>

Fonts:
  primary: <family>
  fallback: [...]
  missing glyph samples: [...]
```

---

## 11. 视觉系统设计

### 11.1 视觉层级

```text
frame background
  -> workspace background
  -> pane background
  -> terminal cell backgrounds
  -> selection background
  -> glyphs
  -> cursor
  -> border/focus ring
  -> overlays/status line/palette
```

### 11.2 Pane visuals

| 元素 | 策略 |
|---|---|
| active border | P1，solid first，gradient later |
| inactive border | 低对比，不能抢文本 |
| gaps | 可配置，默认适中 |
| radius | 可配置，高 DPI 对齐 |
| opacity | 可关闭 |
| blur | 可关闭，不支持即 tinted solid |
| glow | 后置，低性能模式关闭 |
| animation | 短、可关闭，不影响 input |

### 11.3 可读性规则

- 文本对比度优先于背景效果。
- selection 必须清晰。
- active pane 必须一眼可辨。
- cursor 在所有主题下可见。
- blur/opacity 不能导致终端输出不可读。
- 高对比模式必须可用。

---

## 12. 依赖与版本策略

### 12.1 依赖原则

- 只为明确边界引入依赖。
- 每个核心依赖必须有替代方案或退出策略。
- renderer 依赖应被封装在 `noctrail-render` 内。
- app 不应直接依赖 text shaping internals。
- terminal core 不应依赖 GPU/text renderer。

### 12.2 当前建议依赖

```toml
# noctrail-app
winit = "0.30"
arboard = "3"

# noctrail-render
wgpu = "29"
glyphon = "0.11"         # 评估期
cosmic-text = "0.19"     # 也可经 glyphon re-export 使用

# noctrail-term
vte = "0.15"
unicode-width = "0.2"
unicode-segmentation = "1" # 建议加入，用于 grapheme-level 测试/处理

# noctrail-pty
portable-pty = "0.9"
```

版本要求：

- 锁定 minor 范围，避免 renderer 依赖频繁破坏 API。
- renderer ADR 每次大版本升级要复核。
- CI 至少覆盖 stable Rust。
- 若继续使用 Rust 2024 edition，要确保 MSRV 与 ecosystem 匹配。

---

## 13. 备选方案与取舍

### 13.1 固定 OpenGL

优点：

- 心智模型简单。
- 老 terminal 项目经验多。

缺点：

- macOS OpenGL 长期不理想。
- Windows/Linux driver 差异仍然存在。
- 与“每个平台使用现代后端”的目标不一致。
- 后续 text/atlas/compute 扩展能力弱于 wgpu 路线。

结论：不建议作为主路径。

### 13.2 自研全部 text renderer

优点：

- 完全控制性能和 atlas。
- 可针对 terminal cell 特化。

缺点：

- 初期风险过高。
- CJK/emoji/fallback/shaping 复杂度大。
- 容易拖慢 terminal MVP。

结论：不建议一开始全自研。先用 `glyphon`/`cosmic-text` 验证，再按瓶颈替换局部。

### 13.3 iced/egui/slint 等 GUI framework

优点：

- UI widget 快速开发。
- 配置面板、palette、settings 可加速。

缺点：

- terminal text grid 是特化高性能场景。
- 可能引入 frame scheduling 与 GPU resource 控制问题。
- 对多 pane terminal renderer 的控制不足。

结论：主 terminal surface 不建议依赖通用 immediate/retained GUI framework。后期 settings/palette 可局部评估。

### 13.4 Software renderer first

优点：

- 可快速出画面。
- CI screenshot 更容易。

缺点：

- 与 GPU terminal 目标偏离。
- 后续迁移成本高。
- 容易让主路径停在 prototype。

结论：允许作为 debug/fallback，不允许作为长期默认主路径。

---

## 14. 推荐实施顺序

### R0：RenderInput 与 Damage API

交付：

- `RenderInput`
- `PaneRenderInput`
- `DamageSet`
- `FrameStats`
- `Renderer` trait
- tests for render-plan construction

不做：

- wgpu；
- glyph atlas；
- animation。

### R1：wgpu clear frame

交付：

- `GpuRenderer::new(window)`
- surface configure；
- clear color；
- resize；
- backend diagnostics；
- safe error handling。

验收：

```bash
cargo run -p noctrail-app
cargo run -p noctrail-cli -- doctor gpu
```

### R2：ASCII glyph path

交付：

- fixed monospace font；
- ASCII glyph raster/cache；
- foreground/background；
- cursor；
- 16 color；
- screenshot fixture。

### R3：Unicode/fallback

交付：

- CJK；
- emoji basic；
- Nerd Font symbols；
- combining marks；
- fallback diagnostics；
- font config。

### R4：Damage-based multi-pane render

交付：

- dirty row render；
- 8 pane support；
- scrollback not full layout every frame；
- performance counters；
- atlas eviction tests。

### R5：Visual polish

交付：

- borders；
- gaps；
- opacity；
- blur fallback；
- animation off switch；
- low power mode。

---

## 15. 验收标准

Renderer MVP 通过标准：

- 默认 backend 是 GPU。
- 80x24 ASCII terminal 可稳定显示。
- ANSI 16/256/true color 可见正确。
- Cursor、selection、underline 可见正确。
- Resize、DPI change 不 crash。
- CJK/emoji/Nerd Font fallback 有明确行为。
- GPU init failure 有诊断或 fallback。
- Renderer 不持有 mutable terminal state。
- Renderer 不依赖 agent/storage/shell integration。
- 10k scrollback 不参与每帧全量 layout。
- 8 pane high output 下 UI 可响应输入和关闭。

Beta 前 renderer 阻断项：

- GPU path 不存在或长期不可用。
- software presenter 是默认主路径。
- 高输出导致 UI 冻结。
- 多 pane 输入输出串线。
- resize 后 glyph/cursor/cell 错位严重。
- font fallback 大面积 tofu。
- GPU error 导致无提示黑屏。
- renderer 修改 terminal state。
- 视觉效果影响文本可读性。

---

## 16. 参考资料

> 以下链接用于技术复核，版本和 API 需要在升级依赖时重新确认。

- `wgpu Backends`: <https://wgpu.rs/doc/wgpu/struct.Backends.html>
- `winit ApplicationHandler`: <https://docs.rs/winit/latest/winit/application/trait.ApplicationHandler.html>
- `glyphon`: <https://docs.rs/glyphon/latest/glyphon/>
- `cosmic-text`: <https://docs.rs/cosmic-text/latest/cosmic_text/>
- `vte`: <https://docs.rs/vte/latest/vte/>
- `portable-pty`: <https://docs.rs/portable-pty/latest/portable_pty/>
- `unicode-width`: <https://docs.rs/unicode-width/latest/unicode_width/>

---

## 17. 最终决策

推荐采用：

```text
winit + wgpu + cosmic-text/glyphon + vte + portable-pty
```

其中：

- `winit` 管窗口、事件、surface lifecycle；
- `wgpu` 管跨平台 GPU backend；
- `cosmic-text` 管 shaping、fallback、layout、rasterization 基础能力；
- `glyphon` 用于快速建立 2D GPU text renderer；
- `vte` 只做 parser；
- `portable-pty` 只做 PTY boundary；
- Noctrail 自己维护 terminal state、runtime event model、layout、damage、frame scheduling、product semantics。

这条路线的目标不是最快做出炫酷 UI，而是先建立一个能长期演进的 GPU terminal renderer。
