# Noctrail 工程路线图

> 建议文件路径：`docs/roadmap.md`  
> 建议替代：`docs/real-plan.md`  
> 状态：Active  
> 维护原则：本文是唯一的产品与工程主路线图；具体技术选型放入 `docs/adr/`；具体协议与数据结构放入 `docs/specs/`。

## 0. 执行摘要

Noctrail 的近期目标不是“AI 终端”，也不是“Hyprland clone”。Noctrail 应先成为一个可靠、低延迟、跨平台、GPU 加速、键盘优先的平铺式 GUI 终端。

核心路线：

1. **先稳定 terminal core**：正确处理 VT 序列、Unicode、scrollback、selection、alternate screen、resize、damage。
2. **再稳定 PTY/runtime**：可靠启动 shell，处理输入输出、resize、close、backpressure、process lifecycle。
3. **再建立真实 GPU 渲染主路径**：`RenderPlan` 只描述“画什么”，renderer 负责“怎么画”，默认路径应走 GPU。
4. **再做 app 与多 pane**：单窗口单 pane 可日常使用后，再接入平铺布局、workspace、scratch pane。
5. **最后做 shell integration、Blocks、Agent**：这些是上层产品语义，不能污染 terminal core，也不能影响传统终端正确性。

默认策略：

- 终端正确性优先于视觉效果。
- 输入延迟优先于动画复杂度。
- GPU 主路径优先于长期 software presenter。
- Blocks/Agent 后置，且默认不影响终端基础使用。
- 每个阶段必须有验收标准、测试命令、阻断项。

---

## 1. 产品边界

### 1.1 Noctrail 是什么

Noctrail 是一个 Rust-native、跨平台、GPU 加速、Hyprland 风格的平铺式 GUI 终端。

它应该具备：

- 传统 terminal emulator 的正确性；
- 多 pane 与 workspace 的空间管理能力；
- 可降级的现代视觉系统；
- 键盘优先的操作模型；
- 后期可选的 command block、structured output、agent review workflow。

### 1.2 Noctrail 不是什么

Noctrail 不应在早期变成以下任何一种东西：

- 不是 shell。
- 不是 Wayland compositor。
- 不是 Hyprland clone。
- 不是一开始就带完整 Agent 的 IDE。
- 不是把 shell integration、storage、policy、agent 全塞进 terminal grid 的产品原型。

### 1.3 第一性原则

| 原则 | 说明 | 工程含义 |
|---|---|---|
| 终端正确性优先 | `nvim`、`tmux`、`fzf`、`ssh`、`less`、`top` 必须可靠 | terminal core 必须先通过 fixture/golden test |
| 状态机要小 | terminal、PTY、runtime、layout、renderer、config 各自维护自己的不变量 | 不跨层共享 mutable state |
| 渲染主路径必须真实 | 目标是 GPU terminal，而不是软件 presenter 原型 | `wgpu`/glyph pipeline 必须尽早跑通 |
| 高输出不能冻结 UI | `yes`、`seq 1 100000`、large build log 都不能让 app 失控 | bounded channel、drain budget、redraw coalescing |
| 多 pane 是核心能力 | workspace、pane、focus、resize、scratch 是产品核心 | app state 不能长期维持单 pane 结构 |
| 效果必须可降级 | blur、opacity、glow、animation 不可用时不影响可读性 | theme 与 renderer 必须有 fallback |
| Agent 永远后置 | Agent 只能建议，不能静默执行 | Agent 不直接写 PTY，必须走 review/policy |

---

## 2. 建议仓库结构

早期保持 8 个核心 crate，不恢复 agent/storage/policy。

