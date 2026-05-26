# ait

## Why `ait` exists

`ait` grew out of a simple problem in AI-heavy development: once several agent
sessions are changing code at the same time, plain commits stop being a good
unit of coordination. You can still keep Git around the edges, but inside the
daily loop you need tasks, checkpoints, reviewable packets, and clean
workspaces that match how the work is actually happening.

In a vibe coding workflow, the hard problem is no longer only "can the model
change files?" It is "can the work still be managed once agents start changing
files in parallel?" A single AI session can produce a useful but
commit-shaped bundle of code, tests, docs, and reasoning. Hand-splitting that
bundle into step-by-step commits often hides the real management boundary: the
task or ticket that caused the work to exist.

`ait` makes that task the core execution object. Each task can carry intent,
dependencies, agent session history, checkpoints, snapshots, patchsets,
reviews, policy evidence, and land state. That gives AI-generated work a
durable shape: not just a diff, but a traceable unit that can be inspected,
rebased, reviewed, merged, or blocked.

`ait` brings these pieces together:

- Git-like storage, compression, and revision identity, while keeping Markdown
  planning lineage separate from task-shaped execution records;
- Jira-like code management, where work is task-driven, dependencies form a
  task DAG, and multiple task outputs can converge through governed patchsets;
- parallel AI execution, so one developer can open many conversation sessions,
  dispatch different DAG nodes, and track outputs without manually turning
  every node into its own branch-and-commit trail;
- multi-device task dispatch through communication surfaces such as Telegram
  and Discord, including multiple tokens or sessions for simultaneous routing;
- isolated task worktrees, so each dispatched unit runs in its own clean
  workspace and other task worktrees can be rebased by agents when one task
  lands;
- `ait-server` as the remote coordination layer for multi-repo storage,
  CI, AI code review, policy evidence, and publication or remote land.

The result is Git-like discipline with Jira-like task control for the AI era:
plan the work, dispatch tasks, preserve provenance, produce review-ready
patchsets, and land only through explicit review, attestation, policy, and land
gates.

`ait` is an **agent-first, Markdown-first workflow system** for turning intent into governed execution.

Instead of starting with a pile of manual workflow objects, you start by stating the need. An agent helps shape that need into Markdown, converges it into an honest task or compact DAG, and carries it through the appropriate land path with review, attestation, policy, and workflow boundaries kept explicit.

Official website:

- https://ait-native.dev

## What You Can Do

With `ait`, one person can coordinate AI work more like a lead developer than a
prompt dispatcher:

- open multiple agent sessions through communication tools such as Discord or
  Telegram, and dispatch parallel tasks into the same repository without
  turning every session into a manual Git branch-management exercise;
- merge code through compact, task-aware packets that spend fewer tokens on
  repeated context, without first building a custom harness or a separate agent
  automation platform;
- manage agents the way a lead manages a team: assign task-oriented work,
  review each completed task as a meaningful progress unit, and decide what is
  ready to land;
- keep history and task management readable, so code changes, decisions,
  reviews, and Jira-like workflow state remain traceable after the AI sessions
  are gone.

## If you know Git

These are mental-model mappings, not one-to-one command replacements:

| Git habit | `ait` shape | Difference |
| --- | --- | --- |
| `git commit` | `ait snapshot create` | A snapshot records a workflow revision tied to task/change context. |
| branch per feature | task-bound worktree | Work is isolated by task, not only by branch. |
| pull request | patchset | Patchsets carry review, provenance, and policy evidence. |
| issue or ticket outside Git | `ait task` | The task is part of the execution model, not external bookkeeping. |
| rebase after another branch lands | task worktree rebase | Other task worktrees can be reconciled after one task lands. |
| CI before merge | attestation, policy, and land gates | CI is one evidence source inside a broader governed land path. |

## Why compact task DAGs can save tokens

The benchmark-backed idea is simple: long AI coding tasks often waste tokens by
reloading the same broad context every time work moves to the next step.
A compact task DAG can separate shared planning context from node-specific
execution context, then give each worker only the intent, dependencies,
acceptance criteria, prior checkpoint, and relevant files it needs.

Under the current benchmark boundary, measured long-DAG prompt packets showed
about `65%` median token savings, reported more precisely as `65.89%` in the
governing token-economics notes. Treat that as a benchmark-derived engineering
target, not a universal workflow promise. The stricter benchmark conclusions are
careful about scope: the strongest evidence is for compact worker-only
`ait_dag` packets on long planning or contract workloads, not for every task
class, every model, or naive one-session-per-node physical fan-out.

The theoretical basis is:

- avoid replaying the full conversation or whole repository context for every
  step;
- compile the plan once, then route compact node packets through the task DAG;
- resume from checkpoints instead of restating all prior reasoning;
- measure token usage by node and compare only runs with comparable quality
  gates.

