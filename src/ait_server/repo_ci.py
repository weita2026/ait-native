from __future__ import annotations

import fnmatch
import json
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from .patchset_ci import _artifact_payload, _load_suite_manifests, _materialize_snapshot, _run_one_suite
from .read_models import task_audit
from .server_content_repo_lines import read_ref
from .server_control import connect
from .server_paths import ServerContext
from .server_store import get_repository, get_task
from .store.workflow_artifacts import _load_snapshot_ci_contract


def _parse_iso_utc(value: Any) -> datetime | None:
    text = str(value or "").strip()
    if not text:
        return None
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _dedupe_strs(values: list[Any] | tuple[Any, ...] | None) -> list[str]:
    seen: set[str] = set()
    out: list[str] = []
    for item in list(values or []):
        text = str(item or "").strip()
        if not text or text in seen:
            continue
        seen.add(text)
        out.append(text)
    return out


def _load_repo_ci_config_payload(workspace: Path) -> dict[str, Any]:
    path = workspace / ".ait" / "config.json"
    if not path.exists():
        return {}
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    if not isinstance(payload, dict):
        return {}
    return payload


def _load_repo_ci_config(workspace: Path) -> dict[str, Any]:
    payload = _load_repo_ci_config_payload(workspace)
    ci = payload.get("ci")
    return dict(ci) if isinstance(ci, dict) else {}


def _restore_snapshot_ci_config(
    workspace: Path,
    preserved_config: dict[str, Any],
    *,
    fallback_ci_config: dict[str, Any] | None = None,
) -> None:
    preserved_ci = preserved_config.get("ci")
    resolved_ci = dict(preserved_ci) if isinstance(preserved_ci, dict) else dict(fallback_ci_config or {})
    if not resolved_ci:
        return
    path = workspace / ".ait" / "config.json"
    current = _load_repo_ci_config_payload(workspace)
    current["ci"] = resolved_ci
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(current, indent=2) + "\n", encoding="utf-8")


def _release_gate_contract(ci_config: dict[str, Any], suite: dict[str, Any]) -> dict[str, Any]:
    rollout = ci_config.get("rollout")
    rollout = dict(rollout) if isinstance(rollout, dict) else {}
    config_gate = rollout.get("release_evidence")
    config_gate = dict(config_gate) if isinstance(config_gate, dict) else {}
    suite_gate = suite.get("release_gate_evidence")
    suite_gate = dict(suite_gate) if isinstance(suite_gate, dict) else {}
    combined = {**config_gate, **suite_gate}
    return {
        "dependency_keys": _dedupe_strs(combined.get("dependency_keys") or []),
        "compliance_keys": _dedupe_strs(combined.get("compliance_keys") or []),
        "required_before_distribution": bool(combined.get("required_before_distribution", False)),
    }


def _selected_repo_suites(
    suites: list[dict[str, Any]],
    ci_config: dict[str, Any],
    *,
    suite_ids: list[str] | None = None,
    plane: str | None = None,
) -> list[dict[str, Any]]:
    allowed_planes = {"nightly", "release", "post_land_regression"}
    normalized_plane = str(plane or "").strip().lower() or None
    if normalized_plane is not None and normalized_plane not in allowed_planes:
        raise ValueError(
            f"Unsupported repo CI plane `{plane}`. Expected one of: nightly, release, post_land_regression."
        )

    normalized_suite_ids = [str(item).strip() for item in list(suite_ids or []) if str(item).strip()]
    selected: list[dict[str, Any]] = []
    missing: list[str] = []
    by_suite_id = {
        str(suite.get("suite_id") or "").strip(): suite
        for suite in suites
        if str(suite.get("suite_id") or "").strip()
    }
    if normalized_suite_ids:
        for suite_id in normalized_suite_ids:
            suite = by_suite_id.get(suite_id)
            if suite is None:
                missing.append(suite_id)
                continue
            suite_plane = str(suite.get("plane") or "").strip().lower()
            if suite_plane not in allowed_planes:
                raise ValueError(
                    f"Suite `{suite_id}` cannot run through repo CI because it belongs to plane `{suite.get('plane')}`."
                )
            selected.append(suite)
        if missing:
            raise ValueError(f"Unknown repo CI suite id(s): {', '.join(missing)}")
    else:
        target_plane = normalized_plane or "nightly"
        configured_suite_ids: list[str] = []
        if target_plane == "nightly":
            configured_suite_ids = _dedupe_strs(ci_config.get("nightly_suites") or [])
        elif target_plane == "release":
            configured_suite_ids = _dedupe_strs(ci_config.get("release_suites") or [])
        if configured_suite_ids:
            selected = []
            missing = []
            for suite_id in configured_suite_ids:
                suite = by_suite_id.get(suite_id)
                if suite is None:
                    missing.append(suite_id)
                    continue
                suite_plane = str(suite.get("plane") or "").strip().lower()
                if suite_plane != target_plane:
                    raise ValueError(
                        f"Suite `{suite_id}` is configured under `{target_plane}` but declares plane `{suite.get('plane')}`."
                    )
                selected.append(suite)
            if missing:
                raise ValueError(
                    f"Configured repo CI suite id(s) for plane `{target_plane}` are missing manifests: {', '.join(missing)}"
                )
        else:
            selected = [
                suite
                for suite in suites
                if str(suite.get("plane") or "").strip().lower() == target_plane
            ]
    selected.sort(key=lambda item: str(item.get("suite_id") or item.get("_artifact_path") or ""))
    if not selected:
        filter_label = f"plane `{normalized_plane}`" if normalized_plane else "the default nightly plane"
        raise ValueError(f"No repo CI suites matched {filter_label}.")
    return selected