```text
crates/
  noctrail-cli/       # doctor、replay、render-fixtures、pty-smoke 等开发入口
  noctrail-config/    # TOML 配置、schema、默认值、热加载校验
  noctrail-term/      # terminal state machine、grid、scrollback、selection、damage
  noctrail-pty/       # PTY/ConPTY、process lifecycle、resize、read/write
  noctrail-runtime/   # Pane runtime registry、event routing、backpressure
  noctrail-layout/    # workspace、pane tree、Dwindle/Master/Monocle 纯逻辑
  noctrail-render/    # RenderPlan、GPU renderer、glyph atlas、fallback、screenshot tests
  noctrail-app/       # winit app loop、window、input routing、frame scheduling、platform glue
```

后期再加入：

```text
crates/
  noctrail-shell-integration/  # shell hooks、OSC marker、cwd/exit/status 捕获
  noctrail-blocks/             # command block model 和 block UI
  noctrail-policy/             # command risk、review、permission model
  noctrail-agent/              # provider adapter、suggestion、patch proposal
  noctrail-storage/            # block history、audit ledger、local persistence
```

### 2.1 Crate 职责边界

| Crate | 做 | 不做 |
|---|---|---|
| `noctrail-term` | bytes → terminal state；snapshot；selection；scrollback；damage | 不知道 pane、workspace、agent、storage、shell hooks |
| `noctrail-pty` | spawn/read/write/resize/close PTY session | 不知道 renderer、layout、terminal grid |
| `noctrail-runtime` | 多 pane runtime registry、event routing、bounded queues | 不解析 VT，不画 UI |
| `noctrail-layout` | pane tree、workspace、rect arrangement、focus model | 不持有 PTY 或 terminal state |
| `noctrail-render` | GPU text rendering、glyph cache、surface、fallback、screenshot tests | 不处理 shell/input/PTY lifecycle |
| `noctrail-app` | event loop、input routing、frame scheduling、window lifecycle | 不塞 terminal/parser/renderer 细节进单体状态 |
| `noctrail-config` | config schema、defaults、validation、hot reload | 不读 secrets，不读 shell history |
| `noctrail-cli` | developer tooling、doctor、fixture replay | 不承载 GUI app 业务逻辑 |

---

## 3. 核心架构

### 3.1 主数据流

```text
Keyboard / Mouse / Clipboard
  -> KeyTranslator
  -> RuntimeCommand::Write { pane_id, bytes }
  -> PtyWriter

PTY output
  -> RuntimeEvent::Output { pane_id, bytes }
  -> TerminalState::advance(bytes)
  -> AdvanceResult { damage, cursor, scroll }
  -> RenderInput per dirty pane
  -> Renderer::render_frame()

Window resize
  -> LayoutState::arrange(surface)
  -> pane rect -> cell size
  -> RuntimeCommand::Resize { pane_id, rows, cols, pixels }
  -> TerminalState::resize(cols, rows)
  -> full pane damage
```

### 3.2 推荐 app state

不要长期维持单 pane `DesktopApp`。即使早期只显示一个 pane，也应让结构支持多 pane。

```rust
pub struct DesktopApp {
    surface: LayoutRect,
    layout: LayoutTree,
    panes: HashMap<PaneId, TerminalPane>,
    active_pane: Option<PaneId>,
    renderer_mode: RendererMode,
}

pub struct TerminalPane {
    pane_id: PaneId,
    terminal: TerminalState,
    runtime: Option<PaneRuntimeHandle>,
    terminal_size: PtySize,
    last_damage: DamageSet,
}
```

### 3.3 Runtime event model

```rust
pub enum RuntimeCommand {
    Write { pane_id: PaneId, bytes: Vec<u8> },
    Resize { pane_id: PaneId, size: PtySize },
    Close { pane_id: PaneId },
    Restart { pane_id: PaneId, command: PtyCommand },
}

pub enum RuntimeEvent {
    Output { pane_id: PaneId, bytes: Bytes },
    Exited { pane_id: PaneId, status: PtyExitStatus },
    Error { pane_id: PaneId, error: RuntimeError },
}
```

要求：

