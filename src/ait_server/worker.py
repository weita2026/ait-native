from __future__ import annotations

import os
import time
from pathlib import Path
from typing import Any

import typer
from rich import print as rprint

from .server_paths import ServerContext
from .patchset_ci import run_patchset_ci
from .repo_ci import run_repo_ci
from .server_queue import (
    JobPayloadError,
    claim_next_job,
    complete_job,
    fail_job,
    get_job,
    list_jobs,
    normalize_async_job_payload,
    reclaim_stale_jobs,
    retry_delay_seconds_for_job,
)
from .server_store import (
    initialize,
    _process_land,
    evaluate_policy,
    evaluate_policy_for_repo,
    optimize_repository_storage,
    pack_repository_storage,
    process_land_for_repo,
    reconcile_repository,
    gc_repository_storage,
)

app = typer.Typer(help="Run native ait background jobs")


def _ctx() -> ServerContext:
    ctx = ServerContext.from_env()
    initialize(ctx)
    return ctx


def process_one(
    ctx: ServerContext,
    worker_id: str = "worker-1",
    repo_name: str | None = None,
    reclaim_stale_seconds: int = 0,
) -> dict[str, Any] | None:
    if reclaim_stale_seconds > 0:
        reclaim_stale_jobs(ctx, reclaim_stale_seconds, repo_name=repo_name)
    job = claim_next_job(ctx, worker_id, repo_name=repo_name)
    if job is None:
        return None
    try:
        result = _dispatch(ctx, job)
        complete_job(ctx, int(job["job_id"]), result)
        job = get_job(ctx, int(job["job_id"]))
        return job
    except JobPayloadError as exc:
        fail_job(ctx, int(job["job_id"]), str(exc), retryable=False)
        return get_job(ctx, int(job["job_id"]))
    except Exception as exc:  # pragma: no cover - exercised through API/worker tests
        fail_job(
            ctx,
            int(job["job_id"]),
            str(exc),
            retryable=True,
            retry_delay_seconds=retry_delay_seconds_for_job(str(job.get("job_type") or "")),
        )
        return get_job(ctx, int(job["job_id"]))



