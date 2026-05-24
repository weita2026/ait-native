from __future__ import annotations

import json
from pathlib import Path, PurePosixPath
from typing import Any

from ait_protocol.common import derive_policy_author_class, derive_policy_content_class, resolve_effective_policy

from ..server_content import read_blob_bytes, snapshot_manifest_map
from ..server_control import latest_policy_status
from .plans import _normalize_optional_text

RULE_LABELS = {
    "require_attestation": "Patchset must include attestation",
    "ai_provenance": "AI provenance must be policy-readable",
    "tests": "Tests must pass",
    "lint": "Lint must pass",
    "security_scan": "Security scan must pass",
    "license_scan": "License scan must pass",
    "code_review_summary": "Code review summary must be recorded",
    "required_human_review": "Required human approvals must be present",
}

POLICY_REQUIREMENT_MAP = {
    "tests": "require_tests",
    "lint": "require_lint",
    "security_scan": "require_security_scan",
    "license_scan": "require_license_scan",
}

CODE_REVIEW_SUMMARY_ACTION = "code_review_summary"
TASK_REVIEW_APPROVE_ACTION = "task_approve"
TASK_REVIEW_REQUEST_CHANGES_ACTION = "task_request_changes"
TASK_REVIEW_COMMENT_ACTION = "task_comment"
TASK_REVIEW_DEFER_ACTION = "task_defer"
TEAM_REVIEW_APPROVE_ACTION = "approve"
TASK_REVIEW_DECISION_ACTIONS = {
    TASK_REVIEW_APPROVE_ACTION,
    TASK_REVIEW_REQUEST_CHANGES_ACTION,
    TASK_REVIEW_DEFER_ACTION,
}
TEAM_REVIEW_DECISION_ACTIONS = {
    TEAM_REVIEW_APPROVE_ACTION,
    "request_changes",
    "defer",
}


def _review_decision_lane(action: str) -> str | None:
    if action in TASK_REVIEW_DECISION_ACTIONS:
        return "task"
    if action in TEAM_REVIEW_DECISION_ACTIONS:
        return "team"
    return None


def _release_artifact_download_path(release_id: str, kind: str) -> str:
    return f"/v1/native/releases/{release_id}/artifacts/{kind}"


def _release_artifact_media_type(kind: str, path: str) -> str:
    normalized_kind = str(kind or "").strip().lower()
    normalized_path = str(path or "").strip().lower()
    if normalized_kind == "manifest" or normalized_path.endswith(".manifest.json"):
        return "application/json"
    if normalized_kind == "checksum" or normalized_path.endswith(".sha256"):
        return "text/plain; charset=utf-8"
    if normalized_kind == "formula" or normalized_path.endswith(".rb"):
        return "text/plain; charset=utf-8"
    if normalized_kind == "wheel" or normalized_path.endswith(".whl"):
        return "application/octet-stream"
    if normalized_kind == "sdist" or normalized_path.endswith(".tar.gz"):
        return "application/gzip"
    return "application/octet-stream"


def _sanitize_release_artifact_path(value: str | None) -> str:
    text = _normalize_optional_text(value) or "artifact"
    candidate = Path(text)
    if not candidate.is_absolute():
        return candidate.as_posix()
    if len(candidate.parts) >= 2:
        return Path(candidate.parts[-2], candidate.parts[-1]).as_posix()
    return candidate.name or "artifact"


def _release_artifact_view(release_id: str, artifact: dict[str, Any]) -> dict[str, Any]:
    out = dict(artifact)
    kind = str(out.get("kind") or "").strip()
    out["download_path"] = _release_artifact_download_path(release_id, kind)
    if not out.get("download_name"):
        out["download_name"] = Path(str(out.get("path") or "")).name or f"{release_id}-{kind}"
    return out


