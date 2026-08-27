# Contributing to ait-native

Thank you for helping improve AIT. This guide covers the public
`weita2026/ait-native` repository and all five component subtrees published in
one release.

## Repository model

The public repository is a deterministic release monorepo. Each tag combines
five independently governed AIT source authorities:

| Public path | Source authority | License |
| --- | --- | --- |
| `ait-core/` | `ait-core` | Apache-2.0 |
| `ait-server/` | `ait-server` | AGPL-3.0-only |
| `ait-runner/` | `ait-runner` | Apache-2.0 |
| `ait-python/` | `ait-python` | Apache-2.0 |
| `ait-node/` | `ait-node` | Apache-2.0 |

`ait-monorepo-source.json` records the exact source Snapshot for every
subtree. Do not hand-edit that mapping, generated release receipts, checksums,
or license texts. Maintainers regenerate them through the protected release
workflow after an accepted change is admitted to its owning authority.

Read [README.md](README.md) and the centralized
[distribution contract](docs/distribution.md) before changing release or
package behavior. Report a suspected vulnerability through
[SECURITY.md](SECURITY.md), not through a public issue.

## Shortest contribution flow

Fork or clone the repository, enter the checkout, and initialize AIT once:

```sh
ait init
```

Then tell your coding agent the outcome you want, including the affected
component and validation expectations. The generated workflow block in
`AGENTS.md` supplies the exact repository-local commands. A conforming agent
will:

1. read the effective AIT workflow and current Plan/configuration;
2. use `ait blame` before repairing an identified regression;
3. route authored Markdown through `ait plan sync`;
4. use `ait task start` and the bound worktree for governed changes;
5. use standalone `ait snapshot create` for intermediate checkpoints when
   needed and run appropriate validation;
6. finish through the configured `ait task finish` path, adding `--message`
   when dirty local work needs its final Snapshot; and
7. stop when required policy, review, CI, authority, or user direction is
   missing.

Do not invent a project-language mode or manually replace the generated
workflow. AIT uses the same command model for Python, Node.js, .NET, PHP, C,
C++, Java, mixed-language, and non-code repositories.

## Change scope and validation

Keep each contribution focused on one reviewable outcome. Change the owning
component and the nearest tests; update the centralized product document only
when public behavior, installation, compatibility, licensing, or distribution
changes.

Run the component's declared CI or test entrypoint and report exactly what ran.
Do not weaken `build-release.mjs`, rewrite `ait-monorepo-source.json`, or alter
checksums merely to make a modified export appear admitted. A clean release
source build and all protected cross-platform receipts are maintainer-owned
release gates.

A pull request or proposed patch should include:

- the problem and intended behavior;
- the affected component or public root surface;
- tests and commands run, including any skipped validation;
- compatibility, migration, security, and license impact;
- documentation changes for user-visible behavior; and
- material AI assistance, including the tool or model when known and whether
  a human reviewed the final result.

Never submit secrets, customer data, private prompts, copied code without a
compatible license, or material you do not have the right to contribute.

## Review and release admission

Maintainers review behavior, test evidence, source provenance, license scope,
and the AIT Task/Change lineage. An accepted change is admitted to the owning
AIT authority and then re-exported; the resulting public commit may therefore
differ from a contributor's proposal while preserving its reviewed behavior.

Release tags and published package bytes are immutable. Fixes ship in a new
version; maintainers never move an existing tag or replace an existing
registry artifact.
