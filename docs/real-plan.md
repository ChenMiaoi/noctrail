# Noctrail 重启计划

本文用于记录从当前实验分支回到干净基线后，重新推进 Noctrail 的工程计划。

核心判断：不要继续在 `codex/noctrail-runtime-polish` 上补丁式修复。该分支已经偏成“功能堆叠型原型”，而不是“可稳定成长为成熟终端的工程基线”。

建议路线：

1. `main` 回退到 `e0f3712207ec8d8799b6a0548008251484a2c650`。
2. 删除所有 `codex/*` 实验分支。
3. 从 `e0f3712` 新开 `rewrite/terminal-core` 或 `rewrite/noctrail-v0`。
4. 只保留当前分支里的经验、测试想法、少量可迁移 fixture。
5. 不继承当前 `app`、`runtime`、`agent`、`storage` 的实现结构。
6. 先做一个稳定、漂亮、可日常用的平铺式终端；Agent、Block、Structured Lens 后置。

## 1. 当前分支问题判断

### 1.1 不是方向错，是工程顺序错

`docs/plan` 的总体方向并不差。计划文档已经多次强调：Noctrail 不是 compositor，不应该复制 Hyprland 的系统职责，而应该吸收动态平铺、workspace、视觉反馈、热配置、键盘优先体验。

问题在于当前分支同时推进了：

- terminal core
- renderer
- app runtime
- shell integration
- agent
- policy
- storage
- block history
- structured output
- config schema
- packaging
- CI/release
- fonts/assets

这会导致每层都不够稳定，却已经互相绑定。当前 workspace 已经拆成十多个 crate，包括 `agent`、`policy`、`storage`、`terminal-core`、`renderer`、`shell-integration`、`ui`、`app`、`compat` 等。这个规模对一个尚未稳定跑通“单窗口单 shell”的项目来说过早。

### 1.2 UI 有 layout tree，但 app 仍是单 pane runtime

`noctrail-ui` 里已经有 `LayoutTree`、`LayoutNode::Leaf/Split`、Dwindle、Horizontal/Vertical split、pane close/focus/resize 等纯逻辑。

但 `noctrail-app` 的真实运行状态仍然是：

- 一个 `TerminalPane`
- 一个 `Option<PaneRuntime>`
- 一个 active pane 的 render plan

`DesktopApp` 字段里是单 pane: `TerminalPane` 和单 pane runtime: `Option<PaneRuntime>`。渲染也是直接从 `self.pane.terminal().grid()` 生成一个 render plan，而不是按 workspace/layout tree 渲染多个 pane。

这就是“看起来有 tiling 状态，实际没有 tiling 终端体验”的核心原因。

### 1.3 Renderer 方向与目标不匹配

当前 renderer 有 `Gpu` / `Software` 选择模型，但实际 conservative capability 是 `gpu: false, software: true`。`app` 的 redraw 里遇到 `Gpu` 分支会直接返回 `GpuUnavailable`，真实路径是 software presenter。

这对“Hyprland 风格、视觉效果优先”的目标不够。不是说必须第一天就完成高级 GPU renderer，而是不能把“GPU-oriented”写进计划，却让主路径长期停在软件 presenter 上。

### 1.4 Terminal core 过早混入 block/agent 上下文

`TerminalGrid` 目前同时承担 grid、scrollback、selection、cursor、modes、current cwd/shell/git、title、hyperlink、command block events、completed command blocks、alternate screen 等状态。

终端核心应该先成为一个纯 terminal emulator state machine。Block、shell integration、agent context 应该作为上层观察者或 event consumer，而不是让 terminal grid 一开始就背负完整产品语义。

### 1.5 PTY/runtime 初步可用，但多 pane 后会放大成本

当前 runtime 是每个 pane 一套 reader/input/control thread 加 bounded channel/backpressure。对单 pane 原型可以接受，但如果平铺终端一上来支持 8、16、32 个 pane，就必须明确资源模型，否则会变成线程、channel、resize、close、event drain 的长期债务。

