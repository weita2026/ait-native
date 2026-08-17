# ait-native

[![Latest release](https://img.shields.io/github/v/release/weita2026/ait-native?include_prereleases&sort=semver&label=release)](https://github.com/weita2026/ait-native/releases)
[![Documentation](https://img.shields.io/badge/docs-ait--native.dev-0ea5e9)](https://ait-native.dev/)
[![Discussions](https://img.shields.io/github/discussions/weita2026/ait-native?label=discussions)](https://github.com/weita2026/ait-native/discussions)
[![License](https://img.shields.io/badge/license-Apache--2.0%20%2B%20AGPL--3.0--only-22c55e)](#license-map)

[Install guide](https://ait-native.dev/local-quickstart/) ·
[Technical documentation](https://ait-native.dev/technical/) ·
[Releases](https://github.com/weita2026/ait-native/releases) ·
[Discussions](https://github.com/weita2026/ait-native/discussions) ·
[Report a bug](https://github.com/weita2026/ait-native/issues/new/choose)

AIT turns an ordinary coding request into an isolated, sprint-bound repository
change with validation evidence and a recoverable land path. It is for
individual developers and maintainers who use coding agents and want the work
to remain reviewable without inventing a workflow for every repository.

AIT does not identify the repository's programming language or project type.
The same workflow applies to Python, Node.js, Rust, Java, mixed-language, and
non-code repositories.

Official website: <https://ait-native.dev/>

## Install and initialize

```sh
python -m pip install ait-native==1.0.0rc9
cd path/to/your-repository
ait init
```

Run `ait init` once inside the repository your coding agent will change. For
Homebrew, npm, apt, and verified native archive routes, use the
[official install guide](https://ait-native.dev/local-quickstart/).

Confirm the selected release before initializing a repository:

```sh
ait --version
```

This is the public `v1.0.0-rc.9` source tree. One tag contains the exact
exported source of `ait-core`, `ait-server`, `ait-runner`, `ait-python`, and
`ait-node`; their AIT Snapshot mapping is recorded in
`ait-monorepo-source.json`.

## What you have after 90 seconds

For a normal package install, the practical 90-second result is an initialized
repository: local AIT authority exists, `AGENTS.md` contains the effective
repository-specific workflow, and the next coding-agent request can use an
isolated Task, validation, Snapshot, and safe land or recovery path. The server
remains off. The 90 seconds covers installation and initialization, not the
completion time of an arbitrary code change.

## Ask for a change

Open your coding agent in the initialized repository and describe the result:

> Update the login flow, preserve the public behavior, and add the relevant
> tests.

You do not need to manually drive Task, Snapshot, or Land commands. `ait init`
creates or updates the generated AIT workflow block in `AGENTS.md`. That block
records the effective repository-specific mode, sprint setting, mutation
scope, commands, and closeout policy that the agent must follow.

## Moving from 0.x

The 0.x requirement to run `ait install` and its task-DAG positioning are
retired. The 1.0 release line starts with `ait init` and a sprint-bound Local
workflow. Keep the existing Git repository and history, but do not treat a
release candidate as proof that legacy 0.x `.ait` data can be migrated in
place. Use a clean clone or a new repository authority unless the selected
release notes explicitly admit that migration. See the
[public transition contract](ait-core/docs/distribution.md#public-0x-to-10-transition).

## What the agent follows from `AGENTS.md`

For a repository mutation, the agent:

1. reads the generated AIT workflow block, the repository plan when present,
   and current AIT configuration before changing files;
2. keeps the requested scope, preserves unrelated user work, and uses
   `ait blame` before repairing an identified regression;
3. writes one exact sprint item, runs `ait task start --from`, and performs
   every code change in the bound isolated worktree;
4. implements the requested code and runs proportionate tests, while routing
   authored Markdown through `ait plan sync` instead of hiding it in a code
   Snapshot;
5. records the completed code with `ait snapshot create`, then uses
   `ait task land` in the configured local or remote scope;
6. verifies closeout of the exact Plan item, Task, worktree, and feature Line;
   and
7. fails closed when required evidence, policy, review, CI, authority, or user
   direction is missing instead of bypassing the repository contract.

The generated block supplies the exact commands. The user supplies the desired
outcome; the agent owns these workflow details.

The same interaction works in Python, Node.js, .NET, PHP, C, C++, Java, shell,
IDE, and CI repositories. The default Local loop does not require a running
`ait-server`; the server remains inactive until explicitly configured and
started.

## Build this source tree

From a clean checkout on macOS or Linux:

```sh
./build-release.sh
```

On Windows PowerShell:

```powershell
.\build-release.ps1
```

The build produces local native commands, a direct PyO3 Python wheel, the
portable JS/TS package, and the current host's direct Node-API addon under
`dist/source-build/`. These source-build outputs and receipts are explicitly
non-publishable; protected release CI promotes only separately admitted family
artifacts.

For Node.js,
`import { NativeRuntime, AgentClient } from "@wa120/ait-native"` loads the
package-owned `native/ait_napi.node` in the current process. The npm `ait`
command calls the same Rust binding through `NativeRuntime.runCli()`.
It does not locate or launch a child executable. `ait-server` is intentionally
not an npm command and remains available through the declared native/server
channels.

## License map

The root [`LICENSE`](LICENSE) is explicit: root release controls,
documentation, `ait-core/**`, `ait-runner/**`, `ait-python/**`, and
`ait-node/**` are Apache-2.0. The sole component exception is
`ait-server/**`, which is AGPL-3.0-only. Each component subtree retains its
exact `LICENSE` and `NOTICE`; bundling does not relicense either component.
No commercial or proprietary license applies to a public 1.0 source path.

The detailed package, source, build, and license contract is centralized in
[`docs/distribution.md`](docs/distribution.md).
