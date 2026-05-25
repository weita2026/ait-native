# ait Package Targets

## What this guide is for

Use this guide when you need to answer:

- which release-facing package target matches the **local-only** path;
- which target family matches the **self-hosted shared control plane**;
- whether `ait-worker` is a standalone package target or a bundled server companion;
- whether the presence of multiple console scripts implies multiple public wheels or one uniform public license boundary.

This guide is intentionally narrower than a future build-system or multi-wheel extraction project. Its job is to make the current public packaging story explicit enough that external agents, external developers, and contributors do not have to guess.

## 1. Current distribution reality

Current public release-family anchor:

- distribution: `ait-native`
- version: `0.10.3`

Public naming rule for this release-facing slice:

- **`ait-native`** is the umbrella brand and package/distribution anchor;
- the concrete developer-facing component names remain `ait`, `aitk`,
  `ait-agent`, `ait-server`, and `ait-worker`; and
- adopting `ait-native` for the website or package-family copy does **not** by
  itself require an immediate rename of those console scripts.

Today, one repository checkout or one built distribution may expose multiple console scripts:

- `ait`
- `ait-agent`
- `ait-server`
- `ait-worker`
- `aitk`

Current install-channel note:

- the macOS Homebrew tap is a convenience path for this same `ait-native`
  distribution;
- it does not create a separate target boundary; and
- it does not add `ait-web` back into the public install surface.

Important boundary rule:

> **One combined distribution or checkout does not mean every command is the same package target, the same install promise, or the same public license surface.**

For the current legal surface map, also read:

- [docs/legal/public_release_license_summary.md](./legal/public_release_license_summary.md)
- [docs/legal/public_package_surface_map.json](./legal/public_package_surface_map.json)
- [docs/legal/component_license_matrix.md](./legal/component_license_matrix.md)
- [docs/legal/module_license_map.md](./legal/module_license_map.md)

## 2. Current release-facing package targets

| Target | Console script(s) | Primary use | Current artifact path | Boundary rule today | Extraction posture |
| --- | --- | --- | --- | --- | --- |
| `ait` | `ait` | local trust-layer CLI | `ait-native` | Apache-facing local workflow surface | defined public target; still shipped inside the combined distribution today |
| `ait-agent` | `ait-agent` | baseline transport/runtime helper | `ait-native` | Apache-facing baseline runtime surface | defined public target; durable workflow authority remains elsewhere |
| `ait-server` | `ait-server` | shared workflow authority | `ait-native[postgres]` today | AGPL/commercial self-hosted control-plane surface | target defined; not yet promised as a separate public wheel |
| `ait-worker` | `ait-worker` | async shared-control-plane worker | `ait-native[postgres]` today | same boundary as `ait-server` | bundled with `ait-server` by the current release-facing rule |
| `aitk` | `aitk` | local read-only history/browser companion | `ait-native` | Apache-facing local companion | defined public target; no shared control-plane promise |

## 3. Install/use decision by developer profile

### Local-only first loop

Use:

- `ait`

Optional:

- `aitk`
- `ait-agent`

Public developer meaning:

- the first local-only md → task → `land-local` loop should be understood as an `ait` target first;
- `aitk` and `ait-agent` are companions, not evidence that you need a shared deployment.

Related guides:

- [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md)
- [CONTRIBUTING.md](./CONTRIBUTING.md)

### Self-hosted shared control-plane core

Use together:

- `ait-server`
- `ait-worker`

Optional developer companion:

- `ait`

Public developer meaning:

- `ait-worker` should currently be treated as a server companion target, not as a separate standalone deployment line;
- PostgreSQL remains required for deployed shared usage.

Related guides:

- [SELF_HOSTED_TEAM_DEPLOYMENT.md](./SELF_HOSTED_TEAM_DEPLOYMENT.md)
- [COMPATIBILITY_MATRIX.md](./COMPATIBILITY_MATRIX.md)

## 4. What this slice does and does not promise

This slice **does** promise:

- one public explanation of the current package-target model;
- one explicit statement that `ait-worker` stays bundled with `ait-server` for now;
- one explicit split between package targets, install profiles, and license surfaces.

This slice **does not** promise:

- that every target already ships as its own separately published public wheel;
- that the monorepo has already been split into separate repositories;
- that future extraction boundaries are frozen forever.

## 5. Which document should you read next?

- shortest local-only developer path:
  [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md)
- self-hosted shared deployment path:
  [SELF_HOSTED_TEAM_DEPLOYMENT.md](./SELF_HOSTED_TEAM_DEPLOYMENT.md)
- supported topology/version combinations:
  [COMPATIBILITY_MATRIX.md](./COMPATIBILITY_MATRIX.md)
- current legal/package boundary map:
  [docs/legal/public_package_surface_map.json](./legal/public_package_surface_map.json)
- current contributor/repo-local workflow contract:
  [CONTRIBUTING.md](./CONTRIBUTING.md) and [LOCAL_DEVELOPMENT.md](./LOCAL_DEVELOPMENT.md)