## 2. Git 重启方案

目标：让 `main` 回到干净的 bootstrap commit，删除所有 `codex/*` 实验分支，从新分支重新开始。

`e0f3712207ec8d8799b6a0548008251484a2c650` 是合理基线。该提交建立了初始 Rust workspace、minimal CLI、LGPL license、stable toolchain、rustfmt/clippy、CI，以及产品计划和治理文件。

### 2.1 建议先临时归档，再删除分支

```bash
git fetch origin --prune

# 可选：给当前最新 codex 分支打一个只读归档 tag
git tag archive/noctrail-runtime-polish-20260517 origin/codex/noctrail-runtime-polish
git push origin archive/noctrail-runtime-polish-20260517
```

归档 tag 不是继续开发用，只是防止删除分支后无法追溯已有实验。

### 2.2 回退 main

```bash
git checkout main
git pull --ff-only origin main

git reset --hard e0f3712207ec8d8799b6a0548008251484a2c650

git push --force-with-lease origin main
```

若 GitHub 拒绝 force push，通常是 `main` 有保护规则。处理方式是：临时允许管理员 force push，完成后立刻恢复保护。

### 2.3 删除 codex 分支

先列出：

```bash
git branch -r --list 'origin/codex/*'
```

再逐个删除：

```bash
git push origin --delete codex/noctrail-runtime-polish
git push origin --delete codex/<另一个-codex-分支名>
```

### 2.4 开新分支

```bash
git checkout -b rewrite/terminal-core
```

建议不要叫 `codex/*`，避免继续把实验分支当主线。

## 3. 新产品边界

### 3.1 Noctrail 应该是什么

Noctrail 是一个 Rust 写的、跨平台、GPU 加速、Hyprland 风格的平铺式 GUI 终端。

不是：

- 不是 shell。
- 不是 Hyprland clone。
- 不是 Wayland compositor。
- 不是系统环境。
- 不是一开始就带完整 AI agent 的 IDE。

Hyprland 的官方定位是 modern compositor，强调 Wayland features、dynamic tiling、eyecandy、plugins、lightweight/responsive。Noctrail 应该吸收它的空间感、动态平铺、视觉反馈、配置文化，而不是复制 compositor 职责。

Alacritty 给 Noctrail 的启发是：终端本体必须快、跨平台、少而稳定；Alacritty 是 fast、cross-platform、OpenGL terminal emulator，并支持 BSD/Linux/macOS/Windows。

Warp 的 Blocks 可以后期借鉴，但只能在终端稳定之后做。Warp 文档把 Block 定义为 command 和 output 的原子单元，可复制命令、复制输出、跳转、重新输入、分享、收藏。

Starship 与 Nushell 的借鉴也应克制：Starship 是模块化 prompt 配置生态，Noctrail 应兼容和读取上下文，而不是替代 prompt；Nushell 的结构化数据思想适合后期做 Structured Lens。

### 3.2 第一性原则

| 原则 | 含义 |
|---|---|
| 终端正确性优先 | `nvim`、`tmux`、`fzf`、`ssh`、`nu`、`pwsh`、`starship` 必须可靠 |
| 平铺是产品核心 | workspace、pane、scratch、focus、resize 是核心，不是附加功能 |
| 视觉效果必须可降级 | blur、透明、glow、动画不可用时不能影响可读性 |
| 状态机必须小 | terminal、PTY、renderer、layout、config 各自维护自己的不变量 |
| Agent 后置 | Agent 不能影响 terminal MVP，默认关闭 |
| 跨平台从第一天做 | Windows/macOS/Linux 都是 P0，不接受“Linux 先糊出来再说” |
| 每个阶段都有验收 | 未通过当前阶段，不进入下一阶段 |

## 4. 从零开始的架构