def _ensure_local_runtime_repo(workspace: Path, repo_name: str, *, ci_config: dict[str, Any] | None = None) -> None:
    control_db = workspace / ".ait" / "control.db"
    preserved_config = _load_repo_ci_config_payload(workspace)
    initialized = False
    if not control_db.exists():
        init = subprocess.run(
            ["ait", "init", "--name", repo_name],
            cwd=workspace,
            text=True,
            capture_output=True,
        )
        if init.returncode != 0:
            raise ValueError(
                f"Unable to bootstrap local repo runtime for CI workspace: {init.stderr.strip() or init.stdout.strip()}"
            )
        initialized = True
    _restore_snapshot_ci_config(workspace, preserved_config, fallback_ci_config=ci_config)
    if initialized:
        seed = subprocess.run(
            ["ait", "snapshot", "create", "--message", "repo ci bootstrap"],
            cwd=workspace,
            text=True,
            capture_output=True,
        )
        if seed.returncode != 0:
            raise ValueError(
                f"Unable to seed local snapshot for repo CI workspace: {seed.stderr.strip() or seed.stdout.strip()}"
            )


def _parse_junit_cases(junit_path: Path) -> list[dict[str, Any]]:
    if not junit_path.exists():
        return []
    root = ET.fromstring(junit_path.read_text(encoding="utf-8"))
    cases: list[dict[str, Any]] = []
    for node in root.iter("testcase"):
        status = "pass"
        detail = None
        if node.find("failure") is not None:
            status = "fail"
            failure = node.find("failure")
            detail = (failure.text or failure.get("message") or "").strip() if failure is not None else None
        elif node.find("error") is not None:
            status = "error"
            error = node.find("error")
            detail = (error.text or error.get("message") or "").strip() if error is not None else None
        elif node.find("skipped") is not None:
            status = "skipped"
        cases.append(
            {
                "file": node.get("file"),
                "classname": node.get("classname"),
                "name": node.get("name"),
                "status": status,
                "detail": detail,
            }
        )
    return cases


def _owner_for_case(case: dict[str, Any], ownership_rules: list[dict[str, Any]]) -> str | None:
    file_path = str(case.get("file") or "").strip()
    for rule in ownership_rules:
        owner = str(rule.get("owner") or "").strip()
        if not owner:
            continue
        prefix = str(rule.get("test_path_prefix") or "").strip()
        if prefix and file_path.startswith(prefix):
            return owner
        glob = str(rule.get("test_glob") or "").strip()
        if glob and file_path and fnmatch.fnmatch(file_path, glob):
            return owner
    return None


