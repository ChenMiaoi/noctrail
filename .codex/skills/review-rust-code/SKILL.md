---
name: review-rust-code
description: "Rust 代码审查 skill。Use when reviewing diffs, pull requests, local changes, or proposed patches for correctness, safety, cross-platform behavior, tests, minimality, and maintainability in Noctrail."
---

# Review Rust Code

## 审查姿态

像成熟开源项目维护者一样审查：先判断能不能安全合并，再讨论风格。优先发现会导致错误行为、安全风险、跨平台回归、数据丢失、性能灾难或维护成本上升的问题。

不要把 review 写成泛泛建议。每条 finding 都要能让作者定位、理解影响并采取行动。

## 代码最小原则

以 Linus 式维护者品味审查代码：代码越多，bug 面、review 成本和长期维护成本越高。好的改动通常让状态更少、路径更直、特殊情况更少。

重点质疑：

- 是否为了一个局部需求引入了通用框架。
- 是否复制了已有逻辑、状态转换、错误处理或平台分支。
- 是否用多个布尔值表达本该由 enum/type 表达的状态。
- 是否新增 public API 但没有稳定调用边界。
- 是否把平台差异扩散到业务逻辑中。
- 是否保留了已经无用的兼容层、helper、fixture 或配置项。
- 是否用更多代码掩盖了数据结构或不变量设计不清。

代码膨胀可以是 review finding：如果冗余会扩大 bug 面、模糊安全边界、破坏 API 稳定性或让后续改动更难，就按 `P1`/`P2` 提出；纯口味问题才降为 `P3`。

## 审查流程

1. 建立 diff 上下文。
   - 查看 `git status --short`、`git diff --stat`、`git diff`。
   - 区分当前任务改动和用户已有无关改动。
   - 如果是 PR，理解目标分支、提交范围、CI 状态和 review 线程。

2. 理解意图。
   - 找出改动声称解决的问题、影响的模块和不变量。
   - 对照 `docs/plan.md` 的架构、安全、跨平台和验收标准。

3. 深读风险文件。
   - 不只看新增代码，也看调用方、错误路径、测试、配置和平台分支。
   - 对 Rust 代码重点追踪 ownership、lifetimes、error propagation、async cancellation、locks、feature flags。

4. 验证。
   - 能运行时优先运行最相关的测试、fmt、clippy 或 targeted smoke。
   - 不能运行时说明原因，并给出未覆盖风险。

## 严重度

- `P0`: 不能合并。会泄露 secret、绕过权限、导致数据丢失、静默执行危险命令、引入 unsound unsafe、稳定复现 crash、破坏 P0 平台。
- `P1`: 应修复后合并。明显正确性 bug、race/deadlock、跨平台回归、错误处理丢失、重大测试缺口、显著性能回归、会明显扩大 bug 面的冗余实现。
- `P2`: 建议修复。API 边界不清、可维护性问题、局部设计不一致、错误信息不足、次要测试缺口、不必要抽象或重复逻辑。
- `P3`: 可选 nit。只有在不会淹没重要问题时提出。

## Noctrail 专项检查

- 终端核心：VT 状态机、grid、scrollback、selection、alternate screen、resize、mouse/IME/Unicode 是否保持不变量。
- PTY：Unix PTY 与 Windows ConPTY 是否都考虑；process lifecycle、resize、EOF、信号、编码是否安全。
- Renderer/UI：高输出是否阻塞 UI；DPI、字体 fallback、透明/blur fallback、动画关闭是否可用。
- Agent/policy：命令执行、文件写入、网络、MCP/tool 是否经过权限；redaction 是否覆盖日志、审计、错误。
- Config/storage：schema 是否可迁移；错误提示是否可恢复；本地状态是否避免泄露敏感信息。
- Async：是否在持锁状态 await；channel 是否有背压；任务是否可取消；panic 是否会杀掉关键流程。
- Minimality：是否能通过更好的数据结构、删除旧路径、复用已有机制或缩小 scope 来少写代码。
- API：public surface 是否最小；跨 crate 边界是否稳定；feature flags 是否清晰；新增 API 是否有真实调用方。
- Tests：行为变更是否有测试；安全修复是否有 corpus；跨平台代码是否至少有 cfg 编译或 smoke 计划。

## 输出格式

先列 findings，按严重度从高到低。不要先写总结。

```text
- [P1] Short title - path/to/file.rs:123
  Explain the concrete bug, when it happens, why it matters, and the smallest credible fix.
```

然后列：

- Open questions，只放会影响合并判断的问题。
- Test gaps 或 validation，说明你运行了什么、没运行什么。
- 如果没有发现问题，明确写“未发现阻塞性问题”，并说明残余风险。

## 审查纪律

- 不为个人偏好阻塞合并；风格问题必须连接到可读性、API 稳定性或维护成本。
- 不要求作者实现本 PR 范围外的新功能。
- 不把已有代码债务算作本改动的问题，除非本改动扩大了风险。
- 不建议大重构，除非当前实现已经无法安全修复。
- 指出代码膨胀时要给出更小的可行实现方向，而不是只说“太复杂”。
- 对安全、隐私、权限和 secret 泄露保持零容忍。