### 4.1 最小 crate 结构

重启后不要一开始就恢复十几个 crate。先控制在 7 个左右。

| Crate | 职责 | 明确不做 |
|---|---|---|
| `noctrail-cli` | `doctor`、`replay`、`theme check`、启动入口 | 不做 agent、storage、复杂 ctl |
| `noctrail-config` | TOML 配置、schema、默认值、热加载校验 | 不读 shell history、不读 secret |
| `noctrail-term` | VT parser adapter、grid、scrollback、selection、modes、damage | 不知道 workspace、agent、block history |
| `noctrail-pty` | Unix PTY、Windows ConPTY、process lifecycle、resize | 不知道 UI、layout、renderer |
| `noctrail-runtime` | Pane runtime registry、bounded channels、event routing、backpressure | 不做渲染、不解析 VT |
| `noctrail-layout` | workspace、tab、pane tree、Dwindle/Master/Monocle 纯逻辑 | 不持有 PTY 或 terminal grid |
| `noctrail-render` | GPU text rendering、glyph atlas、surface、fallback、screenshot tests | 不处理 shell/input |
| `noctrail-app` | winit event loop、window、input routing、frame scheduling、platform glue | 不塞业务逻辑进单体状态 |

后期再加：

| Crate | 开始条件 |
|---|---|
| `noctrail-shell-integration` | terminal MVP + multi-pane 稳定后 |
| `noctrail-blocks` | shell integration 真实通过后 |
| `noctrail-policy` | Agent 或 command review 进入路线时 |
| `noctrail-agent` | Block、policy、review、audit 全部稳定后 |
| `noctrail-storage` | block history/audit 需要持久化时 |

### 4.2 核心数据流

```text
Window/Input
  -> KeyTranslator
  -> RuntimeCommand::WriteToPane(PaneId, bytes)
  -> PtyWriter

PTY output
  -> RuntimeEvent::Bytes(PaneId, bytes)
  -> TerminalState::advance(bytes)
  -> DamageSet
  -> RenderPlan

LayoutState
  -> PaneId -> Rect
  -> RenderPlan per pane
  -> Compositor-style frame composition inside one app window
```

### 4.3 必须长期维持的不变量

| 不变量 | 说明 |
|---|---|
| 一个 `PaneId` 对应一个 terminal state | terminal state 不能被 workspace/tab 复制 |
| 一个 live pane 最多一个 PTY session | 防止 ghost process、重复 reader、重复 resize |
| terminal mutation 单线程化 | PTY reader 可以后台读，但 grid 修改必须在 app/runtime owner 线程 |
| channel bounded | 高输出不能无限吃内存 |
| renderer 只读 snapshot | renderer 不修改 terminal/layout 状态 |
| layout 是纯函数 | 输入 workspace tree + surface rect，输出 pane rects |
| Agent 无权直接写 PTY | 任何 command execution 都必须走 review/policy |

## 5. 详细重启计划

下面按阶段推进。每个阶段都有子计划与验收标准。

### Phase 0：仓库重置与工程护栏

目标：建立干净、可审查、可回滚的工程基线。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 0.1 回退主线 | `main` reset 到 `e0f3712`，删除 `codex/*` 分支 | GitHub `main` HEAD 等于 `e0f3712207ec8d8799b6a0548008251484a2c650`；`git branch -r --list 'origin/codex/*'` 无结果 |
| 0.2 建立新分支 | 从 `main` 创建 `rewrite/terminal-core` | 新分支无历史实验代码；首个 PR 只改文档和 workspace skeleton |
| 0.3 精简 workspace | 只创建 cli/config/term/pty/runtime/layout/render/app | `cargo metadata` 能正确列出 crate；没有 agent/storage/policy |
| 0.4 统一质量门禁 | fmt、clippy、test、deny unsafe 默认策略 | fmt、clippy、test 全过 |
| 0.5 Commit 纪律 | 每个 commit 小而完整 | 单个 commit 不混文档、重构、功能；每个 PR 有验收说明 |
| 0.6 计划文档重写 | 用一份 `docs/plan/rewrite-roadmap.md` 替代多份发散计划 | 文档包含阶段、子计划、验收、阻断项；不再同时维护多份相互重复 roadmap |