def _recent_landed_tasks(
    ctx: ServerContext,
    repo_name: str,
    *,
    target_line: str,
    limit: int = 5,
    window_days: int = 7,
    risk_tier: str | None = None,
    require_land_status: str | None = "succeeded",
) -> list[dict[str, Any]]:
    limit = max(int(limit), 0)
    if limit == 0:
        return []
    repo = get_repository(ctx, repo_name)
    cutoff = datetime.now(timezone.utc) - timedelta(days=max(int(window_days), 0))
    where_risk = ""
    params: list[Any] = [str(repo.get("repo_id") or ""), repo_name, target_line]
    if require_land_status:
        params.append(require_land_status)
        where_status = " and lr.status = ?"
    else:
        where_status = ""
    if risk_tier:
        where_risk = " and t.risk_tier = ?"
        params.append(risk_tier)
    with connect(ctx) as conn:
        rows = [
            dict(row)
            for row in conn.execute(
                """
                select t.task_id, t.title, t.risk_tier, t.status as task_status, lr.updated_at as landed_at,
                       lr.submission_id, lr.change_id, lr.status as land_status
                from land_requests lr
                join changes c on c.change_id = lr.change_id
                join tasks t on t.task_id = c.task_id
                where (c.repo_id = ? or (c.repo_id is null and c.repo_name = ?))
                  and lr.target_line = ?
                """
                + where_status
                + where_risk
                + """
                order by lr.updated_at desc, lr.submission_id desc
                """,
                tuple(params),
            )
        ]
    selected: list[dict[str, Any]] = []
    seen: set[str] = set()
    for row in rows:
        landed_at = _parse_iso_utc(row.get("landed_at"))
        if landed_at is None or landed_at < cutoff:
            continue
        task_id = str(row.get("task_id") or "").strip()
        if not task_id or task_id in seen:
            continue
        seen.add(task_id)
        audit = task_audit(ctx, task_id, target_line=target_line)
        selected.append(
            {
                "task_id": task_id,
                "title": row.get("title"),
                "risk_tier": row.get("risk_tier"),
                "task_status": row.get("task_status"),
                "landed_at": row.get("landed_at"),
                "submission_id": row.get("submission_id"),
                "change_id": row.get("change_id"),
                "land_status": row.get("land_status"),
                "audit_verdict": audit.get("verdict"),
                "audit_workflow": (audit.get("workflow") or {}).get("state"),
            }
        )
        if len(selected) >= limit:
            break
    return selected


def _task_selector_entry(
    ctx: ServerContext,
    repo_name: str,
    task_id: str,
    *,
    target_line: str,
    selection_reason: str,
    role_hint: str | None = None,
) -> dict[str, Any]:
    task_id = str(task_id or "").strip()
    if not task_id:
        raise ValueError("Task-batch selection cannot contain blank task ids.")
    try:
        task = get_task(ctx, task_id)
    except KeyError:
        return {
            "task_id": task_id,
            "title": None,
            "risk_tier": None,
            "task_status": None,
            "landed_at": None,
            "submission_id": None,
            "change_id": None,
            "land_status": None,
            "selection_reason": selection_reason,
            "role_hint": role_hint,
            "missing_task": True,
        }

    repo = get_repository(ctx, repo_name)
    with connect(ctx) as conn:
        row = conn.execute(
            """
            select lr.updated_at as landed_at, lr.submission_id, lr.change_id, lr.status as land_status
            from changes c
            left join land_requests lr on lr.change_id = c.change_id and lr.target_line = ?
            where c.task_id = ?
              and (c.repo_id = ? or (c.repo_id is null and c.repo_name = ?))
            order by case when lr.updated_at is null then 1 else 0 end asc,
                     lr.updated_at desc,
                     c.updated_at desc,
                     c.change_id desc
            limit 1
            """,
            (target_line, task_id, str(repo.get("repo_id") or ""), repo_name),
        ).fetchone()
    return {
        "task_id": task_id,
        "title": task.get("title"),
        "risk_tier": task.get("risk_tier"),
        "task_status": task.get("status"),
        "landed_at": row["landed_at"] if row is not None else None,
        "submission_id": row["submission_id"] if row is not None else None,
        "change_id": row["change_id"] if row is not None else None,
        "land_status": row["land_status"] if row is not None else None,
        "selection_reason": selection_reason,
        "role_hint": role_hint,
        "missing_task": False,
    }


def _load_curated_corpus(workspace: Path, suite: dict[str, Any], corpus_name: str) -> dict[str, Any]:
    runner = dict(suite.get("runner") or {})
    corpus_dir = str(runner.get("curated_corpus_dir") or "ci/task_corpora").strip() or "ci/task_corpora"
    filename = corpus_name if corpus_name.endswith(".json") else f"{corpus_name}.json"
    path = workspace / corpus_dir / filename
    if not path.exists():
        raise ValueError(f"Task-batch curated corpus `{corpus_name}` was not found at {corpus_dir}/{filename}.")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"Task-batch curated corpus `{corpus_name}` is not valid JSON.") from exc

    task_ids: list[str] = []
    role_hints: dict[str, str] = {}
    extra_behavior_suite_ids: list[str] = []

    if isinstance(payload, list):
        task_ids = _dedupe_strs(payload)
    elif isinstance(payload, dict):
        task_ids = _dedupe_strs(payload.get("task_ids") or [])
        for task_id in _dedupe_strs(payload.get("lineage_only_task_ids") or []):
            if task_id not in task_ids:
                task_ids.append(task_id)
            role_hints[task_id] = "lineage_only"
        for item in list(payload.get("items") or []):
            if isinstance(item, str):
                task_id = str(item).strip()
                if task_id and task_id not in task_ids:
                    task_ids.append(task_id)
                continue
            if not isinstance(item, dict):
                continue
            task_id = str(item.get("task_id") or "").strip()
            if not task_id:
                continue
            if task_id not in task_ids:
                task_ids.append(task_id)
            role = str(item.get("role") or "").strip().lower()
            if role == "lineage_only":
                role_hints[task_id] = role
            extra_behavior_suite_ids.extend(_dedupe_strs(item.get("suite_ids") or []))
        extra_behavior_suite_ids.extend(_dedupe_strs(payload.get("behavior_suite_ids") or []))
    else:
        raise ValueError(f"Task-batch curated corpus `{corpus_name}` must be a JSON object or list.")

    return {
        "corpus_name": corpus_name,
        "task_ids": task_ids,
        "role_hints": role_hints,
        "behavior_suite_ids": _dedupe_strs(extra_behavior_suite_ids),
    }


