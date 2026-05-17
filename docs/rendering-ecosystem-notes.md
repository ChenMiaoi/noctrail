# Hyprland / Warp / Nushell / Starship 渲染栈对照

本文整理四个常被拿来和 Noctrail 对照的项目，重点只看“它们怎么渲染、用了什么框架、哪些信息是官方公开可确认的”。

结论先说：

- **Hyprland** 负责桌面合成，属于 Wayland compositor。
- **Warp** 负责终端应用 UI，公开确认有 Vulkan / OpenGL 等图形后端选项，但内部 UI 框架没有完整公开。
- **Nushell** 负责 shell 和交互式命令行，输入层主要依赖 Reedline，终端事件和控制则走 crossterm 一类的终端库。
- **Starship** 不做 UI 渲染，它只是 prompt 生成器，最终渲染仍然交给 shell 和终端。

## 1. Hyprland

Hyprland 不是 terminal，而是 **Wayland compositor**。它的工作是管理窗口、输入、输出、布局和桌面合成。

公开可确认的渲染链路大致是：

- Wayland compositor
- 自研合成器
- OpenGL / OpenGL ES
- EGL / GBM
- shader 驱动的桌面合成

官方仓库明确强调它不是基于 `wlroots` 的实现，而是自己的 compositor 代码路径。

可参考：

- [Hyprland 首页](https://hypr.land/)
- [Hyprland 官方说明：Independent Hyprland](https://hypr.land/news/independentHyprland/)
- [Hyprland 仓库](https://github.com/hyprwm/Hyprland)

## 2. Warp

Warp 是终端应用，但它的公开文档更偏产品能力，不像 Hyprland 那样把内部图形架构完整写出来。

目前官方公开能确认的是：

- 它有图形后端选择
- Windows 上公开提到过 Vulkan 和 OpenGL
- 也支持 integrated GPU 相关设置
- Linux 路径下公开支持 Wayland

因此更稳妥的说法是：

- Warp 是一个 **原生终端 UI 应用**
- 公开资料确认它使用 **图形化渲染后端**
- 但具体内部 UI 框架、scene graph、文本栅格引擎，官方没有完整公开

可参考：

- [Warp 文档首页](https://docs.warp.dev/)
- [Warp: Universal Input](https://docs.warp.dev/terminal/universal-input)
- [Warp: Prompt 相关文档](https://docs.warp.dev/terminal/appearance/prompt)

## 3. Nushell

Nushell 是 shell，不是 compositor，也不是独立桌面终端。

它的交互输入和屏幕绘制分层比较清楚：

- **Reedline** 负责 line editor
- **crossterm** 等终端库负责键盘事件、光标控制和终端绘制
- Nushell 自身负责 parser、IR、evaluation 和结构化数据流

官方文档对 Reedline 的定位很明确：它负责 history、completion、hints、validations 和 screen paint。

这意味着 Nushell 的“渲染”本质仍是：

- 在终端里画文本
- 通过终端控制序列更新编辑行
- 不是 GUI 框架，也不是图形 compositor

可参考：

- [Nushell 首页](https://www.nushell.sh/)
- [Nushell book: Line editor / Reedline](https://www.nushell.sh/book/line_editor.html)
- [Reedline 仓库](https://github.com/nushell/reedline)

## 4. Starship

Starship 不是终端 UI，也不是 shell 本体，而是 **prompt 生成器**。

它的方式很简单：

- shell 启动时执行 `starship init ...`
- Starship 读取当前上下文
- 生成 prompt 文本
- shell 把这段文本显示出来

所以 Starship 自己并不负责真正的界面渲染，它只负责把 prompt 内容算好。

这类设计的特点是：

- 与 shell 解耦
- 与 terminal emulator 解耦
- 输出是纯文本，框架依赖很薄

可参考：

- [Starship 首页](https://starship.rs/)
- [Starship 仓库](https://github.com/starship/starship)

## 5. 对 Noctrail 的直接启发

如果把这四个项目映射到 Noctrail 现在的工程阶段，最关键的是不要把所有职责混在一起。

建议拆法是：

- **终端语义层**：像 Nushell 的 line editor 那样，终端状态和交互语义分开
- **渲染层**：像 Warp 那样，把“要画什么”与“怎么画”分开
- **桌面合成层**：如果以后扩到窗口管理，再参考 Hyprland，而不是反过来把 compositor 逻辑塞进 terminal core
- **prompt/status 层**：像 Starship 一样做成独立模块，不塞回 terminal core

当前 Noctrail 的渲染后端选择是 **OpenGL**，也就是说：

- `RenderPlan` 负责提供要画什么
- `GUI` / 渲染层负责把这些 glyph、selection、cursor 通过 OpenGL 画到窗口里
- 同时保留软件 fallback，避免单一后端把桌面可用性卡死

这也解释了为什么 Noctrail 当前的路线应该保持：

- `TerminalState` 只管终端状态
- `RenderPlan` 只管渲染计划
- `GUI` 只负责把计划画出来
- `shell integration`、`prompt`、`agent`、`storage` 都不要提前侵入 core

## 6. 注意事项

本文只整理 **官方公开可确认** 的信息。

对 Warp 这种产品，很多内部实现细节并没有完整公开，所以这里只写到了公开文档和可确认的后端能力，不把推测写成事实。
