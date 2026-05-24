from __future__ import annotations

import os
from typing import Any

from .patchset_ci import mark_patchset_ci_pending, patchset_ci_contract_available, run_patchset_ci
from .server_queue import enqueue_async_job
from .server_store import get_change, get_land_request, get_patchset


def _queue_mode() -> str:
    mode = os.environ.get("AIT_NATIVE_QUEUE_MODE", "inline").strip().lower()
    return mode if mode in {"inline", "async"} else "inline"


def _policy_job_payload(ctx: Any, patchset_id: str) -> dict[str, Any]:
    patchset = get_patchset(ctx, patchset_id)
    change = get_change(ctx, patchset["change_id"])
    return {
        "patchset_id": patchset_id,
        "repo_name": change["repo_name"],
        "repo_id": patchset.get("repo_id") or change.get("repo_id"),
        "change_id": change["change_id"],
        "change_seq": change.get("change_seq"),
        "patchset_number": patchset.get("patchset_number"),
    }


def _patchset_ci_job_payload(ctx: Any, patchset_id: str) -> dict[str, Any]:
    patchset = get_patchset(ctx, patchset_id)
    change = get_change(ctx, patchset["change_id"])
    return {
        "patchset_id": patchset_id,
        "repo_name": change["repo_name"],
        "repo_id": patchset.get("repo_id") or change.get("repo_id"),
        "change_id": change["change_id"],
        "change_seq": change.get("change_seq"),
        "patchset_number": patchset.get("patchset_number"),
    }


def _land_job_payload(ctx: Any, submission_id: str) -> dict[str, Any]:
    land = get_land_request(ctx, submission_id)
    change = get_change(ctx, land["change_id"])
    return {
        "submission_id": submission_id,
        "repo_name": change["repo_name"],
        "repo_id": land.get("repo_id") or change.get("repo_id"),
        "change_id": change["change_id"],
        "change_seq": change.get("change_seq"),
        "patchset_id": land.get("patchset_id"),
        "land_seq": land.get("land_seq"),
    }


def _maybe_enqueue_policy(ctx: Any, patchset_id: str) -> dict[str, Any] | None:
    if _queue_mode() != "async":
        return None
    payload = _policy_job_payload(ctx, patchset_id)
    return enqueue_async_job(ctx, payload["repo_name"], "policy.evaluate", payload, max_attempts=5, dedupe_active=True)


def _patchset_publish_policy_followup(patchset_id: str) -> dict[str, Any]:
    queue_mode = _queue_mode()
    reason = (
        "Patchset publish keeps policy evaluation off the request path until patchset evidence changes "
        "through attestation, review, patchset selection, or waiver actions."
    )
    policy_followup: dict[str, Any] = {
        "state": "deferred",
        "queue_mode": queue_mode,
        "reason": reason,
        "activation_events": [
            "patchset.selected",
            "attestation.upserted",
            "review.recorded",
            "policy.waived",
        ],
    }
    if queue_mode == "inline":
        policy_followup["command"] = f"ait policy eval {patchset_id}"
    return {"policy_followup": policy_followup}


def _maybe_enqueue_patchset_ci(ctx: Any, patchset_id: str) -> dict[str, Any] | None:
    result = _maybe_start_patchset_ci(ctx, patchset_id, trigger="queued_rerun")
    if isinstance(result, dict) and isinstance(result.get("ci_job"), dict):
        return dict(result["ci_job"])
    return None


def _maybe_follow_patchset_publish_with_ci(ctx: Any, patchset_id: str) -> dict[str, Any] | None:
    if not patchset_ci_contract_available(ctx, patchset_id):
        return None
    if _queue_mode() == "async":
        return _maybe_start_patchset_ci(ctx, patchset_id, trigger="patchset_publish")
    return {
        "ci_followup": {
            "state": "deferred",
            "trigger": "patchset_publish",
            "queue_mode": _queue_mode(),
            "reason": "Patchset publish keeps patchset CI off the inline request path; run patchset CI explicitly through the dedicated CI route.",
            "command": f"ait patchset rerun-ci {patchset_id}",
        }
    }


def _maybe_start_patchset_ci(ctx: Any, patchset_id: str, *, trigger: str) -> dict[str, Any] | None:
    if not patchset_ci_contract_available(ctx, patchset_id):
        return None
    if _queue_mode() == "async":
        mark_patchset_ci_pending(ctx, patchset_id, trigger=trigger, job_state="queued")
        payload = _patchset_ci_job_payload(ctx, patchset_id)
        return {
            "ci_job": enqueue_async_job(
                ctx,
                payload["repo_name"],
                "patchset.ci",
                payload,
                max_attempts=3,
                dedupe_active=True,
            )
        }
    return {
        "ci_result": run_patchset_ci(ctx, patchset_id, trigger=trigger),
    }


def _maybe_enqueue_land(ctx: Any, submission_id: str) -> dict[str, Any] | None:
    if _queue_mode() != "async":
        return None
    payload = _land_job_payload(ctx, submission_id)
    return enqueue_async_job(ctx, payload["repo_name"], "land.process", payload, max_attempts=5, dedupe_active=True)
