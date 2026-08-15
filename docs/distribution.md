# ait-native 1.0.0 Distribution Contract [plan-ref: ait-native-distribution/root]

Authority: this centralized product distribution contract. Internal planning
lineage remains governed by AIT Plan revisions and is not duplicated here.

Status: target contract plus the current and prior RC records. Contract
sections define the admitted family; dated status sections distinguish the
frozen RC.6 release identity from historical endpoint evidence.

## RC.6 Release Identity (2026-08-15)

`v1.0.0-rc.6` is the current immutable release identity and the latest public
AIT release. Its five source authorities are frozen at these AIT Snapshots:

| Authority | Snapshot |
| --- | --- |
| `ait-core` | `SNP-1372CA70FB06` |
| `ait-server` | `SNP-25FF61FEEA4C` |
| `ait-runner` | `SNP-E50374CBA6E6` |
| `ait-python` | `SNP-DF2C871D5400` |
| `ait-node` | `SNP-46BB35869747` |

The public monorepo uses the corrected license topology: `ait-core`,
`ait-runner`, `ait-python`, and `ait-node` are Apache-2.0, while `ait-server`
is AGPL-3.0-only. Root `LICENSE`, `NOTICE`, `CONTRIBUTING.md`, and
`SECURITY.md` describe the aggregate without relicensing a component. The
deterministic exporter and protected validation reject missing, altered, or
misplaced legal and policy material.

Public source commit `5427892c62cf6042632abb2f369ea5ae39824548` is bound by
the annotated `v1.0.0-rc.6` tag and GitHub Release `370800356`. Component
receipt run `31833306553`, protected-promotion run `31836098106`, and endpoint
publication run `31836538473` succeeded for
`REL-FAM-7B0B9D945B74D95D`; the Release exposes 84 checksum-bound assets and
is a non-draft regular Release with `prerelease=false`.

RC.6 is the approved default/latest candidate without being renamed to
`1.0.0`. GitHub `latest`, every npm package's `latest` dist-tag, and both
GHCR images' `latest` tag resolve to the already-published RC.6 identities;
their `rc` aliases remain in place. PyPI has no mutable dist-tag and pip
excludes prereleases by default, so its selectors remain
`ait-native==1.0.0rc6` or `pip install --pre ait-native`. Homebrew exposes the
latest RC through `ait-native-rc`, APT exposes it through `testing`, and the
WinGet files remain validation assets until a community manifest is reviewed
and merged. None of these native RC routes authorizes a synthetic stable
`1.0.0` artifact.

The RC.6 protected release dossier binds the exact public Git commit and tag,
workflow runs, Release ID, asset digests, endpoint receipts, and external
readback. Mutable latest aliases are a separate, idempotent routing operation:
they never rebuild a component, replace an immutable version, move the Git
tag, or change the five source Snapshot authorities.

## Previous Public RC.5 Record (2026-08-14)

`v1.0.0-rc.5` remains an immutable public release. Its five source authorities
are frozen at these AIT Snapshots:

| Authority | Snapshot |
| --- | --- |
| `ait-core` | `SNP-64B101AAE684` |
| `ait-server` | `SNP-0445E16F63EB` |
| `ait-runner` | `SNP-2458874F5737` |
| `ait-python` | `SNP-9EE8E9FFF1D1` |
| `ait-node` | `SNP-261F4DA754BE` |

RC.5 remains immutable, but its public source layout is not an accepted
baseline for later releases: the `ait-core` subtree retained obsolete AGPL and
commercial-reference files, while the repository root omitted a `LICENSE`.
The component manifest and `ait-core/LICENSE` still identify `ait-core` as
Apache-2.0, so those misplaced reference files did not relicense the core.
They are nevertheless an invalid and misleading publication layout. The tag
must not be moved or overwritten; the next release candidate must use the
corrected license topology and validation gate below.

GitHub publishes this RC tag as a non-draft regular Release with
`prerelease=false`. The version remains `1.0.0-rc.5`; this GitHub presentation
choice does not promote package-registry routes to stable. npm uses the `rc`
dist-tag, PyPI uses `1.0.0rc5`, OCI uses the moving `rc` tag, Homebrew retains
the RC formula, APT uses `testing`, and WinGet remains on its validation route.
The top-level npm product is `@wa120/ait-native`, backed by six exact-version
scoped Node-API implementation packages; RC.5 does not use the historical npm
namespace supplement.

The exact Git commit, tag object, protected workflow runs, release ID, asset
digests, endpoint receipts, and external readback results are recorded by the
RC.5 protected release dossier as publication proceeds. They are not guessed
or copied from an earlier candidate in this source document.

## Previous Public RC.4 Record (2026-08-13)