def _task_batch_selection(
    ctx: ServerContext,
    repo_name: str,
    *,
    suite: dict[str, Any],
    selector: str,
    task_ids: list[str] | None,
    curated_corpus: str | None,
    target_line: str,
    count: int,
    window_days: int,
    require_land_status: str | None,
    include_lineage_representatives: bool,
    workspace: Path,
) -> tuple[list[dict[str, Any]], dict[str, str], list[str]]:
    role_hints: dict[str, str] = {}
    extra_behavior_suite_ids: list[str] = []
    if selector == "recent_remote_landed":
        return (
            _recent_landed_tasks(
                ctx,
                repo_name,
                target_line=target_line,
                limit=count,
                window_days=window_days,
                require_land_status=require_land_status,
            ),
            role_hints,
            extra_behavior_suite_ids,
        )
    if selector == "recent_remote_landed_high_risk":
        return (
            _recent_landed_tasks(
                ctx,
                repo_name,
                target_line=target_line,
                limit=count,
                window_days=window_days,
                risk_tier="high",
                require_land_status=require_land_status,
            ),
            role_hints,
            extra_behavior_suite_ids,
        )
    if selector == "explicit_task_ids":
        selected = [
            _task_selector_entry(
                ctx,
                repo_name,
                task_id,
                target_line=target_line,
                selection_reason="explicit_task_ids",
            )
            for task_id in _dedupe_strs(task_ids)
        ]
        return selected, role_hints, extra_behavior_suite_ids
    if selector == "curated_corpus":
        corpus_name = str(curated_corpus or "").strip()
        if not corpus_name:
            raise ValueError("Task-batch selector `curated_corpus` requires a corpus name.")
        corpus = _load_curated_corpus(workspace, suite, corpus_name)
        role_hints = dict(corpus.get("role_hints") or {})
        extra_behavior_suite_ids = _dedupe_strs(corpus.get("behavior_suite_ids") or [])
        selected = [
            _task_selector_entry(
                ctx,
                repo_name,
                task_id,
                target_line=target_line,
                selection_reason=f"curated_corpus:{corpus_name}",
                role_hint=role_hints.get(task_id),
            )
            for task_id in corpus.get("task_ids") or []
        ]
        if not include_lineage_representatives:
            role_hints = {}
        return selected, role_hints, extra_behavior_suite_ids
    raise ValueError(f"Unsupported task-batch selector `{selector}`.")


def _run_materialized_suite(
    ctx: ServerContext,
    repo_name: str,
    snapshot_id: str,
    ci_config: dict[str, Any],
    suite: dict[str, Any],
    *,
    target_line: str,
    dependency_evidence: list[str] | None = None,
    compliance_evidence: list[str] | None = None,
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"ait-repo-ci-{repo_name.lower()}-") as temp_dir:
        workspace = Path(temp_dir)
        _materialize_snapshot(ctx, snapshot_id, workspace)
        _ensure_local_runtime_repo(workspace, repo_name, ci_config=ci_config)
        raw_result = _run_one_suite(workspace, suite)
        return _augment_suite_result(
            ctx,
            repo_name,
            ci_config,
            suite,
            raw_result,
            target_line=target_line,
            dependency_evidence=dependency_evidence,
            compliance_evidence=compliance_evidence,
        )


