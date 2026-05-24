from __future__ import annotations

import difflib
import json
import os
import posixpath
import re
from pathlib import Path
from typing import Any, Callable, Mapping

from .local_repo_seams import RepoContext, load_config
from .task_dag_seams import build_task_graph_execution_strategy, build_task_graph_progress, compute_task_graph_readiness, topological_node_order, validate_task_graph
from ait_protocol.task_statuses import (
    TASK_STATUS_LATER_PROMOTION_EXCLUDED,
    is_task_abandoned_status,
    is_task_later_promotion_excluded_status,
    task_status_matches_filter,
)
from ait_protocol.common import utc_now, workflow_id_matches

from .authority_store import list_authority_graph, replace_authority_graph
from .server_content import (
    connect as connect_content,
    ensure_repository,
    list_repository_groups,
    read_blob_bytes,
    read_ref,
    snapshot_manifest_map,
)
from .server_control import connect, latest_policy_status
from .server_paths import ServerContext
from .server_db import postgres_schema_upgrade_checks
from .server_queue import job_diagnostics, list_jobs
from .shared_runtime_policy import evaluate_shared_runtime_policy
from .store.repo_ops import _repo_id
from .read_models_domains.ci_status import patchset_ci_status, repository_ci_runs
try:
    from .live_turns import snapshot_live_turn_metrics
except ImportError:  # pragma: no cover - exercised when the runtime helper is absent during partial bootstraps.
    def snapshot_live_turn_metrics() -> dict[str, Any]:
        return {
            "active_turns": 0,
            "active_repositories": {},
            "oldest_active_turn_started_at": None,
            "oldest_active_turn_age_seconds": None,
            "recent_completed_turns": [],
            "recent_failed_turns": [],
            "recent_completed_p95_seconds": None,
        }
from .server_store import (
    _review_summary,
    get_attestation,
    get_change,
    get_change_for_repo,
    get_land_request,
    get_patchset,
    get_patchset_for_repo,
    get_plan,
    get_policy_status,
    get_repository,
    get_repository_storage,
    get_stack,
    get_stack_graph,
    get_task,
    get_task_for_repo,
    list_changes,
    list_lines,
    list_patchsets,
    list_patchsets_for_repo,
    list_reviews,
    list_session_checkpoints,
    list_session_events,
    list_sessions,
    list_tasks,
)


REVIEWABLE_CHANGE_STATES = {"review", "gated", "approved", "landable", "blocked"}
TASK_QUEUE_STATE_PRIORITY = {
    "attention_required": 0,
    "ready_to_land": 1,
    "ready_to_complete": 2,
    "in_review": 3,
    "in_progress": 4,
    "planning": 5,
    "completed": 6,
    "abandoned": 7,
    "later_promotion_excluded": 7,
    "canceled": 7,
}


