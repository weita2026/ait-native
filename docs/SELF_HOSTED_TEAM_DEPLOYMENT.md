# ait Self-Hosted Team Deployment

## What this guide is for

Use this guide when:

- you are an external developer who needs a **shared** workflow authority instead of a local-only loop;
- you want PostgreSQL-backed `ait-server` plus `ait-worker`;
- you may later attach external runtime workers.

Do **not** use this guide when you only want the first local md → task → `land-local` loop. For that path, use [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md).

## 0. License and surface boundary first

The public self-hosted minimum is the **shared control-plane core**:

- `postgres`
- `ait-server`
- `ait-worker`

Important boundary:

- `ait-server` / `ait-worker` follow `AGPL-3.0-only OR LicenseRef-AIT-Commercial`

Read these first if you are deciding what you may deploy:

- [docs/legal/public_release_license_summary.md](./legal/public_release_license_summary.md)
- [docs/legal/public_package_surface_map.json](./legal/public_package_surface_map.json)
- [docs/legal/component_license_matrix.md](./legal/component_license_matrix.md)

## 1. Minimal supported topology

Treat the first public self-hosted shape as:

1. `postgres` — shared content/control-plane database
2. `ait-server` — shared workflow authority
3. `ait-worker` — async queue execution support
4. external runtime workers (`ait-agent`, Telegram, Discord, etc.) — optional follow-on attachments

The deployment/bootstrap surface for that topology now lives outside this repository. On a developer-managed machine, keep the shared-stack runtime configuration in an operator-managed workspace outside the active `ait` checkout and treat that workspace as the owner of:

- environment files and secret injection
- process-supervisor or service-manager definitions
- runtime-root mount choices
- package/build pinning policy
- optional deployment surfaces

This repository intentionally does **not** ship `deploy/dev/**`, in-repo stack lifecycle entrypoints, or other deployment bootstrap assets anymore.

## 2. Prerequisites

Minimum baseline:

- a trusted shell on the machine that will run `ait-server` and `ait-worker`
- PostgreSQL reachable from that machine
- host paths for PostgreSQL data, server runtime data, and repository retirement exports that live **outside** the active `ait` repository checkout when you use bind mounts
- enough CLI access to set runtime environment variables and run `ait` doctor/readiness commands

Hard rules:

- deployed shared stacks are **PostgreSQL-only**
- do **not** use SQLite for deployed `ait-server` or `ait-worker`
- do **not** store shared runtime data inside `.ait`, `.ait-server`, or the active `ait` repository checkout
- do **not** point `AIT_SERVER_RETIRE_EXPORT_ROOT` inside the active `AIT_NATIVE_SERVER_DATA` runtime root

## 3. Prepare the operator environment

From the machine that owns the shared stack, define the runtime environment before you start services:

```bash
export AIT_NATIVE_SERVER_DATA=/srv/ait/server-data
export AIT_SERVER_RETIRE_EXPORT_ROOT=/srv/ait/retire-exports
export AIT_NATIVE_SERVER_DB_BACKEND=postgres
export AIT_NATIVE_SERVER_POSTGRES_DSN='postgresql://<user>:<password>@<host>:5432/ait_native'
export AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA=ait_native_content
export AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA=ait_native_control
export AIT_NATIVE_SERVER_HOST=127.0.0.1
export AIT_NATIVE_SERVER_PORT=8088
mkdir -p "$AIT_NATIVE_SERVER_DATA" "$AIT_SERVER_RETIRE_EXPORT_ROOT"
```

Persist the same values in your service manager, shell profile, or other operator-managed runtime configuration.

If you use the checked-in `./ait.sh` wrapper with its default local host settings, `AIT_RUNTIME_ROOT` resolves to `/Volumes/lyravo/ait-runtime`, `AIT_NATIVE_SERVER_DATA` resolves to `/Volumes/lyravo/ait-runtime/server-data`, and `AIT_SERVER_RETIRE_EXPORT_ROOT` resolves to `/Volumes/lyravo/ait-runtime/retire-exports`.

If you use bind mounts, keep the PostgreSQL data path, the `AIT_NATIVE_SERVER_DATA` path, and the `AIT_SERVER_RETIRE_EXPORT_ROOT` path outside the `ait` checkout. Your operator-managed deployment workspace should be the only place that decides service names, runtime-root layout, and wrapper scripts.

## 4. Validate the deployment shape before boot

Example validation commands:

```bash
pg_isready -d "$AIT_NATIVE_SERVER_POSTGRES_DSN"
ait doctor runtime-root --server-data "$AIT_NATIVE_SERVER_DATA" --json
test -d "$AIT_SERVER_RETIRE_EXPORT_ROOT" && test -w "$AIT_SERVER_RETIRE_EXPORT_ROOT"
ait doctor postgres \
  --server-data "$AIT_NATIVE_SERVER_DATA" \
  --dsn "$AIT_NATIVE_SERVER_POSTGRES_DSN" \
  --content-schema "$AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA" \
  --control-schema "$AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA" \
  --connect \
  --json
```

These checks should succeed before you start long-lived shared services.

If you are using bind mounts, confirm:

- PostgreSQL data and server runtime data point at developer-managed paths
- those paths live outside the `ait` checkout
- no repo-managed local shared runtime is already writing to the same server-data directory

## 5. Boot order

Conceptual boot order:

1. `postgres`
2. `ait-server`
3. `ait-worker`
4. optional external runtime workers

Reference startup commands:

```bash
pg_isready -d "$AIT_NATIVE_SERVER_POSTGRES_DSN"
AIT_NATIVE_SERVER_DATA="$AIT_NATIVE_SERVER_DATA" \
AIT_SERVER_RETIRE_EXPORT_ROOT="$AIT_SERVER_RETIRE_EXPORT_ROOT" \
AIT_NATIVE_SERVER_DB_BACKEND=postgres \
AIT_NATIVE_SERVER_POSTGRES_DSN="$AIT_NATIVE_SERVER_POSTGRES_DSN" \
AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA="$AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA" \
AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA="$AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA" \
AIT_NATIVE_SERVER_HOST="$AIT_NATIVE_SERVER_HOST" \
AIT_NATIVE_SERVER_PORT="$AIT_NATIVE_SERVER_PORT" \
ait-server

AIT_NATIVE_SERVER_DATA="$AIT_NATIVE_SERVER_DATA" \
AIT_SERVER_RETIRE_EXPORT_ROOT="$AIT_SERVER_RETIRE_EXPORT_ROOT" \
AIT_NATIVE_SERVER_DB_BACKEND=postgres \
AIT_NATIVE_SERVER_POSTGRES_DSN="$AIT_NATIVE_SERVER_POSTGRES_DSN" \
AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA="$AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA" \
AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA="$AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA" \
ait-worker run --worker-id worker-1
```

If you run the shared stack under a service manager, keep the same environment values in the managed service definition instead of relying on an interactive shell.

If your deployment already uses `launchd`, `systemd`, or another real service
manager, keep that manager as the source of truth for normal restarts. The
repo helper below is the fallback for host-side direct-binary runs where
`ait-server` is started from the checked-in `./ait.sh` wrapper and `/healthz`
is the first failure signal.

## 6. Readiness and health checks

After boot, verify the shared control plane before you invite users or attach agent transports.

Process health:

```bash
curl -fsS "http://${AIT_NATIVE_SERVER_HOST}:${AIT_NATIVE_SERVER_PORT}/healthz"
```

Database preflight from the same runtime environment:

```bash
ait doctor postgres \
  --server-data "$AIT_NATIVE_SERVER_DATA" \
  --dsn "$AIT_NATIVE_SERVER_POSTGRES_DSN" \
  --content-schema "$AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA" \
  --control-schema "$AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA" \
  --connect \
  --json
```

Repository-scoped readiness checks from a trusted `ait` checkout already configured to talk to the running server:

```bash
ait repo readiness --json
ait repo jobs --diagnostics --json
```

Minimum success criteria:

- `healthz` succeeds for every service you intentionally started
- PostgreSQL connectivity passes
- readiness reports the shared runtime as ready
- job diagnostics do not show unexplained stale/blocked worker state

## 6A. Restart `ait-server` when `/healthz` is down

Keep one operator-managed env file outside the repository checkout with the same
values from section 3. Example path: `/srv/ait/ait-server.env`.

When the shared server is down or unresponsive, reload that env file into the
current shell and restart only `ait-server` through `ait.sh`:

```bash
if ! curl -fsS "http://${AIT_NATIVE_SERVER_HOST}:${AIT_NATIVE_SERVER_PORT}/healthz"; then
  set -a
  . /srv/ait/ait-server.env
  set +a
  ./ait.sh server restart
fi
```

Behavior:

- `./ait.sh server restart` only restarts `ait-server`; it does not touch
  PostgreSQL or `ait-worker`
- `./ait.sh server status` shows the current pid, URL, runtime root, and log
  paths before or after the restart
- `./ait.sh` keeps the pid file and log under its configured runtime/log roots,
  so the same wrapper can stop or adopt the recovered process on the next run

After the helper returns success, verify the process again:

```bash
curl -fsS "http://${AIT_NATIVE_SERVER_HOST}:${AIT_NATIVE_SERVER_PORT}/healthz"
./ait.sh server status
```

## 7. Backup, restore, and upgrade discipline

Before treating the stack as a real team surface, set the developer runbook:

- back up **both** PostgreSQL content/control schemas **and** the server runtime root
- treat `AIT_SERVER_RETIRE_EXPORT_ROOT` as operator-owned archive storage and include it in retention/replication policy when you use `repo retire`
- keep the runtime root outside the repo checkout
- stop new write traffic before quiesced backups, upgrades, or restore tests
- restore into a dedicated target before touching the only live copy

Use these documents as the developer authority:

- [docs/server_backup_restore_dr.md](./server_backup_restore_dr.md)
- [docs/server_disaster_recovery_checklist.md](./server_disaster_recovery_checklist.md)

If you want the daily rolling backup helper:

```bash
python3 scripts/runtime_backup.py --runtime-root "$AIT_NATIVE_SERVER_DATA" --output-dir /srv/ait-backups --keep 8
```

Do not treat database-only dumps as sufficient. Object payloads, refs, and runtime metadata still matter.

## 8. Decision boundary: stay local, self-host, or stop

Stay on [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md) when:

- one developer and one external agent are enough
- you do not need shared review/policy/landing or shared job execution

Use this self-hosted guide when:

- you need a shared workflow authority
- multiple users, shared review/policy, or worker-backed background execution are part of the requirement

Stop and revisit package/licensing first when:

- you need surfaces beyond the published self-hosted core
- you do not yet know which deployment/package rights apply to those extra surfaces

## 9. Where to go next

- local-only first loop: [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md)
- supported combinations and upgrade posture: [COMPATIBILITY_MATRIX.md](./COMPATIBILITY_MATRIX.md)
- package/extraction boundary map: [PACKAGE_TARGETS.md](./PACKAGE_TARGETS.md)
- contributor contract: [CONTRIBUTING.md](./CONTRIBUTING.md)
- repo-local development rules: [LOCAL_DEVELOPMENT.md](./LOCAL_DEVELOPMENT.md)
- operator-managed deployment workspace outside the active `ait` checkout