def _task_batch_markdown(
    *,
    selector: str,
    config: dict[str, Any],
    selected_tasks: list[dict[str, Any]],
    lineage_findings: dict[str, Any],
    behavior_regressions: dict[str, Any],
) -> str:
    lines = [
        "# Task Batch CI Summary",
        "",
        f"- selector: `{selector}`",
        f"- target line: `{config.get('target_line')}`",
        f"- selected tasks: {len(selected_tasks)}",
        f"- behavior candidates: {lineage_findings.get('behavior_candidate_count', 0)}",
        f"- lineage-only representatives: {lineage_findings.get('lineage_only_count', 0)}",
        f"- sentinels: {lineage_findings.get('sentinel_count', 0)}",
        f"- behavior status: `{behavior_regressions.get('status')}`",
    ]
    failing_suite_ids = behavior_regressions.get("failing_suite_ids") or []
    if failing_suite_ids:
        lines.append(f"- failing suites: {', '.join(f'`{item}`' for item in failing_suite_ids)}")
    lines.extend(["", "## Selected tasks", ""])
    if not selected_tasks:
        lines.append("- none")
    for task in selected_tasks:
        lines.append(
            "- "
            + f"`{task.get('task_id')}` "
            + f"({task.get('classification')}) "
            + f"audit={task.get('audit_verdict') or 'n/a'} "
            + f"land={task.get('land_status') or 'none'}"
        )
    problems = list(lineage_findings.get("problems") or [])
    if problems:
        lines.extend(["", "## Lineage findings", ""])
        for item in problems:
            detail = str(item.get("detail") or "").strip()
            suffix = f": {detail}" if detail else ""
            lines.append(f"- `{item.get('task_id')}` {item.get('classification')}{suffix}")
    return "\n".join(lines).rstrip() + "\n"