WORKFLOW_CONTEXT_SPECS: dict[str, tuple[dict[str, str], ...]] = {
    "task": (
        {
            "path": "docs/ait_roadmap.md",
            "heading": "## Phase 3: Web as a Real Collaboration Surface",
            "layer": "command",
            "document_kind": "Roadmap",
            "relevance": "Current implementation window for the plan-first collaboration surface.",
        },
        {
            "path": "docs/milestone.md",
            "heading": "### M5: Web Collaboration and Executive Visibility",
            "layer": "command",
            "document_kind": "Milestone Index",
            "relevance": "Current milestone routing for web workflow and executive visibility.",
        },
        {
            "path": "docs/ait_product_segmentation.md",
            "heading": "## Product Surfaces",
            "layer": "legal",
            "document_kind": "Product Segmentation",
            "relevance": "Explains the human collaboration role that the task page should support.",
        },
        {
            "path": "docs/ait_architecture_principles.md",
            "heading": "## Architecture Checkpoints",
            "layer": "legal",
            "document_kind": "Architecture Principle",
            "relevance": "Defines workflow, read-model, and review boundaries for the task surface.",
        },
        {
            "path": "docs/plan.md",
            "heading": "## Constitutional Statements",
            "layer": "constitutional",
            "document_kind": "Constitution",
            "relevance": "Top-level repository authority for plan-first workflow and Markdown governance.",
        },
    ),
    "change": (
        {
            "path": "docs/ait_roadmap.md",
            "heading": "## Phase 3: Web as a Real Collaboration Surface",
            "layer": "command",
            "document_kind": "Roadmap",
            "relevance": "Current delivery scope for the change-review browser surface.",
        },
        {
            "path": "docs/milestone.md",
            "heading": "### M5: Web Collaboration and Executive Visibility",
            "layer": "command",
            "document_kind": "Milestone Index",
            "relevance": "Milestone routing for reviewer, manager, and executive web visibility.",
        },
        {
            "path": "docs/ait_product_segmentation.md",
            "heading": "## Product Surfaces",
            "layer": "legal",
            "document_kind": "Product Segmentation",
            "relevance": "Product framing for change review, queues, and operator UX.",
        },
        {
            "path": "docs/ait_architecture_principles.md",
            "heading": "## Architecture Checkpoints",
            "layer": "legal",
            "document_kind": "Architecture Principle",
            "relevance": "Architectural rules for patchset-centric review plus Markdown and Mermaid context.",
        },
        {
            "path": "docs/plan.md",
            "heading": "## Constitutional Statements",
            "layer": "constitutional",
            "document_kind": "Constitution",
            "relevance": "Highest-authority statement for the web workflow surface and Markdown governance.",
        },
    ),
}

def _table_exists(conn, table_name: str) -> bool:
    try:
        if conn.backend == "sqlite":
            row = conn.execute(
                "select 1 from sqlite_master where type in ('table', 'view') and name = ?",
                (table_name,),
            ).fetchone()
        else:
            row = conn.execute(
                "select 1 from information_schema.tables where table_schema = current_schema() and table_name = ?",
                (table_name,),
            ).fetchone()
        return row is not None
    except Exception:
        return False


def _table_has_column(conn, table_name: str, column_name: str) -> bool:
    try:
        if conn.backend == "sqlite":
            for row in conn.execute(f"pragma table_info({table_name})").fetchall():
                if str(row.get("name") or "").strip() == column_name:
                    return True
            return False
        row = conn.execute(
            "select 1 from information_schema.columns where table_schema = current_schema() and table_name = ? and column_name = ?",
            (table_name, column_name),
        ).fetchone()
        return row is not None
    except Exception:
        return False


def _repo_scope_predicate(alias: str | None = None) -> str:
    prefix = f"{alias}." if alias else ""
    return f"({prefix}repo_id = ? or ({prefix}repo_id is null and {prefix}repo_name = ?))"


def _repo_scope_filter(ctx: ServerContext, repo_name: str, *, alias: str | None = None) -> tuple[str, tuple[Any, ...]]:
    try:
        repo_id = str(_repo_id(ctx, repo_name)).strip()
    except Exception:
        repo_id = ""
    if repo_id:
        return _repo_scope_predicate(alias), (repo_id, repo_name)
    prefix = f"{alias}." if alias else ""
    return f"{prefix}repo_name = ?", (repo_name,)


def _repo_scoped_change(ctx: ServerContext, change_id: str, repo_name: str | None) -> dict[str, Any]:
    if repo_name is None:
        return get_change(ctx, change_id)
    try:
        return get_change_for_repo(ctx, repo_name, change_id)
    except KeyError:
        # Compatibility fallback: drifted repo-name rows should still resolve by global change id.
        return get_change(ctx, change_id)


def _repo_scoped_task(ctx: ServerContext, task_id: str, repo_name: str | None) -> dict[str, Any]:
    if repo_name is None:
        return get_task(ctx, task_id)
    try:
        return get_task_for_repo(ctx, repo_name, task_id)
    except KeyError:
        # Compatibility fallback for drifted repository name metadata.
        return get_task(ctx, task_id)


