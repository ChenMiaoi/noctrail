# Noctrail：Hyprland 风格 Rust Agent 终端完整计划与验收标准

**版本**：v0.1 规划稿
**日期**：2026-05-14
**对外品牌**：Noctrail
**内部命名**：Cargo workspace crate 统一使用 `noctrail-*` 前缀；CLI 命令使用 `noctrail`。
**核心目标**：用 Rust 构建一个跨平台、GPU 加速、Hyprland 风格、面向现代 AI agent 工作流的开发者终端。

---

## 目录

1. [项目主旨](#1-项目主旨)
2. [参考基线与借鉴方向](#2-参考基线与借鉴方向)
3. [产品定位](#3-产品定位)
4. [目标平台定义](#4-目标平台定义)
5. [范围与非范围](#5-范围与非范围)
6. [核心用户故事](#6-核心用户故事)
7. [功能优先级](#7-功能优先级)
8. [系统架构](#8-系统架构)
9. [Rust 工程结构](#9-rust-工程结构)
10. [UI/UX 设计计划](#10-uiux-设计计划)
11. [终端核心计划](#11-终端核心计划)
12. [Shell、Prompt 与 Nushell 兼容计划](#12-shellprompt-与-nushell-兼容计划)
13. [Agent 系统计划](#13-agent-系统计划)
14. [安全、隐私与权限模型](#14-安全隐私与权限模型)
15. [跨平台工程计划](#15-跨平台工程计划)
16. [里程碑计划](#16-里程碑计划)
17. [测试计划](#17-测试计划)
18. [验收标准](#18-验收标准)
19. [发布标准](#19-发布标准)
20. [风险与缓解措施](#20-风险与缓解措施)
21. [团队与分工建议](#21-团队与分工建议)
22. [附录：建议技术栈](#22-附录建议技术栈)
23. [附录：参考资料](#23-附录参考资料)

---

## 1. 项目主旨

本项目的主旨不是复制 Hyprland，也不是做一个只能在 Hyprland/Wayland 下运行的终端，而是做一个：

> **具有 Hyprland 视觉与交互气质、用 Rust 构建、全桌面平台通用、深度集成现代 AI agent 工作流的开发者终端。**

关键词：

- **Hyprland 风格**：动态布局、圆角、边框、透明、动画、workspace、键盘驱动、视觉反馈强。
- **Warp 风格能力**：命令 block、agent 面板、AI 辅助解释、代码任务执行、命令审查。
- **Starship 风格 prompt**：快速、模块化、跨 shell、自定义强、上下文提示明显。
- **Nushell 风格结构化思路**：命令上下文、结构化输出、表格化展示、pipeline 友好。
- **Rust 构建**：高性能、内存安全、跨平台核心、可维护工程结构。
- **全平台通用**：Windows、macOS、Linux 为第一优先级；Web/移动端作为远期远程客户端，而不是第一阶段原生终端目标。

---

## 2. 参考基线与借鉴方向

| 参考对象 | 可借鉴点 | 不直接照搬的点 |
|---|---|---|
| Hyprland | 动态平铺、视觉效果、轻量响应、插件感、keyboard-first | 不依赖 Wayland compositor 技术栈；不做窗口管理器 |
| Warp | Agentic terminal、命令 block、AI 任务、现代 UI | 不做封闭平台；默认本地优先；权限更保守 |
| Starship | 跨 shell prompt、TOML 配置、模块化 segments、速度 | 不强制替代用户现有 prompt；优先兼容 Starship |
| Nushell | 结构化数据、跨平台 shell、插件思路 | 不把项目做成新 shell；只吸收结构化终端上下文能力 |
| WezTerm / Alacritty | Rust/GPU/跨平台终端工程经验 | 不做纯传统终端；重点加入 Hyprland 风格和 agent 流程 |

### 2.1 设计原则

1. **终端第一，agent 第二**
   没有稳定终端核心，AI 功能没有意义。

2. **用户确认优先于自动化**
   Agent 可以建议、解释、准备 patch，但破坏性操作必须确认。

3. **跨平台一致，但允许平台特性增强**
   例如 macOS 可用 vibrancy，Windows 可用 Mica/Acrylic，Linux 可用 compositor 透明；但这些都必须有降级方案。

4. **配置优先，主题优先，键盘优先**
   面向 power users，必须可配置、可脚本化、可禁用。

5. **本地优先，云可选**
   终端历史、环境变量、项目上下文不得默认上传。

---

## 3. 产品定位

### 3.1 一句话定位

> **Noctrail 是一个 Rust 原生、GPU 加速、Hyprland 风格的 agent-native 终端。**

### 3.2 产品形态

产品不是单纯的 terminal emulator，而是由三部分组成：

```text
Terminal Emulator + Visual Workspace + Agent Runtime
```

具体表现为：

- 像传统终端一样可靠地运行 shell、ssh、tmux、nvim、fzf、git、cargo、npm 等工具。
- 像 Hyprland 一样拥有视觉层次、动态 pane、workspace、浮动面板和键盘驱动体验。
- 像现代 agent IDE 一样理解当前命令、当前目录、失败日志、git diff、项目结构，并能给出可审查操作。

### 3.3 与现有工具的关系

| 工具 | 本项目关系 |
|---|---|
| Bash/Zsh/Fish/PowerShell/Nushell | 运行并兼容，不替代 |
| Starship | 兼容并可读取 prompt/context；不强制替换 |
| tmux | 兼容；后续可提供内置 workspace/pane 替代部分场景 |
| Claude Code/Codex/Gemini CLI 等 CLI agent | 可接入；不锁死单一 agent |
| MCP server | 作为可选工具协议；默认受权限控制 |

### 3.4 品牌与命名约定

- 对外产品品牌统一使用 **Noctrail**。
- CLI 命令统一使用 `noctrail`。
- 内部 Rust crate、workspace 包名和发布包名前缀统一使用 `noctrail-*`。
- 配置、缓存、ignore 文件等用户可见但偏工程侧的命名使用 `noctrail`，例如 `.noctrailignore`。

---

## 4. 目标平台定义

“全平台通用”在本计划中分为三个层级：

### 4.1 P0：原生桌面平台

P0 必须支持：

| 平台 | 最低目标 | 备注 |
|---|---|---|
| Windows | Windows 10 22H2 / Windows 11 | 使用 ConPTY；支持 PowerShell、cmd、WSL、Git Bash |
| macOS | macOS 13+ | 支持 Apple Silicon 与 Intel；优先 Apple Silicon |
| Linux | Ubuntu 24.04+、Fedora、Arch | 支持 Wayland 与 X11；透明/blur 依赖 compositor 能力 |

### 4.2 P1：包管理和开发者分发

P1 目标：

- Windows：`.msi`、`.exe installer`、winget。
- macOS：`.dmg`、Homebrew cask。
- Linux：`.AppImage`、`.deb`、`.rpm`、AUR。
- Rust 开发者：`cargo install` 可选，但不作为主分发方式。

### 4.3 P2：远程/轻量客户端

P2 可选目标：

- Web 端只做 remote session viewer/control，不做完整本地 PTY 替代。
- 移动端只做远程控制/查看，不承诺完整本地 terminal emulator。

---

## 5. 范围与非范围

### 5.1 必须做

- Rust 原生桌面应用。
- 跨平台窗口、渲染、PTY、配置和打包。
- 可靠终端模拟器核心。
- GPU 文本渲染。
- Hyprland-inspired 视觉系统。
- Workspace、pane、tab、command palette。
- Agent 面板。
- Agent 解释命令输出、推荐命令、生成 patch、用户确认后执行。
- Shell integration。
- Starship 兼容。
- Nushell 兼容。
- 安全权限模型。
- 本地优先隐私策略。

### 5.2 第一阶段不做

- 不做新的 shell 语言。
- 不做 Hyprland compositor。
- 不直接管理系统窗口。
- 不承诺移动端原生 terminal emulator。
- 不做无确认自动执行危险命令。
- 不默认上传 shell history、环境变量、SSH key、token。
- 不强制用户使用某个云模型。

### 5.3 可选后续功能

- 内置 multiplexer。
- 远程 session sync。
- Team workspace。
- Plugin marketplace。
- 本地小模型辅助。
- Voice command。
- Web remote client。
- 内置 file explorer / diff viewer。

---

## 6. 核心用户故事

### 6.1 终端用户

> 作为开发者，我希望它像传统终端一样稳定运行 nvim、tmux、ssh、git、cargo、npm、docker、kubectl，这样我不需要为 AI 功能牺牲终端可靠性。

验收点：

- 上述命令在 Windows/macOS/Linux 上可运行。
- 终端布局、颜色、光标、输入法、复制粘贴正常。
- 高输出情况下 UI 不冻结。

### 6.2 Hyprland 风格用户

> 作为喜欢 Hyprland/rice 风格的用户，我希望终端具有动态 pane、圆角边框、透明背景、动画、workspace 和 keyboard-first 操作。

验收点：

- pane 分割、移动、聚焦、关闭有平滑动画。
- workspace 切换可配置快捷键。
- 主题可通过配置文件定制。
- 不支持 blur 的平台有视觉降级方案。

### 6.3 Agent 用户

> 作为开发者，我希望 agent 能读懂当前失败命令和项目上下文，并给出可审查的修复建议，而不是直接乱执行命令。

验收点：

- Agent 能引用当前 command block 的输出。
- Agent 能解释错误并给出下一步。
- Agent 推荐命令必须显示原因、风险和权限。
- 高风险命令必须强制确认。

### 6.4 跨 shell 用户

> 作为使用 zsh/fish/PowerShell/Nushell 的用户，我希望不更换 shell 也能使用这个终端。

验收点：

- 支持主流 shell 初始化脚本。
- 不破坏 Starship prompt。
- Nushell 表格输出、ANSI、补全、history 可正常工作。

---

## 7. 功能优先级

### 7.1 P0：MVP 必须具备

| 模块 | 功能 |
|---|---|
| Terminal | PTY 创建、shell 启动、输入输出、resize、scrollback、copy/paste |
| Rendering | GPU 文本渲染、基础 ANSI 色彩、字体配置、DPI 缩放 |
| UI | 单窗口、tabs、panes、command palette、基础主题 |
| Hyprland 风格 | 圆角、边框、透明、动画、workspace 切换 |
| Agent | 读取当前输出、解释错误、推荐命令、确认后执行 |
| Config | TOML 配置、热加载、keymap、theme |
| Platform | Windows/macOS/Linux 桌面可运行 |
| Safety | 命令风险提示、secret redaction、权限确认、审计日志 |

### 7.2 P1：Beta 必须具备

| 模块 | 功能 |
|---|---|
| Terminal | OSC 8 hyperlink、bracketed paste、alternate screen、mouse reporting |
| Rendering | emoji、wide char、font fallback、ligature 可选 |
| UI | pane drag、workspace overview、floating agent panel |
| Agent | git diff 读取、patch preview、project context、multi-step plan |
| Shell Integration | bash/zsh/fish/nu/pwsh 初始化脚本 |
| Prompt | Starship 兼容检测、prompt block metadata |
| Packaging | signed installer、auto update、checksums |

### 7.3 P2：V1.5 可选

| 模块 | 功能 |
|---|---|
| Multiplexer | session restore、remote attach、named sessions |
| Plugins | WASM/plugin API、theme marketplace |
| Agent | MCP 工具接入、本地模型、agent task queue |
| Collaboration | shareable command block、team snippets |
| Remote | web viewer、mobile remote client |

---

## 8. 系统架构

### 8.1 总体架构

```text
┌──────────────────────────────────────────────────────────┐
│ Desktop App Shell                                         │
│ winit/tao event loop, windows, menus, platform integration│
├──────────────────────────────────────────────────────────┤
│ Hyprland-inspired UI Layer                                │
│ workspaces, panes, command palette, agent panel, themes    │
├──────────────────────────────────────────────────────────┤
│ GPU Renderer                                              │
│ wgpu, text atlas, glyph cache, animations, transparency    │
├──────────────────────────────────────────────────────────┤
│ Terminal Core                                             │
│ VT parser, grid, scrollback, selection, cursor, modes      │
├──────────────────────────────────────────────────────────┤
│ PTY / Process Layer                                       │
│ Unix PTY, Windows ConPTY, process lifecycle, resize        │
├──────────────────────────────────────────────────────────┤
│ Shell Integration                                         │
│ command blocks, cwd, exit code, prompt metadata            │
├──────────────────────────────────────────────────────────┤
│ Agent Runtime                                             │
│ context collector, model provider, tools, command review   │
├──────────────────────────────────────────────────────────┤
│ Security & Policy Layer                                   │
│ permissions, redaction, audit logs, command classifier     │
├──────────────────────────────────────────────────────────┤
│ Storage / Config                                          │
│ TOML config, SQLite/redb state, local logs, themes         │
└──────────────────────────────────────────────────────────┘
```

### 8.2 数据流

```text
User input
  -> UI event
  -> terminal input encoder
  -> PTY stdin
  -> shell/process
  -> PTY stdout/stderr
  -> VT parser
  -> terminal grid
  -> renderer
  -> screen
```

Agent 数据流：

```text
Terminal block / cwd / exit code / git status / selected text
  -> context collector
  -> redaction layer
  -> agent prompt builder
  -> model provider
  -> response parser
  -> command/patch reviewer
  -> user confirmation
  -> tool executor / shell write / file patch
```

### 8.3 线程/任务模型

| 组件 | 模型 | 要求 |
|---|---|---|
| UI event loop | 主线程 | 不执行阻塞 IO |
| Renderer | 主线程或 render thread | 60 FPS 动画目标 |
| PTY read | async task / dedicated thread | 高输出不阻塞 UI |
| Agent | async worker | 支持取消、超时、重试 |
| File indexing | background worker | 限速、权限范围内 |
| Storage | async/blocking pool | 不阻塞 terminal input |

---

## 9. Rust 工程结构

建议使用 Cargo workspace：

```text
noctrail/
  Cargo.toml
  crates/
    noctrail-app/                 # 桌面入口、窗口、平台集成
    noctrail-terminal-core/       # grid、parser adapter、scrollback、selection
    noctrail-pty/                 # Unix PTY / Windows ConPTY adapter
    noctrail-renderer/            # wgpu renderer、glyph atlas、animation
    noctrail-ui/                  # panes、tabs、workspaces、palette、theme
    noctrail-agent/               # agent runtime、provider abstraction
    noctrail-agent-tools/         # shell/file/git/MCP tools
    noctrail-policy/              # permissions、command risk、redaction
    noctrail-shell-integration/   # shell init scripts and protocol
    noctrail-config/              # config schema、hot reload、validation
    noctrail-storage/             # SQLite/redb persistence
    noctrail-cli/                 # `noctrail` command: config, shell-init, doctor
    noctrail-testkit/             # golden tests、PTY test harness
  assets/
    themes/
    fonts/
    icons/
  scripts/
    package/
    ci/
    smoke/
  docs/
    user-guide.md
    security.md
    plugin-api.md
    shell-integration.md
```

### 9.1 关键模块职责

| Crate | 职责 |
|---|---|
| `noctrail-app` | 应用生命周期、窗口、菜单、系统托盘、更新入口 |
| `noctrail-terminal-core` | 终端状态机、grid、scrollback、光标、选择、模式处理 |
| `noctrail-pty` | 跨平台 PTY 创建、resize、读写、子进程生命周期 |
| `noctrail-renderer` | GPU 渲染、文本缓存、透明/blur fallback、动画 |
| `noctrail-ui` | workspace、pane、tab、agent panel、palette、布局 |
| `noctrail-agent` | LLM 请求、上下文构建、streaming、tool planning |
| `noctrail-policy` | 命令风险分类、权限检查、secret redaction、审计 |
| `noctrail-shell-integration` | shell hook、command block、cwd/exit code 采集 |
| `noctrail-cli` | 安装 shell hook、诊断平台能力、导出配置 |

---

## 10. UI/UX 设计计划

### 10.1 视觉方向

Hyprland-inspired 不等于只加透明背景，而是：

- **动态 pane**：pane 分割、交换、浮动、聚焦有动画。
- **Workspace**：用数字/图标切换不同开发上下文。
- **Border language**：活跃 pane 有明显边框、glow 或 gradient。
- **Rounded surfaces**：tab、pane、panel、toast 统一圆角。
- **Transparency**：背景透明可调；不支持平台自动降级为 opaque theme。
- **Blur fallback**：支持平台启用 blur；不支持时使用半透明 tint + noise/solid fallback。
- **Keyboard-first**：所有重要动作必须可由快捷键完成。

### 10.2 基础布局

```text
┌────────────────────────────────────────────────────────┐
│ Top bar: workspace | project | branch | agent status    │
├───────────────┬────────────────────────────────────────┤
│ Pane A        │ Pane B                                  │
│ terminal      │ terminal                                │
├───────────────┴────────────────────────────────────────┤
│ Floating command palette / agent panel / task review    │
└────────────────────────────────────────────────────────┘
```

### 10.3 关键交互

| 动作 | 默认快捷键建议 | 说明 |
|---|---|---|
| 打开命令面板 | `Ctrl/Cmd + Shift + P` | 所有命令入口 |
| 新建 tab | `Ctrl/Cmd + T` | 默认当前 cwd |
| 新建 pane | `Ctrl/Cmd + D` | 根据当前 pane 分割 |
| 切换 workspace | `Alt + 1..9` | Hyprland 风格 |
| 聚焦 pane | `Alt + H/J/K/L` | Vim-like |
| 打开 agent | `Ctrl/Cmd + I` | 当前上下文提问 |
| 解释当前输出 | `Ctrl/Cmd + Shift + E` | 读取当前 block |
| 复制当前 block | `Ctrl/Cmd + Shift + C` | block-aware copy |
| 安全执行建议命令 | `Ctrl/Cmd + Enter` | 必须经过 review |

### 10.4 主题系统

配置示例：

```toml
[theme]
name = "hypr-dark"
opacity = 0.86
blur = true
corner_radius = 14
border_width = 1.5
active_border = ["#7aa2f7", "#bb9af7"]
inactive_border = "#2a2a37"
background = "#101014"
foreground = "#c0caf5"
accent = "#7dcfff"

[animation]
enabled = true
duration_ms = 160
curve = "ease-out-cubic"

[font]
family = "JetBrainsMono Nerd Font"
size = 13.0
ligatures = true
emoji = true
```

### 10.5 UI 验收

- 所有核心操作可用键盘完成。
- 主题热加载不重启应用。
- 不支持透明/blur 的系统仍显示完整 UI。
- 4K / Retina / 125% / 150% / 200% DPI 下布局不错位。
- 动画可关闭。
- 低性能机器上可进入 performance mode。

---

## 11. 终端核心计划

### 11.1 核心能力

| 能力 | P0 | P1 | P2 |
|---|---:|---:|---:|
| PTY 输入输出 | ✅ | ✅ | ✅ |
| ANSI / VT parser | ✅ | ✅ | ✅ |
| 24-bit color | ✅ | ✅ | ✅ |
| Scrollback | ✅ | ✅ | ✅ |
| Alternate screen | ✅ | ✅ | ✅ |
| Mouse reporting | ✅ | ✅ | ✅ |
| Bracketed paste | ✅ | ✅ | ✅ |
| OSC 8 hyperlink |  | ✅ | ✅ |
| OSC 52 clipboard |  | ✅ | ✅ |
| Sixel / image protocol |  |  | 可选 |
| Ligature |  | ✅ | ✅ |
| Advanced shaping |  | ✅ | ✅ |
| IME | ✅ | ✅ | ✅ |
| Accessibility |  | ✅ | ✅ |

### 11.2 实现策略

推荐顺序：

1. **先复用成熟 VT parser / terminal core**
   避免从零实现所有 escape sequence。

2. **先做正确性，再做性能**
   不要为了动画牺牲 vim/tmux/nvim 的可靠性。

3. **先支持主流工具**
   nvim、tmux、fzf、git、cargo、npm、ssh、docker、kubectl。

4. **先做可测试终端核心**
   terminal core 不能和 UI 强耦合。

### 11.3 终端兼容目标

P0 必须正常运行：

- `bash`
- `zsh`
- `fish`
- `pwsh`
- `cmd.exe`
- `nushell`
- `nvim`
- `vim`
- `tmux`
- `fzf`
- `git`
- `ssh`
- `cargo`
- `npm` / `pnpm`
- `python` REPL
- `node` REPL
- `docker` CLI
- `kubectl` CLI

### 11.4 终端验收重点

- 光标位置正确。
- 宽字符、emoji、CJK 字符宽度正确。
- resize 后 shell 和全屏程序能正确重绘。
- alternate screen 进入/退出正确。
- copy/paste 不破坏换行和选择区域。
- bracketed paste 不导致命令误执行。
- Windows ConPTY 下 PowerShell、WSL、cmd 正常。

---

## 12. Shell、Prompt 与 Nushell 兼容计划

### 12.1 Shell integration 协议

提供 CLI：

```bash
noctrail shell-init bash
noctrail shell-init zsh
noctrail shell-init fish
noctrail shell-init nu
noctrail shell-init pwsh
```

Shell integration 采集：

- 当前命令开始时间。
- 当前命令文本。
- 当前命令退出码。
- 当前工作目录。
- Git branch/status 简要信息。
- Prompt 起止位置。
- Command block 起止位置。

### 12.2 Starship 兼容

原则：

- 不替代 Starship。
- 不要求用户卸载 Starship。
- 支持用户继续使用自己的 prompt。
- 可通过 shell integration 识别 prompt block。
- 提供 Starship-like status line，但默认关闭。

可选功能：

- 读取 `~/.config/starship.toml` 中的部分主题 token。
- 根据 Starship segments 在 top bar 显示 cwd、branch、runtime、exit code。
- 提供 `noctrail prompt preview` 查看 prompt metadata。

### 12.3 Nushell 兼容

Nushell 兼容重点：

- 支持 `nu` 作为默认 shell。
- 不破坏 Nushell 表格输出。
- 支持 Nushell ANSI 色彩。
- 支持 Nushell history 和 completion。
- shell integration 通过 Nushell hook 实现 command block。

Nushell-inspired 功能：

- Agent 可请求当前 command block 的结构化摘要。
- 对 JSON/YAML/TOML/CSV 输出提供可选 structured viewer。
- 支持将选中输出转换为 table view。
- 支持 `explain as table`、`summarize json` 等命令面板动作。

### 12.4 验收

| 项目 | 验收方式 | 通过标准 |
|---|---|---|
| zsh + Starship | 启动、执行、切换目录、git repo | prompt 不错位，block 正确 |
| fish + Starship | 同上 | prompt 不错位，退出码捕获正确 |
| Nushell | `ls`、管道、表格、错误输出 | 表格布局正常，颜色正常 |
| PowerShell | profile、prompt、脚本执行 | 命令 block 正确，ConPTY 正常 |
| WSL | Windows 终端内启动 WSL shell | 输入输出和 resize 正常 |

---

## 13. Agent 系统计划

### 13.1 Agent 能力分层

| 层级 | 能力 | 默认状态 | 风险 |
|---|---|---|---|
| A0 | 解释当前输出 | 开启 | 低 |
| A1 | 推荐命令 | 开启 | 中 |
| A2 | 生成 patch/diff | 开启 | 中 |
| A3 | 用户确认后执行命令 | 开启 | 中高 |
| A4 | 多步任务自动执行 | 关闭 | 高 |
| A5 | 联网、调用外部工具、MCP | 关闭 | 高 |

### 13.2 Agent 核心功能

P0：

- 解释当前 command block。
- 解释选中文本。
- 根据错误输出推荐下一步命令。
- 生成命令前显示：命令、理由、风险、影响范围。
- 用户确认后把命令写入 shell 或直接执行。
- 支持取消 streaming response。

P1：

- 读取 git diff。
- 读取项目文件，但必须受 scope 限制。
- 生成 patch preview。
- 多步计划，但每步执行前确认。
- 支持本地/云模型 provider abstraction。
- 支持 CLI agent 接入。

P2：

- MCP client。
- Tool marketplace。
- Agent task queue。
- Agent session replay。
- 多 agent 并行任务。

### 13.3 Agent UI

Agent 面板包含：

```text
Task title
Context chips: cwd, branch, command, exit code, selected text
Model/provider selector
Response stream
Proposed commands
Patch preview
Risk badge
Confirm / edit / reject buttons
Audit log link
```

### 13.4 命令审查格式

```text
将要执行：
  cargo test --workspace

原因：
  验证刚才修复是否解决编译失败。

影响范围：
  当前项目目录，只运行测试，不修改文件。

风险等级：
  低

需要权限：
  shell.execute

操作：
  [执行] [复制] [编辑] [拒绝]
```

### 13.5 Agent 上下文规则

默认允许读取：

- 当前 command block 输出。
- 当前选中文本。
- 当前 cwd 路径。
- 当前 shell 类型。
- 当前命令退出码。
- git branch 和 dirty 状态。

默认不允许读取：

- 全量 shell history。
- 全量环境变量。
- SSH key。
- token。
- 浏览器 cookie。
- 用户 home 目录全文。
- 未授权项目文件。

读取项目文件时：

- 必须显示读取范围。
- 大文件要摘要化。
- `.gitignore`、`.noctrailignore`、用户 denylist 必须生效。

---

## 14. 安全、隐私与权限模型

### 14.1 权限类型

| 权限 | 描述 | 默认 |
|---|---|---|
| `terminal.read_current_block` | 读取当前命令输出 | 允许 |
| `terminal.read_scrollback` | 读取 scrollback | 询问 |
| `shell.suggest` | 生成建议命令 | 允许 |
| `shell.write` | 写入命令但不执行 | 询问 |
| `shell.execute` | 执行命令 | 每次确认 |
| `fs.read_project` | 读取当前项目文件 | 询问 |
| `fs.write_project` | 修改当前项目文件 | 每次确认 |
| `git.diff_read` | 读取 git diff | 询问 |
| `network.access` | 访问网络 | 询问/默认关闭 |
| `mcp.use_server` | 使用 MCP server | 默认关闭 |

### 14.2 命令风险分类

| 风险 | 示例 | 行为 |
|---|---|---|
| 低 | `cargo test`、`git status`、`ls` | 可确认后执行 |
| 中 | `npm install`、`docker compose up`、`git checkout` | 显示影响范围 |
| 高 | `rm -rf`、`chmod -R`、`sudo`、`curl | sh` | 强制二次确认 |
| 极高 | 删除 home、读取 key、上传 env、破坏性 git reset | 默认拒绝或要求手动输入确认短语 |

### 14.3 Secret redaction

必须识别并遮蔽：

- `OPENAI_API_KEY`、`ANTHROPIC_API_KEY` 等 API key。
- GitHub token。
- AWS access key。
- SSH private key。
- JWT。
- `.env` 常见 secret。
- URL 中的 token query 参数。

### 14.4 审计日志

Agent 操作必须记录：

- 时间。
- 读取了哪些上下文类型。
- 生成了什么命令。
- 用户是否确认。
- 执行结果。
- 修改了哪些文件。

审计日志默认本地保存，不上传。

### 14.5 安全验收硬门槛

以下任一项不通过，不允许发布 Beta：

- Agent 可在无确认情况下执行破坏性命令。
- Agent 默认上传环境变量。
- Agent 默认读取 SSH key 或 token。
- MCP/tool 可绕过权限模型。
- Redaction 单元测试未覆盖常见 secret。
- 审计日志缺失 command execution 记录。

---

## 15. 跨平台工程计划

### 15.1 平台能力矩阵

| 能力 | Windows | macOS | Linux Wayland | Linux X11 |
|---|---:|---:|---:|---:|
| PTY | ConPTY | Unix PTY | Unix PTY | Unix PTY |
| GPU | D3D12/Vulkan/OpenGL backend | Metal backend | Vulkan/OpenGL | Vulkan/OpenGL |
| Transparent window | ✅ | ✅ | compositor dependent | compositor dependent |
| Blur/Mica/Vibrancy | Mica/Acrylic optional | Vibrancy optional | compositor dependent | compositor dependent |
| IME | ✅ | ✅ | ✅ | ✅ |
| Global shortcuts | limited | limited | limited | limited |
| Signed installer | ✅ | ✅ | distro dependent | distro dependent |

### 15.2 降级策略

| 特性 | 不支持时降级 |
|---|---|
| Blur | 使用 opaque/semi-transparent background |
| Transparency | 使用纯色主题 |
| Mica/Acrylic | 使用普通窗口背景 |
| Font fallback | 使用系统 fallback + warning |
| GPU backend failure | 尝试备用 backend；失败进入诊断页 |
| Shell integration failure | 回退普通终端模式 |
| Agent provider failure | 不影响终端使用 |

### 15.3 跨平台验收环境

最小测试矩阵：

| OS | Shell | GPU/显示 | 验收状态 |
|---|---|---|---|
| Windows 11 x64 | PowerShell 7 | D3D12 | 必测 |
| Windows 11 x64 | WSL Ubuntu | D3D12 | 必测 |
| Windows 10 22H2 | PowerShell/cmd | D3D12/OpenGL fallback | 必测 |
| macOS 14+ Apple Silicon | zsh | Metal/Retina | 必测 |
| macOS 13+ Intel | zsh | Metal | 建议测 |
| Ubuntu 24.04 | bash/zsh | Wayland/Vulkan | 必测 |
| Ubuntu 24.04 | bash/zsh | X11/OpenGL | 必测 |
| Fedora latest | bash/fish | Wayland | 建议测 |
| Arch Linux | zsh/fish/nu | Wayland | 建议测 |

---

## 16. 里程碑计划

时间估算基于 3–5 人小团队。单人开发需要显著延长。

### Phase 0：产品与技术验证

**周期**：2 周
**目标**：锁定架构、风险、MVP 范围。

交付物：

- 产品需求文档。
- 技术选型文档。
- 终端核心选型 PoC。
- 渲染 PoC。
- PTY PoC。
- Agent 安全模型草案。

验收标准：

- 能在至少 2 个平台打开窗口并启动 shell。
- 能渲染基本终端输出。
- 选定 terminal core 和 PTY 方案。
- 明确第一版不做的功能。

---

### Phase 1：Terminal Prototype

**周期**：4–6 周
**目标**：做出可输入输出的跨平台终端原型。

交付物：

- Rust workspace 初始化。
- PTY adapter。
- terminal grid。
- 基础 parser 接入。
- wgpu 文本渲染原型。
- 单窗口单 pane。
- 基础配置文件。

验收标准：

- Windows/macOS/Linux 至少各有一个环境可启动默认 shell。
- 支持输入、输出、resize。
- 支持基本 ANSI 颜色。
- 支持 scrollback 1 万行。
- `vim` 或 `nvim` 可启动并退出。
- UI 不因大量输出直接冻结。

---

### Phase 2：MVP Terminal Core

**周期**：8–10 周
**目标**：达到日常开发最低可用。

交付物：

- tabs。
- panes。
- copy/paste。
- selection。
- alternate screen。
- mouse reporting。
- bracketed paste。
- IME 基础支持。
- Shell integration 初版。
- 终端兼容测试套件。

验收标准：

- `nvim`、`tmux`、`fzf`、`git`、`ssh`、`cargo`、`npm` 可用。
- resize 后全屏应用重绘正确。
- scrollback 10 万行可用。
- 输入延迟 p95 小于 20ms。
- 空闲 CPU 小于 2%。
- 冷启动小于 1s，参考机器上测量。

---

### Phase 3：Hyprland-inspired UI

**周期**：4–6 周
**目标**：建立独特视觉和 workspace 体验。

交付物：

- Theme system。
- 圆角 pane。
- active border。
- 透明背景。
- blur fallback。
- workspace 切换。
- command palette。
- pane 动画。
- keymap 配置。

验收标准：

- 所有 UI 操作可键盘完成。
- 主题热加载成功。
- 动画 60 FPS 目标；低性能模式可关闭。
- 不支持 blur 的平台降级后仍可用。
- DPI 125%、150%、200% 下无明显错位。

---

### Phase 4：Agent MVP

**周期**：6–8 周
**目标**：可安全解释错误、推荐命令、用户确认后执行。

交付物：

- Agent panel。
- Provider abstraction。
- Context collector。
- Redaction layer。
- Command risk classifier。
- Command review UI。
- Audit log。
- `explain current block`。
- `suggest next command`。

验收标准：

- Agent 能解释至少 20 个常见开发错误样例。
- 推荐命令必须经过 review。
- 高风险命令强制二次确认。
- Secret redaction 测试通过。
- Agent 失败不影响终端使用。
- 用户可完全关闭 agent。

---

### Phase 5：跨平台硬化

**周期**：8–12 周
**目标**：从“能跑”进入“可公开 beta”。

交付物：

- Windows installer。
- macOS dmg。
- Linux AppImage/deb/rpm。
- Auto update 初版。
- Crash reporting 选项。
- Platform diagnostic command：`noctrail doctor`。
- CI matrix。
- Golden rendering tests。

验收标准：

- 目标平台测试矩阵通过。
- 安装、升级、卸载流程通过。
- 主要 shell integration 通过。
- 100 次启动无 crash。
- 8 小时持续运行无明显内存泄漏。

---

### Phase 6：Private Beta / Public Beta

**周期**：6–8 周
**目标**：真实用户试用，修复兼容性和安全问题。

交付物：

- Beta 文档。
- Feedback system。
- Issue triage 流程。
- 性能 dashboard。
- Security disclosure policy。
- Migration/import docs。

验收标准：

- 50–200 名测试用户。
- P0 crash issue 清零。
- P0 security issue 清零。
- 关键平台安装成功率大于 95%。
- 用户能完成至少 3 小时真实开发工作流。

---

### Phase 7：V1.0

**周期**：4–6 周
**目标**：发布稳定版本。

交付物：

- V1.0 release。
- 完整用户文档。
- 安全白皮书。
- 配置 schema。
- API/plugin 规划。
- Release notes。

验收标准：

- V1.0 总体验收全部通过。
- 发布包签名。
- Checksums 发布。
- 回滚策略可用。
- 用户可关闭所有 AI 功能而不影响终端。

---

## 17. 测试计划

### 17.1 单元测试

| 模块 | 测试内容 |
|---|---|
| Terminal core | grid、scrollback、selection、cursor、mode state |
| Parser adapter | escape sequence、SGR、OSC、CSI |
| Config | schema、默认值、错误提示、热加载 |
| Agent policy | permission、risk classifier、redaction |
| Shell integration | block marker、exit code、cwd capture |
| Renderer | glyph cache、layout、DPI transform |

### 17.2 集成测试

- 启动 shell。
- 执行命令。
- resize。
- copy/paste。
- bracketed paste。
- alternate screen。
- `nvim` smoke test。
- `tmux` smoke test。
- `fzf` smoke test。
- PowerShell/WSL smoke test。
- Nushell smoke test。

### 17.3 Golden screenshot 测试

场景：

- ANSI 16 色。
- 256 色。
- true color。
- CJK 宽字符。
- emoji。
- underline/bold/italic。
- cursor styles。
- pane border。
- command palette。
- agent review panel。

### 17.4 性能测试

| 指标 | 目标 |
|---|---:|
| 冷启动 | < 1000ms |
| warm start | < 500ms |
| 空闲 CPU | < 2% |
| 非 agent 模式 idle memory | < 180MB |
| agent panel idle memory | < 300MB |
| 输入延迟 p95 | < 20ms |
| 高输出时输入延迟 p95 | < 50ms |
| scrollback | 100k 行 |
| 动画 | 60 FPS 目标，可降级 |
| 8 小时运行内存增长 | < 20% |

### 17.5 安全测试

- Secret redaction corpus。
- 危险命令分类 corpus。
- Prompt injection 样例。
- Tool permission bypass 测试。
- MCP server 沙箱测试。
- 文件读取范围测试。
- Agent 审计日志测试。

---

## 18. 验收标准

## 18.1 MVP 验收标准

MVP 发布条件：

| 类别 | 验收标准 | 通过条件 |
|---|---|---|
| 平台 | Windows/macOS/Linux 可运行 | 三平台各至少 1 个环境通过 smoke test |
| 终端 | shell 输入输出正常 | bash/zsh/pwsh 至少通过 |
| 兼容性 | nvim/tmux/fzf/git/cargo/npm 可用 | 无严重显示或输入错误 |
| UI | tabs/panes/workspace 可用 | 键盘可完成核心操作 |
| 风格 | Hyprland-inspired 主题可见 | 圆角、边框、透明/降级、动画存在 |
| Agent | 可解释当前输出 | 至少 20 个错误样例通过 |
| 安全 | 命令执行需确认 | 无静默执行高风险命令 |
| 配置 | TOML 配置可热加载 | 改主题/字体/keymap 生效 |
| 稳定性 | 3 小时日常使用不 crash | 测试环境通过 |

### 18.2 V1.0 总体验收标准

V1.0 必须同时满足以下标准。

#### A. 平台验收

| 编号 | 标准 | 通过条件 |
|---|---|---|
| A1 | Windows 11 支持 | PowerShell、cmd、WSL smoke test 通过 |
| A2 | Windows 10 支持 | PowerShell/cmd smoke test 通过 |
| A3 | macOS Apple Silicon 支持 | zsh、Homebrew 环境通过 |
| A4 | macOS Intel 支持 | 基础 smoke test 通过 |
| A5 | Linux Wayland 支持 | Ubuntu/Fedora 至少一个通过 |
| A6 | Linux X11 支持 | Ubuntu X11 通过 |
| A7 | 高 DPI 支持 | 125/150/200% 缩放无严重错位 |
| A8 | 安装包 | 三平台安装/升级/卸载通过 |

#### B. 终端正确性验收

| 编号 | 标准 | 通过条件 |
|---|---|---|
| B1 | ANSI/SGR | 16 色、256 色、true color 正确 |
| B2 | Alternate screen | nvim/tmux 进入退出正确 |
| B3 | Mouse reporting | nvim/fzf/tmux 鼠标功能可用 |
| B4 | Bracketed paste | 粘贴多行命令不误执行 |
| B5 | Resize | 全屏程序 resize 后重绘正确 |
| B6 | Scrollback | 100k 行可滚动、搜索、选择 |
| B7 | Unicode | CJK、emoji、wide char 基本正确 |
| B8 | IME | 中文/日文/韩文输入基础可用 |
| B9 | Clipboard | copy/paste 跨平台可用 |
| B10 | Hyperlink | OSC 8 hyperlink P1 通过 |

#### C. 性能验收

| 编号 | 指标 | 目标 |
|---|---|---:|
| C1 | 冷启动 | < 1000ms |
| C2 | 空闲 CPU | < 2% |
| C3 | 输入延迟 p95 | < 20ms |
| C4 | 高输出输入延迟 p95 | < 50ms |
| C5 | 非 agent 模式内存 | < 180MB |
| C6 | Agent panel 开启内存 | < 300MB |
| C7 | 动画 | 60 FPS 目标，低性能可关闭 |
| C8 | 长时间运行 | 8 小时无明显泄漏 |

#### D. UI/UX 验收

| 编号 | 标准 | 通过条件 |
|---|---|---|
| D1 | Keyboard-first | 核心操作均有快捷键 |
| D2 | Command palette | 可搜索并执行核心动作 |
| D3 | Workspace | 至少 9 个 workspace 快捷切换 |
| D4 | Pane | 新建、关闭、移动、聚焦、resize 可用 |
| D5 | Theme | 配置热加载成功 |
| D6 | Visual fallback | 无 blur/透明平台可正常显示 |
| D7 | Accessibility | 字体大小、动画关闭、对比度配置可用 |
| D8 | Error UX | 配置错误有明确提示，不 silent fail |

#### E. Agent 验收

| 编号 | 标准 | 通过条件 |
|---|---|---|
| E1 | Explain output | 能解释当前 block 和选中文本 |
| E2 | Suggest command | 推荐命令包含理由和风险 |
| E3 | Command review | 执行前必须显示 review UI |
| E4 | High-risk guard | 高风险命令二次确认 |
| E5 | Patch preview | 文件修改前显示 diff |
| E6 | Context scope | 读取范围可见，可配置 |
| E7 | Provider failure | 模型失败不影响终端 |
| E8 | Disable AI | 用户可完全关闭 agent |

#### F. 安全与隐私验收

| 编号 | 标准 | 通过条件 |
|---|---|---|
| F1 | Secret redaction | corpus 测试通过 |
| F2 | Env safety | 默认不上传环境变量 |
| F3 | Shell history safety | 默认不上传全量 history |
| F4 | File scope | 默认只读授权项目范围 |
| F5 | Audit log | Agent 操作有本地审计记录 |
| F6 | MCP/tool safety | tool 不能绕过权限 |
| F7 | Telemetry | 默认 opt-in 或清晰可关闭 |
| F8 | Security doc | 发布安全模型文档 |

#### G. 配置与可维护性验收

| 编号 | 标准 | 通过条件 |
|---|---|---|
| G1 | Config schema | 有版本化 schema |
| G2 | Hot reload | theme/keymap/font 改动生效 |
| G3 | Config migration | 旧版本配置可迁移或提示 |
| G4 | Logging | 可诊断但不泄露 secret |
| G5 | `noctrail doctor` | 能输出平台诊断信息 |
| G6 | CI | PR 必须跑测试和 lint |
| G7 | Crash handling | crash 有本地报告和恢复提示 |

### 18.3 不允许发布的阻断项

任一项出现即阻断发布：

- 终端无法在任一 P0 平台启动。
- nvim/tmux 在主平台严重不可用。
- Agent 可绕过确认执行命令。
- Secret redaction 明显失效。
- Windows ConPTY 经常挂死或无法 resize。
- Linux Wayland/X11 任一完全不可用。
- 配置错误导致应用无法恢复启动，且无 safe mode。
- 关闭 agent 后终端仍尝试联网。

---

## 19. 发布标准

### 19.1 Alpha 发布

Alpha 可以缺少：

- 完整打包。
- 完整 Agent patch。
- 完整 shell integration。
- 完整文档。

Alpha 不可缺少：

- 终端可用。
- 基础安全确认。
- 明确风险提示。

### 19.2 Beta 发布

Beta 必须具备：

- 三平台安装包。
- Agent 可关闭。
- 风险命令保护。
- 基础文档。
- Bug report 入口。
- Crash recovery。

### 19.3 V1.0 发布

V1.0 必须具备：

- 签名安装包。
- Checksums。
- Release notes。
- Security policy。
- Privacy policy。
- User guide。
- Config reference。
- Troubleshooting guide。
- 回滚/降级策略。

---

## 20. 风险与缓解措施

| 风险 | 影响 | 缓解措施 |
|---|---|---|
| 终端兼容复杂 | 高 | 复用成熟 parser/core；建立兼容测试 |
| Windows ConPTY 坑多 | 高 | 早期纳入 Windows；单独 owner 负责 |
| GPU 文本渲染难 | 高 | 先正确后优化；glyph cache；benchmark |
| Unicode/emoji/IME | 高 | 尽早测试 CJK/emoji/IME；不要后补 |
| Hyprland 视觉跨平台不一致 | 中 | 平台增强 + fallback theme |
| Agent 误执行命令 | 极高 | 默认确认、风险分类、审计、二次确认 |
| Secret 泄露 | 极高 | redaction、scope、默认不上传 env/history |
| 功能范围膨胀 | 高 | 明确 P0/P1/P2；V1 不做 shell replacement |
| 与 Warp/WezTerm/Alacritty 差异不够 | 中 | 聚焦 Hyprland 风格 + 安全 agent workflow |
| 用户不信任 AI terminal | 高 | 本地优先、可关闭、审计透明、最小权限 |

---

## 21. 团队与分工建议

### 21.1 最小团队

| 角色 | 人数 | 职责 |
|---|---:|---|
| Rust terminal engineer | 1 | PTY、parser、grid、兼容性 |
| Rust renderer/UI engineer | 1 | wgpu、UI、动画、主题 |
| Agent/security engineer | 1 | agent runtime、权限、安全 |
| Product/design | 0.5–1 | UX、主题、文档、测试场景 |
| QA/release | 0.5 | 跨平台测试、打包、CI |

### 21.2 单人开发取舍

单人开发建议：

1. 不从零写 terminal parser。
2. 先做 Linux/macOS，再补 Windows；但 Windows 不能拖到最后。
3. Agent 先只做 explain + suggest，不做多步自动执行。
4. UI 先做主题和 pane，不追求复杂插件系统。
5. V1 前不做 web/mobile。

---

## 22. 附录：建议技术栈

| 层 | 推荐方案 | 备注 |
|---|---|---|
| 语言 | Rust | 主工程语言 |
| Async | tokio | PTY/agent/storage background tasks |
| 窗口 | winit 或 tao | 跨平台窗口和事件循环 |
| GPU | wgpu | Vulkan/Metal/D3D12/OpenGL 后端 |
| Terminal parser/core | alacritty_terminal / vte | 先复用成熟组件 |
| PTY | portable-pty 或自研 adapter | Unix PTY + Windows ConPTY |
| 文本 shaping | swash / cosmic-text / fontdue 组合评估 | 重点 CJK/emoji/ligature |
| UI | 自研 wgpu UI；部分面板可评估 egui/iced | 终端渲染建议自控 |
| Config | serde + toml | Starship-like 可读配置 |
| Storage | SQLite / redb | 本地状态、审计、sessions |
| Agent HTTP | reqwest | provider abstraction |
| Tool protocol | MCP 可选 | 默认关闭、权限控制 |
| 打包 | cargo-dist / custom scripts | 三平台分发 |
| CI | GitHub Actions | matrix test |

---

## 23. 附录：参考资料

> 以下资料用于校准项目定位与技术方向。链接在 2026-05-14 检索。

- Hyprland official site：`https://hypr.land/`
- Warp official site：`https://www.warp.dev/`
- Starship official site：`https://starship.rs/`
- Nushell official site：`https://www.nushell.sh/`
- wgpu official site：`https://wgpu.rs/`
- winit docs：`https://docs.rs/winit/`
- Windows Pseudoconsole docs：`https://learn.microsoft.com/en-us/windows/console/pseudoconsoles`
- Model Context Protocol docs：`https://modelcontextprotocol.io/docs/getting-started/intro`
- vte crate / Alacritty parser：`https://github.com/alacritty/vte`
- alacritty_terminal crate：`https://docs.rs/alacritty_terminal/`

---

# 最终判断

这个项目的可行路径是：

```text
先做可靠终端核心
再做 Hyprland 风格 UI
再做受权限控制的 agent
最后做跨平台硬化和分发
```

最关键的验收底线是：

1. **终端必须稳定。**
2. **Agent 必须可控。**
3. **跨平台必须真实测试。**
4. **Hyprland 风格必须是系统化设计，而不是只加透明背景。**
5. **用户必须能关闭 AI，仍然得到一个好用终端。**
