# ait-native 1.0.0 Distribution Contract [plan-ref: ait-native-distribution/root]

Authority: this centralized product distribution contract. Internal planning
lineage remains governed by AIT Plan revisions and is not duplicated here.

Status: target contract. It defines the admitted public release family; it is
not evidence that publication has already completed.

## Release Family

The public brand and release family are:

```text
product: ait-native
version: 1.0.0
```

One frozen release manifest must bind every component, platform artifact,
package identity, source revision, license, checksum, signature, and clean-host
receipt. No component may drift to an independent semantic version while
claiming membership in the 1.0.0 compatibility family.

### Internal authority and public source layout

AIT keeps five independent internal source authorities. Their repository names
and selected Snapshots remain the inputs to component builds and receipts:

```text
ait-core     ait-server     ait-runner     ait-python     ait-node
```

GitHub source publication is deliberately different: the 1.0 family has one
public repository, `weita2026/ait-native`, one release commit, and one family
tag. The deterministic exported tree has these fixed paths:

```text
ait-native/
├── ait-core/
├── ait-server/
├── ait-runner/
├── ait-python/
├── ait-node/
├── ait-release-family.json
├── ait-monorepo-source.json
├── build-release.mjs
├── build-release.sh
├── build-release.ps1
└── docs/distribution.md
```

`ait-monorepo-source.json` maps each subtree to its exact internal AIT
Snapshot, Snapshot manifest hash and creation time, license, components,
source-cache evidence digest, pre-transform content digest, and exported
content digest. It also records the coordinator Snapshot, manifest hash, and
creation time that deterministically identify the v3 family candidate. This is
an export boundary, not an AIT repository merge: `source_repository` and
`source_snapshot` in receipts continue to name the five internal authorities.

The export contains no Git submodule, `.git`, `.ait`, `.ait-external`,
`.ait-runtime`, or task worktree. It permits only two declared source rewrites:
`ait-runner/Cargo.toml`
uses `../ait-core/rust/crates/ait-core`, and `ait-python/pyproject.toml` uses
`../ait-core/rust/crates/ait-py/Cargo.toml`. The exporter fails if either
literal is absent, repeated, already transformed, or if any other transform is
declared.

A clean tagged checkout validates and builds the current host without an AIT
server:

```text
git clone --branch v1.0.0-rc.1 https://github.com/weita2026/ait-native.git
cd ait-native
./build-release.sh
```

Windows uses `build-release.ps1`. The shared Node implementation builds the
native `ait`, `ait-agent`, `ait-server`, and `ait-runner` executables, the
Python wheel, the npm command envelope, and the current host's `ait` and
`ait-server` npm payloads. Its locally synthesized receipts and all resulting
files are marked `publishable: false`; they prove clean source usability but
cannot replace protected component receipts or be uploaded by `ait release`.

The public repository is migrated in place and should not be deleted. Any old
component GitHub mirrors should be retained as archived/read-only provenance
with a pointer to `ait-native`; they are not declared distribution identities
and therefore cannot enter the v3 family. Deletion is reserved for a separate
security or legal requirement because it would break existing links, tags,
forks, and source traceability.

## Distribution Objective

The 1.0.0 distribution objective is broad activation across repositories and
supported platforms. A developer should be able to obtain the same admitted
native `ait` through a familiar operating-system package manager or package
registry, then complete the same sprint-bound Local loop.
No channel may present a separate product implementation or stale workflow
story.

### Repository-language neutrality

`ait` does not identify a project type or programming language. It applies the
same explicit command, repository, Line, Snapshot, Task, Change, and validation
contracts in Python, Node.js, .NET, PHP, C, C++, Java, mixed-language, and
non-code repositories. Git does not need project recognition, and neither does
AIT. A repository supplies its own commands and CI policy; AIT executes those
declared commands without scanning manifests, selecting a framework profile,
or silently changing workflow behavior.

### Shortest installed workflow

After installing `ait-native`, enter any existing repository and initialize
AIT once:

```text
cd <repository>
ait init
```

Then tell the coding agent what to change. There is no separate Python, Node,
.NET, PHP, C, C++, or Java mode. `ait init` writes or refreshes the generated
AIT workflow block in `AGENTS.md`; an AIT-aware agent uses that block as the
repository-local execution contract. In particular, the agent:

1. reads the effective workflow mode, mutation scope, sprint mode, and current
   repository Plan/configuration before changing files;
2. records authored Markdown through the configured Plan sync path;
3. for a normal governed change, creates a sprint item and Task, works only in
   the Task-bound worktree, and preserves unrelated user edits;