def _repo_scoped_patchset(ctx: ServerContext, patchset_id: str, repo_name: str | None) -> dict[str, Any] | None:
    if patchset_id is None:
        return None
    if repo_name is None:
        return get_patchset(ctx, patchset_id)
    try:
        return get_patchset_for_repo(ctx, repo_name, patchset_id)
    except KeyError:
        # Compatibility fallback when repo-scoped rows carry legacy repo names.
        return get_patchset(ctx, patchset_id)


def _safe_int(value: Any, *, default: int = 0) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def _blob_text(ctx: ServerContext, blob_id: str) -> tuple[str | None, bool]:
    data = read_blob_bytes(ctx, blob_id)
    try:
        return data.decode("utf-8"), True
    except UnicodeDecodeError:
        return None, False



def _snapshot_files(ctx: ServerContext, snapshot_id: str) -> dict[str, dict[str, Any]]:
    with connect_content(ctx) as conn:
        rows = conn.execute(
            """
            select sf.path, sf.blob_id, sf.size_bytes, sf.mode, b.sha256
            from snapshot_files sf
            join blobs b on b.blob_id = sf.blob_id
            where sf.snapshot_id = ?
            order by sf.path asc
            """,
            (snapshot_id,),
        ).fetchall()
    out: dict[str, dict[str, Any]] = {}
    for row in rows:
        out[row["path"]] = {
            "path": row["path"],
            "blob_id": row["blob_id"],
            "size_bytes": row["size_bytes"],
            "mode": row["mode"],
            "sha256": row["sha256"],
        }
    return out



def _line_stats(old_text: str | None, new_text: str | None) -> tuple[int, int, str, bool]:
    if old_text is None and new_text is None:
        return 0, 0, "", False
    if old_text is None:
        lines = new_text.splitlines() if new_text else []
        diff_text = "\n".join(f"+ {line}" for line in lines[:400])
        return len(lines), 0, diff_text, True
    if new_text is None:
        lines = old_text.splitlines() if old_text else []
        diff_text = "\n".join(f"- {line}" for line in lines[:400])
        return 0, len(lines), diff_text, True

    old_lines = old_text.splitlines()
    new_lines = new_text.splitlines()
    sm = difflib.SequenceMatcher(a=old_lines, b=new_lines)
    insertions = 0
    deletions = 0
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag in {"insert", "replace"}:
            insertions += j2 - j1
        if tag in {"delete", "replace"}:
            deletions += i2 - i1
    diff_text = "\n".join(
        difflib.unified_diff(
            old_lines,
            new_lines,
            fromfile="before",
            tofile="after",
            lineterm="",
            n=3,
        )
    )
    return insertions, deletions, diff_text[:50000], True



def _latest_land_summary(ctx: ServerContext, change_id: str) -> dict[str, Any] | None:
    with connect(ctx) as conn:
        row = conn.execute(
            "select submission_id from land_requests where change_id = ? order by created_at desc limit 1",
            (change_id,),
        ).fetchone()
    if row is None:
        return None
    land = get_land_request(ctx, row["submission_id"])
    result = land.get("result") or {}
    return {
        "submission_id": land["submission_id"],
        "change_id": land["change_id"],
        "patchset_id": land["patchset_id"],
        "target_line": land["target_line"],
        "status": land["status"],
        "blocker_class": result.get("code"),
        "suggested_action": result.get("message"),
        "updated_at": land["updated_at"],
        "result": result,
    }


def _missing_requirements(policy: dict[str, Any]) -> list[str]:
    missing: list[str] = []
    for check in policy.get("checks", []):
        if check.get("status") in {"pending", "hard_fail", "soft_fail"}:
            missing.append(check.get("name") or check.get("label") or "unknown")
    return missing