def _release_row(row: dict[str, Any] | Any) -> dict[str, Any]:
    out = dict(row)
    out["line"] = out.pop("line_name")
    out["package"] = {
        "name": out.pop("package_name", None),
        "version": out.pop("package_version", None),
        "requires_python": out.pop("package_requires_python", None),
    }
    for source_key, target_key, default in (
        ("checks_json", "checks", []),
        ("artifacts_json", "artifacts", []),
        ("formula_json", "formula", {}),
        ("metadata_json", "metadata", {}),
    ):
        raw = out.pop(source_key, None)
        try:
            out[target_key] = json.loads(raw or json.dumps(default))
        except Exception:
            out[target_key] = default
    artifacts = [
        _release_artifact_view(str(out["release_id"]), artifact)
        for artifact in out.get("artifacts", [])
        if isinstance(artifact, dict)
    ]
    out["artifacts"] = artifacts
    formula = out.get("formula") if isinstance(out.get("formula"), dict) else {}
    if formula:
        source_kind = str(formula.get("artifact_kind") or "sdist")
        source_artifact = next((artifact for artifact in artifacts if str(artifact.get("kind") or "") == source_kind), None)
        formula_artifact = next((artifact for artifact in artifacts if str(artifact.get("kind") or "") == "formula"), None)
        if source_artifact is not None:
            formula["url"] = source_artifact["download_path"]
            formula["sha256"] = source_artifact.get("sha256")
        if formula_artifact is not None:
            formula["path"] = formula_artifact.get("path")
            formula["download_path"] = formula_artifact["download_path"]
    out["formula"] = formula
    out["next_action"] = {
        "code": "published_remote",
        "detail": "Release is published to ait-server with downloadable release artifacts.",
    }
    return out


def _patchset_diff_stats(patchset: dict[str, Any] | Any) -> dict[str, Any]:
    if isinstance(patchset, dict):
        if isinstance(patchset.get("diff_stats"), dict):
            return patchset["diff_stats"]
        raw = patchset.get("diff_stats_json")
    else:
        raw = patchset["diff_stats_json"]
    try:
        return json.loads(raw or "{}")
    except Exception:
        return {}


def _patchset_changed_paths(patchset: dict[str, Any] | Any) -> list[str]:
    diff_stats = _patchset_diff_stats(patchset)
    paths = diff_stats.get("paths") or {}
    changed_paths: list[str] = []
    for key in ("added", "deleted", "modified"):
        for path in paths.get(key) or []:
            text = str(path).strip()
            if text:
                changed_paths.append(text)
    return sorted(dict.fromkeys(changed_paths))


def _policy_context_for_patchset(
    repo_policy: dict[str, Any] | None,
    patchset: dict[str, Any] | Any,
    attestation_row: Any | None,
) -> dict[str, Any]:
    changed_paths = _patchset_changed_paths(patchset)
    content_class = derive_policy_content_class(changed_paths)
    author_mode_value = attestation_row["author_mode"] if attestation_row is not None else patchset["author_mode"]
    author_class = derive_policy_author_class(author_mode_value)
    resolved = resolve_effective_policy(
        repo_policy,
        content_class=content_class,
        author_class=author_class,
    )
    provenance_summary = json.loads(attestation_row["provenance_summary_json"]) if attestation_row is not None else {}
    resolved.update(
        {
            "changed_paths": changed_paths,
            "provenance_summary": provenance_summary,
        }
    )
    return resolved


def _requires_code_review_summary(policy_context: dict[str, Any]) -> bool:
    effective_requirements = policy_context.get("effective_requirements")
    configured_requirement = False
    if isinstance(effective_requirements, dict):
        configured_requirement = bool(effective_requirements.get("require_code_review_summary", False))
    return configured_requirement or (
        policy_context.get("content_class") == "code_change"
        and policy_context.get("author_class") == "ai_related"
    )


def _dedupe_text_values(values: list[Any] | tuple[Any, ...] | None) -> list[str]:
    seen: set[str] = set()
    out: list[str] = []
    for item in list(values or []):
        text = str(item or "").strip()
        if not text or text in seen:
            continue
        seen.add(text)
        out.append(text)
    return out


_CHECKED_IN_CI_CONTRACT_PATH = "ci/config.contract.json"


