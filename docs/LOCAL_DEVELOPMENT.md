# ait Local Development Guide

## Purpose

Use this guide when you want to develop `ait` in a local checkout with enough public context to bootstrap, run targeted tests, and contribute safely.

If you want the shortest developer-facing step-by-step path first, read [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md) before this guide.

This guide is intentionally broader than the public quickstart but still narrower than a future self-hosted deployment guide.

## 1. Requirements

Minimum baseline:

- Python `>=3.11`
- `pip` (or another way to perform an editable install)
- a local shell environment where you can run `pytest`

Optional extras:

- `.[test]` for contributor test work
- `.[postgres,test]` if you need PostgreSQL-backed server/runtime paths

The local `ait` CLI does **not** require Docker, PostgreSQL, `ait-server`, or `ait-worker` just to develop the local trust layer.

## 2. Editable install bootstrap

From the repository root:

```bash
python3 -m venv .venv
. .venv/bin/activate
python3 -m pip install --upgrade pip
python3 -m pip install -e .[test]
```

If you need PostgreSQL-backed runtime coverage too:

```bash
python3 -m pip install -e .[postgres,test]
```

Sanity checks:

```bash
ait --help
python3 -m pytest tests/test_public_contributor_workflow_contract.py -q
```

## 3. Choose the right operating path

### Stay local-only when:

- the work should stay private;
- you are validating the local trust layer;
- you do not need shared review/policy/remote land;
- you want the shortest repo-local contributor loop.

### Use remote-backed `solo_remote` workflow when:

- the task/change lineage should be shared from the start;
- you need remote-backed review/policy/land;
- Markdown lineage and execution should converge through the repository's shared workflow path.

### Reach for shared/self-hosted surfaces only when needed

If you need `ait-server`, `ait-worker`, PostgreSQL, or the shared developer-managed stack, you are moving beyond simple repo-local development.

This repository intentionally does **not** carry deployment bootstrap commands or stack assets anymore. Start with:

- [docs/ait_native_runtime_operations.md](./ait_native_runtime_operations.md)
- the sibling deployment repository at `../ait_docker` on the same machine when you need shared stack lifecycle control via `../ait_docker/ait-docker.sh`

## 4. Normal contributor loop

### Step A — shape the work in Markdown

Before implementation, shape the work in the narrowest honest Markdown surface.
For authored Markdown lineage, sync it explicitly:

```bash
ait plan sync <file-or-dir>
ait plan sync <file-or-dir> --remote origin
```

### Step B — open a task

Typical bootstrap:

```bash
ait task start --title "<title>" --intent "<intent>"
```

For the taught `solo_local` path, keep `workflow_mode=solo_local` and use the
same plain bootstrap:

```bash
ait task start --title "<title>" --intent "<intent>"
```

Treat `--local` as an explicit override, not the normal solo-local teaching
surface.

Follow the printed `cd` command after `task start`. The bound task worktree is the normal authoring surface for code/runtime changes.

### Step C — implement in the bound worktree

Inside the task worktree:

- keep code/runtime changes there;
- run targeted tests first;
- keep docs changes honest and synced from repo root when they are authored Markdown lineage;
- avoid using the repo-root default line as the main code-authoring surface.

### Step D — verify honestly

Examples:

```bash
python3 -m pytest tests/<target>.py -q
python3 -m pytest -q
```

Use the narrowest honest command first. If you only ran targeted coverage, say so.

### Step E — land using the correct path

Remote/shared helper path:

```bash
ait workflow land <change-id> --apply
```

Local-only helper path:

```bash
ait workflow land-local <change-id>
```