- PTY bytes 不随意丢弃。
- 可以合并 redraw request。
- 可以限制每帧 drain budget。
- 高输出时 UI 仍能响应输入、关闭、resize。
- pane close 必须回收 child process，不能留 zombie。

### 3.4 Terminal advance result

```rust
pub struct AdvanceResult {
    pub damage: DamageSet,
    pub cursor_moved: bool,
    pub scrolled: bool,
    pub alternate_screen_changed: bool,
    pub title_changed: bool,
}

pub struct DamageSet {
    pub dirty_rows: SmallVec<[usize; 32]>,
    pub full_frame: bool,
}
```

Damage 规则：

- 写入字符所在行 dirty。
- 清行/清屏所在范围 dirty。
- scroll 时可以标记 full visible frame，也可以记录 scroll delta。
- 光标 old row 与 new row 都 dirty。
- selection 改变时 affected rows dirty。
- resize、font change、DPI change、theme change 必须 full frame。
- alternate screen 进出必须 full frame。

---

## 4. 长期不变量

| 不变量 | 说明 |
|---|---|
| 一个 `PaneId` 对应一个 `TerminalState` | terminal state 不能被 layout/workspace 复制 |
| 一个 live pane 最多一个 PTY session | 防止重复 reader、重复 resize、ghost process |
| terminal mutation 单线程化 | 后台可以读 PTY，但 grid mutation 必须在 owner 线程 |
| channel bounded | 高输出不能无限吃内存 |
| PTY bytes 不被 renderer 丢弃 | renderer 只能跳帧，不能破坏 terminal state |
| renderer 只读 snapshot/render input | renderer 不修改 terminal/layout/runtime 状态 |
| layout 是纯逻辑 | 输入 tree + surface rect，输出 pane rects |
| shell integration 是观察者 | 禁用后必须退回传统终端 |
| Agent 无权直接写 PTY | 任何命令执行必须走 review/policy |
| unsafe 默认禁止 | 只有通过 ADR 的局部 unsafe 可被允许 |

---

## 5. 阶段计划

## Phase 0：仓库基线与工程护栏

目标：建立可审查、可回滚、可测试的工程基线。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 0.1 单一 roadmap | 使用 `docs/roadmap.md` 作为唯一活跃路线图 | README 指向该文件；旧 plan 只做 redirect |
| 0.2 技术决策记录 | 新建 `docs/adr/` | renderer、runtime、terminal core 均有 ADR |
| 0.3 精简 workspace | 只保留 8 个核心 crate | 无 agent/storage/policy |
| 0.4 质量门禁 | fmt、clippy、test、deny unsafe 默认策略 | CI 全过 |
| 0.5 fixture 策略 | terminal、render、PTY smoke 都有 fixture 目录 | CLI 能跑 replay/smoke |
| 0.6 commit 纪律 | 小 PR，小 commit，避免功能混杂 | 每个 PR 有验收说明 |

验收命令：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

阻断项：

- 多份 roadmap 同时有效。
- agent/storage/policy 在 terminal MVP 前进入 workspace。
- renderer 选型没有 ADR。
- CI 不稳定但继续堆功能。

---

## Phase 1：Terminal Core MVP

目标：实现正确、可测试、可 replay 的 terminal state machine。

`noctrail-term` 只处理：

- bytes → terminal state；
- grid/cell/cursor/style；
- scrollback；
- selection/copy；
- alternate screen；
- terminal modes；
- damage tracking；
- immutable snapshot。

它不处理：

