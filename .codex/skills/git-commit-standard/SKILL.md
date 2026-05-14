---
name: git-commit-standard
description: "Git 提交规范 skill。Use when preparing, staging, writing, reviewing, splitting, or committing Git changes in this repository; distills mature open-source practices for atomic, bisectable, auditable, signed-off commits and precise commit messages."
---

# Git Commit Standard

## 核心原则

像成熟开源项目维护者一样提交：每个 commit 必须小、完整、可审查、可二分、可回滚。优先表达「为什么改」和「行为边界」，不要把提交当成文件快照。

默认采用 Conventional Commits；如果仓库将来形成自己的历史风格，以仓库已有风格为准。

## 提交流程

1. 先读工作区。
   - 运行 `git status --short`、`git diff`、`git diff --stat`。
   - 识别用户已有改动，绝不把无关文件混入当前提交。
   - 如果同一文件里混有无关改动，只 stage 与当前任务相关的 hunk。

2. 定义一个逻辑变更。
   - 一个 bug 修复、一个功能切片、一组测试补充或一份文档更新可以是一个 commit。
   - 行为变更和无关重构分开。
   - 格式化噪音和逻辑修改分开，除非格式化只影响刚改的代码。

3. 提交前验证。
   - Rust 代码默认考虑 `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-targets`。
   - 只改文档时检查链接、标题、示例命令和验收口径。
   - 如果项目尚未初始化 Cargo workspace，明确记录验证未运行的原因。

4. 有意 stage。
   - 使用路径级或 hunk 级 staging，避免 `git add .` 把缓存、日志、构建产物、真实 `.env`、token 或用户草稿带进去。
   - stage 后运行 `git diff --cached` 和 `git diff --cached --stat`。
   - staged diff 必须能单独说明这个 commit 的意图。

5. 写消息后再 commit。
   - subject 使用祈使语气或动作短语，英文小写开头，不以句号结尾。
   - subject 建议不超过 72 字符；commit message 任意单行硬上限为 80 字符。
   - 改动很小且意图从 subject 就能完全理解时，可以只写一行 commit header，再由 `-s` 自动追加 sign-off。
   - 改动较大、涉及行为/架构/安全/API/跨平台，或这个改动本身有长期意义时，必须写 commit body。
   - body 采用内核式说明：先讲问题和动机，再讲方案，再讲影响、风险、迁移或验证；不要复述 diff，并按 80 字符以内换行。
   - 需要破坏性变更时使用 `!` 和 `BREAKING CHANGE:` footer。
   - 每个提交必须使用 `git commit -s`，生成 `Signed-off-by` trailer。
   - 不要手写伪造他人的 sign-off；使用当前 Git identity，必要时先让用户修正 `user.name` / `user.email`。

## 消息格式

小改动：

```text
<type>(<scope>): <subject>

Signed-off-by: Name <email@example.com>
```

较大或重要改动：

```text
<type>(<scope>): <subject>

<problem and motivation>

<solution and behavior>

<impact, risk, migration, or validation>

Signed-off-by: Name <email@example.com>
<other footers>
```

body 写法要求：

- 第一段解释为什么需要这个改动，读者不看 diff 也能理解背景。
- 后续段落解释实现选择、边界、兼容性、安全或性能影响。
- 如果有测试或验证，把关键验证放在末尾一段。
- 不写空泛句子，例如 “update files” 或 “improve code”；用具体事实。
- 每一行都必须不超过 80 字符，长句手动换行，不依赖 Git 或编辑器自动折行。

常用 type：

- `feat`: 新功能或用户可见能力。
- `fix`: bug 修复、兼容性修复、安全修复。
- `docs`: 文档、计划、设计说明。
- `test`: 测试、测试工具、fixtures。
- `refactor`: 不改变行为的结构调整。
- `perf`: 性能优化。
- `build`: Cargo、依赖、打包、构建脚本。
- `ci`: CI、检查、发布流水线。
- `chore`: 维护性杂项，谨慎使用。

Noctrail scope 建议来自 crate、模块或职责边界，例如 `terminal-core`、`pty`、`renderer`、`ui`、`agent`、`policy`、`config`、`docs`。

## 好的提交示例

```text
docs(plan): add Noctrail product roadmap
feat(pty): add Windows ConPTY resize adapter
fix(policy): redact tokens in agent audit logs
test(terminal-core): cover alternate screen resize
refactor(renderer): isolate glyph cache eviction policy
```

## 硬性拒绝条件

不要创建 commit，除非用户明确要求继续，并在结果中说明风险：

- staged diff 含有 secret、真实凭据、私钥、生产 token 或真实 `.env`。
- staged diff 混入无关用户改动。
- 相关测试明显失败且原因未解释。
- commit 依赖未提交的本地状态才能编译或运行。
- commit 命令没有使用 `-s`，或最终消息缺少当前提交者的 `Signed-off-by` trailer。
- commit message 存在超过 80 字符的单行。
- 需要 `--amend`、rebase、reset、force push 等历史改写，但用户没有明确授权。

## 完成报告

提交完成后报告：

- commit SHA 和 subject。
- 确认提交包含 `Signed-off-by` trailer。
- 实际进入 commit 的文件。
- 已运行的验证命令及结果；未运行的验证要说明原因。