`v1.0.0-rc.4` remains an immutable public release. Annotated tag object
`a23c61fed8b75e6c8881ceea0a043bd82331f98f` peels to source commit
`ea2d347010d3ead2cdfb304e6df448cbf9fe0c4e`; the public Release is
[`weita2026/ait-native v1.0.0-rc.4`](https://github.com/weita2026/ait-native/releases/tag/v1.0.0-rc.4)
and is presented as a non-draft regular Release with `prerelease=false`.

Component-receipt run
[`31716406486`](https://github.com/weita2026/ait-native/actions/runs/31716406486),
protected-promotion run
[`31721469565`](https://github.com/weita2026/ait-native/actions/runs/31721469565),
and endpoint-publication run
[`31723274386`](https://github.com/weita2026/ait-native/actions/runs/31723274386)
all completed successfully. The endpoint run published and read back GitHub,
PyPI `1.0.0rc4`, all seven scoped npm `1.0.0-rc.4` identities, Homebrew, the
signed APT repository, and both GHCR images. Its WinGet result is the RC
validation artifact route, not a claim that a community manifest has merged
or is searchable. RC.4 bytes and endpoint evidence are never rewritten as
RC.5.

## Previous Public RC.3 Record (2026-08-13)

`v1.0.0-rc.3` is the previous immutable public prerelease. Annotated tag object
`810265c705ffececba3d74924f60ed2d0453ef7d` peels to source commit
`ba368cf4d0750035345f14a8a91c22fb9e450260`; neither identity was moved by
endpoint publication. The public release is
[`weita2026/ait-native v1.0.0-rc.3`](https://github.com/weita2026/ait-native/releases/tag/v1.0.0-rc.3).

Corrected component-receipt run
[`31664713921`](https://github.com/weita2026/ait-native/actions/runs/31664713921)
completed all 35 jobs and froze `REL-FAM-600EFDC327FE7860`: 31 receipts and 37
component artifacts bound to the tagged source above. Protected-environment
run
[`31666479359`](https://github.com/weita2026/ait-native/actions/runs/31666479359)
authorized only that exact dossier. Reviewed endpoint-publisher control commit
`30672445b7321226f81db280f3e2531ad6fc2a5d` then ran endpoint attempt
[`31668411148`](https://github.com/weita2026/ait-native/actions/runs/31668411148).

The exact current endpoint state is:

| Endpoint | RC.3 state |
| --- | --- |
| GitHub Release | Public prerelease with 84 frozen source, native, package, checksum, validation, and receipt assets. |
| PyPI | `ait-native==1.0.0rc3` is public with all six admitted `cp311-abi3` wheels; every registry SHA-256 matches the frozen dossier. |
| GHCR | `ait-server:1.0.0-rc.3` and `ait-runner:1.0.0-rc.3` are public OCI indexes for Linux `amd64` and `arm64`; immutable index digests are `sha256:1494fb3ff9ea05e876d5894e70b599f0718d85e8e1bddf369eab7f89caaed0b4` and `sha256:a2b759b02240acb14440df99ea71012cf2f39c21d368f6da1381abbf235e9957`. |
| Homebrew | `weita2026/homebrew-ait-native` contains `Formula/ait-native-rc.rb` for RC.3 with the four exact GitHub asset URLs and checksums; strict formula audit, clean install, and formula test pass. |
| apt | The signed `testing` repository retains RC.2 and publishes RC.3 for both `ait-native` and `ait-runner` on `amd64` and `arm64`. A new Debian client finds both exact names with `apt-cache search --names-only`; RC.3 is the candidate version. |
| npm | The supported `@wa120/ait-native@1.0.0-rc.3` envelope and all six `@wa120/ait-native-<os>-<cpu>` Node-API implementation packages are public, provenance-attested, and anonymously read back with exact shasum/integrity. A clean registry install selects only the matching platform addon and passes the direct in-process Node-API smoke. Five original unscoped implementation packages remain only as historical endpoint state; their rejected sixth identity and withheld top-level envelope are not supported install routes. |
| WinGet | Community PR `microsoft/winget-pkgs#416596` carries the frozen RC.3 manifests. Checks 01–07, 09, 10, and CLA pass and the PR has `Azure-Pipeline-Passed`; installation verification discovers both executables and aliases on `x64` and `arm64`, then requests manual review because its executable probe invokes the CLI and inactive local server without arguments. Microsoft review and merge are still pending, so the package is not yet available through `winget search`. |

The endpoint attempt completed GitHub, PyPI, GHCR, Homebrew, signed apt, and
WinGet-validation publication before failing visibly at the npm name-policy
boundary. Its final aggregate endpoint evidence was therefore not emitted. The
repository owner subsequently approved the exact `@wa120` namespace mapping.
Protected supplement run
[`31674704785`](https://github.com/weita2026/ait-native/actions/runs/31674704785)
on reviewed control commit `8ad0faa8f5bbf7c5ddcf8bb5a32ac0cfdff9403b`
published and anonymously read back all seven scoped identities. Evidence
artifact `9171186837` has digest
`sha256:736f2d3e12d634d73e4504e79877a8f4e409154ca297cde03dab0e49718702f1`;
its publication and anonymous-readback evidence SHA-256 values are
`d3d33773fc881b106ea6dfdf1ed62d3dc6dee20844273ac06dedb104e3d038ae`
and `a627a33291ddef6f94f990332711140163929a52e652a571a45869ac5a24da2d`.

The supplement rebuilds only the npm JavaScript/package envelopes required by
the new registry identities and copies each previously admitted RC.3 native
Node-API addon byte for byte. It does not rebuild the native addon, release
family, source tag, or GitHub Release, and it does not write the rejected
unscoped identities. Because these new npm identities currently contain only
RC.3, npm exposes both `rc` and its required initial `latest` tag as
`1.0.0-rc.3`; stable 1.0.0 publication must move `latest` to `1.0.0`. The
declared RC endpoint set is complete through the original endpoint evidence
plus this immutable supplement; no replacement aggregate was synthesized for
the earlier failed run. RC.1 and RC.2 bytes remain immutable and are never
relabelled as RC.3.

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
├── .gitattributes
├── .github/workflows/
├── CONTRIBUTING.md
├── LICENSE
├── LICENSES/
├── NOTICE
├── SECURITY.md
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
├── ci/
└── docs/distribution.md
```

`ait-monorepo-source.json` maps each subtree to its exact internal AIT
Snapshot, Snapshot manifest hash and creation time, license, components,
source-cache evidence digest, pre-transform content digest, and exported
content digest. It also records the coordinator Snapshot, manifest hash, and
creation time that deterministically identify the v3 family candidate. This is
an export boundary, not an AIT repository merge: `source_repository` and
`source_snapshot` in receipts continue to name the five internal authorities.
Root `ci/` contains only the reviewed receipt-matrix, native-platform,
Repository-authority, and protected-verifier release controls projected from
the coordinator. These files are not overlaid into `ait-core/`; historical
coordination files inside an admitted component subtree remain immutable source
history and have no current release-control authority.

The export contains no Git submodule, `.git`, `.ait`, `.ait-external`,
`.ait-runtime`, or task worktree. It permits only two declared source rewrites:
`ait-runner/Cargo.toml`
uses `../ait-core/rust/crates/ait-core`, and `ait-python/pyproject.toml` uses
`../ait-core/rust/crates/ait-py/Cargo.toml`. The exporter fails if either
literal is absent, repeated, already transformed, or if any other transform is
declared.

The root `.gitattributes` is exactly `* -text`. It disables Git's checkout-time
line-ending conversion for the complete public tree, so the bytes validated by
`ait-monorepo-source.json` remain identical on Windows, macOS, and Linux even
when a client enables `core.autocrlf`. The exporter includes this file in the
content digest, the root validator rejects any other policy, and regression
coverage validates a fresh `core.autocrlf=true` clone before release use.

A clean tagged checkout validates and builds the current host without an AIT
server:

```text
git clone --branch v1.0.0-rc.6 https://github.com/weita2026/ait-native.git
cd ait-native
./build-release.sh
```

Windows uses `build-release.ps1`. The shared Node implementation builds the
native `ait`, `ait-agent`, `ait-server`, and `ait-runner` executables, the
Python wheel, the portable npm JS/TS envelope, and the current host's direct
Node-API addon. Its locally synthesized receipts and all resulting files are
marked `publishable: false`; they prove clean source usability but cannot
replace protected component receipts or be uploaded by `ait release`.

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

- the product-facing `ait-native` bundles on Homebrew, apt, WinGet, and
  PyPI/pip distribute the admitted `ait` and `ait-server` executables together;
- the sole PyPI `ait-native` registry identity additionally exposes the direct
  Python integration, while the sole npm identity exposes the direct Node-API
  JS/TS facade and in-process `ait` command without `ait-server`;
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
| `ait-node` | portable JS/TS facade and six Node-API addon packages | `ait-node` + pinned `ait-core` | Apache-2.0 | direct in-process Node.js integration and `ait` command without a separate `ait-node` registry product |

`ait-web` is excluded from 1.0.0.

## License And Source Publication Gate

The family is an aggregate of separately licensed components; bundling does
not relicense them. `ait`, `ait-agent`, `ait-runner`, `ait-python`, and
the `ait-node` envelope and addons are Apache-2.0. `ait-server` is
AGPL-3.0-only. The public source tree has this exact license topology:

| Public path | License authority |
| --- | --- |
| repository root controls and product documentation | Apache-2.0 |
| `ait-core/**` | Apache-2.0 |
| `ait-runner/**` | Apache-2.0 |
| `ait-python/**` | Apache-2.0 |
| `ait-node/**` | Apache-2.0 |
| `ait-server/**` | AGPL-3.0-only |

The repository-root `LICENSE` states the Apache-2.0 default and the sole
`ait-server` exception. Root `LICENSES/Apache-2.0.txt` and
`LICENSES/AGPL-3.0-only.txt` contain the two complete reference texts and must
be byte-equal to the corresponding component license authorities. Every
component subtree retains its own authoritative root `LICENSE` and `NOTICE`.
Public 1.0 source and packages have no commercial, proprietary, or
`LicenseRef-*` alternative grant.

The deterministic exporter and release builder enforce that topology. They
reject a missing or mismatched root map, an AGPL or commercial license artifact
inside an Apache subtree, an incomplete AGPL server license, or any component
license that differs from the frozen family manifest. A combined package may
declare `Apache-2.0 AND AGPL-3.0-only` only when it actually contains both
separately licensed components; that artifact expression never changes either
component's own license.

Every binary package must install the exact full `LICENSE` and `NOTICE` bytes
from each owning Snapshot. For Rust repositories, `NOTICE` also contains the
deterministic locked-dependency inventory and the complete deduplicated
upstream legal texts generated by:

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
- npm contains only the Apache-2.0 `ait-node` envelope/addon packages and their
  complete legal material; the AGPL server is not part of npm.

## Public Channel Roles

| Channel | Required 1.0.0 role |
| --- | --- |
| GitHub Release | Canonical native assets and fallback download for every declared target |
| Homebrew | The `ait-native` formula installs `ait` and `ait-server` together on macOS/Linux; RCs use a non-stable route and stable admission follows Homebrew policy |
| apt | The signed `ait-native` package installs `ait` and `ait-server` together on Debian/Ubuntu for `amd64` and `arm64`; `ait-runner` retains a separate package identity |
| WinGet | The `ait-native` product package installs `ait` and `ait-server` together on Windows `x64` and `arm64` |
| PyPI/pip | The sole `ait-native` project publishes platform wheels containing `ait`, `ait-server`, and the direct Python binding; no separate `ait-python` project is published |
| npm | The sole supported top-level `@wa120/ait-native` package exposes the JS/TS API and an in-process `ait` command, selecting one exact-version implementation-only Node-API addon package; it does not install `ait-server` |
| OCI | Immutable `ait-server` and `ait-runner` images |

The shared product-facing identity for all five acquisition channels is
`ait-native`. The Homebrew formula, apt package, and PyPI project use that
exact registry identity. npm uses the owner namespace in
`@wa120/ait-native`, and WinGet uses its required registry-qualified
identifier; both still present the same `ait-native` product identity.
PyPI must not publish a separate `ait-python` project, and npm must not publish
or document `ait-node` as a separate installable product. Every exact registry
package, implementation scope, formula, apt, and WinGet identifier must be
reserved, recorded in the frozen family or an owner-approved immutable
supplement, and smoke-tested before GA. A name that cannot be secured requires
an explicit owner-approved mapping rather than an ad hoc per-channel alias.

## Bundled Server Contract

The Homebrew, apt, WinGet, and PyPI/pip `ait-native` package is one
install, upgrade, and uninstall unit containing at least these two commands:

```text
ait
ait-server
```

The PyPI unit additionally contains the admitted Python binding. npm is not a
server bundle: it contains the portable JS/TS facade plus the platform-selected
Node-API addon and exposes only `ait` as a command. Neither registry creates a
second product or independently selectable version.

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
| WinGet | Resolve `$ctl` from the installed `ait-server.exe` link with the PowerShell snippet below, then run `powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ctl start`; replace `start` with `status` or `stop` as needed | WinGet exposes only the two supported executable portable aliases. The adjacent controller remains supporting archive content; it is user-session only, stores PID/log state below `%LOCALAPPDATA%\AIT\runtime`, uses `%LOCALAPPDATA%\AIT\server-data`, verifies PID ownership before stopping, and does not install or claim a Windows SCM service. |
| PyPI/pip | Run `ait-server run`, or pass the installed executable to the user's own service manager | The wheel installs the same native command and adds no install hook or second lifecycle implementation. |

WinGet accepts executable portable command aliases, not the packaged
PowerShell controller itself. Resolve that supporting file beside the actual
installed server executable rather than treating the `.ps1` file as a WinGet
alias. Each architecture's installer manifest also identifies `ait.exe` and
`ait-server.exe` as launch files with `InvocationParameter: --help`. This gives
WinGet's executable validation a non-mutating invocation that exits without
initializing a repository or starting the server; it does not alter either
command's normal user-facing behavior:

```powershell
$link = Get-Item (Get-Command ait-server.exe).Source
$serverPath = @($link.Target)[0]
if (-not $serverPath) {
    $serverPath = $link.FullName
} elseif (-not [System.IO.Path]::IsPathRooted($serverPath)) {
    $serverPath = Join-Path $link.DirectoryName $serverPath
}
$ctl = Join-Path (Split-Path -Parent $serverPath) 'ait-server-control.ps1'
```

npm has no server-lifecycle row because it does not install `ait-server`.

### OCI container deployment

The RC publishes two Linux `amd64`/`arm64` images from the exact frozen native
binaries, without compiling a component or downloading a component during the
image build:

```text
ghcr.io/weita2026/ait-server:1.0.0-rc.6
ghcr.io/weita2026/ait-runner:1.0.0-rc.6
```

The immutable version tags are the evidence and deployment boundary. The
corresponding `:rc` tags are moving RC conveniences and must resolve to the
same digest before use. Both images run as numeric UID/GID 65532, contain the
owning component's full legal material and provenance, and use a
digest-pinned Dockerfile frontend and Debian base image.

After the GHCR endpoint is published, the shortest persistent local server
deployment is:

```sh
docker network create ait-native-rc
docker volume create ait-native-rc-data
docker run --detach \
  --name ait-server \
  --network ait-native-rc \
  --publish 127.0.0.1:8088:8088 \
  --restart unless-stopped \
  --volume ait-native-rc-data:/var/lib/ait \
  ghcr.io/weita2026/ait-server:1.0.0-rc.6
curl --fail http://127.0.0.1:8088/healthz
```

The image sets `AITSERVER_LISTEN=0.0.0.0:8088` only inside the container so
Docker networking can reach the process. The example still publishes the host
port only on `127.0.0.1`; omitting `--publish` keeps the server private to the
named container network. Pulling the image does not start or initialize a
server. The named volume is initialized only when `docker run` starts the
container and remains intact after `docker stop ait-server` and
`docker rm ait-server`.

An explicitly invoked runner can share that network and mount the repository
whose declared CI command it must execute:

```sh
docker run --rm \
  --network ait-native-rc \
  --volume "$PWD:/workspace" \
  ghcr.io/weita2026/ait-runner:1.0.0-rc.6 \
  serve --server http://ait-server:8088 --source-root /workspace --once
```

The runner image has no implicit public server and does not identify the
repository language. It executes only typed work admitted by the referenced
AIT server and the repository-authored validation contract.

The post-publication package names and commands are:

| Channel | RC / stable install command |
| --- | --- |
| Homebrew | after adding the release tap, `brew install ait-native-rc`; stable uses `brew install ait-native` |
| apt | after adding the signed AIT repository, `sudo apt install ait-native` |
| WinGet | RC validation uses `winget install --manifest <generated-manifest-directory>`; stable uses `winget install --id Weita.AitNative --exact` |
| PyPI | `python -m pip install ait-native==1.0.0rc6` or `python -m pip install --pre ait-native`; PyPI has no mutable `latest` alias |
| npm | `npm install --global @wa120/ait-native` resolves `latest` to `1.0.0-rc.6`; exact and `@rc` selectors remain supported |

The signed APT route must be added and searched before installation:

```sh
curl -fsSL https://raw.githubusercontent.com/weita2026/apt-ait-native/main/ait-native-archive-keyring.gpg \
  | sudo tee /usr/share/keyrings/ait-native-archive-keyring.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/ait-native-archive-keyring.gpg] https://raw.githubusercontent.com/weita2026/apt-ait-native/main testing main" \
  | sudo tee /etc/apt/sources.list.d/ait-native.list
sudo apt update
apt-cache search --names-only '^ait-native$'
apt-cache search --names-only '^ait-runner$'
sudo apt install ait-native
```

The RC.6 publisher performs the same signed update and both exact-name searches
in an isolated APT client root. It writes successful `apt_cache_search`
evidence only after both names are found. These identifiers are the release
contract, not a claim that RC.6 is already live; publication still requires
the frozen family, signatures, clean-host evidence, and public readback below.

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

Every Windows receipt build uses the exact target-scoped
`-Ctarget-feature=+crt-static` Rust flag. Receipt admission opens the native PE
payload in executables, Python wheels, and Node addon archives and rejects any
dynamic `VCRUNTIME140`, `MSVCP140`, or `CONCRT140` import. WinGet nested
portable manifests do not declare the unsupported `Scope` field. Together
these checks keep the portable Windows acquisition paths independent of a
separately preinstalled Visual C++ Redistributable.

The four executables must have native artifacts for all six targets.
`ait-python` must produce binding payloads for the matching six PyPI
`ait-native` platform wheels. `ait-node` must produce one portable npm JS/TS
envelope and six target-specific Node-API addon packages, without a separate
user-facing npm product, install-time build, subprocess relay, or custom binary
download.

The no-subprocess requirement applies to the installed Node.js API and `ait`
command: both enter the selected Rust Node-API addon in-process. Build,
packaging, and validation tools may launch bounded tool processes. In
particular, the Windows release smoke loads the installed addon in a bounded
Node process that exits before its temporary npm tree is removed; that process
is validation isolation for DLL cleanup, not a runtime transport.

The supported npm implementation identities are exactly
`@wa120/ait-native-{darwin,linux,win32}-{arm64,x64}`. The top-level
`@wa120/ait-native` package declares all six as exact-family-version optional
dependencies and selects the one matching package by OS, architecture, and,
for Linux, C library. Both `*-unknown-linux-gnu` implementation packages must
declare the npm selector `libc: ["glibc"]`; Darwin and Windows implementation
packages must omit the npm `libc` selector. The v2 payload contract, addon
metadata, and provenance still carry an exact `libc` field for every row:
`"glibc"` for GNU Linux and explicit `null` for non-Linux targets. This keeps
npm from admitting a GNU addon on musl before Node attempts to load it.
Each addon package is Apache-2.0, has no npm `bin`, scripts, dependencies,
independent version line, or supported direct-install surface, and contains
only the addon, package metadata, provenance, `LICENSE`, and `NOTICE`. All
seven scoped npm identities must be controlled by the owner before
publication; a registry availability check is not a reservation. The five
previously published unscoped implementation identities are historical RC.3
endpoint artifacts, not supported install identities.

The immutable `1.0.0-rc.4` npm artifacts predate this v2 libc selector and are
not rewritten or republished. They do not constitute a musl support claim.
RC.5 binds corrected `ait-node` Snapshot `SNP-261F4DA754BE`; its Linux addon
packages must publish and read back `libc: ["glibc"]`, while Darwin and Windows
must omit the selector. Passing that gate establishes GNU Linux support for
RC.5 and still does not claim musl support.

RC.6 binds `ait-node` Snapshot `SNP-46BB35869747` and carries the same v2 GNU
libc admission contract forward with the direct in-process Node-API runtime.
Its Linux addon packages must publish and read back `libc: ["glibc"]`, while
Darwin and Windows must omit the selector.

The PyPI `ait-native` wheels must pair `ait` and `ait-server` on all six
targets. Homebrew must pair them on the four admitted macOS/Linux targets, apt
on the two admitted Linux targets, and WinGet on the two admitted Windows
targets. npm instead requires the matching Node-API addon for each of the same
six targets. A channel must not publish a target when any component declared by
that channel is absent.

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
- The npm facade selects only the adjacent package-owned `.node` addon by
  declared OS/architecture/libc, exposes the typed import API, and sends `ait`
  argv to `runCli` in-process without `child_process`.
- PyPI uses `ait-native` and npm uses `@wa120/ait-native` as their sole
  supported registry install identities; the npm scope is a registry namespace,
  not a second product, and component repository names are not alternate
  install names.
- The `ait` entry in the npm and PyPI distributions exposes the same native
  command semantics as the operating-system packages; a binding may locate
  adjacent package bytes but must not become a workflow-policy implementation.
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
   package, including the embedded Python or Node.js binding where
   applicable;
9. coherent upgrade, reinstall, offline, and uninstall proof;
10. for npm, exact staged and registry-readback presence/value equality for
    `os`, `cpu`, `libc`, addon metadata, and optional dependencies, including a
    glibc-admitted/musl-omitted install-selection regression;
11. exact release-manifest membership.

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
legal files do not increase or weaken the 37 product-artifact requirements.

The v3 family manifest is the sole release-identity input. An RC uses version
`X.Y.Z-rc.N`, Python version `X.Y.ZrcN`, tag `vX.Y.Z-rc.N`, and channel `rc`;
a stable release uses `X.Y.Z` for both family and Python versions, tag
`vX.Y.Z`, and channel `stable`. The scripts reject every disagreement among
those values. Promotion first creates a credential-free protected-CI handoff;
publisher jobs then promote the frozen bytes without rebuilding. Stable
`1.0.0` is a separate admitted family build, not an RC tag rename.

### Script-only next-release operator SOP

This is the complete maintainer operator path for a later RC or stable release.
It uses repository scripts and existing GitHub Actions only; it does not add or
invoke a new AIT product command, change `ait release publish`, require a
runner-reachable `ait-server`, or publish from the maintainer machine.
Preparation and evidence binding are non-publishing unless `--dispatch` is
explicitly present, and even `--dispatch` only starts the next reviewed
workflow. All package and endpoint writes remain inside the `pypi` protected
GitHub environment.

Prerequisites are `bash`, Git, `jq`, Node.js, `base64`, a SHA-256 utility, and
an authenticated `gh` CLI for live workflow binding or dispatch. Start only
from the five canonical sibling repository roots. In particular, do not run a
release from a recovery copy, a task worktree, or a directory whose private
`.ait` authority is not the retained canonical Binary DB. `remote land` stores
the landed Snapshot in the local Binary DB that executes the command; using a
second recovery root therefore produces a second local authority even when
both roots point at the same Remote.

First create a new, empty release-record directory. The authority preflight
checks all five repository indexes, identities, clean workspaces, selected
Snapshots, canonical `main` ancestry, and Remote URLs. The source-bundle
coordinator then materializes the exact five Snapshot authorities without
copying a recovery `.ait` directory:

```bash
export AIT_CANONICAL_CORE=/absolute/path/to/canonical/ait-core
export AIT_RELEASE_RECORDS=/absolute/path/to/new/release-records
mkdir -p "${AIT_RELEASE_RECORDS}"

./ci/release_authority_preflight.sh \
  "${AIT_CANONICAL_CORE}" \
  "${AIT_RELEASE_RECORDS}/00-authority.json"
./ci/release_source_bundles.sh \
  "${AIT_CANONICAL_CORE}" \
  "${AIT_RELEASE_RECORDS}/source-bundles"
```

The public repository settings are part of the release boundary, not an
informal one-time setup. Before every dispatch, verify that the default branch
is `main`, the active `refs/tags/v*` ruleset blocks update, deletion, and
non-fast-forward operations without bypass, and both `pypi` plus the selected
`rc-promotion` or `stable-promotion` environment have required reviewers. The
repository-level GitHub immutable-release switch is recorded separately: it
protects only future Releases and must not be enabled until the endpoint
publisher has been migrated to upload every asset to a draft before publishing
it. The current exact tag ruleset, checksum readback, and protected-environment
evidence remain authoritative for already-published RCs.

The public repository must then contain one reviewed, clean deterministic
export with an immutable family tag resolving to its current commit. Produce
that export with the existing script after selecting and landing the five
source Snapshots:

```bash
AIT_RELEASE_COORDINATOR_SNAPSHOT=SNP-XXXXXXXXXXXX \
AIT_RELEASE_COORDINATOR_MANIFEST_HASH=<64-lowercase-hex> \
AIT_RELEASE_COORDINATOR_CREATED_AT=<unix-seconds> \
./ci/release_monorepo_export.sh \
  /absolute/path/to/ait-release-family.json \
  "${AIT_RELEASE_RECORDS}/source-bundles" \
  /absolute/path/to/ait-native-export \
  /absolute/path/to/export-evidence.json
```

Review the export, commit it to `weita2026/ait-native`, create its declared
annotated tag exactly once, push the commit and tag, and validate a clean clone
before dispatch. The operator refuses a dirty checkout, a moved or mismatched
tag, an invalid source mapping, or inconsistent RC/stable/Python versions:

```bash
export AIT_PUBLIC_SOURCE=/absolute/path/to/clean/ait-native

cd "${AIT_PUBLIC_SOURCE}"
./build-release.sh --validate-only --git-commit "$(git rev-parse HEAD)"
./ci/release_operator.sh prepare \
  --source-root "${AIT_PUBLIC_SOURCE}" \
  --output "${AIT_RELEASE_RECORDS}/01-prepare.json" \
  --dispatch
```

After the component-receipt workflow succeeds, copy only its numeric run ID.
The script queries the exact successful run, finds the unique frozen dossier,
downloads it, and derives its Release ID, artifact ID, digest, control commit,
Snapshot, and frozen hashes before optionally starting protected promotion:

```bash
cd "${AIT_PUBLIC_SOURCE}"
./ci/release_operator.sh bind-receipts \
  --prepare "${AIT_RELEASE_RECORDS}/01-prepare.json" \
  --run-id <component-receipt-run-id> \
  --output "${AIT_RELEASE_RECORDS}/02-receipts.json" \
  --dispatch
```

Approve that exact run in `rc-promotion` or `stable-promotion`, according to
the manifest channel. After it succeeds, copy only its numeric run ID. The
next command verifies the protected artifact and evidence against the frozen
dossier, generates the canonical endpoint configuration from reviewed static
defaults, binds its SHA-256, and optionally dispatches endpoint publication:

```bash
./ci/release_operator.sh bind-authorization \
  --receipts "${AIT_RELEASE_RECORDS}/02-receipts.json" \
  --run-id <protected-promotion-run-id> \
  --output "${AIT_RELEASE_RECORDS}/03-endpoints.json" \
  --dispatch
```

To inspect before dispatch, first omit `--dispatch` and use a distinct output
filename; outputs are intentionally create-once and are never overwritten.
Then repeat the same evidence binding with `--dispatch` and a new output file.
The generated route is `rc`/RC formula/`testing`/WinGet validation for an RC,
or `latest`/stable formula/`stable`/WinGet community submission for a stable
release. Endpoint identities, credential *names*, and immutable OCI bases live
only in `release/endpoint-publication.defaults.json`; secret values never
enter source or operator records.

After the endpoint workflow succeeds, copy its numeric run ID and generate the
final machine-readable status:

```bash
./ci/release_operator.sh status \
  --config "${AIT_RELEASE_RECORDS}/03-endpoints.json" \
  --run-id <endpoint-publication-run-id> \
  --output "${AIT_RELEASE_RECORDS}/04-status.json"
```

Success proves exact readback for GitHub, PyPI, npm, Homebrew, signed apt, and
both OCI images. npm success includes equality between each staged package and
the registry version for the presence and value of `os`, `cpu`, `libc`, addon
metadata, and optional dependencies; Linux must read back `libc: ["glibc"]`
while Darwin and Windows must read back no `libc` selector. Success also
includes `apt-cache search` visibility for `ait-native` and `ait-runner`. RC
WinGet output stops at validated release assets by contract;
stable WinGet still requires the generated community manifest to be submitted,
reviewed, merged, and independently found with `winget search`.

For a release that the repository owner explicitly chooses as the default,
promote only mutable aliases after the exact endpoint status above succeeds.
The approval value is the exact `REL-FAM-*` ID from `03-endpoints.json`, not a
version wildcard. Production promotion runs only through the protected `pypi`
GitHub environment so the maintainer machine never needs the npm or GHCR
credential. Dispatch the reviewed workflow from public `main` with the exact,
checksum-bound operator records:

```bash
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

endpoint_config="${AIT_RELEASE_RECORDS}/03-endpoints.json"
operator_status="${AIT_RELEASE_RECORDS}/04-status.json"
gh workflow run ait-release-latest-alias.yml \
  --repo weita2026/ait-native \
  --ref main \
  -f release_id="$(jq -r '.release.id' "${endpoint_config}")" \
  -f endpoint_config_sha256="$(sha256_file "${endpoint_config}")" \
  -f endpoint_config_b64="$(base64 <"${endpoint_config}" | tr -d '\r\n')" \
  -f operator_status_sha256="$(sha256_file "${operator_status}")" \
  -f operator_status_b64="$(base64 <"${operator_status}" | tr -d '\r\n')" \
  -f promote_exact_release=true
```

Approve that exact pending deployment. The workflow verifies the annotated Git
tag and public commit, all seven npm versions, both immutable OCI digests, and
the retained `rc` tags before any write; it then invokes
`release_latest_alias.sh` in `apply` and independent `verify` modes, attests
both JSON records, and uploads them as
`ait-release-latest-alias-<REL-FAM-ID>`. Direct local `apply` is only an
equivalent break-glass route for a machine that already has the same bounded
npm, GHCR, and GitHub credentials; it is not the normal release SOP.

For an RC this changes only GitHub's latest presentation, npm's `latest`
dist-tag, and GHCR's `latest` tag; npm/GHCR `rc` remains on the same bytes.
PyPI, Homebrew, APT, and WinGet retain their native prerelease selectors and
no stable `1.0.0` artifact or route is synthesized. Alias rollback uses the
same script and an older, still-valid endpoint dossier with a new explicit
approval and evidence path; immutable package versions, tags, and assets are
never deleted or overwritten.

Run the documented clean-host install, upgrade, command-smoke, and uninstall
matrix from fresh macOS, Linux, and Windows hosts after publication. Preserve
`00-authority.json`, source-bundle evidence, export evidence, the four operator
records, both latest-alias records, workflow artifacts, endpoint receipts,
clean-host logs, and their SHA-256 inventory together as the permanent release
dossier. Any rerun must use a new output path and bind a new successful
workflow run; scripts never relabel or overwrite earlier evidence.

### Implemented package-assembly boundary

After all 37 component requirements pass `ait release check` and are frozen by
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
| npm | the portable `@wa120/ait-native` JS/TS envelope plus six exact-version, OS/CPU/libc-restricted scoped Node-API addon packages |

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

The original RC.3 npm assembler preserves its seven frozen unscoped tarballs
byte for byte. The owner-approved namespace supplement does not alter those
tarballs or the GitHub Release. It validates each original target archive and
native digest, copies `native/ait_napi.node` byte for byte, and deterministically
rebuilds only the scoped npm package envelopes and metadata. The portable
`@wa120/ait-native` envelope is generated from admitted `ait-node` Snapshot
`SNP-22993C1FEF52`; its native binding remains the frozen RC.3 `ait-core`
Snapshot `SNP-158C9C5BB3D7`.

Both paths validate the envelope's JS/TS exports, sole in-process `ait` bin,
exact optional-dependency map, and absence of lifecycle hooks, downloads,
subprocess transport, and project detection. Each addon package must contain
exactly `native/ait_napi.node`, package metadata, provenance, `LICENSE`, and
`NOTICE`; its binding Snapshot must match the family `ait-core` authority. No
addon has a `bin`, scripts, dependencies, independent version, runtime
download, or supported direct-install surface.

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
[`ait-release-component-receipts.yml`](https://github.com/weita2026/ait-native/blob/main/.github/workflows/ait-release-component-receipts.yml)
workflow is the sole cross-repository component-matrix entrypoint. Its required
`coordinator_snapshot` input is the exact landed AIT Snapshot named by the
selected source mapping, and `source_commit` is the exact immutable public Git
commit to build. The dispatched `github.sha` is a separate reviewed
release-control commit. Every runner checks out the selected source commit
under `source/`; only contract and matrix-projection jobs also check out
`github.sha` under `control/`. `ait-monorepo-source.json` proves how the source
tree was exported from the coordinator Snapshot and five component Snapshots;
a Git checkout is never represented as a selected local AIT Snapshot checkout.

The deterministic monorepo exporter projects the reviewed workflows and exact
release-control files to the public repository root because GitHub Actions
discovers workflows only under root `.github/workflows/`. Component build and
family-admission shell steps execute from `source/ait-core/`; release-control
steps execute from `control/` and consume the source checkout's root family
manifest explicitly. The nested component copy is source history, not a second
dispatch entrypoint or current control directory. This projection does not
create another GitHub repository or change any component Snapshot authority.

Before dispatch, deterministically export the landed controls, review and
commit that complete tree to `weita2026/ait-native`, confirm that the selected
tag still resolves to `source_commit`, and confirm that the selected source's
`ait-monorepo-source.json` names `coordinator_snapshot`. Protect the default
branch, immutable tag, workflow, and manual dispatch through normal GitHub
repository controls. A post-tag control correction may change `github.sha` but
must never rewrite the selected source commit or tag. This workflow has no
environment secret and specifically must not receive `AIT_RELEASE_SERVER_URL`:
GitHub-hosted runners neither connect to an AIT server nor download private AIT
repository state. PyPI, npm, GitHub Release, Homebrew, apt, WinGet, OCI,
signing, and publication credentials also remain absent.

The pre-commit export is a maintainer-side authority operation, separate from
the hosted build. It may read already-landed Snapshots from the maintainer's
local authority or a team-owned self-hosted `ait-server`; that does not create
a public upload service and is not part of installation or ordinary use.
`ait init` remains fully local, while teams opt into their own server only when
they need shared remote authority.

The workflow performs these bounded operations:

1. check out exact reviewed control and selected source commits with persisted
   credentials disabled; validate both public export contracts, then verify the
   source content digest, family manifest, mapping, and requested coordinator
   Snapshot before any build command runs;
2. project 31 target/portable receipt jobs and 37 component artifacts from the
   mapped family manifest;
3. run each repository-owned generic adapter directly in its fixed public
   subtree on the matching native runner;
4. emit a `public_git_commit` receipt that binds the artifact bytes to one Git
   commit, mapping digest, coordinator Snapshot, source-repository Snapshot,
   adapter definition, target, and legal material;
5. archive the exact Git commit and its mapping as non-public run evidence,
   without regenerating source or creating a second repository; and
6. reconstruct the same deterministic `REL-FAM-*` candidate from exported
   coordinator metadata, reject mixed authority or commits, admit all 31
   receipts, and upload one frozen internal dossier.

`public_git_commit` is only the receipt's source-authority label. The actual
`authority.git_commit` value is the full immutable `source_commit` checked out
from `weita2026/ait-native`; it is not the release-control `github.sha`, a
server, command, upload target, or replacement for an AIT Snapshot. The mapping
keeps internal Snapshot provenance, while this Git SHA proves which public
source bytes a runner built. Run-scoped source evidence separately records the
workflow-control commit so protected authorization can prove both identities.

All artifacts are run-scoped and `public_publish` remains false. The workflow
does not create a tag, call `release promote`, sign an artifact, activate AIT
remote Release authority, create a GitHub Release, or write to any registry.
The additional monorepo source artifact does not change the five internal
Snapshot authorities, 31 receipt, or 37 component-artifact counts. Hosted
release source-cache count and live AIT server connection count are both zero.
The GitHub-hosted runner labels are pinned in the root release-control
[`native_bootstrap_matrix.json`](https://github.com/weita2026/ait-native/blob/main/ci/native_bootstrap_matrix.json); confirm
their current availability in GitHub's hosted-runner reference before an RC or
GA dispatch.

### Protected authorization without publication

The public monorepo root owns
`.github/workflows/ait-release-protected-promotion.yml` as the sole
authorization boundary after the immutable handoff. Its job runs only behind
the channel-selected `rc-promotion` or `stable-promotion` GitHub environment
and accepts exact values for the source
workflow run and attempt, dossier artifact ID and GitHub artifact digest,
family Release, tag, public Git commit, source-workflow control commit,
coordinator Snapshot, frozen-manifest SHA-256, and frozen `SHA256SUMS` SHA-256.
The source-run API record must identify that exact control commit, while the
dossier and anonymous tag readback must identify the exact source commit.
Approval therefore applies to one already-built byte set rather than a version
label or moving branch.

The protected job reads the selected tag anonymously, downloads only the
exact dossier artifact from the successful component-receipt run, revalidates
every frozen and assembled-package checksum, proves the tagged checkout is
byte- and executable-mode-equal to the archived corresponding source, runs the
tagged public-source contract, projects expected counts only from the reviewed
root control files plus the tagged root family, and asks the frozen host-native
`ait` binary to reproduce the credential-free promotion handoff. It then emits
and attests one `ait.release.family.protected-promotion/v1` evidence record
containing both source and source-workflow commit identities.

That evidence may set protected authorization and source readback to verified,
but it still records every artifact rebuild, registry credential load,
registry write, GitHub Release write, tag write, AIT Remote Release activation,
and service mutation as false. Its only next action is to request separate
explicit authorization for each exact publication endpoint. The protected
workflow consequently cannot be used as an implicit registry-publish command.

The deterministic public source export contains the component-receipt,
protected-promotion, and generic endpoint-publication workflows, the
script-only operator, publisher scripts, and reviewed static endpoint defaults.
It contains no per-release endpoint configuration, workflow-run ID, artifact
ID, artifact digest, or credential. After protected promotion succeeds,
`release_operator.sh bind-authorization` verifies those exact live identities
and generates the SHA-256-bound configuration consumed by the generic endpoint
workflow from the default branch. That workflow consumes the immutable tag and
frozen dossier and cannot move the tag. This ordering prevents older endpoint
authority or guessed future identities from becoming publication evidence,
without requiring a release-specific workflow or source patch.

## Previous RC.3 Record And Historical RC Evidence

RC.3 retains the direct Node-API architecture. Its public Git commit and tag,
successful protected 31-receipt/37-artifact matrix, frozen
`REL-FAM-600EFDC327FE7860` dossier, and protected authorization now exist.
The declared RC endpoint publication is complete. The owner-approved scoped
npm supplement changes npm registry names and package envelopes only; it does
not change RC.3 native bytes, the release family, the annotated tag, or GitHub
Release assets. Its protected run and immutable evidence artifact complete the
npm record without rewriting the failed original endpoint attempt or inventing
a replacement aggregate. Nothing from RC.1 or RC.2 can be promoted or
relabelled into RC.3.

The immutable RC.3 source tag object is
`810265c705ffececba3d74924f60ed2d0453ef7d`; it peels to commit
`ba368cf4d0750035345f14a8a91c22fb9e450260`. Tagged coordinator Snapshot
`SNP-B0271928FD9B` has manifest hash
`b0271928fd9b290e9eb8fafdeeb8f70c1547dbe8a2b56710b1186c821ad9b125`.
The tagged family-manifest SHA-256 is
`7c20810f16676b8e10f74b8fe576bb41e29eac2d1a4898fe495258279b03b9a8`,
the public mapping SHA-256 is
`4bc3ce9dac8da6c6f0b7adb3a9d55ab49e45bef3b7684f4e7c4cc1982c9961f0`,
and its mapped content SHA-256 is
`98be79d828fd06ece7313efa185145abb893798cb56dfba7f41361c7cd7f5a48`.
Anonymous exact-tag checkout preserves 1,599 tracked files and 28 executable
modes and passes the commit-bound public-source validator.

Initial run
[`31663031294`](https://github.com/weita2026/ait-native/actions/runs/31663031294)
failed in its contract job before a matrix build or dossier. It proved that the
tagged root RC.3 family was being combined with historical RC.2 control files
inside `ait-core`. Release-control correction `SNP-D121B248E5E1` passed the
complete local patchset, remote CI, attestation, review, and policy, then
landed atomically as `RCT-1384/C-01/P-02`. Corrected run `31664713921` used
that reviewed control while selecting the unchanged tagged source commit and
produced the frozen dossier. Protected run `31666479359` admitted it, and
endpoint run `31668411148` produced the exact partial-publication state listed
at the start of this document.

The records below preserve immutable RC.1 evidence for comparison; they do not
authorize an RC.3 endpoint write.

At the 2026-08-12 checkpoint, the immutable reviewed RC source is annotated
tag `v1.0.0-rc.1` on
[`weita2026/ait-native`](https://github.com/weita2026/ait-native). The tag
peels to Git commit `f9d260a8f7046f82a6c3e271d539dd0bbce7bc14`, the merge
commit for [PR #7](https://github.com/weita2026/ait-native/pull/7). The tag
exists, but no GitHub Release for this tag, AIT Release activation, signature,
or registry write has been made.

Tagged coordinator Snapshot `SNP-FFDF9798A111` binds that exact public source
to these five internal source authorities:

| Public subtree | Internal source Snapshot | Components |
| --- | --- | --- |
| `ait-core` | `SNP-BBAFC78C7AB9` | `ait`, `ait-agent` |
| `ait-server` | `SNP-1D1960F54FD0` | `ait-server` |
| `ait-runner` | `SNP-31053B5CB6D6` | `ait-runner` |
| `ait-python` | `SNP-8480292492FC` | `ait-python` |
| `ait-node` | `SNP-D51020FA5568` | `ait-node` |

The tagged coordinator manifest hash is
`ffdf9798a11115389183ef4a8edeb68e26b553a267e752c39f1d104c731cbd8e`;
the tagged family-manifest SHA-256 is
`a17aa3ae9349793bed383be16cf055ee52f4d0a9b1e0cf1eaf2d1eeba5ce29f1`.
The tagged public mapping SHA-256 is
`b957e53ad15deaf60543e3b29ad682fbefb558793177a8b39b13c88dab9d30b4`,
and the mapped public content SHA-256 is
`bcd0ca26487ba9b56bbd737ad93aefc98e09b0371ae83826c571f79005863c4d`.
Anonymous tag readback proved 1,570 regular files and 23 executable modes
against the archived corresponding source.

[Run 31543619357](https://github.com/weita2026/ait-native/actions/runs/31543619357)
completed the public-source contract, exact-source archive, all 25 component
receipt jobs, and the isolated family-dossier job. It created run-scoped
frozen dossier `REL-FAM-D84070909C7F5CA9` as artifact `9122098178`, with
GitHub digest
`sha256:196ec5404c1ca01196ac9d348a7a106f329f3bb410bda4ef6815b27ea40fca1b`.
The frozen family-manifest SHA-256 is
`e0abcd0047e4b13117d3b6517006ce3315b9b2a1d1722c919ed0642ddc87dca9`,
and the frozen `SHA256SUMS` SHA-256 is
`dfffdd7f9cd64297f6d498f5b328e79acf12dfb4f4d61bac2a3f0cd5475f7bc7`.
Every one of its 42 frozen checksum entries and all five assembled package
channels has been read back locally without publication.

The post-tag release-control source is coordinated by Snapshot
`SNP-6E736EFA6D2F`, with manifest hash
`6e736efa6d2fb607e4bc5098ff0978d1008d847695f0338bf6d61a0ab38e472c`.
It advances only the `ait-core` source authority to `SNP-A161A1821FA5` and
adds the protected authorization workflow, verifier, and exact direct-root
artifact extraction used by [PR #9](https://github.com/weita2026/ait-native/pull/9).
It does not move the tag, replace the frozen dossier, rebuild an artifact, or
grant any publication endpoint authority.

[Protected run 31549673366](https://github.com/weita2026/ait-native/actions/runs/31549673366)
received the required `rc-promotion` environment approval for the exact
recorded inputs and passed at release-control Git commit
`5d6b0e4a16539a8f36be2eb1089359c1ffe2ad7e`. Its evidence artifact is
`9123775170`, with GitHub artifact digest
`sha256:cd4b9ab658a7b1242489a6f128f95f79ce24c40582f7f76bf90041a485f297e1`.
The evidence JSON SHA-256 is
`ff8f594d7065bb3f1fa326754e38e9cb30bd7fcf1cb7b1d0b98facc934e0e34e`;
its GitHub SLSA provenance verifies the protected workflow, `main` source
ref, release-control commit, hosted-runner boundary, and one transparency-log
timestamp. The evidence authorizes only a future request for explicit
per-endpoint promotion and records every mutation field as false.

The dossier remains deliberately non-public. Its recorded promotion state
has `performed: false` and `registry_write: false`; local and remote AIT
Release authority are not activated. Both receipt creation and protected
authorization use frozen GitHub artifacts and an anonymous tag checkout, so a
GitHub-hosted runner does not need an internet-reachable
`AIT_RELEASE_SERVER_URL`.

For the historical RC.1 dossier, code and its cross-platform receipt matrix
were no longer the blocker. Its remaining publication gates were:

- preserve the attested protected handoff without substituting artifacts and
  obtain separate explicit owner authorization for every publication
  endpoint;
- sign the frozen artifacts and complete the real GitHub, PyPI, npm,
  Homebrew, apt, WinGet, and OCI endpoint metadata from that dossier;
- prove package-name ownership, credentials, and endpoint configuration
  before the first write, keeping PyPI/npm/Homebrew/apt/WinGet/GitHub/OCI
  publication disabled until all source gates pass; and
- run clean install, first `ait init`, agent-directed first land, upgrade,
  uninstall, and endpoint readback on the six declared targets before the RC
  is called published.

The install-to-first-land corpus may contain representative Python, Node.js,
.NET, PHP, C, C++, and Java repository files, but every case must run the same
explicit native AIT commands and repository-authored validation. It must not
add language detection, manifest inspection, framework profiles, or
language-specific product paths; the sole behavior authority remains
[the centralized repository-language neutrality contract](#repository-language-neutrality).

## Superseded RC Checkpoints

The remainder of this section records pre-success checkpoints. Its
"current" and "remaining" wording describes those historical checkpoints and
must not override the baseline and gates above.

At the 2026-08-11 Git-source receipt checkpoint, the exact RC component
versions and source authorities are landed on their five internal main Lines.
Coordinator Snapshot `SNP-012AAE09336F` owns the current v3
[`ait-release-family.json`](../ait-release-family.json): it binds `ait` and
`ait-agent` to `SNP-30623730029F`, `ait-server` to `SNP-067518622F5C`,
`ait-runner` to `SNP-31053B5CB6D6`, `ait-python` to `SNP-9EA7C957FB31`, and
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
while a tracked operational path or Gitlink fails closed. The first protected
public-Git dispatch from commit `35f24fc02fa0914a4bc809905e31f48e5370c4a5`
([Actions run 31497799228](https://github.com/weita2026/ait-native/actions/runs/31497799228))
then produced 15 non-public artifacts and exposed two exact cross-platform
gaps before any candidate or publication: Windows checkout converted mapped
LF bytes under `core.autocrlf`, and the Python adapter admitted only the
internal `.ait-external/ait-core` layout instead of the validated exported
`../ait-core` layout. The Python correction passed repository CI and was
remote-landed at `SNP-9EA7C957FB31`. The exact Git byte policy, its
`core.autocrlf=true` clone regression, and the final family rebind passed
protected remote CI as Worker Job `#2494` and were atomically landed as
`RCT-1345`; the public export now preserves committed bytes on all declared
platforms and Python receipts accept only the two mapped core layouts. The
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
pre-byte-policy coordinator `SNP-5E2C7A5FA23F` also never received a family
candidate and was superseded after the protected public-Git run exposed the
Windows checkout and Python public-layout gaps. Current coordinator
`SNP-012AAE09336F` is not yet bound to a family candidate.
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
byte-exact root `LICENSE` and `NOTICE`; and every npm Node-API addon requires
non-empty `LICENSE` and `NOTICE` with exact provenance. The platform
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
- until that dispatch succeeds, all 31 final receipt bundles and all 37 final
  component artifact keys remain unadmitted and no final v3
  `REL-FAM-*` dossier is complete;
- the npm JS/TS envelope and six addon packages require admitted `ait-node`
  receipts tied to the exact `ait-core` binding Snapshot, registry reservation
  evidence, direct-load/in-process-command proof, and clean-install proof on all
  six targets; install hooks, runtime downloads, subprocess relays, and
  language detection remain forbidden;
- the final Python binding and byte-exact wheel legal checks pass locally
  and in internal CI, but six current-source `cp311-abi3` component receipts
  and final assembly from the complete family remain unproved;
- no verified registry reservation, public 1.0 product-page transition, or
  endpoint-owner readback evidence is recorded for a v3 candidate;
- all five channel assemblers prove their declared component/addon mapping,
  legal-material inventory, deterministic output, and zero-publication
  boundary with complete fixtures, but no real channel package can be admitted
  until the final zero-of-37 starting state becomes a complete frozen
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