4. runs the repository-authored validation or `ci/run` contract instead of
   guessing commands from filenames or language manifests;
5. records the result as an AIT Snapshot and completes it through the exact
   local or remote Task-land path printed by the generated workflow; and
6. reports any failed gate without silently bypassing review, policy, CI, or
   remote authority.

The user-facing loop is therefore only `ait init` followed by an ordinary
agent request. `AGENTS.md` supplies the detailed AIT bookkeeping; project
recognition is neither required nor performed.

Distribution has three distinct roles:

- the product-facing `ait-native` bundles on Homebrew, apt, WinGet, PyPI/pip,
  and npm distribute the admitted `ait` and `ait-server` executables together;
- the sole PyPI `ait-native` registry identity additionally exposes the direct
  Python integration, while the sole npm identity remains command-only;
- standalone GitHub and OCI artifacts expose applicable native components for
  direct verification or deployment without changing the bundle contract.

A bundle or installer envelope is a release artifact, not a seventh product
component. It must contain or depend only on target-specific, package-owned
bytes admitted from the same frozen family. Install scripts may select an
adjacent target package through normal declared registry dependency
resolution, but must not perform a custom network fetch, compile, rebuild, or
reinterpret the native runtime.

## Public Components

| Component | Public surface | Source authority | License | 1.0.0 role |
| --- | --- | --- | --- | --- |
| `ait` | native executable | `ait-core` | Apache-2.0 | repository and workflow CLI |
| `ait-agent` | native executable | `ait-core` | Apache-2.0 | agent runtime and transports |
| `ait-server` | native executable | `ait-server` | AGPL-3.0-only | remote protocol and durable authority |
| `ait-runner` | native executable | `ait-runner` | Apache-2.0 | remote native execution plane |
| `ait-python` | PyO3 binding payload embedded in PyPI `ait-native` | `ait-python` + pinned `ait-core` | Apache-2.0 | direct in-process integration without a separate PyPI project |
| `ait-node` | portable npm CLI launcher and package envelope | `ait-node` + admitted `ait-core`/`ait-server` receipts | Apache-2.0 | command-only npm distribution without a separate npm product or Node.js API |

`ait-web` is excluded from 1.0.0.

## License And Source Publication Gate

The family is an aggregate of separately licensed components; bundling does
not relicense them. `ait`, `ait-agent`, `ait-runner`, `ait-python`, and
the `ait-node` command envelope are Apache-2.0. `ait-server` is
AGPL-3.0-only. Every binary package must install the exact full `LICENSE` and
`NOTICE` bytes from each owning Snapshot. For Rust repositories, `NOTICE`
also contains the deterministic locked-dependency inventory and the complete
deduplicated upstream legal texts generated by:

```text
bash ci/generate_rust_notice.sh \
  --manifest <repository>/rust/Cargo.toml \
  --notice <repository>/NOTICE \
  --project <repository-name>
```

Release validation repeats the same command with `--check`; any byte drift
fails without rewriting the notice.

The generator intentionally inventories the complete locked Cargo closure,
including target-, build-, and test-only packages. A changed lock graph with a
stale notice, an abbreviated project license, an unknown SPDX expression, or
missing legal material is a release blocker.

Public source is a prerequisite, not a later documentation task. Before any
binary endpoint may be written, protected promotion must prove:

1. all five selected AIT Snapshots were sanitized and exported to their fixed
   subtrees with only the two declared sibling-core path rewrites;
2. `ait-monorepo-source.json` binds those five Snapshot, license, component,
   transform, and content-digest mappings;
3. one public Git commit tree is byte-equal to the complete deterministic
   export;
4. the exact family tag resolves to that one commit and is readable
   anonymously;
5. locked dependencies, source locks, root build entrypoints, and the product
   document are present at the tagged source URL;
6. a clean clone passes root validation and the admitted host builds; and
7. public readback evidence was recorded before the first binary publication.

This gate supplies the corresponding source path for the AGPL server and the
source path for every Apache component. A moving branch, an AIT Snapshot ID
without a public tree mapping, or a tag created after binary publication does
not satisfy it. The promotion handoff therefore starts with
`binary_publication_allowed: false`, one `required_unverified`
release-monorepo row, and five subtree mappings. Only protected publisher
evidence may authorize later endpoint writes. A v2 manifest, multiple GitHub
identities, incomplete subtree set, or undeclared transform is rejected before
a candidate can be created.

Channel metadata must preserve the aggregate boundary:

- Homebrew uses an `all_of` expression and installs each repository's legal
  material;
- Debian copyright assigns each executable and repository legal directory its
  own license stanza and points to the complete installed/common-license text;