验收命令：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

### Phase 1：Terminal Core MVP

目标：先写一个正确、可测试、可 replay 的 terminal state machine。

`noctrail-term` 只处理：

- bytes -> terminal state
- terminal state -> immutable snapshot
- selection/copy
- scrollback
- damage tracking
- terminal modes

它不处理：

- workspace
- panes
- block history
- agent
- storage
- theme animation

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 1.1 Grid/Cell 模型 | `Grid<Cell>`、cursor、style、wide char placeholder、dirty line | 单元测试覆盖 narrow/wide/CJK/emoji/combining mark；越界写入不 panic |
| 1.2 VT parser adapter | 使用 `vte`，实现基础 printable、C0、ESC、CSI、SGR | ANSI 16 色、256 色、true color、bold/italic/underline/reset golden test 通过 |
| 1.3 光标与 erase | CUP/CUU/CUD/CUF/CUB、CR/LF/BS、EL/ED | 光标移动、清行、清屏、wrap 行为通过 recording test |
| 1.4 Scrollback | fixed limit scrollback、logical row、copy visible/scrollback | 10k 行输出可滚动；清 scrollback 不影响当前屏 |
| 1.5 Alternate screen | 进入/退出 alternate screen，保存 primary screen | `nvim` 进入退出后 primary screen 恢复；alternate screen 不污染 scrollback |
| 1.6 Resize | grid resize、cursor clamp、wide char 修正 | resize 20 次后无 panic、无越界、全屏程序能重绘 |
| 1.7 Selection/copy | normal/line/block selection，LF/CRLF 输出 | 跨行选择、CJK 选择、末尾空格裁剪策略测试通过 |
| 1.8 Damage tracking | 每次 `advance` 输出 dirty rows/ranges | 高输出时 renderer 可只处理 dirty lines；damage 不漏当前 cursor row |
| 1.9 Recording harness | `.ntrec` 输入 bytes + expected grid snapshot | 至少 30 个 terminal fixture；`noctrail replay fixtures/*.ntrec` 全过 |

验收命令：

```bash
cargo test -p noctrail-term --all-targets
cargo run -p noctrail-cli -- replay tests/fixtures/terminal/*.ntrec
```

通过条件：

- 30+ recording fixture 通过。
- 100k bytes 随机 printable/ANSI 输入不 panic。
- Unicode width 行为有明确测试。
- Terminal core 无 agent/block/storage/config 依赖。
- `unsafe` 为 0。

### Phase 2：PTY 与 Pane Runtime

目标：可靠启动 shell、读写输入输出、处理 resize/close/backpressure。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 2.1 Platform shell resolver | Windows: pwsh/cmd/WSL；Unix: `$SHELL` fallback | `noctrail doctor shell` 能显示实际 shell、args、cwd |
| 2.2 PTY session | Unix PTY + Windows ConPTY，经统一 trait 暴露 | Windows/macOS/Linux 各能 spawn shell 并 echo 文本 |
| 2.3 Runtime registry | `HashMap<PaneId, PaneRuntime>` 管理多 pane | 创建 4 个 pane，各自 shell 独立；关闭一个不影响其他 |
| 2.4 Bounded event channel | output channel 有高/低水位与丢弃策略说明 | `yes`/大量输出时 UI 不冻结，内存不无限涨 |
| 2.5 Input writer | active pane 写入、paste、Ctrl-C、Ctrl-D | shell 输入、Ctrl-C 中断、paste 多行通过 |
| 2.6 Resize pipeline | layout rect -> cell size -> PTY resize -> terminal resize | 连续拖动窗口 resize 不 crash；shell 收到正确 rows/cols |
| 2.7 Lifecycle | close、kill、restart、EOF、reader error | 关闭 pane 后 child process 被回收；无 zombie |
| 2.8 Cross-platform smoke | `echo`、`pwd`、`exit`、`stty size` / PowerShell equivalent | 三平台基础 smoke 全过 |