- workspace；
- panes；
- PTY；
- renderer；
- block history；
- agent；
- storage；
- shell integration。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 1.1 Grid/Cell | `Grid<Cell>`、cursor、wide char placeholder、dirty rows | ASCII/CJK/emoji/combining mark 单测 |
| 1.2 VT parser adapter | 使用 `vte`，实现 C0、ESC、CSI、SGR | ANSI 16/256/true color golden test |
| 1.3 Cursor/Erase | CUP/CUU/CUD/CUF/CUB、CR/LF/BS、EL/ED | wrap、clear、cursor movement fixture |
| 1.4 Scrollback | fixed limit scrollback、logical rows | 10k 行输出可 replay |
| 1.5 Alternate screen | enter/exit alternate screen | `nvim` 退出后 primary screen 恢复 |
| 1.6 Resize | resize grid、cursor clamp、wide char 修复 | 连续 resize 不 panic、不越界 |
| 1.7 Selection/copy | normal/line/block selection | CJK、wrapped line、CRLF/LF 策略测试 |
| 1.8 Damage | `advance` 返回 damage | cursor row 不漏，scroll/resize full damage |
| 1.9 Recording harness | `.ntrec` 输入 bytes + expected snapshot | 30+ fixture 通过 |

验收命令：

```bash
cargo test -p noctrail-term --all-targets
cargo run -p noctrail-cli -- replay tests/fixtures/terminal/*.ntrec
```

通过条件：

- 30+ terminal fixture 通过。
- 100k bytes 随机 printable/ANSI 输入不 panic。
- Unicode width 与 known limitation 有明确记录。
- `noctrail-term` 无 runtime/render/layout/agent/storage 依赖。
- `unsafe` 为 0。

阻断项：

- terminal core 存储 cwd/git/title/block/agent context。
- `nvim` alternate screen 污染 primary scrollback。
- resize 可导致越界或半个宽字符残留。
- selection copy 与 visible text 不一致。

---

## Phase 2：PTY 与 Pane Runtime

目标：可靠启动 shell、读写输入输出、处理 resize/close/backpressure。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 2.1 Shell resolver | Unix `$SHELL` fallback；Windows COMSPEC/pwsh/cmd/WSL 策略 | `doctor shell` 输出实际命令、cwd、env |
| 2.2 PTY session | Unix PTY + Windows ConPTY 统一 trait | 三平台能 spawn shell 并 echo |
| 2.3 Runtime handle | 每 pane 一组 reader/writer/control | 4 pane 独立读写 |
| 2.4 Bounded channel | output channel 有容量、watermark、drain budget | `yes` 不无限涨内存 |
| 2.5 Input writer | active pane 写入、paste、Ctrl-C、Ctrl-D | shell smoke 通过 |
| 2.6 Resize pipeline | layout rect → cells → PTY resize → terminal resize | `stty size` 或等价命令一致 |
| 2.7 Lifecycle | close、kill、restart、EOF、reader error | close pane 后无 child process 残留 |
| 2.8 Cross-platform smoke | echo/pwd/exit/stty 或 PowerShell equivalent | Windows/macOS/Linux smoke 通过 |

验收命令：

```bash
cargo test -p noctrail-pty -p noctrail-runtime --all-targets
cargo run -p noctrail-cli -- doctor
cargo run -p noctrail-cli -- pty-smoke
```

通过条件：

- 三平台均可启动 shell。
- 4 pane 同时运行命令，不串输入输出。
- 高输出 30 秒 UI 仍可关闭和输入。
- resize 后 shell rows/cols 与 UI 计算一致。
- 关闭 app 后无残留 shell process。

阻断项：

- PTY output 被随意丢弃。
- 高输出冻结 UI。
- pane close 留 zombie。
- resize 与 terminal grid 不一致。
- reader thread 不能被可靠停止。

---

## Phase 3：Renderer MVP

目标：建立真实可发展的 GPU 文本渲染主路径。

本阶段拆成两层：