- WinGet uses the combined expression and an exact-tag `LicenseUrl` to this
  section;
- PyPI uses Metadata 2.4 `License-Expression` plus every installed
  `License-File`; and
- npm keeps the Apache command envelope distinct from AGPL server payloads,
  with each payload retaining its owning component license.

## Public Channel Roles

| Channel | Required 1.0.0 role |
| --- | --- |
| GitHub Release | Canonical native assets and fallback download for every declared target |
| Homebrew | The `ait-native` formula installs `ait` and `ait-server` together on macOS/Linux; RCs use a non-stable route and stable admission follows Homebrew policy |
| apt | The signed `ait-native` package installs `ait` and `ait-server` together on Debian/Ubuntu for `amd64` and `arm64`; `ait-runner` retains a separate package identity |
| WinGet | The `ait-native` product package installs `ait` and `ait-server` together on Windows `x64` and `arm64` |
| PyPI/pip | The sole `ait-native` project publishes platform wheels containing `ait`, `ait-server`, and the direct Python binding; no separate `ait-python` project is published |
| npm | The sole supported top-level `ait-native` package installs `ait` and `ait-server` as commands; it has no JavaScript API or native addon, and any target-specific payload package is an implementation-only exact-version dependency, not a separate product |
| OCI | Immutable `ait-server` and `ait-runner` images |

The shared product-facing identity for the five bundled channels is
`ait-native`. The Homebrew formula, apt package, PyPI project, and supported
top-level npm package use that exact identity; WinGet uses the required
registry-qualified identifier while presenting the same product identity.
PyPI must not publish a separate `ait-python` project, and npm must not publish
or document `ait-node` as a separate installable product. Every exact registry
package, implementation scope, formula, apt, and WinGet identifier must be
reserved, recorded in the family manifest, and smoke-tested before GA. A name
that cannot be secured requires an explicit owner-approved mapping rather than
an ad hoc per-channel alias.

## Bundled Server Contract

The Homebrew, apt, WinGet, PyPI/pip, and npm `ait-native` package is one
install, upgrade, and uninstall unit containing at least these two commands:

```text
ait
ait-server
```

The PyPI unit additionally contains the admitted Python binding. The npm unit
adds only the portable command launcher/envelope; its target-specific native
bytes are still the admitted `ait` and `ait-server` components. Neither
registry creates a second product or independently selectable version.

The two executables retain independent source-component receipts and license
notices. Package construction must prove that each installed byte is copied
from the matching frozen `ait-core` or `ait-server` artifact; it must not
rebuild either component at an endpoint.

Bundling is distribution convenience, not implicit server activation. Install
and upgrade hooks must not start `ait-server`, enable or register a persistent
service with a service manager, open a port, initialize server authority, or
make the Local CLI depend on a running server. A package may install an
inactive declarative service definition or an explicit user-session
controller. Activation remains a separate user command. Standalone GitHub and
OCI server artifacts may coexist with the bundle but must resolve to the same
admitted server digest.

### Installed server lifecycle

The installed native executable, not `ait.sh`, owns the portable lifecycle:

```text
ait-server init
ait-server probe --defer-ci-admission
ait-server run --init-if-missing --defer-ci-admission
```

Plain `ait-server run` is the shortest user-mode form. With no `--data` flag or
runtime environment, it selects the platform default below and safely creates
a new Binary v0 authority only when that path is missing or empty. `init` is
idempotent for an existing valid activation and refuses a symlink, legacy
conversion input, non-directory, or non-empty unactivated path without
deleting its contents.

Runtime-root precedence is `--data`, `AIT_NATIVE_SERVER_DATA`,
`AIT_RUNTIME_DATA`, then the user default:

| Platform | Default user-mode server data |
| --- | --- |
| macOS | `$HOME/Library/Application Support/AIT/server-data` |
| Linux/Unix | `$XDG_STATE_HOME/ait/server-data`, otherwise `$HOME/.local/state/ait/server-data` |
| Windows | `%LOCALAPPDATA%\AIT\server-data`, otherwise `%USERPROFILE%\AppData\Local\AIT\server-data` |

The server binds only `127.0.0.1:8088` by default. `--defer-ci-admission`
defers only the startup-time RAM-workspace probe; a later managed CI allocation
still fails closed until an admitted memory-backed root is configured.

The generated operating-system packages add these explicit controls while
remaining inactive after install or upgrade:

| Channel | Explicit activation | Data and lifecycle behavior |
| --- | --- | --- |
| Homebrew | `brew services start ait-native-rc` for the RC formula, or `brew services start ait-native` for stable; stop with the matching `brew services stop` command | The formula service runs the installed binary and uses `$HOMEBREW_PREFIX/var/ait-native/server-data`; the formula installation itself neither initializes nor starts it. |
| apt | `sudo systemctl daemon-reload && sudo systemctl enable --now ait-server`; stop with `sudo systemctl disable --now ait-server` | The shipped, initially disabled unit uses `DynamicUser`, `StateDirectory=ait-native`, and `/var/lib/ait-native/server-data`. The Debian package has zero maintainer scripts. |
| WinGet | In PowerShell, set `$ctl = (Get-Command ait-server-control.ps1).Source`, then run `powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ctl start`; replace `start` with `status` or `stop` as needed | The controller is user-session only, stores PID/log state below `%LOCALAPPDATA%\AIT\runtime`, uses `%LOCALAPPDATA%\AIT\server-data`, verifies PID ownership before stopping, and does not install or claim a Windows SCM service. |
| PyPI/pip or npm | Run `ait-server run`, or pass the installed executable to the user's own service manager | These registry packages install the same native command and add no install hook or second lifecycle implementation. |

The post-publication package names and commands are:

| Channel | RC / stable install command |
| --- | --- |
| Homebrew | after adding the release tap, `brew install ait-native-rc`; stable uses `brew install ait-native` |
| apt | after adding the signed AIT repository, `sudo apt install ait-native` |
| WinGet | RC validation uses `winget install --manifest <generated-manifest-directory>`; stable uses `winget install --id Weita.AitNative --exact` |
| PyPI | `python -m pip install ait-native==1.0.0rc1`; stable uses `ait-native==1.0.0` |
| npm | `npm install --global ait-native@1.0.0-rc.1`; stable uses `ait-native@1.0.0` |

These identifiers are the release contract, not a claim that the currently
blocked RC endpoints are already live. Endpoint publication still requires the
frozen family, signatures, clean-host evidence, and public readback described
below.

## Platform Matrix

The public 1.0.0 target set is:

| Rust target | OS | Architecture |
| --- | --- | --- |
| `aarch64-apple-darwin` | macOS | arm64 |
| `x86_64-apple-darwin` | macOS | x86_64 |
| `aarch64-unknown-linux-gnu` | Linux/glibc | arm64 |
| `x86_64-unknown-linux-gnu` | Linux/glibc | x86_64 |
| `aarch64-pc-windows-msvc` | Windows | arm64 |
| `x86_64-pc-windows-msvc` | Windows | x86_64 |

The four executables must have native artifacts for all six targets.
`ait-python` must produce binding payloads for the matching six PyPI
`ait-native` platform wheels. `ait-node` must produce one portable npm CLI
launcher/envelope. Final npm packages combine that envelope with the admitted
target-specific `ait` and `ait-server` artifacts without a separate
user-facing npm product, install-time build, or custom binary download.

The RC npm implementation identities are exactly
`ait-native-ait-{darwin,linux,win32}-{arm64,x64}` and
`ait-native-server-{darwin,linux,win32}-{arm64,x64}`. The top-level
`ait-native` package declares all twelve only as exact-family-version optional
dependencies and selects the two matching packages by OS and architecture.
The `ait` payload packages retain `Apache-2.0`; the `ait-server` payload
packages retain `AGPL-3.0-only`. None has an npm `bin`, API, independent
version line, or supported direct-install surface. All thirteen npm identities
must be reserved before publication; a registry availability check is not a
reservation.

The PyPI and npm `ait-native` bundles must pair `ait` and `ait-server` on all
six targets. Homebrew must pair them on the four admitted macOS/Linux targets,
apt on the two admitted Linux targets, and WinGet on the two admitted Windows
targets. A channel must not publish a target if either required bundle member
is absent.

This six-target matrix is the exact 1.0.0 cross-platform claim. It is not a
claim of support for every operating system or C library. Linux/musl, other
architectures, mobile platforms, and additional operating systems require an
explicit later target and their own clean-host evidence.