验收命令：

```bash
cargo test -p noctrail-pty -p noctrail-runtime --all-targets
cargo run -p noctrail-cli -- doctor
cargo run -p noctrail-cli -- pty-smoke
```

通过条件：

- Windows/macOS/Linux 均可启动 shell。
- 4 pane 同时运行命令，不串输出。
- `yes` 运行 30 秒后应用仍可响应输入和关闭。
- resize 后 shell 中显示的 rows/cols 与 UI 计算一致。
- 关闭 app 后无残留 shell process。

### Phase 3：Renderer MVP

目标：建立真实可发展的 GPU 文本渲染路径。视觉效果先少，但路径必须对。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 3.1 Render snapshot | terminal snapshot + layout rect -> render plan | render crate 不持有 terminal mutable reference |
| 3.2 Font system | monospace 字体、fallback、CJK/emoji 基础支持 | CJK、emoji、Nerd Font symbol 不显示为空方块，fallback 有日志 |
| 3.3 GPU backend | `wgpu` 或等价 GPU path；software 仅 debug/fallback | 默认 backend 是 GPU；GPU 不可用时清晰 fallback |
| 3.4 Glyph atlas | glyph cache、atlas eviction、DPI scale | 同一文本重复渲染不重复 rasterize；DPI 125/150/200% 正常 |
| 3.5 Cell background | SGR foreground/background、selection、cursor | 16 色、256 色、true color、selection、cursor golden screenshot 通过 |
| 3.6 Damage render | dirty rows/ranges 局部更新策略 | 高输出时 frame time 不随 scrollback 线性增长 |
| 3.7 Screenshot tests | deterministic software/golden path | 至少 20 个 screenshot/glyph/layout fixture 通过 |

验收命令：

```bash
cargo test -p noctrail-render --all-targets
cargo run -p noctrail-cli -- render-fixtures
```

通过条件：

- 80x24 基础终端可 60 FPS 目标渲染。
- 10k 行 scrollback 不参与每帧全量布局。
- CJK/emoji/ASCII/ANSI color snapshot 通过。
- GPU backend 失败时应用不崩溃，有清晰错误或 fallback。
- renderer 不依赖 agent/storage/shell integration。

### Phase 4：单窗口单 Pane 可用终端

目标：做出第一个真正可用的 GUI terminal。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 4.1 winit app loop | window create、surface init、event loop、frame scheduler | app 可启动窗口并显示 shell |
| 4.2 Input routing | key event -> PTY bytes，支持普通键、Ctrl/Alt、功能键 | bash/zsh/pwsh 基础输入正常 |
| 4.3 Clipboard | copy selection、paste、bracketed paste | bracketed paste 开启时多行粘贴不误执行 |
| 4.4 IME | composition/preedit/candidate area | 中文输入基础可用，不破坏 cursor |
| 4.5 Mouse | selection、scroll、terminal mouse mode | 普通滚动与 `nvim`/`fzf` mouse mode 不冲突 |
| 4.6 Frame scheduling | PTY output、input、resize、cursor blink 都能 request redraw | 空闲 CPU 低于 3%；高输出不卡死 |
| 4.7 Safe startup | config missing/broken fallback | 配置损坏时进入 safe mode，窗口仍能打开 |

手动 smoke：

```bash
echo hello
printf '\e[31mred\e[0m\n'
seq 1 10000
```

验收命令：

```bash
cargo run -p noctrail-app
```

通过条件：

