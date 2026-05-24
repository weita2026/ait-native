from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path, PurePosixPath
from typing import Any

from .server_content import read_blob_bytes, snapshot_manifest_map
from .server_queue import enqueue_async_job
from .server_store import evaluate_policy, get_attestation, get_change, get_patchset, upsert_attestation


def _queue_mode() -> str:
    mode = os.environ.get("AIT_NATIVE_QUEUE_MODE", "inline").strip().lower()
    return mode if mode in {"inline", "async"} else "inline"


def _truncate_text(value: str, *, limit: int = 4000) -> str:
    if len(value) <= limit:
        return value
    return value[: limit - 15] + "\n...<truncated>"


def _snapshot_manifest(ctx, snapshot_id: str) -> dict[str, dict[str, Any]]:
    return snapshot_manifest_map(ctx, snapshot_id)


def patchset_ci_contract_available(ctx, patchset_id: str) -> bool:
    patchset = get_patchset(ctx, patchset_id)
    manifest = _snapshot_manifest(ctx, patchset["revision_snapshot_id"])
    return any(
        PurePosixPath(path).parts[:2] == ("ci", "suites") and path.endswith(".json")
        for path in manifest
    )


def _load_snapshot_json(ctx, snapshot_manifest: dict[str, dict[str, Any]], path: str) -> dict[str, Any]:
    blob_id = snapshot_manifest[path]["blob_id"]
    return json.loads(read_blob_bytes(ctx, blob_id).decode("utf-8"))


def _load_suite_manifests(ctx, snapshot_id: str) -> list[dict[str, Any]]:
    manifest = _snapshot_manifest(ctx, snapshot_id)
    suite_paths = sorted(
        path
        for path in manifest
        if PurePosixPath(path).parts[:2] == ("ci", "suites") and path.endswith(".json")
    )
    suites: list[dict[str, Any]] = []
    for path in suite_paths:
        payload = _load_snapshot_json(ctx, manifest, path)
        payload["_artifact_path"] = path
        suites.append(payload)
    return suites


def _selected_patchset_suites(suites: list[dict[str, Any]]) -> list[dict[str, Any]]:
    selected = [
        suite
        for suite in suites
        if str(suite.get("plane") or "").strip().lower() == "patchset"
        and str(suite.get("mode") or "").strip().lower() == "gate"
    ]
    selected.sort(key=lambda item: str(item.get("suite_id") or item.get("_artifact_path") or ""))
    return selected


def mark_patchset_ci_pending(ctx, patchset_id: str, *, trigger: str = "manual_rerun", job_state: str = "queued") -> dict[str, Any] | None:
    try:
        existing_attestation = get_attestation(ctx, patchset_id)
    except KeyError:
        existing_attestation = None
    patchset = get_patchset(ctx, patchset_id)
    change = get_change(ctx, patchset["change_id"])
    suites = _selected_patchset_suites(_load_suite_manifests(ctx, patchset["revision_snapshot_id"]))
    suite_ids = [str(suite.get("suite_id") or "") for suite in suites if str(suite.get("suite_id") or "").strip()]
    blocking_suite_ids = [
        str(suite.get("suite_id") or "")
        for suite in suites
        if str(suite.get("suite_id") or "").strip() and bool(suite.get("default_blocking", False))
    ]
    evaluation_summary = dict((existing_attestation or {}).get("evaluation_summary") or {})
    evaluation_summary["tests"] = "pending"
    provenance_summary = dict((existing_attestation or {}).get("provenance_summary") or {})
    detail = dict((existing_attestation or {}).get("detail") or {})
    detail["patchset_ci"] = {
        "trigger": trigger,
        "patchset_id": patchset_id,
        "change_id": change["change_id"],
        "base_snapshot_id": patchset["base_snapshot_id"],
        "revision_snapshot_id": patchset["revision_snapshot_id"],
        "selected_suite_ids": suite_ids,
        "blocking_suite_ids": blocking_suite_ids,
        "blocking_failures": [],
        "tests_status": "pending",
        "suite_results": [],
        "job_state": job_state,
    }
    return upsert_attestation(
        ctx,
        patchset_id,
        (existing_attestation or {}).get("author_mode") or patchset.get("author_mode") or "ai_with_human_review",
        evaluation_summary,
        provenance_summary,
        detail,
    )


def _materialize_snapshot(ctx, snapshot_id: str, target_dir: Path) -> None:
    manifest = _snapshot_manifest(ctx, snapshot_id)
    for rel_path, entry in manifest.items():
        path = target_dir / rel_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read_blob_bytes(ctx, entry["blob_id"]))
        try:
            path.chmod(int(str(entry.get("mode") or "0o644"), 0))
        except (TypeError, ValueError, OSError):
            pass


