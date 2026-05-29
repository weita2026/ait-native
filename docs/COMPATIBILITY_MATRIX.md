# ait Compatibility Matrix

Authority: command layer under [plan.md](./plan.md), the applicable legal-layer governance documents, and [PACKAGE_TARGETS.md](./PACKAGE_TARGETS.md).
Status: current release-facing compatibility matrix.
Scope: supported local-only and self-hosted shared-control-plane combinations, version-skew policy, and upgrade direction for the `ait-native` release family.

## What this matrix is for

Use this matrix when you need to answer:

- which current combinations are supported for the **local-only** path;
- which current combinations are supported for the **self-hosted shared control plane**;
- whether version skew is tolerated across `ait`, `ait-server`, and `ait-worker`;
- which upgrade order is supported for the current public release family.

This matrix is intentionally conservative. It tells you what is supported **now**, not every combination that might happen to boot.

## 1. Current release-family rule

Current public release-family anchor:

- distribution: `ait-native`
- version: `0.10.6`

Today, the release-facing support rule is:

> **The only supported shared-surface combinations are same-release-family combinations.**

That means:

- local-only usage can stay on the `ait` trust-layer path without shared services;
- deployed shared stacks should keep `ait-server` and `ait-worker` on the **same release-family version**;
- long-lived mixed-version shared deployments are **not** a supported developer target yet.

This matrix does **not** imply that every console script exposed by one checkout shares the same public license boundary. For release-facing package and license boundaries, also read:

- [docs/legal/public_release_license_summary.md](./legal/public_release_license_summary.md)
- [docs/legal/public_package_surface_map.json](./legal/public_package_surface_map.json)
- [docs/legal/component_license_matrix.md](./legal/component_license_matrix.md)

## 2. Supported combinations

| Profile | Required surfaces | Optional surfaces | Current version rule | Supported | Notes |
| --- | --- | --- | --- | --- | --- |
| Local-only first loop | `ait` | `ait-agent` | `ait-native 0.10.6` for the local CLI path | Yes | No PostgreSQL, `ait-server`, or `ait-worker` required. Start here with [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md). |
| Local CLI plus transport/runtime helper | `ait` | `ait-agent` | Same release family recommended | Yes | Durable shared workflow authority is still out of scope unless you add the self-hosted control plane. |
| Self-hosted shared control-plane core | PostgreSQL, `ait-server`, `ait-worker` | `ait` CLI for developer checks | Same release-family version across the shared core | Yes | This is the minimum supported shared deployment shape. Use [SELF_HOSTED_TEAM_DEPLOYMENT.md](./SELF_HOSTED_TEAM_DEPLOYMENT.md). |

## 3. Skew tolerance

Current public skew policy:

- **Local-only path:** no shared skew question exists because the first loop does not require server/web/worker/PostgreSQL.
- **Shared control plane:** no long-lived version skew is currently supported across `ait-server` and `ait-worker`.
- **Developer CLI against a shared deployment:** use the same release-family version as the shared stack when you expect support.

Practical developer rule:

> a short rolling-restart window may exist during an upgrade, but the supported target state is still **full convergence on one release-family version**.

If you need a broader skew promise later, that belongs in a future compatibility-matrix revision after separate packaging targets and public release process hardening land.

## 4. Unsupported mixes

These are intentionally **unsupported** today:

- deployed shared `ait-server` / `ait-worker` stacks backed by SQLite instead of PostgreSQL
- `ait-worker` running as a long-lived shared service without the matching `ait-server`
- long-lived mixed-version shared deployments where server / worker / developer CLI intentionally stay on different release families
- assuming that a monorepo checkout exposing multiple commands creates one uniform Apache public surface

## 5. Upgrade direction

For the shared control-plane path, use this upgrade order:

1. take a quiesced backup of PostgreSQL plus the server runtime root
2. upgrade `ait-server`
3. upgrade `ait-worker`
4. run readiness and job diagnostics
5. reconnect developer CLI usage and any external runtime workers

Required developer checks after upgrade:

```bash
ait repo readiness --json
ait repo jobs --diagnostics --json
ait doctor postgres --connect --json
```

Backup / restore / DR authority:

- [docs/server_backup_restore_dr.md](./server_backup_restore_dr.md)
- [docs/server_disaster_recovery_checklist.md](./server_disaster_recovery_checklist.md)
- [docs/ait_native_runtime_operations.md](./ait_native_runtime_operations.md)

## 6. Which guide should you use?

- first local md → task → `land-local` loop:
  [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md)
- shared PostgreSQL-backed deployment:
  [SELF_HOSTED_TEAM_DEPLOYMENT.md](./SELF_HOSTED_TEAM_DEPLOYMENT.md)
- package/extraction boundary map:
  [PACKAGE_TARGETS.md](./PACKAGE_TARGETS.md)
- contribution and repo-local development rules:
  [CONTRIBUTING.md](./CONTRIBUTING.md) and [LOCAL_DEVELOPMENT.md](./LOCAL_DEVELOPMENT.md)
