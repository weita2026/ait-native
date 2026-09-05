# ait-native

[English](README.md) · **[简体中文](README_CN.md)** · [繁體中文](README_ZH.md)

**将多个并行的 coding agent 对话，变成经过验证、可追溯的 Task。**

AIT 是以本地工作为主的 CLI，适合将代码修改交给 agent、并对结果负责的个人开发者与维护者。
它将每个需求关联到独立 worktree、通过检查的确切版本，以及发生回归时需要的历史记录。
使用你自己的 coding agent，由你掌握任务意图与验收。

[![最新稳定版本](https://img.shields.io/github/v/release/weita2026/ait-native?label=stable)](https://github.com/weita2026/ait-native/releases)
[![文档](https://img.shields.io/badge/docs-ait--native.dev-0ea5e9)](https://ait-native.dev/)
[![许可](https://img.shields.io/badge/license-Apache--2.0%20%2B%20AGPL--3.0--only-22c55e)](#license-map)

**[试用第一个 Task](https://ait-native.dev/zh-cn/local-quickstart/#first-task)** ·
[观看演示](https://ait-native.dev/zh-cn/demo/#in-action) ·
[技术文档](https://ait-native.dev/technical/) ·
[获取帮助](https://github.com/weita2026/ait-native/discussions)

[简体中文入门](https://ait-native.dev/zh-cn/local-quickstart/#first-task) ·
[繁體中文入門](https://ait-native.dev/zh-tw/local-quickstart/#first-task)

## 走完一个 AIT 任务

[![AIT Task 实际录制：初始化、开始隔离工作、测试、完成并追溯结果](https://ait-native.dev/public/tour/ait-task-tour.gif)](https://ait-native.dev/zh-cn/demo/#in-action)

这是 AIT 1.1.1 实际命令输出的录制，剪辑成 33 秒的回放，展示一个 Task 如何完成下方可下载的同一个示例。
路径已简化；播放时间不是性能测量。
[观看附字幕与完整文字记录的版本](https://ait-native.dev/zh-cn/demo/#in-action)。

若要了解多个 Task，请查看另外提供的
[并行工作与回归情境示意](https://ait-native.dev/zh-cn/demo/#demo-scene)，
或[以繁体中文标注的工作流说明图](https://ait-native.dev/public/tour/ait-workflow-zh-tw.png)。

## 试用第一个 Task

请从全新的示例文件夹开始。你需要自己的 coding agent，以及执行此小示例所需的 Node.js 22+；
Node.js 并非 AIT 对仓库的通用要求。

**1. 安装 AIT。**

```sh
python -m pip install ait-native==1.1.1
ait --version
```

其他软件包渠道请参阅[安装指南](https://ait-native.dev/zh-cn/local-quickstart/)。

**2. 准备示例。**

[下载示例 ZIP](https://ait-native.dev/downloads/ait-first-task.zip)
及[它的 SHA-256](https://ait-native.dev/downloads/ait-first-task.sha256)。
解压缩后，在解压得到的 `ait-first-task` 文件夹打开终端。
将 `your-name` 换成你希望记录于本地审查的名称。

```sh
node --test tests/baseline.test.mjs
ait init
ait config set --user-name "your-name"
ait snapshot create --message "Start the AIT example"
```

三项基准测试应全部通过。这个初始 Snapshot 会在 Task 开始前记录尚未修改的示例。
若使用现有项目，请改依[入门指南](https://ait-native.dev/technical/getting-started/)操作。

**3. 在 coding agent 中打开示例，并给它以下需求。**

> 阅读 AGENTS.md，遵循此仓库的 AIT 工作流。在 src/tasks.mjs 新增 openTasks(tasks)：
> 返回新的数组，包含 done 属性不等于 true 的任务，保留原有顺序、任务对象及输入数据。
> 保持 taskTitles 正常工作，加入针对性的测试。不要修改 checks/、移除现有测试或新增依赖包。
> 完成 Task 前，执行现有测试及 node checks/first-task.mjs。

Agent 会记录 sprint 项目，在返回的 Task worktree 中实现修改并执行检查。
生成的 `AGENTS.md` 区块会提供该仓库适用的确切命令与完成要求。

**4. 回到原始示例文件夹检查结果。**

```sh
node --test tests/*.test.mjs
node checks/first-task.mjs
```

功能检查会输出 `FIRST_TASK_ACCEPTED`。也请确认实际的 Task finish 结果：Task 已完成、
worktree 已清理，并在适用时关闭绑定的 sprint 项目。功能验收与工作流完成是两项分开的检查。
审阅修改后，再到自己的项目尝试一个小任务。

## 你能得到什么

- **让需求与工作持续相连。** 明确的 Markdown sprint 项目及其 Plan 版本会绑定至 Task。
  上下文压缩后，生成的指示要求 agent 重新阅读该项目。
- **以项目当前的状态完成工作。** 独立 Task 各自拥有 worktree。AIT 会重新检查目标，
  对兼容修改进行 rebase，遇到实质冲突则停止，并在 Task finish 成功后清理工作区。
- **出问题时找到相关上下文。** `ait blame` 将受影响的代码或 Plan 文字关联到已记录的版本与可用工作流历史。
  Agent 可据此诊断问题、做出范围明确的修正，并验证结果。

## 除了 worktree，为什么还需要 AIT？

Worktree 为任务提供独立文件。AIT 则以同一个 Task 生命周期管理需求、版本、适用检查与完成状态。
请和你现有的 agent、Git、issue tracker、CI 及脚本一起评估：

| 你需要回答的问题 | AIT 记录或协调的内容 |
| --- | --- |
| 这个修改对应哪个需求与验收条件？ | 绑定至 Task 的确切 Plan 项目与版本。 |
| 哪个结果通过检查？它能否整合至今天的目标？ | 版本身份、适用证据、目标检查与可恢复的完成状态。 |
| 后来出现的回归从哪里开始？ | 可通过 blame 取得的版本与工作流上下文。未知来源仍标示为未知。 |

实现与验证可以并行；应用至共享目标 Line 前，准入会重新验证并依序处理。
干净的 rebase 仍需要相关验证。Worktree 指示是工作流约束，并非操作系统沙箱；
blame 提供来源记录，不会自动诊断或修复问题。

试用[相同的工作流情境](https://ait-native.dev/zh-cn/demo/)，比较确认意图、审查、整合及调查问题所花的时间。

## 与 Git worktree 的测量比较

两轮分别公开的 200-session 实验，在相同的五个固定游戏开发测试项目上使用 GPT-5.6 Sol 的 max 推理设置。
每种工作负载各有 20 组通过准入的配对尝试。每轮有效结果包含 100 个全新 AIT session 与 100 个全新 Git session，
两组的功能验收结果均为 100/100。

| 实验 | 工作流 | 有效 session | 各工作负载 token 节省比例的中位数（95% CI） | 各工作负载耗时节省比例的中位数 |
| --- | --- | ---: | ---: | ---: |
| 已发布的 1.1.0 基准 | Sprint 禁用 | 100 AIT + 100 Git | **34.95%** (27.85%-39.77%) | **21.04%** |
| 自然查看重复实验 | Sprint 启用 | 100 AIT + 100 Git | **36.28%** (28.26%-41.83%) | **15.22%** |

这些模型供应商 token 的工作负载中位数结果，仅适用于指定模型与固定测试数据。
Session 是依序执行的，因此结果不衡量并行吞吐量，也不保证你的仓库会得到相同节省。

<details>
<summary>方法、排除项目与尚未完成的 Claude Fable 实验</summary>

已发布基准的 AIT 使用 46,300,272 tokens，Git 使用 70,140,925 tokens（减少 33.99%）；
证据历史包含 201 次执行，其中一笔功能结果被排除。
Sprint 启用的重复实验分别使用 45,432,262 与 71,238,660 tokens（减少 36.23%）；
证据历史包含 203 次执行，并披露三项排除记录。
表中的工作负载平衡指标是主要测量值；合并总量仅供描述。

两轮实验共享相同测试数据字节、工作负载矩阵、固定模型，以及对称的只读查看权限。
它们的工作流模式、提示词、受测 AIT 可执行文件、随机种子、日期及恢复历史不同。
这是结果方向相近的重复实验，不是合并观察值，也不是能推论 sprint 开关因果效果的 A/B 测试，
因此不能将 1.33 个百分点的差异归因于 sprint 模式。
结果范围仍限于这些固定测试数据与依序执行的全新 session，不能承诺适用所有工作负载，也未测量高并发执行。

[已发布基准的证据](https://github.com/weita2026/ait-native/tree/v1.1.0/ait-core/release/benchmarks/game-v1-g56s-max-complete200-fx27-20260826) ·
[Sprint 启用重复实验的证据](https://github.com/weita2026/ait-native/tree/benchmark-sprint-on-20260829/ait-core/release/benchmarks/game-v1-g56s-max-sprint-on-natural-complete200-20260828)

### Claude Fable 评测——仍在进行

我们正在执行一轮设置已固定的 200-session 评测，比较 AIT 的任务导向工作流与 agent 自行管理的本地 Git worktree，
两组使用相同的五种游戏开发工作负载。

**进度：22 / 200 个 session**

当前观察到的 22 次执行全部有效且通过验收，没有切换至备用模型。
实验尚未完成，仍为 `claim_eligible=false`。
无论剩余结果偏向 AIT 或 Git，我们都会继续完成全部 200 个 session。

最新的平衡检查点为 20/200，每种工作负载各有两组完整 AIT/Git 配对：

| 工作负载 | 有效配对 | AIT token 节省比例 | Bootstrap CI95 |
| --- | ---: | ---: | ---: |
| GD-01 | 2 | 20.32% | 9.13% to 32.72% |
| GD-02 | 2 | -4.35% | -18.16% to 14.35% |
| GD-03 | 2 | 37.56% | 6.57% to 52.02% |
| GD-04 | 2 | 23.77% | 23.50% to 23.97% |
| GD-05 | 2 | 2.13% | -26.89% to 25.93% |

各工作负载 token 节省比例的中位数为 **20.32%**，整体 bootstrap CI95 为 **6.57% to 25.93%**。
此检查点的全部 20 次执行均有效且通过验收，没有统计排除，也没有切换至备用模型。
每种工作负载仅有两组配对，部分区间仍很宽或跨越零，因此公布这些中期数字是为了透明披露，并非产品成效宣称。

</details>

## 我为什么打造 AIT

<details>
<summary>促使我打造 AIT 的六个问题</summary>

1. **AI agent 经常产出一个巨大的 commit，却很难看出它代表什么工作。**

   Agent 可能修改数十个文件，再将所有内容塞进同一个 commit。
   Commit 看得出改了什么，却不清楚说明 agent 想完成哪件工作。
   我希望历史记录围绕有意义的任务组织，而非围绕 agent 刚好存下工作的时点。

2. **Sprint 卡应该对应到真正的工程工作。**

   我想要类似 Jira 的工作流：开一张 sprint 卡，就启动一个真正隔离的任务；
   完成任务，应代表问题确实获得解决。Ticket、agent、代码、验证与最终结果，应属于同一个生命周期。

3. **传统 Git 工作流是围绕人的行为设计的。**

   人通常先做一个小修改，再审查、暂存、commit、rebase，然后继续下一步。
   在 vibe coding 时代，agent 产出任务规模修改的速度更快。
   每次都替每个 agent 重复这些手动 Git 操作，开始成为工作阻力。

4. **Markdown 应该不只是仓库里的另一个文件。**

   Markdown 可能是人与 agent 最好的共同语言。Git 可以存储 Markdown，
   却不理解一个清单项目代表计划、任务或验收条件。
   我希望写在 Markdown 里的意图，能持续关联到实现它的代码。

5. **当 agent 弄坏东西时，我希望很快得到答案。**

   我不想为了理解一个回归，翻找旧对话、零散 commit 与互不相连的 ticket。
   AIT 将任务、版本、验证、agent 上下文与 Task finish 历史连在一起，
   让 `ait blame` 能从出错的代码行追溯至引入问题的工作。

6. **命令优先为 agent 设计。**

   CLI 的设计重点，不是让人反复输入时觉得顺手，而是让 agent 难以误解：
   稳定的命令、明确的状态、结构化结果、确切的工作区、清楚的失败消息，以及明确的下一步。
   人仍然决定意图、审阅结果，并承担后果。

</details>

## 搭配现有工具使用

AIT 不限定仓库使用哪种语言，也不尝试检测项目类型；构建、测试与忽略规则都来自你的仓库。
Coding agent 负责实现及选定的检查；AIT 管理任务生命周期，并执行适用的工作流条件。

`ait init` 建立本地 `.ait` 权威数据，并生成仓库的 `AGENTS.md` 工作流区块。
该生成的区块是有效命令的依据；本地工作不需要执行中的 `ait-server`。

AIT 有两种工作流默认模式：`solo_local` 将工作与 Task finish 保留在本地；
`solo_remote` 加入明确选定的服务器与经审查的完成流程。
Agent 根据生成的指示执行 `ait task start`、以 `ait snapshot create` 保存中途检查点、
以 `ait plan sync` 保存 Markdown 沿革，并使用适用的 `ait task finish` 或 `ait workflow finish` 收尾。

- [Git 导入、导出与退出路径](https://ait-native.dev/technical/cli/reference/git/)
- [功能工作流](https://ait-native.dev/technical/workflows/feature/)与[回归修复](https://ait-native.dev/technical/workflows/regression/)
- [组件](https://ait-native.dev/zh-cn/components/)与[发行状态](https://ait-native.dev/zh-cn/proof/)

当前公开版本：**v1.1.1**。如需该版本的确切源代码，请使用不可变的发行标签；
`ait-monorepo-source.json` 记录组件 Snapshot 映射。两次发行之间，`main` 的文档可能继续更新。

## 各安装渠道提供什么

<details>
<summary>软件包内容与历史版本差异</summary>

| 渠道 | 安装内容 |
| --- | --- |
| PyPI `ait-native` | `ait`、默认不启动的 `ait-server`，以及直接 `ait-python` 绑定。 |
| npm `@wa120/ait-native` | `ait` 及直接在进程内运作的 Node-API 绑定；不安装 `ait-server`。 |
| Homebrew 与 WinGet | 1.1.1 产品组合包含原生 `ait`、`ait-server` 与 `ait-runner`。安装时不启动任何后台进程；渠道可用性请查阅发行状态。 |
| APT | 在 1.1.1，`ait-native` 拥有全部三个命令；`ait-runner` 是仅声明依赖关系的过渡别名。软件包内的服务仍仅适用服务器。 |
| OCI | 分开提供 `ait-server` 与 `ait-runner` 镜像。 |
| GitHub Release | 绑定校验码的原生压缩包，以及各声明渠道使用的软件包资产。 |

不可变的 1.1.0 Homebrew、apt 与 WinGet 产品软件包包含 `ait`／`ait-server` 组合，apt 另外提供 runner。
这项历史例外保持不变。现行渠道操作请参阅[安装指南](https://ait-native.dev/zh-cn/local-quickstart/)，
确切资产请查阅[发行页面](https://github.com/weita2026/ait-native/releases)。

</details>

## 从 0.x 升级

1.x 没有 `ait install` 命令。请通过软件包管理工具升级，并以 `ait --version` 确认版本。
保留现有 `.ait` 权威数据，迁移前先查看确切版本的转换指示。
建立新的权威数据与升级现有历史，是不同的操作。
请参阅[转换契约](ait-core/docs/distribution.md#public-0x-to-10-transition)
与[Git 退出参考](https://ait-native.dev/technical/cli/reference/git/)。

## 构建这份源代码

<details>
<summary>原生源码构建与语言绑定</summary>

在 macOS 或 Linux 的干净 checkout 中执行：

```sh
./build-release.sh
```

在 Windows PowerShell 中执行：

```powershell
.\build-release.ps1
```

构建会在 `dist/source-build/` 生成本地原生命令、直接 PyO3 Python wheel、
可移植的 JS/TS 软件包，以及当前主机适用的直接 Node-API addon。
这些源码构建产物及其凭据明确标示为不可发布；受保护的发行 CI 只会提升另外通过准入的家族产物。

在 Node.js，`import { NativeRuntime, AgentClient } from "@wa120/ait-native"`
会在当前进程加载软件包自有的 `native/ait_napi.node`。
npm 的 `ait` 命令通过 `NativeRuntime.runCli()` 调用同一个 Rust 绑定，不会寻找或启动子可执行文件。

</details>

## 分享结果或获取帮助

[提问或分享工作流](https://github.com/weita2026/ait-native/discussions)，
或[报告问题](https://github.com/weita2026/ait-native/issues/new/choose)。
请告诉我们第一个 Task 是否完成、在哪里需要协助，以及是否能在另一个任务重复完成此工作流。
只分享你可以公开的信息；提供反馈不需要私有仓库。

<a id="license-map"></a>

## 许可对照

根目录的 [`LICENSE`](LICENSE) 已明确说明：根层发行控制、文档、`ait-core/**`、
`ait-runner/**`、`ait-python/**` 与 `ait-node/**` 采用 Apache-2.0。
唯一的组件例外为 `ait-server/**`，采用 AGPL-3.0-only。
各组件子树保留其确切的 `LICENSE` 与 `NOTICE`；捆绑发行不会变更任一组件的许可。
公开 1.0 源代码路径不适用商业或专有许可。

完整的软件包、源代码、构建与许可契约位于 [`docs/distribution.md`](docs/distribution.md)。