def _artifact_payload(workspace: Path, artifacts: dict[str, Any]) -> dict[str, Any]:
    payload: dict[str, Any] = {}
    for key, rel_path in artifacts.items():
        text = str(rel_path or "").strip()
        if not text:
            continue
        absolute = workspace / text
        payload[key] = {
            "path": text,
            "exists": absolute.exists(),
            "size_bytes": absolute.stat().st_size if absolute.exists() and absolute.is_file() else None,
        }
    return payload


def _optional_int(value: Any) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _artifact_existing_path(workspace: Path, artifacts: dict[str, Any], key: str) -> Path | None:
    artifact = artifacts.get(key)
    if isinstance(artifact, dict):
        text = str(artifact.get("path") or "").strip()
    else:
        text = str(artifact or "").strip()
    if not text:
        return None
    candidate = workspace / text
    return candidate if candidate.is_file() else None


def _tg1_required_summary_from_artifacts(workspace: Path, suite: dict[str, Any], artifacts: dict[str, Any]) -> dict[str, Any] | None:
    if str(suite.get("suite_id") or "").strip() != "tg1_required":
        return None
    summary_path = _artifact_existing_path(workspace, artifacts, "summary_json")
    if summary_path is None:
        return None
    try:
        payload = json.loads(summary_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    pytest = dict(payload.get("pytest") or {})
    live_count = _optional_int(payload.get("live_count"))
    minimum_count = _optional_int(payload.get("minimum_count"))
    return {
        "status": str(payload.get("status") or pytest.get("status") or "").strip() or None,
        "validation_status": str(payload.get("validation_status") or "").strip() or None,
        "pytest_status": str(pytest.get("status") or "").strip() or None,
        "live_count": live_count,
        "minimum_count": minimum_count,
    }


def _run_command_bundle_suite(workspace: Path, suite: dict[str, Any]) -> dict[str, Any]:
    commands = [str(item).strip() for item in list(((suite.get("runner") or {}).get("commands") or [])) if str(item).strip()]
    if not commands:
        raise ValueError(f"Suite {suite.get('suite_id') or suite.get('_artifact_path')} does not define any commands.")
    artifacts = dict(suite.get("artifacts") or {})
    command_reports: list[dict[str, Any]] = []
    overall_status = "pass"
    for index, command in enumerate(commands, 1):
        started = time.monotonic()
        completed = subprocess.run(
            command,
            shell=True,
            cwd=workspace,
            text=True,
            capture_output=True,
        )
        duration = round(time.monotonic() - started, 3)
        command_reports.append(
            {
                "index": index,
                "command": command,
                "exit_code": completed.returncode,
                "duration_seconds": duration,
                "stdout": _truncate_text(completed.stdout),
                "stderr": _truncate_text(completed.stderr),
                "status": "pass" if completed.returncode == 0 else "fail",
            }
        )
        if completed.returncode != 0:
            overall_status = "fail"
            break
    log_path = str(artifacts.get("log_path") or "").strip()
    if log_path:
        target = workspace / log_path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(
            "\n\n".join(
                [
                    f"$ {row['command']}\nexit_code={row['exit_code']}\n{row['stdout']}\n{row['stderr']}".strip()
                    for row in command_reports
                ]
            )
            + "\n",
            encoding="utf-8",
        )
    return {
        "status": overall_status,
        "command_reports": command_reports,
        "artifacts": _artifact_payload(workspace, artifacts),
    }


def _run_pytest_suite(workspace: Path, suite: dict[str, Any]) -> dict[str, Any]:
    runner = dict(suite.get("runner") or {})
    args = [str(item) for item in list(runner.get("args") or [])]
    artifacts = dict(suite.get("artifacts") or {})
    junit_rel = str(artifacts.get("junit_xml") or "").strip()
    if junit_rel and not any(arg.startswith("--junitxml") for arg in args):
        junit_path = workspace / junit_rel
        junit_path.parent.mkdir(parents=True, exist_ok=True)
        args = args + ["--junitxml", str(junit_path)]
    command = [sys.executable, "-m", "pytest", *args]
    started = time.monotonic()
    completed = subprocess.run(
        command,
        cwd=workspace,
        text=True,
        capture_output=True,
    )
    duration = round(time.monotonic() - started, 3)
    log_path = str(artifacts.get("log_path") or "").strip()
    if log_path:
        target = workspace / log_path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text((completed.stdout or "") + ("\n" + completed.stderr if completed.stderr else ""), encoding="utf-8")
    return {
        "status": "pass" if completed.returncode == 0 else "fail",
        "command": command,
        "exit_code": completed.returncode,
        "duration_seconds": duration,
        "stdout": _truncate_text(completed.stdout),
        "stderr": _truncate_text(completed.stderr),
        "artifacts": _artifact_payload(workspace, artifacts),
    }


def _run_one_suite(workspace: Path, suite: dict[str, Any]) -> dict[str, Any]:
    runner = dict(suite.get("runner") or {})
    kind = str(runner.get("kind") or "").strip().lower()
    if kind == "command_bundle":
        result = _run_command_bundle_suite(workspace, suite)
    elif kind == "pytest":
        result = _run_pytest_suite(workspace, suite)
    else:
        raise ValueError(
            f"Unsupported patchset CI runner kind `{runner.get('kind')}` for suite "
            f"{suite.get('suite_id') or suite.get('_artifact_path')}."
        )
    payload = {
        "suite_id": suite.get("suite_id"),
        "display_name": suite.get("display_name"),
        "artifact_path": suite.get("_artifact_path"),
        "plane": suite.get("plane"),
        "mode": suite.get("mode"),
        "blocking": bool(suite.get("default_blocking", False)),
        "purpose": suite.get("purpose"),
        "runner_kind": runner.get("kind"),
        **result,
    }
    tg1_required_summary = _tg1_required_summary_from_artifacts(
        workspace,
        suite,
        dict(result.get("artifacts") or {}),
    )
    if tg1_required_summary is not None:
        payload["tg1_required_summary"] = tg1_required_summary
    return payload


def _policy_job_payload(patchset: dict[str, Any], change: dict[str, Any]) -> dict[str, Any]:
    return {
        "patchset_id": patchset["patchset_id"],
        "repo_name": change["repo_name"],
        "repo_id": patchset.get("repo_id") or change.get("repo_id"),
        "change_id": change["change_id"],
        "change_seq": change.get("change_seq"),
        "patchset_number": patchset.get("patchset_number"),
    }


def run_patchset_ci(ctx, patchset_id: str, *, trigger: str = "manual_rerun") -> dict[str, Any]:
    patchset = get_patchset(ctx, patchset_id)
    change = get_change(ctx, patchset["change_id"])
    suites = _selected_patchset_suites(_load_suite_manifests(ctx, patchset["revision_snapshot_id"]))
    if not suites:
        raise ValueError(
            f"Patchset {patchset_id} does not expose any runnable patchset gate manifests under ci/suites/."
        )

    with tempfile.TemporaryDirectory(prefix=f"ait-patchset-ci-{patchset_id.lower()}-") as temp_dir:
        workspace = Path(temp_dir)
        _materialize_snapshot(ctx, patchset["revision_snapshot_id"], workspace)
        suite_results = [_run_one_suite(workspace, suite) for suite in suites]

    blocking_failures = [suite for suite in suite_results if suite["blocking"] and suite["status"] != "pass"]
    tests_status = "pass" if not blocking_failures else "fail"

    try:
        existing_attestation = get_attestation(ctx, patchset_id)
    except KeyError:
        existing_attestation = None

    evaluation_summary = dict((existing_attestation or {}).get("evaluation_summary") or {})
    evaluation_summary["tests"] = tests_status
    provenance_summary = dict((existing_attestation or {}).get("provenance_summary") or {})
    detail = dict((existing_attestation or {}).get("detail") or {})
    detail["patchset_ci"] = {
        "trigger": trigger,
        "patchset_id": patchset_id,
        "change_id": change["change_id"],
        "base_snapshot_id": patchset["base_snapshot_id"],
        "revision_snapshot_id": patchset["revision_snapshot_id"],
        "selected_suite_ids": [suite["suite_id"] for suite in suite_results],
        "blocking_suite_ids": [suite["suite_id"] for suite in suite_results if suite["blocking"]],
        "blocking_failures": [suite["suite_id"] for suite in blocking_failures],
        "tests_status": tests_status,
        "suite_results": suite_results,
    }
    attestation = upsert_attestation(
        ctx,
        patchset_id,
        patchset.get("author_mode") or "ai_with_human_review",
        evaluation_summary,
        provenance_summary,
        detail,
    )

    policy_job = None
    policy = None
    if _queue_mode() == "async":
        policy_job = enqueue_async_job(
            ctx,
            change["repo_name"],
            "policy.evaluate",
            _policy_job_payload(patchset, change),
            max_attempts=5,
            dedupe_active=True,
        )
    else:
        policy = evaluate_policy(ctx, patchset_id)

    return {
        "patchset_id": patchset_id,
        "change_id": change["change_id"],
        "repo_name": change["repo_name"],
        "trigger": trigger,
        "tests_status": tests_status,
        "blocking_suite_ids": [suite["suite_id"] for suite in suite_results if suite["blocking"]],
        "blocking_failures": [suite["suite_id"] for suite in blocking_failures],
        "suite_results": suite_results,
        "attestation": attestation,
        "policy_job": policy_job,
        "policy": policy,
    }
