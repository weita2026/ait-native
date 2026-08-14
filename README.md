# ait-native

AIT is a language-neutral, agent-first workflow for repository changes.
Initialize AIT once, then tell your coding agent what outcome you want. AIT
does not identify the repository's programming language or project type.

This is the public `v1.0.0-rc.5` source tree. One tag contains the exact
exported source of `ait-core`, `ait-server`, `ait-runner`, `ait-python`, and
`ait-node`; their AIT Snapshot mapping is recorded in
`ait-monorepo-source.json`.

## Start with an agent

After installing a verified `ait-native` release, initialize the repository
that you want the agent to change:

```sh
cd path/to/your-repository
ait init
```

Then open your coding agent in that repository and describe the result:

> Update the login flow, preserve the public behavior, and add the relevant
> tests.

You do not need to manually drive Task, Snapshot, or Land commands. `ait init`
creates or updates the generated AIT workflow block in `AGENTS.md`. That block
records the effective repository-specific mode, sprint setting, mutation
scope, commands, and closeout policy that the agent must follow.

## What the agent follows from `AGENTS.md`

For a repository mutation, the agent:

1. reads the generated AIT workflow block, the repository plan when present,
   and current AIT configuration before changing files;
2. keeps the requested scope, preserves unrelated user work, and uses
   `ait blame` before repairing an identified regression;
3. evaluates the bounded edit with `ait workflow tier --json`; it uses a quick
   Snapshot only when the generated policy explicitly permits it;
4. for governed work, writes one exact sprint item, runs `ait task start`, and
   performs the change in the bound isolated worktree;
5. implements the requested code and runs proportionate tests, while routing
   authored Markdown through `ait plan sync` instead of hiding it in a code
   Snapshot;
6. records the completed code with `ait snapshot create`, then uses
   `ait task land` in the configured local or remote scope;
7. verifies closeout of the exact Plan item, Task, worktree, and feature Line;
   and
8. fails closed when required evidence, policy, review, CI, authority, or user
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

For Node.js, `import { NativeRuntime, AgentClient } from "ait-native"` loads the
package-owned `native/ait_napi.node` in the current process. The npm `ait`
command calls the same Rust binding through `NativeRuntime.runCli()`. It
does not locate or launch a child executable. `ait-server` is intentionally
not an npm command and remains available through the declared native/server
channels.

The detailed package, source, build, and mixed-license contract is centralized
in [`docs/distribution.md`](docs/distribution.md). Each component subtree also
retains its exact `LICENSE` and `NOTICE` files.