def _run_task_batch_suite(
    ctx: ServerContext,
    repo_name: str,
    snapshot_id: str,
    ci_config: dict[str, Any],
    suite: dict[str, Any],
    all_suites: list[dict[str, Any]],
    *,
    target_line: str,
    trigger: str,
    selector: str | None,
    task_ids: list[str] | None,
    curated_corpus: str | None,
    count: int | None,
    window_days: int | None,
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"ait-task-batch-ci-{repo_name.lower()}-") as temp_dir:
        workspace = Path(temp_dir)
        _materialize_snapshot(ctx, snapshot_id, workspace)
        _ensure_local_runtime_repo(workspace, repo_name, ci_config=ci_config)

        ci_config = _load_repo_ci_config(workspace)
        task_batch_defaults = dict(suite.get("defaults") or {})
        task_batch_config = {**task_batch_defaults, **dict(ci_config.get("task_batch") or {})}
        runner = dict(suite.get("runner") or {})
        supported_selectors = {
            str(item).strip()
            for item in list(runner.get("supported_selectors") or [])
            if str(item).strip()
        }
        resolved_selector = str(selector or task_batch_config.get("selector") or "recent_remote_landed").strip()
        if supported_selectors and resolved_selector not in supported_selectors:
            allowed = ", ".join(sorted(supported_selectors))
            raise ValueError(f"Task-batch selector `{resolved_selector}` is not supported by this repository. Expected one of: {allowed}.")

        resolved_count = int(count) if count is not None else int(task_batch_config.get("count") or 0)
        resolved_window_days = int(window_days) if window_days is not None else int(task_batch_config.get("window_days") or 7)
        resolved_remote = str(task_batch_config.get("remote") or ci_config.get("default_remote") or "origin")
        resolved_require_land_status = str(task_batch_config.get("require_land_status") or "").strip() or None
        resolved_blocking = bool(task_batch_config.get("blocking", suite.get("default_blocking", False)))
        resolved_max_parallel = max(int(task_batch_config.get("max_parallel") or 1), 1)
        resolved_audit_first = bool(task_batch_config.get("audit_first", runner.get("audit_first", True)))
        include_lineage_representatives = bool(task_batch_config.get("include_lineage_representatives", True))
        default_behavior_suite_ids = _dedupe_strs(runner.get("behavior_suite_ids") or [])

        selected_candidates, role_hints, extra_behavior_suite_ids = _task_batch_selection(
            ctx,
            repo_name,
            suite=suite,
            selector=resolved_selector,
            task_ids=task_ids,
            curated_corpus=curated_corpus,
            target_line=target_line,
            count=resolved_count,
            window_days=resolved_window_days,
            require_land_status=resolved_require_land_status,
            include_lineage_representatives=include_lineage_representatives,
            workspace=workspace,
        )

        behavior_suite_ids = _dedupe_strs(default_behavior_suite_ids + extra_behavior_suite_ids)
        suites_by_id = {
            str(item.get("suite_id") or "").strip(): item
            for item in all_suites
            if str(item.get("suite_id") or "").strip()
        }
        selected_tasks: list[dict[str, Any]] = []
        lineage_problems: list[dict[str, Any]] = []
        candidate_behavior_suite_ids: set[str] = set()
        behavior_candidate_count = 0
        lineage_only_count = 0
        sentinel_count = 0

        for candidate in selected_candidates:
            task_id = str(candidate.get("task_id") or "").strip()
            task_entry = dict(candidate)
            task_entry.setdefault("role_hint", role_hints.get(task_id))
            audit_summary = None
            audit_error = None
            if resolved_audit_first and not task_entry.get("missing_task"):
                try:
                    audit_summary = task_audit(ctx, task_id, target_line=target_line)
                except Exception as exc:  # pragma: no cover - defensive surface for corrupted lineage.
                    audit_error = str(exc)

            task_entry["audit_verdict"] = audit_summary and (audit_summary.get("summary") or {}).get("verdict")
            task_entry["audit_workflow"] = audit_summary and (audit_summary.get("workflow") or {}).get("state")
            task_entry["change_target_states"] = [row.get("target_state") for row in list((audit_summary or {}).get("changes") or [])]
            task_entry["audit_error"] = audit_error

            classification = "behavior_candidate"
            detail = ""
            if task_entry.get("missing_task"):
                classification = "lineage_only"
                detail = "task not found in repository workflow state"
            elif audit_error:
                classification = "lineage_only"
                detail = audit_error
            elif task_entry.get("role_hint") == "lineage_only":
                classification = "lineage_only"
                detail = "curated corpus requested lineage-only representation"
            elif any(state in {"no_patchset", "archived"} for state in task_entry["change_target_states"]):
                classification = "sentinel"
                detail = ", ".join(state for state in task_entry["change_target_states"] if state in {"no_patchset", "archived"})
            elif resolved_require_land_status and str(task_entry.get("land_status") or "").strip() != resolved_require_land_status:
                classification = "lineage_only"
                detail = (
                    f"latest land status `{task_entry.get('land_status') or 'none'}` did not match "
                    f"required `{resolved_require_land_status}`"
                )
            else:
                effective_count = int(((audit_summary or {}).get("summary") or {}).get("effective_on_target_change_count") or 0)
                if effective_count <= 0:
                    classification = "lineage_only"
                    detail = f"task is not yet effective on `{target_line}`"

            task_entry["classification"] = classification
            task_entry["detail"] = detail or None
            if classification == "behavior_candidate":
                task_entry["behavior_suite_ids"] = list(behavior_suite_ids)
                behavior_candidate_count += 1
                candidate_behavior_suite_ids.update(behavior_suite_ids)
            else:
                task_entry["behavior_suite_ids"] = []
                if classification == "sentinel":
                    sentinel_count += 1
                else:
                    lineage_only_count += 1
                lineage_problems.append(
                    {
                        "task_id": task_id,
                        "classification": classification,
                        "detail": detail,
                        "audit_verdict": task_entry.get("audit_verdict"),
                        "land_status": task_entry.get("land_status"),
                    }
                )
            selected_tasks.append(task_entry)

        missing_behavior_suites = [suite_id for suite_id in sorted(candidate_behavior_suite_ids) if suite_id not in suites_by_id]
        if missing_behavior_suites:
            raise ValueError(
                "Task-batch behavior suite id(s) are not defined in ci/suites: " + ", ".join(missing_behavior_suites)
            )

        behavior_suites: list[dict[str, Any]] = []
        for suite_id in behavior_suite_ids:
            if suite_id not in candidate_behavior_suite_ids:
                continue
            candidate_suite = suites_by_id[suite_id]
            if str(((candidate_suite.get("runner") or {}).get("kind") or "")).strip().lower() == "task_batch":
                raise ValueError(f"Task-batch behavior suite `{suite_id}` cannot recursively point at another task_batch runner.")
            behavior_suites.append(candidate_suite)

        if len(behavior_suites) <= 1:
            behavior_results = [
                _run_materialized_suite(
                    ctx,
                    repo_name,
                    snapshot_id,
                    ci_config,
                    candidate_suite,
                    target_line=target_line,
                )
                for candidate_suite in behavior_suites
            ]
        else:
            worker_count = min(resolved_max_parallel, len(behavior_suites))
            with ThreadPoolExecutor(max_workers=worker_count) as executor:
                futures = [
                    executor.submit(
                        _run_materialized_suite,
                        ctx,
                        repo_name,
                        snapshot_id,
                        ci_config,
                        candidate_suite,
                        target_line=target_line,
                    )
                    for candidate_suite in behavior_suites
                ]
                behavior_results = [future.result() for future in futures]

        behavior_failures = [item for item in behavior_results if item.get("status") != "pass"]
        behavior_status = "pass"
        if behavior_suites:
            behavior_status = "pass" if not behavior_failures else "fail"
        elif behavior_candidate_count == 0:
            behavior_status = "not_applicable"

        lineage_findings = {
            "behavior_candidate_count": behavior_candidate_count,
            "lineage_only_count": lineage_only_count,
            "sentinel_count": sentinel_count,
            "problem_count": len(lineage_problems),
            "problems": lineage_problems,
        }
        behavior_regressions = {
            "status": behavior_status,
            "selected_suite_ids": [str(item.get("suite_id") or "") for item in behavior_suites],
            "failing_suite_ids": [str(item.get("suite_id") or "") for item in behavior_failures],
            "suite_results": behavior_results,
        }
        resolved_config = {
            "selector": resolved_selector,
            "count": resolved_count,
            "window_days": resolved_window_days,
            "remote": resolved_remote,
            "target_line": target_line,
            "require_land_status": resolved_require_land_status,
            "blocking": resolved_blocking,
            "max_parallel": resolved_max_parallel,
            "audit_first": resolved_audit_first,
            "include_lineage_representatives": include_lineage_representatives,
        }
        overall_status = "pass" if not lineage_problems and not behavior_failures else "fail"

        summary_json = {
            "trigger": trigger,
            "selector": resolved_selector,
            "config": resolved_config,
            "selected_tasks": selected_tasks,
            "lineage_findings": lineage_findings,
            "behavior_regressions": behavior_regressions,
        }
        artifacts = dict(suite.get("artifacts") or {})
        summary_json_path = str(artifacts.get("summary_json") or "").strip()
        if summary_json_path:
            target = workspace / summary_json_path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(json.dumps(summary_json, indent=2, ensure_ascii=False), encoding="utf-8")
        summary_markdown_path = str(artifacts.get("summary_markdown") or "").strip()
        if summary_markdown_path:
            target = workspace / summary_markdown_path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(
                _task_batch_markdown(
                    selector=resolved_selector,
                    config=resolved_config,
                    selected_tasks=selected_tasks,
                    lineage_findings=lineage_findings,
                    behavior_regressions=behavior_regressions,
                ),
                encoding="utf-8",
            )

        return {
            "suite_id": suite.get("suite_id"),
            "display_name": suite.get("display_name"),
            "artifact_path": suite.get("_artifact_path"),
            "plane": suite.get("plane"),
            "mode": suite.get("mode"),
            "blocking": resolved_blocking,
            "purpose": suite.get("purpose"),
            "runner_kind": runner.get("kind"),
            "status": overall_status,
            "trigger": trigger,
            "selector": resolved_selector,
            "config": resolved_config,
            "selected_tasks": selected_tasks,
            "lineage_findings": lineage_findings,
            "behavior_regressions": behavior_regressions,
            "artifacts": _artifact_payload(workspace, artifacts),
        }


