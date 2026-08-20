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

You ask for a change in plain language. AIT gives that work its own sprint
item and an isolated worktree, runs your repository's checks, and keeps a
record of what passed before anything lands — so work a coding agent
writes stays inspectable and recoverable. Built for
individual developers and maintainers.

AIT doesn't care what language your repository is in. It never tries to
detect a project type: the same workflow runs on Python, Node.js, Rust,
Java, mixed-language, and non-code repositories.

Official website: <https://ait-native.dev/>

## Install and initialize

```sh
python -m pip install ait-native==1.0.0
cd path/to/your-repository
ait --version
ait init
```

`ait init` sets up the local `.ait` authority, defaults the repository to
`solo_local` with sprint mode on, creates `docs/sprints/`, and writes the
repository's own workflow block into `AGENTS.md`. It does not start
`ait-server`.

`ait config show --json` shows the effective mode and scope. Homebrew, npm,
APT, WinGet, OCI, and native-archive routes are covered in the
[install guide](https://ait-native.dev/local-quickstart/).

This is the public `v1.0.0` source tree. The tag binds the exported
source of `ait-core`, `ait-server`, `ait-runner`, `ait-python`, and `ait-node`;
`ait-monorepo-source.json` records their exact AIT Snapshot mapping.

## What `ait init` gives you

- A repository-local AIT authority, and a generated `AGENTS.md` block that
  routes the workflow.
- A sprint-backed Task and a dedicated worktree for every code change.
- Snapshots, check evidence, `ait blame` for tracing a regression to its
  revision, and a recoverable Task Land closeout.
- A server that stays off: local work never needs a running `ait-server`.

The generated `AGENTS.md` block is the source of truth for your repository.
It carries the current commands for your configured workflow and sprint
modes; the generic examples in this README never override it.

## Local and reviewed workflows

AIT has three workflow presets:

| Mode | Authoring and closeout |
| --- | --- |
| `solo_local` | Task, Change, Snapshot, and `ait task land` all stay local unless you explicitly promote to a remote. No reviewer, no server. |
| `solo_remote` | You prepare one exact Patchset with `ait workflow ready <change-id> --apply`; a reviewer runs `ait workflow land <change-id> --apply`. |
| `team_remote` | The same author-ready and reviewer-land boundary, with remote-backed reviewable Changes for team work. |

With sprint mode on, start from one exact sprint item:

```sh
ait task start --from docs/sprints/<card>.md#<item-ref> --intent "<intent>"
```

With sprint mode off, use `ait task start --title "<title>" --intent
"<intent>"` instead. Either way, the command creates the Task's bound
worktree; the code is written there and recorded with `ait snapshot create`.
Authored Markdown goes through the generated `ait plan sync` scope rather
than being buried inside a code Snapshot.

In a reviewed remote flow, `workflow ready` is the author's side: it takes
care of Snapshot freshness, publishing the exact Patchset, CI, and
attestation. `workflow land` is the reviewer's side: it runs the Review and
Policy gates, then hands the already-ready change to atomic Task Land. It
never redoes the author's build or CI.

In `solo_local`, finish with `ait task land <task-or-change-id>`. The final
local Task Land updates the target Line, completes the Task, removes its
bound worktree, and closes the bound sprint item when there is one.

## Ask for a change

Open your coding agent in the initialized repository and say what you want:

> Update the login flow, preserve public behavior, and add the relevant tests.

The agent reads `AGENTS.md`, starts the right kind of Task, works in the
emitted worktree, runs the checks, creates a Snapshot, and follows your
configured local or reviewed closeout. To see where things stand, use `ait
queue summary`, `ait task audit <task-id>`, and the other commands the
generated block names.

## What each install route gives you

| Route | Installed surface |
| --- | --- |
| PyPI `ait-native` | `ait`, an inactive-by-default `ait-server`, and the direct `ait-python` binding. |
| npm `@wa120/ait-native` | `ait` and the direct in-process Node-API binding; it does not install `ait-server`. |
| Homebrew and WinGet | Native `ait` and `ait-server` commands. |
| APT | `ait-native` (`ait` plus `ait-server`) and the separately installable `ait-runner` package. |
| OCI | Separate `ait-server` and `ait-runner` images. |
| GitHub Release | Checksum-bound native archives and the package assets used by the declared routes. |

## Upgrading from 0.x

There is no `ait install` command in 1.0. Install or upgrade the `ait-native`
package with your package manager, check it with `ait --version`, and run
`ait init` only when you are creating a new 1.0 repository authority. Keep
your Git repository and its history — but don't assume a release candidate
can migrate an existing 0.x `.ait` authority in place. Keep the old
authority around for recovery, and use a clean clone or a new repository
authority unless the release notes for your exact version say migration is
supported. See the
[public transition contract](ait-core/docs/distribution.md#public-0x-to-10-transition).

## Build this source tree

From a clean checkout on macOS or Linux:

```sh
./build-release.sh
```

On Windows PowerShell:

```powershell
.\build-release.ps1
```

The build produces the local native commands, a direct PyO3 Python wheel,
the portable JS/TS package, and the current host's direct Node-API addon
under `dist/source-build/`. These source-build outputs and their receipts
are explicitly non-publishable; protected release CI promotes only
separately admitted family artifacts.

For Node.js, `import { NativeRuntime, AgentClient } from
"@wa120/ait-native"` loads the package-owned `native/ait_napi.node` in the
current process. The npm `ait` command calls the same Rust binding through
`NativeRuntime.runCli()`; it does not locate or launch a child executable.

## License map

The root [`LICENSE`](LICENSE) is explicit: root release controls,
documentation, `ait-core/**`, `ait-runner/**`, `ait-python/**`, and
`ait-node/**` are Apache-2.0. The sole component exception is
`ait-server/**`, which is AGPL-3.0-only. Each component subtree keeps its
exact `LICENSE` and `NOTICE`; bundling does not relicense either component.
No commercial or proprietary license applies to a public 1.0 source path.

The detailed package, source, build, and license contract lives in
[`docs/distribution.md`](docs/distribution.md).