def _effective_validation_state(
    policy: dict[str, Any],
    attestation_summary: dict[str, Any] | None,
    *,
    key: str,
    requirement_key: str,
) -> str:
    effective_requirements = policy.get("effective_requirements") or {}
    if not bool(effective_requirements.get(requirement_key, False)):
        return "not_required"
    if attestation_summary is None:
        return "pending"
    return (attestation_summary.get("evaluation_summary") or {}).get(key) or "pending"


def _remote_land_gate_state(
    change: dict[str, Any],
    *,
    current_patchset: dict[str, Any] | None,
    review_summary: dict[str, Any],
    policy_summary: dict[str, Any],
    freshness: dict[str, Any],
    tests_state: str,
) -> str:
    if current_patchset is None:
        return "pending"
    if int(review_summary.get("blocking") or 0) > 0 or not freshness.get("base_is_fresh", True):
        return "blocked"
    decision = str(policy_summary.get("decision") or "pending").strip().lower()
    if decision == "pass" and change["status"] in {"review", "gated", "approved", "landable"}:
        return "pass"
    if decision in {"hard_fail", "soft_fail", "fail", "failed"} or str(tests_state or "").strip().lower() == "fail":
        return "blocked"
    return "pending"


def _task_ci_summary(
    ctx: ServerContext,
    change: dict[str, Any],
    *,
    current_patchset: dict[str, Any] | None,
    review_summary: dict[str, Any],
    policy_summary: dict[str, Any],
    freshness: dict[str, Any],
    tests_state: str,
) -> dict[str, Any] | None:
    if current_patchset is None:
        return None
    ci_status = patchset_ci_status(ctx, current_patchset["patchset_id"], recent_limit=1)
    tests_status = str(ci_status.get("tests_status") or tests_state or "pending").strip() or "pending"
    return {
        "patchset_id": current_patchset["patchset_id"],
        "patchset_number": current_patchset.get("patchset_number"),
        "tests_status": tests_status,
        "selected_suite_ids": [str(item).strip() for item in ci_status.get("selected_suite_ids") or [] if str(item).strip()],
        "blocking_failures": [str(item).strip() for item in ci_status.get("blocking_failures") or [] if str(item).strip()],
        "tg1_required": dict(ci_status.get("tg1_required") or {}),
        "remote_land_gate": _remote_land_gate_state(
            change,
            current_patchset=current_patchset,
            review_summary=review_summary,
            policy_summary=policy_summary,
            freshness=freshness,
            tests_state=tests_status,
        ),
    }



def _normalize_inbox_filter(value: str | None) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _repo_root() -> Path:
    configured = os.environ.get("AIT_REPO_ROOT")
    if configured:
        return Path(configured).expanduser().resolve()
    return Path(__file__).resolve().parents[2]


def _repo_display_name(repo_root: Path) -> str:
    try:
        repo_ctx = RepoContext.discover(repo_root)
        config = load_config(repo_ctx)
    except Exception:
        return repo_root.name
    value = config.get("repo_name")
    if isinstance(value, str) and value.strip():
        return value.strip()
    return repo_root.name


def _section_title(heading: str) -> str:
    return re.sub(r"^#+\s*", "", heading).strip()