def _full_repo_triage(
    ctx: ServerContext,
    repo_name: str,
    suite: dict[str, Any],
    suite_result: dict[str, Any],
    *,
    target_line: str,
) -> dict[str, Any]:
    triage = dict(suite.get("triage") or {})
    ownership_rules = list(triage.get("ownership_rules") or [])
    artifacts = dict((suite_result.get("artifacts") or {}))
    junit_path = str(((artifacts.get("junit_xml") or {}).get("path")) or "").strip()
    cases = _parse_junit_cases(Path(junit_path)) if junit_path else []
    failed_cases = [case for case in cases if case.get("status") in {"fail", "error"}]
    ownership_matches: list[dict[str, Any]] = []
    ownership_counts: dict[str, int] = {}
    unmapped_failures = 0
    for case in failed_cases:
        owner = _owner_for_case(case, ownership_rules)
        if owner is None:
            unmapped_failures += 1
            owner = "unmapped"
        ownership_counts[owner] = ownership_counts.get(owner, 0) + 1
        ownership_matches.append({**case, "owner": owner})
    suspect_selector = str(triage.get("suspect_task_selector") or "").strip()
    suspect_tasks: list[dict[str, Any]] = []
    if suspect_selector == "recent_remote_landed":
        suspect_tasks = _recent_landed_tasks(ctx, repo_name, target_line=target_line, limit=5, window_days=7)
    elif suspect_selector == "recent_remote_landed_high_risk":
        suspect_tasks = _recent_landed_tasks(
            ctx,
            repo_name,
            target_line=target_line,
            limit=5,
            window_days=7,
            risk_tier="high",
        )
    return {
        "ownership_rules": ownership_rules,
        "failed_test_count": len(failed_cases),
        "failed_test_cases": failed_cases,
        "ownership_summary": ownership_counts,
        "ownership_matches": ownership_matches,
        "unmapped_failures": unmapped_failures,
        "suspect_task_selector": suspect_selector or None,
        "suspect_tasks": suspect_tasks,
    }