1. `RenderPlan` / `RenderInput`：描述“画什么”。
2. `GpuRenderer`：实现“怎么画”。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 3.1 Render input | terminal snapshot + layout rect + damage + active state | renderer 不持有 mutable terminal |
| 3.2 wgpu surface | window surface、adapter、device、queue、surface config | GPU clear frame 可跑 |
| 3.3 Font system | monospace、fallback、CJK/emoji/Nerd Font | 不出现大量 tofu，fallback 有日志 |
| 3.4 Glyph path | glyph cache、atlas、DPI scale、subpixel strategy | 同一文本不重复 rasterize |
| 3.5 Cell painting | background、foreground、cursor、selection、underline | ANSI color screenshot 通过 |
| 3.6 Damage render | dirty rows/ranges 局部更新或重建策略 | frame time 不随 scrollback 线性增长 |
| 3.7 Screenshot tests | deterministic render fixtures | 20+ screenshot/glyph/layout fixture |
| 3.8 Fallback | GPU init 失败时 clear error 或 software fallback | 不 panic，不黑屏卡死 |

验收命令：

```bash
cargo test -p noctrail-render --all-targets
cargo run -p noctrail-cli -- render-fixtures
cargo run -p noctrail-app
```

通过条件：

- 默认路径为 GPU renderer。
- 80x24 ASCII terminal 可稳定渲染。
- 16/256/true color 可见正确。
- CJK/emoji/Nerd Font 至少有 fallback 策略。
- GPU backend 失败时可诊断。
- renderer 不依赖 agent/storage/shell integration。

阻断项：

- 长期默认 software presenter。
- redraw 每帧重建全 scrollback。
- renderer 修改 terminal/layout/runtime。
- GPU init 失败导致 app 无法进入 safe mode。
- DPI 改变后 glyph/cursor/border 明显错位。

---

## Phase 4：单窗口单 Pane 可用终端

目标：第一个真正可日常使用的 GUI terminal。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 4.1 winit app loop | window create、surface init、event loop | window 能显示 shell |
| 4.2 Output pump | PTY output → TerminalState → Damage → render | shell 输出可见 |
| 4.3 Input routing | key event → PTY bytes | bash/zsh/pwsh 基础输入正常 |
| 4.4 Clipboard | selection copy、paste、bracketed paste | 多行 paste 不误执行 |
| 4.5 IME | composition/preedit/candidate area | 中文输入基础可用 |
| 4.6 Mouse | selection、scroll、terminal mouse mode | `nvim`/`fzf` mouse mode 不冲突 |
| 4.7 Frame scheduling | output/input/resize/cursor blink request redraw | idle CPU < 3% |
| 4.8 Safe startup | broken config/GPU fallback | safe mode 可启动 |

手动 smoke：

```bash
echo hello
printf '\e[31mred\e[0m\n'
seq 1 10000
nvim
tmux
fzf
```

通过条件：

- 三平台至少各一个环境能启动 GUI。
- 单 pane shell 可日常输入输出。
- copy/paste/resize/scrollback/IME 基础可用。
- 连续运行 1 小时无 crash。
- 此阶段仍不做 Agent。

阻断项：

- 只能更新 title，不能真实绘制 terminal。
- PTY output 没接进 app redraw。
- 输入路径依赖单 pane 结构，后续难以扩展。
- 空闲 CPU 明显偏高。
- 配置损坏导致无法启动。

---

## Phase 5：平铺与 Workspace Core

目标：从“单窗口终端”变成“平铺式终端工作台”。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 5.1 Pane registry | 每个 pane 有 terminal state、runtime、layout leaf | split 出的新 pane 有独立 shell |
| 5.2 Dwindle layout | BSP-like tree，按父 rect 决定 split axis | 1/2/3/4/8 pane 布局稳定 |
| 5.3 Focus model | active pane、focus ring、keyboard focus | Alt+H/J/K/L 或配置键切换 |
| 5.4 Resize/move | split ratio、swap pane、close pane | resize 后每个 PTY 收到正确 cells |
| 5.5 Workspace 1-9 | 每 workspace 有独立 layout/session set | 切换 workspace 不杀进程 |
| 5.6 Scratch pane | special workspace / dropdown pane | show/hide 不影响主 layout |
| 5.7 Command palette | 核心动作可搜索执行 | keyboard-only 完成常用操作 |
| 5.8 Layout tests | layout 不依赖 winit/PTY/render | 单元测试覆盖 split/close/focus |

