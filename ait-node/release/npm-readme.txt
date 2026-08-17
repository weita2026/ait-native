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

## What you have after 90 seconds

For a normal registry install, the practical 90-second result is an initialized
repository: local AIT authority exists, `AGENTS.md` contains the effective
repository-specific workflow, and the next coding-agent request can use an
isolated Task, validation, Snapshot, and safe land or recovery path. The server
remains off. The 90 seconds covers installation and initialization, not the
completion time of an arbitrary code change.

You describe the result. The coding agent follows the generated contract and
owns the workflow details; you do not manually drive Task, Snapshot, or Land
commands.

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

## Moving from 0.x

The 0.x requirement to run `ait install` and its task-DAG positioning are
retired. The 1.0 release line starts with `ait init` and a sprint-bound Local
workflow. Keep the existing Git
repository and history, but do not treat this release candidate as proof that
legacy 0.x `.ait` data can be migrated in place. Use a clean clone or a new
repository authority unless the selected release notes explicitly admit that
migration.

## Package boundary

The package installs one `ait` command backed by the same addon. `ait-server`
is distributed separately and is not part of npm. Runtime code does not use
install hooks, downloads, project-language detection, or `child_process`
transport.

The complete product, platform, source, and licensing contract is published
at <https://github.com/weita2026/ait-native/blob/main/docs/distribution.md>.
