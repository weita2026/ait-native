# Contributing to ait

Thanks for your interest in contributing to `ait`.

This repository is intentionally **agent-first**. The public contribution path is not “edit random files and hope a maintainer reconstructs intent later.” The expected path is:

1. shape the work in Markdown;
2. sync the relevant Markdown lineage when needed;
3. open an honest `task` / `change` boundary;
4. implement in a bound task worktree;
5. review, attest, and land honestly.

## Who this guide is for

Use this guide if you are:

- an external agent helping a developer shape work into Markdown and workflow state;
- an external contributor preparing a docs/code change for review;
- a developer trying to understand how public contribution work should flow in this repository.

For repo bootstrap and test commands, also read [LOCAL_DEVELOPMENT.md](./LOCAL_DEVELOPMENT.md).

## Read these documents first

Before contributing, read these repository documents in this order:

1. [README.md](../README.md)
2. [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md)
3. [LOCAL_DEVELOPMENT.md](./LOCAL_DEVELOPMENT.md)
4. [SELF_HOSTED_TEAM_DEPLOYMENT.md](./SELF_HOSTED_TEAM_DEPLOYMENT.md)
5. [PACKAGE_TARGETS.md](./PACKAGE_TARGETS.md)
6. [docs/legal/public_release_license_summary.md](./legal/public_release_license_summary.md)
7. [docs/legal/component_license_matrix.md](./legal/component_license_matrix.md)
8. [docs/legal/module_license_map.md](./legal/module_license_map.md)
9. [docs/legal/contributor_rights_policy.md](./legal/contributor_rights_policy.md)

## Current release-facing contribution boundary

Do **not** assume every repository surface is under one identical public license.

Current release-facing posture:

- `ait`, `aitk`, `ait-agent`, `ait_protocol`, `ait_storage`, and `ait_chat` are the current Apache-facing local/baseline/passive surfaces;
- `ait-server` and `ait-worker` are shared-control-plane surfaces with reciprocal/commercial boundaries;
- docs/examples/templates must be treated according to their own published boundary, not as an automatic whole-repository grant.

Use these files when you need the current public boundary instead of guessing:

- [docs/legal/public_release_license_summary.md](./legal/public_release_license_summary.md)
- [docs/legal/public_package_surface_map.json](./legal/public_package_surface_map.json)
- [docs/legal/component_license_matrix.md](./legal/component_license_matrix.md)
- [docs/legal/module_license_map.md](./legal/module_license_map.md)

## The public contribution contract

### 1. Shape work in Markdown first

Before opening implementation work, shape the request in the narrowest honest Markdown surface:

- a repo-root guide/doc when the work is release-facing documentation;
- a `docs/sprints/*.md` artifact when the work needs a sprint slice or plan-backed execution story;
- another focused Markdown artifact when the work belongs to a narrower command-layer surface.

Substantial work should start from explicit Markdown, not only from an implied chat summary.

### 2. Sync authored Markdown when lineage matters

For authored Markdown changes, use `ait plan sync` instead of pretending the first durable step is raw code movement.

Typical forms:

```bash
ait plan sync <file-or-dir>
ait plan sync <file-or-dir> --remote origin
```

Use `--remote` when the Markdown lineage should become shared/durable before the execution step continues.

### 3. Open an honest task/change boundary

When the work is ready to execute, open or continue an honest `task`.

Typical bootstrap:

```bash
ait task start --title "<title>" --intent "<intent>"
```

If the repo/mode is intentionally local-only, keep `workflow_mode=solo_local`
and finish through `land-local` without teaching `--local` as the normal
bootstrap.
If the repo/mode is intentionally `solo_remote`, remote-backed task/change lineage is usually the default.

Do not bind a fresh task to an old or misleading title just to avoid opening a new task.

### 4. Implement inside the bound task worktree

After `ait task start`, follow the printed `cd` command and do implementation work inside the bound worktree.

Rules:

- code/runtime behavior changes belong in the task worktree;
- authored Markdown lineage is usually synced from repo root with `ait plan sync`;
- do not treat repo-root default-line authoring as the normal place for task execution;
- open a new `change` only when the review/ownership boundary actually changes.

### 5. Keep verification and documentation honest

Before asking for remote land:

- run the narrowest tests that honestly cover your change;
- update the nearest user-facing docs when CLI, API, runtime, or workflow behavior changes;
- say explicitly when a broader suite was not run;
- avoid “all green” wording when only a targeted check was run.

### 6. Use the right land path

For shared/remote-backed work, the normal helper path is:

```bash
ait workflow land <change-id> --apply
```

For intentionally local-only work, the normal helper path is:

```bash
ait workflow land-local <change-id>
```

Do not claim shared review/policy/land happened when the work only completed locally.

## Review expectations

Contributors should keep these expectations visible:

- task review comes first: did the slice actually solve the stated problem?
- code review stays honest about risk, tests, and doc coverage;
- if the work changes user-visible behavior, public docs should move with it;
- if the work stays partial, say so instead of presenting a draft as complete.

For larger or riskier changes, discuss the intended slice before opening a broad implementation task.

## AI-assisted contributions

AI assistance is allowed, but contributors must disclose material assistance in the PR/change summary.

Minimum disclosure:

- whether AI assistance was used;
- tool/model if known;
- whether a human reviewed the final result;
- any provenance or license concerns that still need maintainer attention.

Contributors must not submit AI-assisted output that:

- copies from unknown or incompatible sources;
- includes confidential customer/internal material;
- skips human review for non-trivial code or workflow changes.

## Contributor rights and provenance

This repository still follows a provisional contributor-rights posture pending founder/counsel finalization.

Until the final public contribution contract is published:

- do not assume every contribution can be merged into every product surface;
- do not submit third-party code unless you have the right to contribute it;
- do not include secrets, private prompts, customer data, or copied material from non-licensed sources;
- expect maintainers to request explicit contributor-rights confirmation before merging changes into dual-licensed or commercially sensitive surfaces.

See [docs/legal/contributor_rights_policy.md](./legal/contributor_rights_policy.md) for the current draft policy background.

## Where to go next

- shortest public local-only developer path: [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md)
- repo bootstrap, editable install, task-bound worktrees, and test commands: [LOCAL_DEVELOPMENT.md](./LOCAL_DEVELOPMENT.md)
- release-facing package/extraction boundary map: [PACKAGE_TARGETS.md](./PACKAGE_TARGETS.md)
- release-facing workflow overview: [README.md](../README.md)
- current license/package boundary summary: [docs/legal/public_release_license_summary.md](./legal/public_release_license_summary.md)