必须支持的键盘操作：

- new pane；
- split horizontal；
- split vertical；
- focus left/right/up/down；
- resize split；
- move/swap pane；
- close pane；
- workspace 1..9；
- scratch show/hide；
- command palette。

通过条件：

- 8 pane 同时运行 shell，不串输入输出。
- workspace 切换保留 process/session。
- active pane resize 后 full-screen app 能重绘。
- layout 是纯逻辑，可单元测试。
- 用户可以只用键盘完成日常工作。

阻断项：

- app state 仍是单 pane。
- layout tree 和 runtime registry 不一致。
- 关闭 pane 后 process 不回收。
- workspace 切换杀掉 shell。
- focus 丢失或输入串线。

---

## Phase 6：视觉系统

目标：形成 Noctrail 的视觉识别，但不牺牲稳定性和可读性。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 6.1 Theme TOML | color、font、border、opacity、cursor、selection | 热加载生效 |
| 6.2 Active border | active/inactive border | 一眼可辨，高对比可用 |
| 6.3 Pane gaps | radius、padding、gap | 高 DPI 下不错位 |
| 6.4 Transparency | window opacity，可关闭 | 不支持平台自动降级 |
| 6.5 Blur fallback | blur 支持则启用，否则 tinted solid | 不影响可读性 |
| 6.6 Animation | pane/workspace transition | 可全局关闭 |
| 6.7 Status line | cwd、shell、git branch、exit status 基础显示 | 不解析/替换 prompt |
| 6.8 Low-power mode | 禁用 blur/glow/animation | 输入延迟不恶化 |

通过条件：

- 默认主题清晰、稳定、不过度炫技。
- 所有效果可关闭。
- 低端设备关闭效果后仍低延迟。
- 视觉层不改变 terminal output。

阻断项：

- blur/opacity 导致文本不可读。
- 动画影响输入延迟。
- status line 误解析 shell prompt。
- theme 错误导致 app 无法启动。

---

## Phase 7：兼容性硬化

目标：达到日常开发可用级别。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 7.1 TUI matrix | `nvim`、`tmux`、`fzf`、`less`、`top/htop`、`ssh` | alt screen、mouse、resize、color 正常 |
| 7.2 Shell matrix | bash、zsh、fish、pwsh、nu、cmd、WSL | 启动、输入、退出、cwd 显示正常 |
| 7.3 Prompt matrix | starship、oh-my-zsh、powerlevel10k | prompt 不错位，不吞 escape |
| 7.4 Unicode/IME | CJK、emoji、combining、fullwidth | 输入、选择、复制、cursor 基本正确 |
| 7.5 Performance | high output、scrollback、multi-pane、idle | idle CPU < 2-3%；input p95 < 30ms |
| 7.6 Soak test | 长时间运行、重复 split/close/resize | 8 小时内存增长 < 20% |
| 7.7 Crash recovery | panic hook、last diagnostic | crash 不损坏配置，不泄露 secret |

阻断项：

- `nvim` 或 `tmux` 主流程不可用。
- Windows ConPTY resize 经常失败。
- 高输出冻结 UI。
- 关闭 pane 留 zombie。
- resize 后 TUI 错乱。
- copy/paste 泄露 OSC 或不可见控制内容。
- 配置错误无法 safe mode 启动。

---

## Phase 8：Shell Integration 与 Command Blocks

目标：在稳定终端之上引入 Warp-like block，而不是让 block 破坏传统终端。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 8.1 OSC protocol | Noctrail OSC marker：prompt、command start/end、cwd、exit code | marker 不显示为可见字符 |
| 8.2 Shell hooks | bash/zsh/fish/pwsh/nu hooks | 捕获 command、cwd、exit、duration |
| 8.3 Block model | block 是 terminal event 上层观察者 | 禁用后终端完全可用 |
| 8.4 Block UI | copy command/output、jump、fold | 最近 100 个 block 可查看 |
| 8.5 Prompt compatibility | 不替换 Starship/prompt | starship + shell matrix 通过 |
| 8.6 Structured output P1 | JSON/CSV/TOML detection/copy | lens 不改变 stdout |
| 8.7 Failure block | 非 0 exit code 高亮 | 不自动调用 Agent |

