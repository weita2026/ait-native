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

The deployment/bootstrap surface for that topology now lives outside this repository. On a developer-managed machine, clone the sibling deployment repository at `../ait_docker` and treat **that** checkout as the owner of:

- `.env.example`
- `ait-docker.sh`
- image pinning/build policy
- shared runtime-root mount choices
- optional deployment surfaces

This repository intentionally does **not** ship `deploy/dev/**`, in-repo stack lifecycle entrypoints, or other deployment bootstrap assets anymore.

## 2. Prerequisites

Minimum baseline:

- a checkout of the sibling deployment repository at `../ait_docker`
- whatever host/runtime prerequisites that deployment repo currently requires
- host paths for PostgreSQL data and server runtime data that live **outside** the active `ait` repository checkout when you use bind mounts
- enough CLI access to run `ait` doctor/readiness commands from inside the developer-managed containers

Hard rules:

- deployed shared stacks are **PostgreSQL-only**
- do **not** use SQLite for deployed `ait-server` or `ait-worker`
- do **not** store shared runtime data inside `.ait`, `.ait-server`, or the active `ait` repository checkout

## 3. Prepare the deployment checkout

From the machine that owns the shared stack, switch into the sibling deployment repository:

```bash
cd ../ait_docker
cp .env.example .env
```

Review the deployment repo README there for the authoritative compose/image/runtime-root guidance.

If you use bind mounts, keep both the PostgreSQL data path and the `AIT_NATIVE_SERVER_DATA` path outside the `ait` checkout. The deployment repo should be the only place that decides Docker project names, compose files, and runtime-root mount layout.

## 4. Validate the deployment shape before boot

From `../ait_docker`:

```bash
./ait-docker.sh stack config
```

This should succeed before you start containers.

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

Reference commands from `../ait_docker`:

```bash
./ait-docker.sh stack up postgres
./ait-docker.sh stack up ait-server
./ait-docker.sh stack up ait-worker
```

## 6. Readiness and health checks

After boot, verify the shared control plane before you invite users or attach agent transports.

Process health:

```bash
curl -fsS https://<ait-server-host>/healthz
```

Database and readiness checks from `../ait_docker`:

```bash
./ait-docker.sh stack exec ait-server ait doctor postgres --connect --json
./ait-docker.sh stack exec ait-server ait repo readiness --json
./ait-docker.sh stack exec ait-server ait repo jobs --diagnostics --json
```

Minimum success criteria:

- `healthz` succeeds for every service you intentionally started
- PostgreSQL connectivity passes
- readiness reports the shared runtime as ready
- job diagnostics do not show unexplained stale/blocked worker state

## 7. Backup, restore, and upgrade discipline

Before treating the stack as a real team surface, set the developer runbook:

- back up **both** PostgreSQL content/control schemas **and** the server runtime root
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
- deployment lifecycle/bootstrap repo: sibling checkout `../ait_docker`