def _load_snapshot_ci_contract(ctx, snapshot_id: str) -> dict[str, Any]:
    snapshot_id = str(snapshot_id or "").strip()
    if not snapshot_id:
        return {"ci": {}, "suites": {}}
    try:
        manifest = snapshot_manifest_map(ctx, snapshot_id)
    except Exception:
        return {"ci": {}, "suites": {}}

    def _load_json_artifact(path: str) -> dict[str, Any]:
        entry = manifest.get(path)
        if not isinstance(entry, dict):
            return {}
        blob_id = str(entry.get("blob_id") or "").strip()
        if not blob_id:
            return {}
        try:
            payload = json.loads(read_blob_bytes(ctx, blob_id).decode("utf-8"))
        except Exception:
            return {}
        return dict(payload) if isinstance(payload, dict) else {}

    config_payload = _load_json_artifact(".ait/config.json")
    ci_config = dict(config_payload.get("ci") or {}) if isinstance(config_payload.get("ci"), dict) else {}
    if not ci_config:
        contract_payload = _load_json_artifact(_CHECKED_IN_CI_CONTRACT_PATH)
        nested_contract = contract_payload.get("ci") if isinstance(contract_payload.get("ci"), dict) else None
        if isinstance(nested_contract, dict):
            ci_config = dict(nested_contract)

    suites: dict[str, dict[str, Any]] = {}
    for path in sorted(manifest):
        parts = PurePosixPath(path).parts
        if parts[:2] != ("ci", "suites") or not path.endswith(".json"):
            continue
        payload = _load_json_artifact(path)
        suite_id = str(payload.get("suite_id") or Path(path).stem).strip()
        if not suite_id:
            continue
        payload["_artifact_path"] = path
        suites[suite_id] = payload
    return {"ci": ci_config, "suites": suites}


def _ci_rollout_for_patchset(ctx, patchset: dict[str, Any] | Any, attestation_row: Any | None) -> dict[str, Any] | None:
    snapshot_id = str(patchset["revision_snapshot_id"] or "").strip()
    contract = _load_snapshot_ci_contract(ctx, snapshot_id)
    suites_by_id = dict(contract.get("suites") or {})
    if not suites_by_id:
        return None

    patchset_suite_ids = [
        suite_id
        for suite_id, suite in suites_by_id.items()
        if str(suite.get("plane") or "").strip().lower() == "patchset"
    ]
    if not patchset_suite_ids:
        return None

    ci_config = dict(contract.get("ci") or {})
    required_suite_ids = _dedupe_text_values(ci_config.get("required_patchset_suites") or [])
    informational_suite_ids = _dedupe_text_values(ci_config.get("informational_patchset_suites") or [])
    if not required_suite_ids:
        required_suite_ids = [
            suite_id
            for suite_id in patchset_suite_ids
            if bool((suites_by_id.get(suite_id) or {}).get("default_blocking", False))
        ]
    if not informational_suite_ids:
        informational_suite_ids = [
            suite_id
            for suite_id in patchset_suite_ids
            if suite_id not in required_suite_ids
        ]

    rollout = dict(ci_config.get("rollout") or {})
    try:
        phase = int(rollout.get("phase") if rollout.get("phase") is not None else 0)
    except (TypeError, ValueError):
        phase = 0
    promotion_candidates = rollout.get("promotion_candidates")
    promotion_candidates = dict(promotion_candidates) if isinstance(promotion_candidates, dict) else {}

    detail_payload: dict[str, Any] = {}
    if attestation_row is not None:
        try:
            decoded = json.loads(attestation_row["detail_json"] or "{}")
        except Exception:
            decoded = {}
        if isinstance(decoded, dict):
            detail_payload = decoded
    patchset_ci = detail_payload.get("patchset_ci") if isinstance(detail_payload.get("patchset_ci"), dict) else {}
    suite_results_by_id: dict[str, dict[str, Any]] = {}
    for item in list(patchset_ci.get("suite_results") or []):
        if not isinstance(item, dict):
            continue
        suite_id = str(item.get("suite_id") or "").strip()
        if not suite_id:
            continue
        suite_results_by_id[suite_id] = dict(item)

    return {
        "phase": max(phase, 0),
        "required_patchset_suites": required_suite_ids,
        "informational_patchset_suites": informational_suite_ids,
        "promotion_candidates": promotion_candidates,
        "suite_results_by_id": suite_results_by_id,
        "selected_suite_ids": _dedupe_text_values(patchset_ci.get("selected_suite_ids") or list(suite_results_by_id)),
        "suites_by_id": suites_by_id,
    }