- 三平台至少各一个环境能启动 GUI。
- 单 pane 中 bash/zsh/pwsh/nu 至少一种 shell 可用。
- copy/paste/resize/scrollback/IME 基础可用。
- 连续运行 1 小时无 crash。
- 此阶段仍不做 Agent。

### Phase 5：Hyprland 风格平铺核心

目标：从“终端窗口”变成“平铺式终端工作台”。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 5.1 Pane registry | 每个 pane 有 terminal state、runtime、layout leaf | split 出的新 pane 有独立 shell 和 scrollback |
| 5.2 Dwindle layout | 类 BSP tree，按父 rect 宽高决定 split axis | 1/2/3/4/8 pane 布局稳定；关闭 pane 后树正确收缩 |
| 5.3 Focus model | active pane、focus ring、keyboard focus | Alt+H/J/K/L 或配置键可切 pane；焦点不丢 |
| 5.4 Resize/move | 调整 split ratio、swap pane、close pane | resize 后所有 PTY 都收到正确 cell size |
| 5.5 Workspace 1-9 | 每个 workspace 有独立 layout/session set | 切换 workspace 后 pane/runtime 状态保留 |
| 5.6 Tabs 后置或简化 | 初期可以 workspace + pane，不急做复杂 tab | 不允许 tabs 和 workspace 混出两套重复模型 |
| 5.7 Scratch pane | 类 special workspace 的临时浮动/下拉 pane | scratch 可打开/隐藏，不影响主 layout |
| 5.8 Command palette | 搜索并执行核心动作 | keyboard-only 可完成 split/focus/close/workspace |

核心操作必须全键盘完成：

- new pane
- split horizontal
- split vertical
- focus left/right/up/down
- resize split
- move pane
- close pane
- workspace 1..9
- scratch show/hide

通过条件：

- 8 pane 同时运行 shell，不串输入输出。
- workspace 切换不杀进程。
- active pane resize 后 full-screen app 能重绘。
- layout 是纯逻辑，可单元测试，不依赖 winit/PTY/render。
- 用户可以只用键盘完成日常工作。

### Phase 6：视觉系统与效果

目标：形成 Noctrail 的 Hyprland 风格识别，但不牺牲稳定性。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 6.1 Theme TOML | color、font、border、opacity、cursor、selection | 修改 theme 文件后热加载生效 |
| 6.2 Active border | active pane gradient/solid border | active/inactive pane 一眼可辨；高对比模式可用 |
| 6.3 Rounded panes | pane border radius、padding、gap | DPI 125/150/200% 不明显错位 |
| 6.4 Transparency | window opacity，可关闭 | 不支持透明的平台自动降级为纯色 |
| 6.5 Blur fallback | blur 支持则启用，不支持则 tinted solid | blur 不可用不影响可读性 |
| 6.6 Animation | workspace/pane transition，默认短动画 | 动画可全局关闭；低性能模式禁用 glow/blur |
| 6.7 Status line | cwd、shell、git branch、exit status 基础显示 | 不解析 Starship prompt，不破坏 prompt 输出 |

通过条件：

- 默认主题“好看但不刺眼”。
- 透明、blur、glow、动画全都可关闭。
- 在低端设备上关闭效果后输入延迟不恶化。
- 高 DPI 下 border、cursor、glyph 对齐。
- 视觉层不改变 terminal output。

### Phase 7：兼容性硬化

目标：让它成为可以日常开发使用的终端。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 7.1 TUI matrix | `nvim`、`tmux`、`fzf`、`less`、`top`/`htop`、`ssh` | 进入/退出、鼠标、resize、颜色、alt screen 正常 |
| 7.2 Shell matrix | bash、zsh、fish、pwsh、nu、cmd、WSL | 启动、输入、退出、cwd 显示正常 |
| 7.3 Prompt matrix | starship、oh-my-zsh、powerlevel10k 基础 | prompt 不错位，不误吞 escape |
| 7.4 Unicode/IME | CJK、emoji、combining marks、fullwidth | 输入、选择、复制、光标位置基本正确 |
| 7.5 Performance | 高输出、scrollback、multi-pane、idle | idle CPU < 2-3%；输入延迟 p95 < 30ms |
| 7.6 Soak test | 长时间运行、重复 split/close/resize | 8 小时运行内存增长 < 20%；无 zombie process |
| 7.7 Crash recovery | panic hook、last session diagnostic | crash 后重启不损坏配置，不泄露 secret |