通过条件：

- shell integration 可一键启用/禁用。
- 禁用后退回普通终端，无功能损坏。
- block 捕获错误不会吞输出。
- `nvim`/`tmux`/`ssh` 中不误生成 command block。

阻断项：

- block 信息写入 terminal core。
- shell hook 失败影响 shell 使用。
- block 捕获吞掉 stdout/stderr。
- prompt 被替换或破坏。

---

## Phase 9：Agent、安全与 Review

目标：可选 AI，不影响终端。Agent 永远不拥有 shell。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 9.1 默认关闭 | 默认不联网、不读 env/history | 默认配置无 provider 请求 |
| 9.2 Context collector | 只读 current block、selection、cwd、explicit files | UI 显示 Agent 将看到什么 |
| 9.3 Redaction | token、SSH key、JWT、云厂商 key | secret corpus 通过 |
| 9.4 Provider adapter | OpenAI-compatible/local/CLI agent | provider 失败不影响 terminal |
| 9.5 Command proposal | Agent 只能建议命令 | 包含 reason、risk、permission |
| 9.6 Review panel | 用户确认后才写入 shell | high/critical 风险有强确认 |
| 9.7 Patch preview | 生成 diff，不直接写文件 | 应用前必须显示 diff |
| 9.8 Audit ledger | context/read/suggest/review/execute 记录 | 用户可查看 Agent 做了什么 |

阻断项：

- Agent 可静默执行命令。
- Critical command 可自动执行。
- 关闭 Agent 后仍联网。
- provider error 影响终端输入输出。
- audit 缺失执行记录。
- secret redaction 在日志或 provider request 中失效。

---

## Phase 10：跨平台 Beta

目标：公开测试，但只在核心体验成熟后进入。

| 子项 | 内容 | 验收标准 |
|---|---|---|
| 10.1 Installer | Windows installer、macOS dmg、Linux AppImage/deb/rpm | 安装/升级/卸载 smoke |
| 10.2 CI matrix | Windows/macOS/Linux build/test | 三平台 CI 通过 |
| 10.3 Config reference | 完整 TOML schema 与示例 | 用户可改 theme/keymap/layout |
| 10.4 Diagnostics | `noctrail doctor` | 检测 shell、PTY、GPU、font、config、permissions |
| 10.5 Release blockers | P0/P1 blocker 列表 | blocker 未清零不得 beta |
| 10.6 Privacy/security docs | Agent、storage、telemetry、logs | 默认无 telemetry；关闭 Agent 不联网 |

Beta 阻断项：

- 任一 P0 平台无法启动。
- 单 pane shell 输入输出不稳定。
- `nvim`、`tmux`、`fzf`、`ssh` 任一主流程严重不可用。
- 高输出冻结 UI。
- resize 后 TUI 显示错乱。
- 多 pane 输入输出串线。
- workspace 切换杀掉进程。
- 关闭 pane 后残留 child process。
- 配置损坏导致无法 safe mode 启动。
- 关闭 Agent 后仍联网。
- Agent 可绕过 review 执行命令。

---

## 6. 前 12 个 PR 建议