def _section_anchor(title: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
    return slug or "section"


def _ordered_unique(items: list[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for item in items:
        if item in seen:
            continue
        seen.add(item)
        ordered.append(item)
    return ordered


def _document_title(lines: list[str], fallback: str) -> str:
    for line in lines:
        if line.startswith("# "):
            return line[2:].strip()
    return fallback


def _extract_markdown_section(doc_path: Path, heading: str) -> tuple[str, int, str] | None:
    try:
        lines = doc_path.read_text(encoding="utf-8").splitlines()
    except FileNotFoundError:
        return None
    start_index = next((index for index, line in enumerate(lines) if line.strip() == heading), None)
    if start_index is None:
        return None
    heading_level = len(heading) - len(heading.lstrip("#"))
    end_index = len(lines)
    for index in range(start_index + 1, len(lines)):
        line = lines[index].strip()
        if not re.match(r"^#+\s", line):
            continue
        level = len(line) - len(line.lstrip("#"))
        if level <= heading_level:
            end_index = index
            break
    content = "\n".join(lines[start_index + 1 : end_index]).strip()
    title = _document_title(lines, doc_path.stem.replace("_", " "))
    return content, start_index + 1, title


def _workflow_context_entry(repo_root: Path, spec: dict[str, str], *, default_open: bool) -> dict[str, Any] | None:
    doc_path = repo_root / spec["path"]
    extracted = _extract_markdown_section(doc_path, spec["heading"])
    if extracted is None:
        return None
    markdown, line_number, document_title = extracted
    section_title = _section_title(spec["heading"])
    return {
        "layer": spec["layer"],
        "document_kind": spec["document_kind"],
        "document_path": spec["path"],
        "document_title": document_title,
        "section_title": section_title,
        "section_anchor": _section_anchor(section_title),
        "line_number": line_number,
        "relevance": spec["relevance"],
        "markdown": markdown,
        "contains_mermaid": "```mermaid" in markdown,
        "default_open": default_open,
    }


def _workflow_context(target: str, *, focus_type: str, focus_id: str, focus_title: str) -> dict[str, Any]:
    repo_root = _repo_root()
    entries = [
        entry
        for index, spec in enumerate(WORKFLOW_CONTEXT_SPECS.get(target, ()))
        if (entry := _workflow_context_entry(repo_root, spec, default_open=index == 0)) is not None
    ]
    layers = _ordered_unique([entry["layer"] for entry in entries])
    return {
        "target": target,
        "focus": {
            "type": focus_type,
            "id": focus_id,
            "title": focus_title,
        },
        "summary": {
            "document_count": len(entries),
            "diagram_count": sum(1 for entry in entries if entry["contains_mermaid"]),
            "layers": layers,
        },
        "entries": entries,
    }


def _markdown_link_targets(markdown: str) -> list[str]:
    targets: list[str] = []
    for _label, target in re.findall(r"\[([^\]]+)\]\(([^)]+)\)", markdown or ""):
        text = str(target or "").strip()
        if text:
            targets.append(text)
    return targets


AuthorityMarkdownLoader = Callable[[str], str | None]


def _local_markdown_paths(repo_root: Path) -> list[str]:
    paths: list[str] = []
    for target in sorted(repo_root.rglob("*.md")):
        rel = target.relative_to(repo_root).as_posix()
        if rel.startswith((".ait/", ".ait-server/", ".pytest_cache/", ".venv/", "build/")):
            continue
        paths.append(rel)
    return paths


def _snapshot_markdown_paths(ctx: ServerContext, repo_name: str) -> list[str]:
    try:
        repo = get_repository(ctx, repo_name)
    except KeyError:
        return []
    default_line = str(repo.get("default_line") or "main")
    snapshot_id = read_ref(ctx, repo_name, default_line)
    if not snapshot_id:
        return []
    return [
        path
        for path in sorted(_snapshot_files(ctx, snapshot_id))
        if path.endswith('.md') and not path.startswith((".ait/", ".ait-server/", ".pytest_cache/", ".venv/", "build/"))
    ]


def _local_markdown_loader(repo_root: Path) -> AuthorityMarkdownLoader:
    def load(path: str) -> str | None:
        target = repo_root / path
        try:
            return target.read_text(encoding="utf-8")
        except FileNotFoundError:
            return None

    return load


def _snapshot_markdown_loader(ctx: ServerContext, repo_name: str) -> AuthorityMarkdownLoader:
    try:
        repo = get_repository(ctx, repo_name)
    except KeyError:
        return lambda _path: None
    default_line = str(repo.get("default_line") or "main")
    snapshot_id = read_ref(ctx, repo_name, default_line)
    if not snapshot_id:
        return lambda _path: None
    snapshot_files = _snapshot_files(ctx, snapshot_id)
    decoded_cache: dict[str, str | None] = {}

    def load(path: str) -> str | None:
        if path in decoded_cache:
            return decoded_cache[path]
        row = snapshot_files.get(path)
        if row is None:
            decoded_cache[path] = None
            return None
        text, is_text = _blob_text(ctx, str(row["blob_id"]))
        decoded_cache[path] = text if is_text else None
        return decoded_cache[path]

    return load


def _normalize_markdown_target(source_path: str, target: str) -> str | None:
    cleaned = str(target or "").strip()
    if not cleaned or cleaned.startswith(("http://", "https://", "mailto:", "#")):
        return None
    cleaned = cleaned.split("#", 1)[0].split("?", 1)[0].strip()
    if not cleaned:
        return None
    if cleaned.startswith("/"):
        normalized = posixpath.normpath(cleaned.lstrip("/"))
    else:
        normalized = posixpath.normpath(posixpath.join(posixpath.dirname(source_path), cleaned))
    if normalized in {"", ".", ".."} or normalized.startswith("../"):
        return None
    if not normalized.lower().endswith(".md"):
        return None
    return normalized


def _matches_inbox_filter(actual: str | None, expected: str | None) -> bool:
    if expected is None:
        return True
    if expected == "missing":
        return actual is None
    return actual == expected



def _matches_author_class(actual: str | None, expected: str | None) -> bool:
    if expected is None:
        return True
    if expected == "missing":
        return actual is None
    if expected == "human_only":
        return actual == "human_only"
    if expected == "ai_related":
        return actual in {"human_with_ai_assist", "ai_with_human_review", "ai_only_experimental"}
    return actual == expected


def _matches_review_filter(review_summary: dict[str, Any], requested_groups: list[str], expected: str | None) -> bool:
    if expected is None:
        return True
    approvals = int(review_summary.get("approvals") or 0)
    blocking = int(review_summary.get("blocking") or 0)
    comments = int(review_summary.get("comments") or 0)
    if expected == "approved":
        return approvals > 0 and blocking == 0
    if expected == "needs_approval":
        return approvals == 0 and blocking == 0
    if expected == "blocking":
        return blocking > 0
    if expected == "commented":
        return comments > 0
    if expected == "requested":
        return bool(requested_groups)
    return False



from .read_models_domains.authority_map import authority_map

from .read_models_domains.repository_overview import (
    repository_index,
    repository_detail,
    repository_worker_status,
)


from .read_models_domains.runtime_metrics import (
    _rounded_optional_float,
    normalize_live_turn_metrics,
    live_turn_pressure_summary,
    annotate_operator_read_payload,
    _path_inventory,
    _telegram_inventory,
    _workflow_session_inventory
)
from .read_models_domains.operator_metrics import (
    _OPERATOR_ACTION_PRIORITY,
    _count_rows,
    _first_present,
    _int_metric,
    _merge_count_summary,
    _ranked_operator_action,
    _readiness_check,
    operator_pressure_cache_ttl_seconds,
    server_metrics,
    server_readiness,
)

from .read_models_domains.task_queue import (
    task_audit,
    task_queue,
)
from .read_models_domains.reviewer_inbox import reviewer_inbox
from .read_models_domains.task_dag import (
    _task_dag_latest_graph_run_summary,
    _task_dag_workflow_facts,
    task_dag_graph,
    task_dag_graph_from_facts,
    task_dag_progress,
    task_dag_progress_from_facts,
    task_dag_readiness,
    task_dag_readiness_from_facts,
    task_dag_schedule,
    task_dag_schedule_from_facts,
)
from .read_models_domains.workflow_detail import (
    change_detail,
    patchset_delta,
    stack_detail,
    task_detail,
)
