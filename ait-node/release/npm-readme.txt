# ait-native

AIT turns an ordinary coding request into an isolated, sprint-bound repository
change with validation evidence and a recoverable land path. It is for
individual developers and maintainers who use coding agents and want the work
to remain reviewable without inventing a workflow for every repository.

AIT is repository-language-neutral. It does not detect or change behavior for
Python, Node.js, Rust, Java, or any other project type.

Official website: <https://ait-native.dev/>

## Install and initialize

```sh
npm install --global @wa120/ait-native@@AIT_NPM_VERSION@
ait init
```

Run `ait init` once inside the repository your coding agent will change. Then
ask for the outcome normally; for example, “Update the login flow, preserve
public behavior, and add the relevant tests.”

## What initialization provides

- Repository-local AIT authority and a generated `AGENTS.md` workflow router.
- Sprint-backed Task creation and a dedicated worktree for each code change.
- Snapshots, validation evidence, regression attribution through `ait blame`,
  and recoverable Task Land closeout.
- An inactive server boundary: local work does not require a running
  `ait-server`.

The generated `AGENTS.md` block is authoritative for the repository. Describe
the result to your coding agent normally; it reads that contract and follows
the configured local or reviewed closeout path.

For a reviewed remote flow, the author prepares the exact Patchset with
`ait workflow ready <change-id> --apply`; the reviewer records the decision
and lands it with `ait workflow finish <change-id> --apply`.

## Node.js API

`@wa120/ait-native` also provides the direct in-process Node-API binding for
the Rust-owned AIT runtime. JavaScript and TypeScript load the package-owned
`native/ait_napi.node`; the package does not launch an `ait` executable.

```js
import { NativeRuntime } from "@wa120/ait-native";

const ait = new NativeRuntime();
console.log(ait.bindingInfo());
const status = ait.runCli(["status"]);
```

## Upgrading from 0.x

There is no `ait install` command in 1.0. Install or upgrade
`@wa120/ait-native` through npm, verify it with `ait --version`, then run `ait
init` only when creating a new 1.0 repository authority. Keep the Git
repository and its history, but do not assume that a release candidate can
migrate an existing 0.x `.ait` authority in place. Preserve the old authority
for recovery and use a clean clone or a new repository authority unless the
selected release notes explicitly admit that migration.

## Package boundary

The package installs one `ait` command backed by the same addon. `ait-server`
is distributed separately and is not part of npm. Runtime code does not use
install hooks, downloads, project-language detection, or `child_process`
transport.

The complete product, platform, source, and licensing contract is published
at <https://github.com/weita2026/ait-native/blob/main/docs/distribution.md>.
