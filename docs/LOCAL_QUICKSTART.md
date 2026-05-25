# ait Local-Only Quickstart

## What this quickstart is for

Use this guide when:

- you are an external developer working with an external agent;
- you want the first real `md -> task -> land` loop;
- you want that loop to stay **local-only**.

This guide is intentionally **not** the self-hosted path. If you need `ait-server`, `ait-worker`, PostgreSQL, or a shared control plane, stop here and use [SELF_HOSTED_TEAM_DEPLOYMENT.md](./SELF_HOSTED_TEAM_DEPLOYMENT.md).

If you are on macOS and want the public convenience installer instead of a
repository checkout first, use [HOMEBREW_TAP.md](./HOMEBREW_TAP.md).

## 1. Prerequisites

Minimum baseline:

- Python `>=3.11`
- `pip`
- a local shell environment where you can run `pytest`

Create and activate a virtualenv from the repository root:

```bash
python3 -m venv .venv
. .venv/bin/activate
python3 -m pip install --upgrade pip
python3 -m pip install -e .[test]
```

Sanity check:

```bash
ait --help
ait status --json
```

Expected success signals:

- `ait --help` prints the CLI help instead of a missing-command or import error;
- `ait status --json` returns structured repository status.

## 2. Ask the agent to shape the work in Markdown first

Before implementation, ask the agent to shape the work into the narrowest honest Markdown artifact.

Typical developer prompts:

- "Shape this request into the right Markdown first; do not open a task yet."
- "Help me decide which Markdown file should own this request, then make it ready for implementation."
- "If this needs a sprint artifact, write it in `docs/sprints/<topic>.md` first."

Typical outcome:

- one focused Markdown artifact;
- clearer scope / acceptance / risks;
- enough structure to decide whether the slice is ready for a task.

## 3. Sync the Markdown lineage locally

When the Markdown is ready, sync it locally first:

```bash
ait plan sync <file-or-dir>
```

Examples:

```bash
ait plan sync docs/sprints/<topic>.md
ait plan sync README.md
```

Expected success signals:

- `status: ok`
- a created or updated plan revision
- no requirement to use `--remote` if you are intentionally keeping this loop local-only

If you need a shared/remote-backed path instead, stop using this guide and switch to the later remote/shared workflow.

## 4. Open a local task in `solo_local`

This quickstart assumes the local teaching path is already using `solo_local`,
so the normal local-only bootstrap stays on the plain `ait task start` surface:

```bash
ait task start --title "<title>" --intent "<intent>"
```

Expected success signals:

- `task_id` and `change_id` are created;
- the output includes a `cd` command for the bound task worktree;
- the task/change stay local-only for this slice because the workflow mode is `solo_local`.

Important: after `task start`, switch into the bound worktree before doing code/runtime edits.

## 5. Implement and verify inside the bound worktree

Inside the bound worktree:

- make the code/runtime/doc change that matches the Markdown scope;
- run the narrowest honest tests first;
- update the nearest user-facing docs if behavior changed;
- say explicitly when broader suites were not run.

Typical verification commands:

```bash
python3 -m pytest tests/<target>.py -q
python3 -m pytest -q
```

Choose the narrowest honest command that matches the slice.

## 6. Land locally

When the local slice is ready, use the local land helper:

```bash
ait workflow land-local <change-id>
```

After land, verify the result:

```bash
ait task audit <task-id>
ait status --json
```

Minimum success criteria:

- the task audit shows the linked change landed on the target line;
- the workspace returns to `clean`;
- the slice completed without needing server/worker/PostgreSQL services.

## 7. Most likely bootstrap failures

### A. You are not actually in `solo_local`

Symptom:

- `task start` follows the repository's remote-backed default in `solo_remote`.

Fix:

- set `ait config set --workflow-mode solo_local`, then reopen the slice with the same plain `ait task start ...` bootstrap.
- do not teach `--local` as the normal `solo_local` quickstart command.

### B. You kept working in repo root instead of the bound worktree

Symptom:

- root guards appear;
- later workflow commands complain about the active bound worktree.

Fix:

- use the printed `cd` command from `ait task start` and continue there.

### C. You skipped Markdown shaping/sync

Symptom:

- the implementation starts before the task intent is clear;
- later docs/task wording drifts from the actual change.

Fix:

- go back, shape the work in Markdown, and run `ait plan sync <file-or-dir>` first.

### D. You expected server/web/PostgreSQL for the first loop

Symptom:

- you start trying to bootstrap external shared services even though the goal is only a local-only slice.

Fix:

- stay on the local trust-layer path here; only move to shared/self-hosted docs when your task truly needs those surfaces.

## 8. Where to go next

- broader contribution contract: [CONTRIBUTING.md](./CONTRIBUTING.md)
- repo-local development details: [LOCAL_DEVELOPMENT.md](./LOCAL_DEVELOPMENT.md)
- self-hosted shared control plane path: [SELF_HOSTED_TEAM_DEPLOYMENT.md](./SELF_HOSTED_TEAM_DEPLOYMENT.md)
- public One-Hour Sprint demo-data pack: [PUBLIC_DEMO_DATA.md](./PUBLIC_DEMO_DATA.md)
- runtime/deployment routing: [docs/ait_native_runtime_operations.md](./ait_native_runtime_operations.md)
- root overview and agent-facing collaboration rhythm: [README.md](../README.md)
