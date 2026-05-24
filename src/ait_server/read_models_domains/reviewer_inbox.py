from __future__ import annotations

from typing import Any

from ..server_paths import ServerContext
from .task_queue import _cache_value, _cache_value_missing_as_none, _scoped_hydrated_changes


def _legacy_read_models_module():
    from .. import read_models as legacy_read_models

    return legacy_read_models


def reviewer_inbox(
    ctx: ServerContext,
    repo_name: str | None = None,
    *,
    author_class: str | None = None,
    author_mode: str | None = None,
    tests: str | None = None,
    policy: str | None = None,
    freshness: str | None = None,
    review: str | None = None,
    cache: dict[str, dict[Any, Any]] | None = None,
) -> dict[str, Any]:
    rm = _legacy_read_models_module()
    author_class_filter = rm._normalize_inbox_filter(author_class)
    author_mode_filter = rm._normalize_inbox_filter(author_mode)
    tests_filter = rm._normalize_inbox_filter(tests)
    policy_filter = rm._normalize_inbox_filter(policy)
    freshness_filter = rm._normalize_inbox_filter(freshness)
    review_filter = rm._normalize_inbox_filter(review)
    items: list[dict[str, Any]] = []
    for change in _scoped_hydrated_changes(ctx, repo_name, cache=cache):
        change_id = change["change_id"]
        if change["status"] not in rm.REVIEWABLE_CHANGE_STATES:
            continue
        task = _cache_value(
            cache,
            "task",
            (change["repo_name"], change["task_id"]),
            lambda: rm._repo_scoped_task(ctx, change["task_id"], change["repo_name"]),
        )
        current_patchset_id = change.get("current_patchset_id")
        current_patchset = _cache_value(
            cache,
            "patchset",
            (change["repo_name"], current_patchset_id),
            lambda: rm._repo_scoped_patchset(ctx, current_patchset_id, change["repo_name"]),
        )
        current_policy = (
            _cache_value(
                cache,
                "policy",
                current_patchset_id,
                lambda: rm.get_policy_status(ctx, current_patchset_id),
            )
            if current_patchset_id
            else {"decision": "pending", "checks": [], "lane": change["lane"]}
        )
        current_attestation = (
            _cache_value_missing_as_none(
                cache,
                "attestation",
                current_patchset_id,
                lambda: rm.get_attestation(ctx, current_patchset_id),
            )
            if current_patchset_id
            else None
        )
        attestation_author_mode = current_attestation.get("author_mode") if current_attestation else None
        attestation_tests = rm._effective_validation_state(
            current_policy,
            current_attestation,
            key="tests",
            requirement_key="require_tests",
        )
        if not rm._matches_author_class(attestation_author_mode, author_class_filter):
            continue
        if not rm._matches_inbox_filter(attestation_author_mode, author_mode_filter):
            continue
        if not rm._matches_inbox_filter(attestation_tests, tests_filter):
            continue
        review_summary = _cache_value(
            cache,
            "reviews",
            change_id,
            lambda: rm.list_reviews(ctx, change_id),
        )
        requested_groups = (
            sorted({row["reviewer_group"] for row in review_summary["review_requests"] if row["patchset_id"] == current_patchset_id})
            if current_patchset_id
            else []
        )
        base_is_fresh = True
        if current_patchset is not None:
            base_head = _cache_value(
                cache,
                "ref",
                (change["repo_name"], change["base_line"]),
                lambda: rm.read_ref(ctx, change["repo_name"], change["base_line"]),
            )
            base_is_fresh = base_head == current_patchset["base_snapshot_id"] if base_head else False
        freshness_state = "fresh" if base_is_fresh else "stale"
        policy_decision = str(current_policy.get("decision", "pending"))
        if not rm._matches_inbox_filter(policy_decision, policy_filter):
            continue
        if not rm._matches_inbox_filter(freshness_state, freshness_filter):
            continue
        if not rm._matches_review_filter(review_summary, requested_groups, review_filter):
            continue
        if repo_name is not None:
            try:
                patchsets = _cache_value(
                    cache,
                    "patchsets_for_repo",
                    (repo_name, change_id),
                    lambda: rm.list_patchsets_for_repo(ctx, repo_name, change_id),
                )
            except KeyError:
                patchsets = _cache_value(
                    cache,
                    "patchsets",
                    change_id,
                    lambda: rm.list_patchsets(ctx, change_id),
                )
        else:
            patchsets = _cache_value(
                cache,
                "patchsets",
                change_id,
                lambda: rm.list_patchsets(ctx, change_id),
            )
        selected_patchset = _cache_value(
            cache,
            "patchset",
            (change["repo_name"], change["selected_patchset_id"]),
            lambda: rm._repo_scoped_patchset(ctx, change["selected_patchset_id"], change["repo_name"]),
        )
        landing_summary = _cache_value(
            cache,
            "landing_summary",
            change_id,
            lambda: rm._latest_land_summary(ctx, change_id),
        )
        items.append(
            {
                "change_id": change_id,
                "title": change["title"],
                "repo": change["repo_name"],
                "base_line": change["base_line"],
                "task": {
                    "task_id": task["task_id"],
                    "title": task["title"],
                    "status": task["status"],
                    "intent": task["intent"],
                },
                "lane": change["lane"],
                "risk_tier": change["risk_tier"],
                "change_status": change["status"],
                "current_patchset": {
                    "patchset_id": current_patchset_id,
                    "patchset_number": current_patchset["patchset_number"] if current_patchset else 0,
                },
                "selected_patchset": {
                    "patchset_id": selected_patchset["patchset_id"],
                    "patchset_number": selected_patchset["patchset_number"],
                }
                if selected_patchset is not None
                else None,
                "patchsets": [
                    {
                        "patchset_id": patchset["patchset_id"],
                        "patchset_number": patchset["patchset_number"],
                        "summary": patchset.get("summary"),
                    }
                    for patchset in patchsets
                ],
                "review_state": {
                    "approvals": review_summary["approvals"],
                    "blocking": review_summary["blocking"],
                    "comments": review_summary["comments"],
                },
                "policy_state": {
                    "decision": policy_decision,
                    "missing_requirements": rm._missing_requirements(current_policy),
                },
                "freshness": {"base_is_fresh": base_is_fresh, "state": freshness_state},
                "attestation": {
                    "completeness": "summary_present" if current_attestation else "missing",
                    "author_mode": attestation_author_mode,
                    "model_name": (current_attestation.get("provenance_summary") or {}).get("model_name") if current_attestation else None,
                    "session_id": (current_attestation.get("provenance_summary") or {}).get("session_id") if current_attestation else None,
                    "checkpoint_id": (current_attestation.get("provenance_summary") or {}).get("checkpoint_id") if current_attestation else None,
                    "evidence_readiness": (current_attestation.get("provenance_summary") or {}).get("evidence_readiness")
                    if current_attestation
                    else None,
                    "tests": attestation_tests,
                    "updated_at": current_attestation.get("updated_at") if current_attestation else None,
                },
                "landing_summary": landing_summary,
                "requested_groups": requested_groups,
                "updated_at": change["updated_at"],
            }
        )
    return {
        "items": items,
        "count": len(items),
        "filters": {
            "repo_name": repo_name,
            "author_class": author_class_filter,
            "author_mode": author_mode_filter,
            "tests": tests_filter,
            "policy": policy_filter,
            "freshness": freshness_filter,
            "review": review_filter,
        },
    }