| PR | 内容 | 不允许包含 |
|---|---|---|
| PR 1 | `docs/roadmap.md` 与旧 plan redirect | 代码实现 |
| PR 2 | `docs/adr/0001-crate-boundaries.md` | feature implementation |
| PR 3 | `docs/adr/0002-renderer-wgpu-text-stack.md` | renderer code |
| PR 4 | workspace skeleton cleanup | agent/storage/policy |
| PR 5 | `noctrail-term` grid/cell/cursor cleanup | PTY/render |
| PR 6 | `noctrail-term` SGR/CSI fixture expansion | block/shell integration |
| PR 7 | terminal recording fixture harness hardening | GUI |
| PR 8 | `noctrail-pty` lifecycle tests | app runtime |
| PR 9 | `noctrail-runtime` event model + bounded output | renderer/layout |
| PR 10 | `noctrail-render` RenderInput + Damage API | wgpu surface |
| PR 11 | `noctrail-render` minimal wgpu clear frame | glyph complexity |
| PR 12 | ASCII glyph path + screenshot fixture | animation/blur |

---

## 7. 文档拆分建议

```text
docs/
  roadmap.md
  rendering-architecture-and-selection.md
  config-reference.md
  privacy-security.md
  release-blockers.md
  rendering-ecosystem-notes.md
  adr/
    0001-crate-boundaries.md
    0002-renderer-wgpu-text-stack.md
    0003-terminal-core-first.md
    0004-runtime-event-model.md
    0005-agent-deferred-review-boundary.md
  specs/
    terminal-state.md
    terminal-sequences.md
    runtime-events.md
    render-input.md
    layout-tree.md
    shell-integration-osc.md
  test-matrix/
    terminal-fixtures.md
    render-fixtures.md
    tui-compatibility.md
    platform-smoke.md
```

原则：

- `roadmap.md` 写“做什么、顺序、验收、阻断项”。
- `rendering-architecture-and-selection.md` 写当前可执行的渲染边界。
- `adr/` 写“为什么这么选”。
- `specs/` 写“具体协议和数据结构”。
- `test-matrix/` 写“如何验证”。

---

## 8. 当前分支可借鉴与不建议继承

### 8.1 可借鉴

| 内容 | 原因 |
|---|---|
| terminal recording fixture 思路 | terminal 正确性必须靠 replay/golden |
| `LayoutTree` 纯逻辑方向 | 方向正确，应接入真实 runtime registry |
| `RenderPlan` 边界思路 | “画什么”和“怎么画”分离正确 |
| PTY session 封装 | 可以作为底层 session 起点 |
| config 热加载思路 | Phase 6 可继续扩展 |
| shell OSC marker 思路 | 后置到 Phase 8 |

### 8.2 不建议直接继承

| 内容 | 原因 |
|---|---|
| 单 pane `DesktopApp` 长期结构 | 会阻碍 tiling/workspace |
| software-first renderer 主路径 | 与 GPU terminal 目标不匹配 |
| terminal core + block/agent 耦合 | 职责过宽 |
| 过早 agent/storage/policy | 拖慢 terminal MVP |
| 多份 roadmap | 制造路线冲突 |
| PTY bytes 丢弃式 backpressure | 会破坏终端正确性 |

---

## 9. 性能预算

早期不需要追求极限，但需要明确目标。

| 场景 | 目标 |
|---|---|
| Idle single pane | CPU < 2-3% |
| Idle 8 pane | CPU < 5% |
| Input latency | p95 < 30ms |
| Cursor blink | 不触发全量 expensive layout |
| 80x24 ASCII render | 60 FPS 目标 |
| 8 pane high output | UI 可输入、可关闭、可切 focus |
| Scrollback 10k lines | 不参与每帧全量 render |
| Resize drag | 不 crash，不积压无限 resize event |
| Memory growth soak | 8 小时 < 20% 增长 |

---

## 10. 最终路线一句话

Noctrail 应先成为一个可靠的 terminal emulator，再成为平铺式终端工作台，最后再成为带 Blocks 和 Agent 的高级开发环境。

不要从“AI 终端”开始。  
不要从“炫酷 UI”开始。  
从 **正确的 terminal state machine + 可靠 PTY runtime + 真实 GPU text renderer + 可测试平铺布局** 开始。