def _ci_rollout_summary_message(rollout_context: dict[str, Any]) -> str:
    phase = int(rollout_context.get("phase") or 0)
    required = ", ".join(f"`{item}`" for item in rollout_context.get("required_patchset_suites") or []) or "none"
    informational = ", ".join(f"`{item}`" for item in rollout_context.get("informational_patchset_suites") or []) or "none"
    promotion_candidates = rollout_context.get("promotion_candidates") if isinstance(rollout_context.get("promotion_candidates"), dict) else {}
    future_labels: list[str] = []
    for phase_name in ("phase1", "phase2"):
        items = _dedupe_text_values((promotion_candidates.get(phase_name) if isinstance(promotion_candidates, dict) else None) or [])
        if items:
            future_labels.append(f"{phase_name}: {', '.join(f'`{item}`' for item in items)}")
    future_message = f" Future promotions are modeled as {', '.join(future_labels)}." if future_labels else ""
    return (
        f"CI rollout phase {phase} blocks {required} and keeps {informational} visible as non-blocking surfaces."
        + future_message
    )


def _ci_rollout_patchset_suite_checks(rollout_context: dict[str, Any]) -> list[dict[str, str]]:
    suite_results_by_id = dict(rollout_context.get("suite_results_by_id") or {})
    checks: list[dict[str, str]] = []

    def _suite_entry(suite_id: str, *, blocking: bool) -> dict[str, str]:
        result = suite_results_by_id.get(suite_id)
        label = f"Patchset CI suite `{suite_id}`"
        if result is None:
            if blocking:
                return {
                    "name": f"ci_patchset_suite_{suite_id}",
                    "label": label,
                    "status": "pending",
                    "message": f"Required patchset suite `{suite_id}` has not produced CI evidence for this patchset.",
                }
            return {
                "name": f"ci_patchset_suite_{suite_id}",
                "label": label,
                "status": "not_required",
                "message": (
                    f"Informational patchset suite `{suite_id}` is visible in rollout status but is not blocking "
                    "for the current phase."
                ),
            }
        status = str(result.get("status") or "").strip().lower()
        if status == "pass":
            message = (
                f"Required patchset suite `{suite_id}` passed."
                if blocking
                else f"Informational patchset suite `{suite_id}` passed."
            )
            check_status = "pass"
        elif blocking:
            message = f"Required patchset suite `{suite_id}` failed."
            check_status = "hard_fail"
        else:
            message = (
                f"Informational patchset suite `{suite_id}` failed; rollout keeps the red baseline visible "
                "without blocking land in the current phase."
            )
            check_status = "optional_fail"
        return {
            "name": f"ci_patchset_suite_{suite_id}",
            "label": label,
            "status": check_status,
            "message": message,
        }

    for suite_id in rollout_context.get("required_patchset_suites") or []:
        checks.append(_suite_entry(str(suite_id), blocking=True))
    for suite_id in rollout_context.get("informational_patchset_suites") or []:
        checks.append(_suite_entry(str(suite_id), blocking=False))
    return checks


def _policy_status_view(
    patchset_id: str,
    *,
    lane: str,
    decision: str,
    checks: list[dict[str, Any]] | None = None,
    evaluated_at: str | None = None,
) -> dict[str, Any]:
    return {
        "patchset_id": patchset_id,
        "lane": lane,
        "decision": decision,
        "checks": list(checks or []),
        "evaluated_at": evaluated_at,
    }


def _effective_policy_status(conn, patchset: dict[str, Any] | Any, *, lane: str) -> dict[str, Any]:
    patchset_id = str(patchset["patchset_id"])
    evaluation_state = str(patchset["evaluation_state"] or "pending").strip() or "pending"
    latest = latest_policy_status(conn, patchset_id)
    if evaluation_state == "pending":
        if latest is not None and str(latest.get("decision") or "") == "pending":
            return latest
        return _policy_status_view(patchset_id, lane=lane, decision="pending")
    if latest is not None:
        latest = dict(latest)
        latest["decision"] = evaluation_state
        latest["lane"] = str(latest.get("lane") or lane).strip() or lane
        return latest
    return _policy_status_view(patchset_id, lane=lane, decision=evaluation_state)


def _invalidate_patchset_policy(conn, patchset_id: str) -> None:
    conn.execute("update patchsets set evaluation_state = 'pending' where patchset_id = ?", (patchset_id,))


def _attestation_id_for_patchset(patchset_id: str) -> str:
    text = str(patchset_id or "").strip()
    if not text:
        raise ValueError("patchset_id is required")
    return f"AT-{text}"


def _land_submission_id_for_change(change_id: str, prior_request_count: int) -> str:
    text = str(change_id or "").strip()
    if not text:
        raise ValueError("change_id is required")
    if prior_request_count < 0:
        raise ValueError("prior_request_count must be non-negative")
    return f"LAND-{text}-{prior_request_count + 1:04d}"