阻断项：

- `nvim` 或 `tmux` 主流程不可用。
- Windows ConPTY resize 经常失败。
- 关闭 pane 留 zombie。
- 高输出冻结 UI。
- 配置错误导致无法恢复启动。
- copy/paste 泄露不应复制的 OSC 内容。

### Phase 8：Command Blocks 与 Shell Integration

目标：在稳定终端之上引入 Warp-like block，而不是让 block 破坏传统终端。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 8.1 OSC protocol | 定义 Noctrail OSC marker：prompt start、command start/end、cwd、exit code | marker 不显示为可见字符；未知 marker 被安全忽略 |
| 8.2 Shell hooks | bash/zsh/fish/pwsh/nu hooks | 每种 shell 可捕获 command、cwd、exit code、duration |
| 8.3 Block model | block 是 terminal event 上层，不嵌入 grid 核心 | 禁用 shell integration 后终端仍完全可用 |
| 8.4 Block UI | copy command、copy output、jump、fold | 最近 100 个 block 可查看/复制 |
| 8.5 Starship compatibility | Starship prompt 不被替换，只采集边界 | starship + zsh/fish/pwsh/nu smoke 通过 |
| 8.6 Structured output P1 | JSON/CSV/TOML 输出可检测并复制 | lens 不改变 stdout，不影响 scrollback |
| 8.7 Failure block | 非 0 exit code block 高亮 | 可从失败 block 打开详情，但不自动调用 agent |

通过条件：

- shell integration 可一键启用/禁用。
- 禁用后 Noctrail 退回普通终端，无功能损坏。
- block 捕获错误不会吞输出。
- block copy plain text/Markdown 正确。
- `nvim`/`tmux`/`ssh` 中不误生成 command block。

### Phase 9：Agent、安全与 Review

目标：可选 AI，不影响终端。Agent 永远不拥有 shell。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 9.1 Agent disabled by default | 默认不联网、不读环境、不读 history | 默认配置下抓包无 provider 请求 |
| 9.2 Context collector | 只读 current block、selection、cwd、explicit files | UI 显示“Agent 将看到什么” |
| 9.3 Redaction | token、SSH key、JWT、AWS/GitHub/OpenAI key | secret corpus 通过；日志/audit/provider request 均脱敏 |
| 9.4 Provider adapter | OpenAI-compatible/local/CLI agent 抽象 | provider 失败不影响 terminal input/output |
| 9.5 Command proposal | Agent 只能建议命令 | 建议命令必须包含 reason、risk、permissions |
| 9.6 Review panel | 用户确认后才写入 shell | low/medium 确认；high 输入确认短语；critical 默认阻断 |
| 9.7 Patch preview | 生成 diff，但不直接写文件 | 应用 patch 前必须显示 diff |
| 9.8 Audit ledger | context/read/suggest/review/execute 本地记录 | 用户可查看 Agent 看了什么、建议了什么、执行了什么 |

阻断项：

- Agent 可静默执行命令。
- Critical command 可被自动执行。
- 关闭 Agent 后仍联网。
- provider error 影响终端使用。
- audit 缺失执行记录。
- secret redaction 在日志或 provider request 中失效。

### Phase 10：跨平台 Beta

目标：公开测试，但只在核心体验成熟后进入。