def _dispatch(ctx: ServerContext, job: dict[str, Any]) -> dict[str, Any]:
    job_type = job["job_type"]
    payload = normalize_async_job_payload(job_type, job.get("payload"))

    def _change_ref_for_repo_scope() -> str | None:
        change_seq = payload.get("change_seq")
        if change_seq is not None:
            return str(int(change_seq))
        change_id = str(payload.get("change_id") or "").strip()
        return change_id or None

    def _repo_scope_args() -> tuple[str | None, str | None]:
        repo_name = str(payload.get("repo_name") or "").strip() or None
        repo_id = str(payload.get("repo_id") or "").strip() or None
        return repo_name, repo_id

    if job_type == "policy.evaluate":
        repo_name, repo_id = _repo_scope_args()
        if repo_name:
            change_ref = _change_ref_for_repo_scope()
            patchset_number = payload.get("patchset_number")
            try:
                if patchset_number is not None and change_ref is not None:
                    return evaluate_policy_for_repo(
                        ctx,
                        repo_name,
                        str(int(patchset_number)),
                        change_ref=change_ref,
                        repo_id=repo_id,
                    )
                return evaluate_policy_for_repo(
                    ctx,
                    repo_name,
                    payload["patchset_id"],
                    change_ref=change_ref,
                    repo_id=repo_id,
                )
            except (KeyError, ValueError) as exc:
                raise JobPayloadError(str(exc)) from exc
        return evaluate_policy(ctx, payload["patchset_id"])
    if job_type == "patchset.ci":
        try:
            return run_patchset_ci(ctx, payload["patchset_id"], trigger="worker_job")
        except (KeyError, ValueError) as exc:
            raise JobPayloadError(str(exc)) from exc
    if job_type == "repo.ci":
        try:
            return run_repo_ci(
                ctx,
                payload["repo_name"],
                suite_ids=payload.get("suite_ids"),
                plane=payload.get("plane"),
                target_line=str(payload.get("target_line") or "main"),
                trigger=str(payload.get("trigger") or "worker_job"),
                selector=payload.get("selector"),
                task_ids=payload.get("task_ids"),
                curated_corpus=payload.get("curated_corpus"),
                count=payload.get("count"),
                window_days=payload.get("window_days"),
                dependency_evidence=payload.get("dependency_evidence"),
                compliance_evidence=payload.get("compliance_evidence"),
            )
        except (KeyError, ValueError) as exc:
            raise JobPayloadError(str(exc)) from exc
    if job_type == "land.process":
        repo_name, repo_id = _repo_scope_args()
        if repo_name:
            land_seq = payload.get("land_seq")
            try:
                if land_seq is not None:
                    return process_land_for_repo(ctx, repo_name, str(int(land_seq)), repo_id=repo_id)
                return process_land_for_repo(ctx, repo_name, payload["submission_id"], repo_id=repo_id)
            except (KeyError, ValueError) as exc:
                raise JobPayloadError(str(exc)) from exc
        return _process_land(ctx, payload["submission_id"])
    if job_type == "reconcile.repo":
        return reconcile_repository(ctx, payload["repo_name"], repair=bool(payload.get("repair", False)))
    if job_type == "content.pack":
        return pack_repository_storage(
            ctx,
            payload["repo_name"],
            repack=bool(payload.get("repack", False)),
            max_members=payload.get("max_members"),
        )
    if job_type == "content.optimize":
        return optimize_repository_storage(ctx, payload["repo_name"], repair=bool(payload.get("repair", True)))
    if job_type == "content.gc":
        return gc_repository_storage(
            ctx,
            payload["repo_name"],
            prune_unreferenced=bool(payload.get("prune_unreferenced", True)),
            prune_orphan_packs=bool(payload.get("prune_orphan_packs", True)),
        )
    raise ValueError(f"Unsupported job type: {job_type}")


@app.command("run")
def run_cmd(
    once: bool = typer.Option(False, "--once", help="Process at most one job and exit."),
    until_empty: bool = typer.Option(False, "--until-empty", help="Drain available jobs and exit."),
    repo: str | None = typer.Option(None, "--repo", help="Restrict worker to one repository."),
    poll_seconds: float = typer.Option(1.0, "--poll-seconds", help="Polling interval when waiting for jobs."),
    worker_id: str = typer.Option("worker-1", "--worker-id", help="Worker identity label."),
    reclaim_stale_seconds: int = typer.Option(0, "--reclaim-stale-seconds", help="Requeue stale running jobs older than this many seconds before claiming work."),
):
    ctx = _ctx()
    processed = 0
    while True:
        job = process_one(
            ctx,
            worker_id=worker_id,
            repo_name=repo,
            reclaim_stale_seconds=reclaim_stale_seconds,
        )
        if job is None:
            if once or until_empty:
                break
            time.sleep(poll_seconds)
            continue
        processed += 1
        state = job["state"]
        rprint({"job_id": job["job_id"], "job_type": job["job_type"], "state": state, "attempt_count": job["attempt_count"]})
        if once:
            break
    rprint({"processed": processed})


@app.command("list")
def list_cmd(
    repo: str | None = typer.Option(None, "--repo"),
    state: str | None = typer.Option(None, "--state"),
    limit: int = typer.Option(50, "--limit"),
):
    ctx = _ctx()
    rows = list_jobs(ctx, repo_name=repo, state=state, limit=limit)
    rprint(rows)


@app.command("reclaim-stale")
def reclaim_stale_cmd(
    stale_seconds: int = typer.Option(300, "--stale-seconds", min=1),
    repo: str | None = typer.Option(None, "--repo"),
):
    ctx = _ctx()
    rprint(reclaim_stale_jobs(ctx, stale_seconds, repo_name=repo))



def main() -> None:
    app()