Repository-language neutrality is independent of this artifact matrix and is
governed solely by [the centralized requirements above](#repository-language-neutrality).

## Compatibility Rules

- Every public component reports or exposes version `1.0.0`.
- Every bundled `ait`/`ait-server` pair reports the same family version while
  retaining independent component digests and license notices.
- Server/runner wire contracts are tested against the exact paired artifacts.
- The Python binding loads package-owned native bytes in-process and does not
  invoke the CLI, inspect ambient `PATH`, or download a runtime after
  installation.
- The npm launcher selects only adjacent package-owned executables by declared
  OS/architecture, forwards argv and status unchanged, and exposes no
  import/require API or native addon.
- PyPI and npm use only their sole supported `ait-native` registry identity;
  component repository names are not alternate install names.
- The `ait` entry in the npm and PyPI bundles exposes the same native command
  semantics as the operating-system packages; a wrapper may locate adjacent
  bytes but must not become a workflow-policy implementation.
- Bundled installation never implies background server execution.
- Installed executables do not discover or depend on a source checkout.
- One platform artifact is built once, promoted without rebuild, and verified
  by digest readback from every public endpoint.
- Licenses and notices are packaged per component and summarized at the
  product-family level.

## Required Publication Evidence

For each component, package, and target:

1. source revision and locked dependencies;
2. native, Python-binding, or portable package-envelope artifact digest;
3. signature/attestation;
4. version and capability output;
5. clean install and package-origin proof;
6. component functional smoke and, for each declared user-facing CLI install
   channel, `ait --version`, capability, and sprint-bound first-loop smoke;
7. for a bundled channel, `ait-server --version`, package-content provenance,
   default-inactive verification, and explicit server start/stop smoke;
8. complete per-component license and notice material inside the combined
   package, including the embedded Python binding or npm launcher where
   applicable;
9. coherent upgrade, reinstall, offline, and uninstall proof;
10. exact release-manifest membership.

The family may be announced only after every required row passes or an
explicitly owner-approved target is removed from this contract before
publication.

## Operational Release Flow

The implemented family coordinator consumes the v3 family manifest described
in this document. Separate component repositories emit Snapshot-bound,
target-specific receipts. `ait-core` binds
their exact repository, Snapshot, ecosystem version, artifact kind, target,
size, and SHA-256 evidence into one `REL-FAM-*` candidate and frozen release.
Each generic receipt also carries the source repository's exact declared
`LICENSE` and `NOTICE` bytes as checksum-covered `license-material`, independent
of the component artifact matrix. Family admission requires those two files for
every source repository, rejects target-receipt disagreement, and freezes one
deduplicated repository/Snapshot copy for downstream package assemblers. These
legal files do not increase or weaken the 31 product-artifact requirements.

For `1.0.0-rc.1`, use family version `1.0.0-rc.1`, Python version
`1.0.0rc1`, tag `v1.0.0-rc.1`, and channel `rc`. Promotion first creates a
credential-free protected-CI handoff; publisher jobs then promote the frozen
bytes without rebuilding. Stable `1.0.0` is a separate admitted family build,
not an RC tag rename.

### Implemented package-assembly boundary

After all 31 component requirements pass `ait release check` and are frozen by
`ait release build`, the same complete `REL-FAM-*` dossier is the only accepted
input to every bundled-channel assembler:

```text
ait release package <REL-FAM-ID> --channel homebrew --json
ait release package <REL-FAM-ID> --channel apt --json
ait release package <REL-FAM-ID> --channel winget --json
ait release package <REL-FAM-ID> --channel pypi --json
ait release package <REL-FAM-ID> --channel npm --json
```

Each command writes below
`dist/<REL-FAM-ID>/packages/<channel>/`. The exact 1.0 matrix produces:

| Channel | Deterministic channel artifacts |
| --- | --- |
| Homebrew | four macOS/Linux product archives and one RC- or stable-routed formula |
| apt | two `ait-native` and two standalone `ait-runner` Debian packages for `arm64`/`amd64` |
| WinGet | two Windows portable ZIPs and the three-file WinGet 1.12 manifest set |
| PyPI | six platform-specific `ait_native` `cp311-abi3` wheels |
| npm | the byte-identical portable `ait-native` envelope plus twelve exact-version, OS/CPU-restricted implementation payloads |

Every channel directory also contains `ait-release.package.json` and
`SHA256SUMS`. Their evidence maps every installed command or binding back to
the frozen component artifact, source repository, and Snapshot; maps exact
repository-scoped `LICENSE` and `NOTICE` bytes to their package destinations;
and records that component rebuild, credential loading, signing, registry
write, public publication, and service start were all false.

Assembly is deterministic from the frozen Snapshot time. Repeating it
validates the existing bytes instead of overwriting them, and a changed output,
unsafe input archive, missing target, component/version drift, or incomplete
legal inventory fails closed. These commands create staging artifacts only:
they do not create an apt repository index, mutate a Homebrew tap, submit a
WinGet manifest, upload to PyPI/npm, create a tag, or authorize later endpoint
mutation.

The PyPI assembler preserves every admitted `ait-python` binding member,
renames only the distribution metadata to `ait-native`, adds package-owned
`ait` and `ait-server` files through the wheel scripts data scheme, installs
all three repositories' legal material, and regenerates an exact final
`RECORD`. The filename, `WHEEL` tag, family target, and `cp311-abi3` contract
must agree.

The npm assembler preserves the frozen seven-file command envelope byte for
byte and validates its exact command-only package shape. The twelve generated
payloads contain one admitted native executable, its owning repository's legal
material, exact `os`/`cpu` restrictions, and Snapshot provenance. Payloads
have no `bin`, API export, lifecycle hook, native addon, independent version,
runtime download, or supported direct-install surface.

The original assembler implementation was locally landed at
`SNP-316B01F59913` for Homebrew/apt/WinGet and `SNP-6EC8DB2D50A6` for
PyPI/npm. The installed-server control correction is Snapshot
`SNP-C9954FEF988B`: it adds the inactive Homebrew service definition, the
hook-free apt systemd unit, the WinGet user-session controller, stronger
zero-activation evidence, and the updated server selection. Protected remote
CI passed as Jobs `#2472` and `#2473` for the earlier assembly baseline; the
installed-server correction passed as Worker Job `#2484` and was atomically
landed as `RCT-1328/C-01/L-02`. These Snapshots prove assembly code and
deterministic fixtures; they do not supply final six-target component artifacts
or create a complete publishable v3 family.

### Protected component-receipt matrix

The manually dispatched
[`ait-release-component-receipts.yml`](https://github.com/weita2026/ait-native/blob/v1.0.0-rc.1/.github/workflows/ait-release-component-receipts.yml)
workflow is the sole cross-repository component-matrix entrypoint. Its required
`coordinator_snapshot` input is the exact landed AIT Snapshot containing the
family manifest to admit. The dispatched `github.sha` is the immutable public
source authority used by every runner. `ait-monorepo-source.json` proves how
that one Git tree was exported from the coordinator Snapshot and the five
component Snapshots; a Git checkout is never represented as a selected local
AIT Snapshot checkout.

The deterministic monorepo exporter projects that reviewed coordinator
workflow to the public repository root because GitHub Actions discovers
workflows only under root `.github/workflows/`. Its shell steps execute from
the exported `ait-core/` subtree, while GitHub action paths remain rooted at
the monorepo checkout. The nested component copy is source history, not a
second dispatch entrypoint. This projection does not create another GitHub
repository or change any component Snapshot authority.

Before dispatch, deterministically export the landed authorities, review and
commit that complete tree to `weita2026/ait-native`, and confirm that the
committed `ait-monorepo-source.json` names the requested coordinator. Protect
the source branch, workflow, and manual dispatch through normal GitHub
repository controls. This workflow has no environment secret and specifically
must not receive `AIT_RELEASE_SERVER_URL`: GitHub-hosted runners neither connect
to an AIT server nor download private AIT repository state. PyPI, npm, GitHub
Release, Homebrew, apt, WinGet, OCI, signing, and publication credentials also
remain absent.

The pre-commit export is a maintainer-side authority operation, separate from
the hosted build. It may read already-landed Snapshots from the maintainer's
local authority or a team-owned self-hosted `ait-server`; that does not create
a public upload service and is not part of installation or ordinary use.
`ait init` remains fully local, while teams opt into their own server only when
they need shared remote authority.

The workflow performs these bounded operations:

1. check out exact `github.sha` with persisted credentials disabled and verify
   its complete content digest, family manifest, mapping, and requested
   coordinator Snapshot before any build command runs;
2. project 25 target/portable receipt jobs and 31 component artifacts from the
   mapped family manifest;
3. run each repository-owned generic adapter directly in its fixed public
   subtree on the matching native runner;
4. emit a `public_git_commit` receipt that binds the artifact bytes to one Git
   commit, mapping digest, coordinator Snapshot, source-repository Snapshot,
   adapter definition, target, and legal material;
5. archive the exact Git commit and its mapping as non-public run evidence,
   without regenerating source or creating a second repository; and
6. reconstruct the same deterministic `REL-FAM-*` candidate from exported
   coordinator metadata, reject mixed authority or commits, admit all 25
   receipts, and upload one frozen internal dossier.

`public_git_commit` is only the receipt's source-authority label. The actual
`authority.git_commit` value is the full immutable commit SHA checked out from
`weita2026/ait-native`; it is not a server, repository, command, upload target,
or replacement for an AIT Snapshot. The mapping keeps internal Snapshot
provenance, while this Git SHA proves which public source bytes a runner built.

All artifacts are run-scoped and `public_publish` remains false. The workflow
does not create a tag, call `release promote`, sign an artifact, activate AIT
remote Release authority, create a GitHub Release, or write to any registry.
The additional monorepo source artifact does not change the five internal
Snapshot authorities, 25 receipt, or 31 component-artifact counts. Hosted
release source-cache count and live AIT server connection count are both zero.
The GitHub-hosted runner labels are pinned in
[`native_bootstrap_matrix.json`](https://github.com/weita2026/ait-native/blob/v1.0.0-rc.1/ait-core/ci/native_bootstrap_matrix.json); confirm
their current availability in GitHub's hosted-runner reference before an RC or
GA dispatch.

## Current RC Baseline And Gap

At the 2026-08-11 Git-source receipt checkpoint, the exact RC component
versions and source authorities are landed on their five internal main Lines.
Coordinator Snapshot `SNP-5E2C7A5FA23F` owns the current v3
[`ait-release-family.json`](../ait-release-family.json): it binds `ait` and
`ait-agent` to `SNP-8F22130AED0D`, `ait-server` to `SNP-067518622F5C`,
`ait-runner` to `SNP-31053B5CB6D6`, `ait-python` to `SNP-22E653510992`, and
`ait-node` to `SNP-D51020FA5568`, and declares only `weita2026/ait-native` as
its GitHub product source identity. `ait-python` remains Apache-2.0 at version
`1.0.0rc1`; the native, server, runner, and npm components remain
`1.0.0-rc.1`. The coordinator includes the deterministic root
`.github/workflows/` projection, the Snapshot-owned agent-first README and its
content validation, the bounded Snapshot-memory work, the verified cross-pack
source-cache overlap repair, and the installed-server package controls defined
above. The selected core also provides exact public-Git receipt admission,
preserves normalized component executable modes across deterministic export,
tests executable and non-executable mode retention, resolves the sole product
document from both the internal repository layout and the public monorepo
layout without duplicating it inside `ait-core/`, and removes non-operative
maintenance commands instead of hiding them. The executable-mode correction
passed protected remote CI as Worker Job `#2490` and was atomically landed as
`RCT-1337`; the centralized-document correction and final family rebind passed
protected remote CI as Worker Job `#2491` and were atomically landed as
`RCT-1339`. The public operational-ignore correction and family rebind passed
protected remote CI as Worker Job `#2492` and were atomically landed as
`RCT-1341`; a full source build may create AIT operational output at the root
or below a component, but that output cannot enter the public Git commit. The
repeatable post-build validation correction and final family rebind passed
protected remote CI as Worker Job `#2493` and were atomically landed as
`RCT-1343`; ignored AIT operational output may remain after a source build,
while a tracked operational path or Gitlink fails closed. The
selected server removes the incident-specific fresh replay
and Plan-lineage converter implementations; its isolated importer retains only
`audit-generation`, `stage`, `upgrade-u64-seconds`, and `activate`, plus
fail-closed read-only admission for generations created before those incident
writers were retired.

Candidate `REL-FAM-8BEDBCAA67CC4DC4` remains immutably bound to the prior
`SNP-6EB77A892610` coordinator and pre-lifecycle server selection. It expects
exactly 31 component artifacts and currently contains zero, but it must not be
filled or relabelled as the corrected RC. `SNP-C9954FEF988B` is remotely
landed as the prior coordinator. Prior coordinator `SNP-E01143558211` is
bound to candidate `REL-FAM-F76D64279183328B`, but that immutable zero-of-31
dossier was created before the Git-source receipt correction. It must not be
filled, relabelled, or promoted. Prior corrected coordinator
`SNP-8AFCEC283738` never received a family candidate and was superseded before
public Git publication when clean-clone validation exposed the executable-mode
loss. Prior coordinator `SNP-FE40E5FD6DBA` also never received a family
candidate and was superseded before public Git publication when the direct
exported self-test exposed its single-repository product-document path.
Prior coordinator `SNP-A3C4F113C71C` also never received a family candidate
and was superseded before public Git publication when the full source build
exposed unignored Maturin/Cargo operational output below `ait-python/.ait/`.
Prior coordinator `SNP-9FD1A37C38E7` also never received a family candidate
and was superseded before public Git publication when post-build validation
rejected its own ignored `ait-python/.ait/` operational output. Current
coordinator `SNP-5E2C7A5FA23F` is not yet bound to a family candidate.
It requires a deterministic public Git commit and a new
`REL-FAM-*` candidate; the protected workflow then builds the complete receipt
matrix from that commit without source hydration.
Prior coordinator `SNP-884E7F274EC5` and candidate
`REL-FAM-B6ECCE624254CBFD` remain immutable pre-agent-first evidence; earlier
coordinator `SNP-9B1B6B36F3CD` and candidate `REL-FAM-D6721954EA9D3656`
remain immutable pre-projection evidence. Their byte-identical exports and
macOS arm64 clean-source build remain useful local validation, but do not
replace the corrected protected six-target receipts.

Prior coordinator Snapshot `SNP-B20BDEF38F2D` produced
`REL-FAM-805C78F36FEBFE80` under the earlier five-public-repository contract.
That zero-of-31 candidate is immutable audit evidence and must not be edited,
relabelled, or promoted through the v3 route. A later v3 candidate receives a
new `REL-FAM-*` identity only after the landed v3 coordinator Snapshot is
selected; the old candidate cannot collide with it because the bound manifest
and manifest hash are part of candidate identity.

Candidate `REL-FAM-31FC913098F30F1E` remains immutable pre-correction audit
evidence with six of 31 artifact keys. Its five component receipts selected
older source Snapshots and legal-material bytes, so neither those receipts nor
that candidate may be relabelled or promoted as the final RC. All earlier
coordinator and candidate evidence remains immutable for comparison. The
current candidate remains at zero of 31. The corrected protected matrix
defines 25 target/portable receipt bundles and 31 component artifacts, plus one
non-public exact-Git-source artifact; it defines no hosted source cache. A
host-local build remains validation evidence, not a substitute for admitted
Git-commit receipts and their Snapshot mapping.

The source-side legal rollout is complete. `ait-core`, `ait-runner`, and the
native closure embedded by `ait-python` carry deterministic locked Rust
dependency notices; `ait-server` carries the complete AGPL-3.0-only text plus
its complete locked Rust notices; Python wheel verification requires
byte-exact root `LICENSE` and `NOTICE`; and every npm native payload now
requires non-empty `LICENSE` and `NOTICE` with exact provenance. The platform
license model is therefore no longer the blocker. Current-source receipts,
public corresponding-source mapping, and endpoint readback remain blockers.

The server and runner now negotiate only `ait.runner.native-job.v3` for the
current path. Its logical `./ci/run` selector maps to `ci/run.sh` on
macOS/Linux and `ci/run.ps1` on Windows without repository-language or project
detection. The Python and Node Windows entrypoints are landed and statically
contract-tested; actual Windows native binaries, wheels, and clean-host
execution remain part of the missing protected matrix evidence.

Family candidate parsing, receipt admission, six-target freezing, deterministic
Homebrew/apt/WinGet/PyPI/npm package assembly, RC/stable routing, and protected
promotion handoff are implemented in `ait-core`. The remaining release
blockers are real matrix artifacts, endpoint authority, and clean-host
admission:

- the one exported `weita2026/ait-native` tree still requires a reviewed public
  Git commit, exact five-subtree Snapshot mapping, anonymous exact-tag
  readback, clean-clone build, and recorded corresponding-source evidence
  before any binary endpoint write;
- the protected matrix requires one successful dispatch of an exact reviewed
  `weita2026/ait-native` commit whose mapping names the new landed v3
  coordinator Snapshot; it requires no runner-reachable AIT server;
- until that dispatch succeeds, all 25 final receipt bundles and all 31 final
  component artifact keys remain unadmitted and no final v3
  `REL-FAM-*` dossier is complete;
- the npm command envelope remains dependency-free and has no API, addon,
  install hook, runtime download, or language detection; the twelve real
  payload tarballs still require admitted target `ait-core`/`ait-server`
  receipts, registry reservation evidence, and clean-install proof on all six
  targets;
- the final Python binding and byte-exact wheel legal checks pass locally
  and in internal CI, but six current-source `cp311-abi3` component receipts
  and final assembly from the complete family remain unproved;
- no verified registry reservation, public 1.0 product-page transition, or
  endpoint-owner readback evidence is recorded for a v3 candidate;
- all five bundled-channel assemblers now prove their multi-component mapping,
  legal-material inventory, deterministic output, and zero-publication
  boundary with complete fixtures, but no real channel package can be admitted
  until the final zero-of-31 starting state becomes a complete frozen
  dossier; and
- Homebrew, apt, npm, PyPI, WinGet, GitHub, and OCI endpoint readback plus
  clean install, upgrade, uninstall, and install-to-first-land smoke have not
  been recorded from one final frozen family.

These are 1.0.0 release blockers, not reasons to narrow the declared product
family.

The install-to-first-land regression corpus may contain representative Python,
Node.js, .NET, PHP, C, C++, and Java repository files, but every case must run
the same explicit native AIT commands and repository-authored validation. It
must not add language detection, manifest inspection, framework profiles, or
language-specific product paths; the sole behavior authority remains
[the centralized repository-language neutrality contract](#repository-language-neutrality).