There is still prompt-engineering work to do before this becomes a reliable
default execution path. `ait` needs better prompt-packet compilers,
dependency-aware handoff templates, context selection, per-node token budgets,
provider-usage import, and quality gates that prove savings without hiding
review risk.

Benchmark evidence starts here:

- [docs/benchmarks/README.md](./docs/benchmarks/README.md)
- [docs/benchmarks/task_dag_token_savings_strict_conclusion_20260504.md](./docs/benchmarks/task_dag_token_savings_strict_conclusion_20260504.md)
- [docs/benchmarks/m3_one_hour_sprint_20_node_strict_conclusion_20260504.md](./docs/benchmarks/m3_one_hour_sprint_20_node_strict_conclusion_20260504.md)

## Start here

- [Official website](https://ait-native.dev)
- [docs/LOCAL_QUICKSTART.md](./docs/LOCAL_QUICKSTART.md) - shortest local-only path
- [docs/SELF_HOSTED_TEAM_DEPLOYMENT.md](./docs/SELF_HOSTED_TEAM_DEPLOYMENT.md) - self-hosted shared control plane
- [docs/PACKAGE_TARGETS.md](./docs/PACKAGE_TARGETS.md) - package targets and install boundaries
- [docs/legal/public_release_license_summary.md](./docs/legal/public_release_license_summary.md) - release-facing license posture
- [docs/PUBLIC_DOCTRINE.md](./docs/PUBLIC_DOCTRINE.md) - governed public operating model

---

## What `ait` is

`ait` is built around one core idea:

> **Markdown is the planning surface, and governed workflow objects are the execution surface.**

In practice, that means:

- you describe the work;
- an agent shapes that work into the right Markdown artifact;
- the Markdown is refined until it is executable;
- the agent opens the right task or compact DAG;
- the work lands through the right local or shared workflow path.

This repository is intentionally **agent-first**. Humans still decide intent, scope, and acceptance, but the agent is the primary workflow-shaping and execution companion.

---

## Who `ait` is for

`ait` is aimed at developers, contributors, and teams who want:

- a workflow that starts from authored intent instead of ad hoc execution;
- explicit planning lineage before implementation begins;
- a clean path from Markdown to task execution to land;
- a local-first trust layer for the first workflow loop;
- a shared control-plane path when review, policy, jobs, or browser visibility must be shared.

---

## Surface map

`ait` currently spans these cooperating surfaces:

| Surface | Role |
| --- | --- |
| `ait` | local trust-layer CLI for repo workflow, Markdown, tasks, snapshots, and `land-local` |
| `ait-agent` | transport/runtime helper for Telegram and other external interaction surfaces |
| `ait-server` | shared workflow authority for review, policy, landing, sessions, and coordination |
| `ait-worker` | background worker for the shared control plane |

A useful first mental model is:

- **`ait`** proves the first local workflow loop;
- **`ait-agent`** connects external surfaces to the workflow runtime;
- **`ait-server` + `ait-worker`** add shared workflow authority;

---

## Two common ways to start

### 1. Local-only first loop

Use this when you want to prove the workflow on one repository without shared infrastructure.

Typical shape:

```text
intent
-> Markdown
-> task
-> snapshot
-> land-local
```

Start here:

- [docs/LOCAL_QUICKSTART.md](./docs/LOCAL_QUICKSTART.md)

### 2. Self-hosted shared control plane

Use this when you need shared review, shared policy, shared jobs, or team-visible workflow authority.

Typical shape:

```text
intent
-> Markdown
-> task / change
-> review / attestation / policy
-> remote land
```

Start here:

- [docs/SELF_HOSTED_TEAM_DEPLOYMENT.md](./docs/SELF_HOSTED_TEAM_DEPLOYMENT.md)
- [docs/COMPATIBILITY_MATRIX.md](./docs/COMPATIBILITY_MATRIX.md)

---

## How the workflow usually feels

The practical workflow rhythm is not "memorize everything first."

It is closer to this:

1. explain the work to the agent;
2. let the agent shape it into the narrowest honest Markdown artifact;
3. refine the Markdown until the scope, acceptance, risks, and dependencies are clear;
4. sync the planning lineage;
5. let the agent open the right execution unit;
6. carry the work through the right land path.

Typical developer prompts look like:

- "Shape this into the right Markdown artifact before opening a task."
- "Refine this until it is executable."
- "Open a local task and take it to land."
- "Do not open a single-path task yet; turn this into a compact DAG first."

---

## Compact DAGs are for complex work, not paperwork inflation

When a slice is genuinely multi-step and dependency-aware, `ait` can converge it into a **compact DAG**.

The point is not to explode every node into its own long-lived shared artifact by default.
The point is to:

- make dependencies explicit;
- distinguish ready work from blocked work;
- preserve one honest converged landing focus by default;
- keep graph-aware execution visible without turning bookkeeping into the center of the workflow.

A useful summary is:

> **Complex work may begin as a compact DAG, but the default goal is still one honest converged land path unless the developer or a real risk boundary requires a split.**

---

## Minimal command map

### Plan and Markdown stage

```bash
ait plan sync <file-or-dir>
```

### Small local task

```bash
ait task start --local --title "<title>" --intent "<intent>"
ait snapshot create --message "<checkpoint>"
ait workflow land-local <change-id>
```

### Shared/self-hosted path

```bash
ait task start --title "<title>" --intent "<intent>"
ait workflow land <change-id>
```

### Compact DAG execution

```bash
ait plan execute <plan-id> --from-json <task-graph-json> --auto-compact-worker --yes
```

Read-only DAG surfaces still exist when you explicitly need inspection, but
`ait plan execute` is the main path.

For broader inventory and readiness:

```bash
ait queue summary
ait task audit <task-id>
ait workflow land <change-id>
```

---

## Documentation map

If you are exploring the public surface of this repository, these are the main entry points:

- [docs/LOCAL_QUICKSTART.md](./docs/LOCAL_QUICKSTART.md) - shortest public local-only path
- [docs/CONTRIBUTING.md](./docs/CONTRIBUTING.md) - contributor workflow contract
- [docs/LOCAL_DEVELOPMENT.md](./docs/LOCAL_DEVELOPMENT.md) - editable install, task worktrees, tests, and repo-local development
- [docs/SELF_HOSTED_TEAM_DEPLOYMENT.md](./docs/SELF_HOSTED_TEAM_DEPLOYMENT.md) - self-hosted shared control plane path
- [docs/COMPATIBILITY_MATRIX.md](./docs/COMPATIBILITY_MATRIX.md) - supported combinations and upgrade direction
- [docs/PACKAGE_TARGETS.md](./docs/PACKAGE_TARGETS.md) - package and extraction boundaries
- [docs/PUBLIC_DEMO_DATA.md](./docs/PUBLIC_DEMO_DATA.md) - public proof-point bundle and claim boundary
- [docs/PUBLIC_DOCTRINE.md](./docs/PUBLIC_DOCTRINE.md) - governed public operating model
- [docs/AIT_WHITEPAPER_DRAFT.md](./docs/AIT_WHITEPAPER_DRAFT.md) - longer release-facing narrative draft
- [docs/WHITEPAPER_PATENT_GATE.md](./docs/WHITEPAPER_PATENT_GATE.md) - publication gate for the longer narrative

---

## Licensing and website boundary

At the public surface level, the current Apache-facing components are:

- `ait` CLI
- `ait-agent`
- `aitk` local history-browser companion

The repository root `LICENSE` also covers narrow passive implementation roots
used by those surfaces. Those package roots, such as `src/ait_protocol/**`,
`src/ait_storage/**`, and `src/ait_chat/**`, are implementation seams rather
than separate public surfaces. Use the module map for path-level license
details.

This does **not** make the whole repository Apache-2.0.

Current release-facing boundary summary:

- `ait`, `ait-agent`, and `aitk` are the current public Apache-facing local/runtime surfaces;
- passive helper roots such as `ait_protocol`, `ait_storage`, and `ait_chat` are package seams documented in the module map, not top-level developer-facing categories;
- `ait-server` and `ait-worker` are reciprocal/commercial surfaces;
- `src/ait_native/**` is a compatibility surface and must be read through the narrower module mapping;
- `site/**` does not receive a blanket open-source software grant merely because it lives in the repository.

For the current release-facing boundary, read:

- [docs/legal/public_release_license_summary.md](./docs/legal/public_release_license_summary.md)
- [docs/legal/component_license_matrix.md](./docs/legal/component_license_matrix.md)
- [docs/legal/module_license_map.md](./docs/legal/module_license_map.md)
- [site/LICENSE](./site/LICENSE)
- [docs/TRADEMARK_POLICY.md](./docs/TRADEMARK_POLICY.md)

The official website domain is:

- https://ait-native.dev

That website/domain identity is part of the release-facing boundary and should be read together with the trademark policy rather than inferred from code-license files alone.

---

## Contributing

If you want to contribute, start with:

- [docs/CONTRIBUTING.md](./docs/CONTRIBUTING.md)
- [docs/LOCAL_DEVELOPMENT.md](./docs/LOCAL_DEVELOPMENT.md)

The expected public contribution posture is:

1. shape work in Markdown first;
2. sync planning lineage when needed;
3. open an honest task / change boundary;
4. implement in the bound worktree;
5. review, attest, and land honestly.

<!-- ait-release-notes:start -->
## Release Notes

### v0.10.5

Initial published release for this profile. Task-based delta notes start after the first published baseline.

<!-- ait-release-notes:end -->