| 子计划 | 内容 | 验收标准 |
|---|---|---|
| 10.1 Installer | Windows installer、macOS dmg、Linux AppImage/deb/rpm | 安装、升级、卸载 smoke 通过 |
| 10.2 CI matrix | Windows/macOS/Linux build/test | 三平台 CI 通过；失败不能被忽略 |
| 10.3 Config reference | 完整 TOML schema 与示例 | 用户可按文档改 theme/keymap/layout |
| 10.4 Diagnostics | `noctrail doctor` | 能检测 shell、PTY、GPU、font、config、permissions |
| 10.5 Release blockers | 明确 P0/P1 blocker | blocker 未清零不得 beta |
| 10.6 Privacy/security docs | Agent、storage、telemetry、logs 说明 | 默认无 telemetry；关闭 Agent 不联网 |

## 6. 第一轮真正要写的东西

从 `e0f3712` 重新开始后，前 10 个 PR 应该非常克制。

| PR | 内容 | 不允许包含 |
|---|---|---|
| PR 1 | `docs/plan/rewrite-roadmap.md` | 代码实现 |
| PR 2 | 精简 workspace skeleton | agent/storage/policy |
| PR 3 | `noctrail-term` grid/cell/cursor | PTY/render |
| PR 4 | `noctrail-term` SGR/CSI 基础 parser | block/shell integration |
| PR 5 | terminal recording fixture harness | GUI |
| PR 6 | `noctrail-pty` spawn/read/write/resize | app runtime |
| PR 7 | `noctrail-runtime` `PaneId` registry | layout/render |
| PR 8 | `noctrail-layout` Dwindle tree | GUI |
| PR 9 | `noctrail-render` render snapshot + basic glyph path | animation/blur |
| PR 10 | `noctrail-app` 单窗口单 pane shell | multi-pane/agent |

## 7. 当前分支哪些东西可以借鉴

### 7.1 可以借鉴但不直接继承

| 可借鉴 | 原因 |
|---|---|
| terminal fixture 思路 | recording/golden 对终端正确性很重要 |
| `LayoutTree` 的纯逻辑方向 | 方向正确，但需要和真实 pane runtime 重新绑定 |
| shell OSC marker 设计 | 可保留协议思想，但后置到 Phase 8 |
| risk/redaction 测试 corpus 思路 | Agent 后期需要 |
| config 热加载思路 | Phase 6 可参考 |

### 7.2 不建议继承

| 不建议继承 | 原因 |
|---|---|
| 当前 `DesktopApp` 结构 | 单 pane runtime 与多 pane UI 脱节 |
| 当前 renderer 主路径 | GPU 目标不成立，software presenter 不能作为长期主线 |
| 当前 terminal core + block 耦合 | 终端核心职责过宽 |
| 过早 agent/storage | 会拖慢 terminal MVP |
| 多份 `docs/plan` | 会制造路线冲突 |

## 8. 发布阻断标准

任何一个出现，都不应该进入 Beta：

- Windows/macOS/Linux 任一 P0 平台无法启动。
- 单 pane shell 输入输出不稳定。
- `nvim`、`tmux`、`fzf`、`ssh` 中任意一个主流程严重不可用。
- 高输出冻结 UI。
- resize 后 TUI 显示错乱。
- 多 pane 输入输出串线。
- workspace 切换杀掉进程。
- 关闭 pane 后残留 child process。
- 配置损坏导致无法 safe mode 启动。
- 关闭 Agent 后仍联网。
- Agent 可绕过 review 执行命令。

## 9. 最终路线一句话

Noctrail 应该先成为一个像 Alacritty 一样可靠、像 Hyprland 一样有空间感和视觉反馈、像 Warp 一样能后期组织 command block、像 Starship/Nushell 一样尊重 shell 生态的 Rust 平铺式终端。

不要从“AI 终端”开始。

不要从“炫酷 UI”开始。

从一个正确的 terminal state machine + 可靠 PTY runtime + 真实 GPU render path + 可测试 Dwindle layout 开始。