def _augment_suite_result(
    ctx: ServerContext,
    repo_name: str,
    ci_config: dict[str, Any],
    suite: dict[str, Any],
    suite_result: dict[str, Any],
    *,
    target_line: str,
    dependency_evidence: list[str] | None = None,
    compliance_evidence: list[str] | None = None,
) -> dict[str, Any]:
    if str(suite.get("suite_id") or "").strip() == "full_repo":
        suite_result["triage"] = _full_repo_triage(ctx, repo_name, suite, suite_result, target_line=target_line)
    if str(suite.get("plane") or "").strip().lower() == "release":
        release_gate = _release_gate_contract(ci_config, suite)
        dependency_items = _dedupe_strs(dependency_evidence)
        compliance_items = _dedupe_strs(compliance_evidence)
        suite_result["release_gate_evidence"] = {
            **release_gate,
            "attached_dependency_evidence": dependency_items,
            "attached_compliance_evidence": compliance_items,
            "missing_dependency_keys": [item for item in release_gate["dependency_keys"] if item not in dependency_items],
            "missing_compliance_keys": [item for item in release_gate["compliance_keys"] if item not in compliance_items],
        }
    return suite_result


def _run_repo_suite(
    ctx: ServerContext,
    repo_name: str,
    snapshot_id: str,
    ci_config: dict[str, Any],
    suite: dict[str, Any],
    all_suites: list[dict[str, Any]],
    *,
    target_line: str,
    trigger: str,
    selector: str | None,
    task_ids: list[str] | None,
    curated_corpus: str | None,
    count: int | None,
    window_days: int | None,
    dependency_evidence: list[str] | None,
    compliance_evidence: list[str] | None,
) -> dict[str, Any]:
    runner_kind = str(((suite.get("runner") or {}).get("kind") or "")).strip().lower()
    if runner_kind == "task_batch":
        return _run_task_batch_suite(
            ctx,
            repo_name,
            snapshot_id,
            ci_config,
            suite,
            all_suites,
            target_line=target_line,
            trigger=trigger,
            selector=selector,
            task_ids=task_ids,
            curated_corpus=curated_corpus,
            count=count,
            window_days=window_days,
        )
    with tempfile.TemporaryDirectory(prefix=f"ait-repo-ci-{repo_name.lower()}-") as temp_dir:
        workspace = Path(temp_dir)
        _materialize_snapshot(ctx, snapshot_id, workspace)
        _ensure_local_runtime_repo(workspace, repo_name, ci_config=ci_config)
        raw_result = _run_one_suite(workspace, suite)
        return _augment_suite_result(
            ctx,
            repo_name,
            ci_config,
            suite,
            raw_result,
            target_line=target_line,
            dependency_evidence=dependency_evidence,
            compliance_evidence=compliance_evidence,
        )


def run_repo_ci(
    ctx: ServerContext,
    repo_name: str,
    *,
    suite_ids: list[str] | None = None,
    plane: str | None = None,
    target_line: str = "main",
    trigger: str = "manual_rerun",
    selector: str | None = None,
    task_ids: list[str] | None = None,
    curated_corpus: str | None = None,
    count: int | None = None,
    window_days: int | None = None,
    dependency_evidence: list[str] | None = None,
    compliance_evidence: list[str] | None = None,
) -> dict[str, Any]:
    snapshot_id = read_ref(ctx, repo_name, target_line)
    contract = _load_snapshot_ci_contract(ctx, snapshot_id)
    ci_config = dict(contract.get("ci") or {})
    all_suites = _load_suite_manifests(ctx, snapshot_id)
    suites = _selected_repo_suites(
        all_suites,
        ci_config,
        suite_ids=suite_ids,
        plane=plane,
    )
    suite_results = [
        _run_repo_suite(
            ctx,
            repo_name,
            snapshot_id,
            ci_config,
            suite,
            all_suites,
            target_line=target_line,
            trigger=trigger,
            selector=selector,
            task_ids=task_ids,
            curated_corpus=curated_corpus,
            count=count,
            window_days=window_days,
            dependency_evidence=dependency_evidence,
            compliance_evidence=compliance_evidence,
        )
        for suite in suites
    ]
    failures = [suite for suite in suite_results if suite["status"] != "pass"]
    blocking_failures = [suite for suite in failures if suite["blocking"]]
    return {
        "repo_name": repo_name,
        "target_line": target_line,
        "snapshot_id": snapshot_id,
        "trigger": trigger,
        "selected_suite_ids": [str(suite.get("suite_id") or "") for suite in suites],
        "selected_planes": sorted({str(suite.get("plane") or "") for suite in suites}),
        "status": "pass" if not failures else "fail",
        "blocking_failures": [str(suite.get("suite_id") or "") for suite in blocking_failures],
        "dependency_evidence": _dedupe_strs(dependency_evidence),
        "compliance_evidence": _dedupe_strs(compliance_evidence),
        "suite_results": suite_results,
    }
