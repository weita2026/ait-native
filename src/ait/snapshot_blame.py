from __future__ import annotations

import hashlib
from difflib import SequenceMatcher
from pathlib import Path
from typing import Any

from ait_protocol.common import connect_sqlite

from . import local_content, local_control
from .local_content_projection import _is_lineage_only_markdown_artifact_path
from .remote_client import (
    RemoteError,
    get_change_detail as remote_get_change_detail,
    get_plan_revision as remote_get_plan_revision,
    list_patchsets as remote_list_patchsets,
)
from .repo_paths import RepoContext
from .store import (
    get_local_plan,
    get_remote as resolve_remote,
    list_local_plan_revisions,
    list_local_plans,
)

PROVENANCE_CONFIDENCE_DIRECT = "direct_snapshot_binding"
PROVENANCE_CONFIDENCE_PLAN = "direct_plan_revision_binding"
PROVENANCE_CONFIDENCE_PATCHSET = "derived_from_patchset"
PROVENANCE_CONFIDENCE_LAND = "derived_from_land"
PROVENANCE_CONFIDENCE_EVENT = "event_inferred"
PROVENANCE_CONFIDENCE_UNKNOWN = "unknown"


class MissingMarkdownRevisionBodyError(ValueError):
    """Raised when a lineage-only Markdown revision body cannot be materialized."""

__all__ = [
    "PROVENANCE_CONFIDENCE_DIRECT",
    "PROVENANCE_CONFIDENCE_PLAN",
    "PROVENANCE_CONFIDENCE_PATCHSET",
    "PROVENANCE_CONFIDENCE_LAND",
    "PROVENANCE_CONFIDENCE_EVENT",
    "PROVENANCE_CONFIDENCE_UNKNOWN",
    "apply_scoped_restore",
    "compute_markdown_plan_blame",
    "compute_snapshot_blame",
    "normalize_blame_path",
    "path_uses_markdown_plan_lineage",
    "preview_scoped_restore",
    "public_blame_payload",
]


def normalize_blame_path(ctx: RepoContext, path_value: str | Path) -> str:
    raw = str(path_value or "").strip()
    if not raw:
        raise ValueError("Path is required.")
    root = ctx.root.resolve()
    candidate = Path(raw).expanduser()
    if candidate.is_absolute():
        resolved = candidate.resolve(strict=False)
    else:
        resolved = (root / candidate).resolve(strict=False)
    try:
        rel_path = resolved.relative_to(root).as_posix()
    except ValueError as exc:
        raise ValueError(f"Path {raw!r} is outside the current workspace root.") from exc
    if not rel_path or rel_path == ".":
        raise ValueError("Choose one file path to blame.")
    return rel_path


def _decode_text_lines(data: bytes, *, label: str) -> list[str]:
    if b"\x00" in data:
        raise ValueError(f"{label} is binary and cannot be blamed.")
    try:
        return data.decode("utf-8").splitlines(keepends=True)
    except UnicodeDecodeError as exc:
        raise ValueError(f"{label} is not valid UTF-8 text and cannot be blamed.") from exc


def _current_workspace_lines(ctx: RepoContext, rel_path: str) -> list[str]:
    abs_path = ctx.root / rel_path
    if not abs_path.exists():
        raise FileNotFoundError(f"Workspace file {rel_path} does not exist.")
    if abs_path.is_dir():
        raise IsADirectoryError(f"Path {rel_path} is a directory, not a file.")
    return _decode_text_lines(abs_path.read_bytes(), label=f"Workspace file {rel_path}")


def _current_repo_root_bytes(ctx: RepoContext, rel_path: str) -> bytes:
    for root in (ctx.root, ctx.repo_root):
        abs_path = root / rel_path
        if not abs_path.exists():
            continue
        if abs_path.is_dir():
            raise IsADirectoryError(f"Path {rel_path} is a directory, not a file.")
        return abs_path.read_bytes()
    raise FileNotFoundError(f"Workspace file {rel_path} does not exist.")


def _snapshot_file_row(conn, snapshot_id: str, rel_path: str) -> dict[str, Any] | None:
    row = conn.execute(
        """
        select sf.snapshot_id, sf.path, sf.blob_id, sf.size_bytes, sf.mode
        from snapshot_files sf
        where sf.snapshot_id = ? and sf.path = ?
        """,
        (snapshot_id, rel_path),
    ).fetchone()
    return dict(row) if row is not None else None


def _snapshot_file_rows_for_path_via_view(conn, snapshot_ids: list[str], rel_path: str) -> dict[str, dict[str, Any]]:
    if not snapshot_ids:
        return {}
    rows_by_snapshot: dict[str, dict[str, Any]] = {}
    chunk_size = 400
    for start in range(0, len(snapshot_ids), chunk_size):
        chunk = snapshot_ids[start:start + chunk_size]
        placeholders = ", ".join("?" for _ in chunk)
        rows = conn.execute(
            f"""
            select sf.snapshot_id, sf.path, sf.blob_id, sf.size_bytes, sf.mode
            from snapshot_files sf
            where sf.path = ? and sf.snapshot_id in ({placeholders})
            """,
            (rel_path, *chunk),
        ).fetchall()
        for row in rows:
            payload = dict(row)
            rows_by_snapshot[str(payload["snapshot_id"])] = payload
    return rows_by_snapshot


def _snapshot_root_tree_ids(conn, snapshot_ids: list[str]) -> dict[str, str]:
    if not snapshot_ids:
        return {}
    rows_by_snapshot: dict[str, str] = {}
    chunk_size = 400
    for start in range(0, len(snapshot_ids), chunk_size):
        chunk = snapshot_ids[start:start + chunk_size]
        placeholders = ", ".join("?" for _ in chunk)
        rows = conn.execute(
            f"""
            select snapshot_id, root_tree_id
            from snapshots
            where snapshot_id in ({placeholders})
            """,
            tuple(chunk),
        ).fetchall()
        for row in rows:
            snapshot_id = str(row["snapshot_id"] or "").strip()
            root_tree_id = str(row["root_tree_id"] or "").strip()
            if snapshot_id and root_tree_id:
                rows_by_snapshot[snapshot_id] = root_tree_id
    return rows_by_snapshot


def _tree_entry_row(
    conn,
    tree_id: str,
    entry_name: str,
    *,
    cache: dict[tuple[str, str], dict[str, Any] | None],
) -> dict[str, Any] | None:
    key = (tree_id, entry_name)
    if key in cache:
        return cache[key]
    size_expr = local_content._tree_entry_blob_size_sql(conn, "te", "b")
    row = conn.execute(
        f"""
        select
            te.entry_type,
            te.target_id,
            {size_expr} as size_bytes,
            te.mode
        from tree_entries te
        left join blobs b on b.blob_id = te.target_id
        where te.tree_id = ? and te.entry_name = ?
        """,
        (tree_id, entry_name),
    ).fetchone()
    payload = dict(row) if row is not None else None
    cache[key] = payload
    return payload


def _snapshot_file_rows_for_path(conn, snapshot_ids: list[str], rel_path: str) -> dict[str, dict[str, Any]]:
    if not snapshot_ids:
        return {}
    path_parts = [part for part in rel_path.split("/") if part]
    if not path_parts:
        return {}
    root_tree_ids = _snapshot_root_tree_ids(conn, snapshot_ids)
    if len(root_tree_ids) != len(snapshot_ids):
        return _snapshot_file_rows_for_path_via_view(conn, snapshot_ids, rel_path)

    tree_entry_cache: dict[tuple[str, str], dict[str, Any] | None] = {}
    rows_by_snapshot: dict[str, dict[str, Any]] = {}
    for snapshot_id in snapshot_ids:
        current_tree_id = root_tree_ids.get(snapshot_id)
        if current_tree_id is None:
            continue
        for index, part in enumerate(path_parts):
            entry = _tree_entry_row(conn, current_tree_id, part, cache=tree_entry_cache)
            if entry is None:
                break
            entry_type = str(entry.get("entry_type") or "")
            if index == len(path_parts) - 1:
                if entry_type != "blob":
                    break
                rows_by_snapshot[snapshot_id] = {
                    "snapshot_id": snapshot_id,
                    "path": rel_path,
                    "blob_id": entry.get("target_id"),
                    "size_bytes": entry.get("size_bytes"),
                    "mode": entry.get("mode"),
                }
                break
            if entry_type != "tree":
                break
            current_tree_id = str(entry.get("target_id") or "").strip()
            if not current_tree_id:
                break
    return rows_by_snapshot


def _blob_text_lines(
    conn,
    ctx: RepoContext,
    blob_id: str,
    *,
    cache: dict[str, list[str]],
    label: str,
) -> list[str]:
    cached = cache.get(blob_id)
    if cached is not None:
        return cached
    data = local_content._blob_bytes_by_id(ctx, conn, blob_id)
    decoded = _decode_text_lines(data, label=label)
    cache[blob_id] = decoded
    return decoded


def _snapshot_text_lines(conn, ctx: RepoContext, snapshot_id: str, rel_path: str) -> list[str] | None:
    row = _snapshot_file_row(conn, snapshot_id, rel_path)
    if row is None:
        return None
    return _blob_text_lines(
        conn,
        ctx,
        str(row["blob_id"]),
        cache={},
        label=f"Snapshot {snapshot_id}:{rel_path}",
    )


def _snapshot_row_map(conn, snapshot_ids: list[str]) -> dict[str, dict[str, Any]]:
    if not snapshot_ids:
        return {}
    placeholders = ", ".join("?" for _ in snapshot_ids)
    rows = conn.execute(
        f"""
        select snapshot_id, parent_snapshot_id, message, line_name, created_at
        from snapshots
        where snapshot_id in ({placeholders})
        """,
        tuple(snapshot_ids),
    ).fetchall()
    return {
        str(row["snapshot_id"]): {
            "snapshot_id": row["snapshot_id"],
            "parent_snapshot_id": row["parent_snapshot_id"],
            "message": row["message"],
            "line_name": row["line_name"],
            "created_at": row["created_at"],
        }
        for row in rows
    }


def path_uses_markdown_plan_lineage(ctx: RepoContext, path_value: str | Path) -> bool:
    rel_path = normalize_blame_path(ctx, path_value)
    return bool(_is_lineage_only_markdown_artifact_path(rel_path))


def _normalize_text(value: Any) -> str | None:
    text = str(value or "").strip()
    return text or None


def _apply_overlay_defaults(entry: dict[str, Any], fallback: dict[str, Any]) -> None:
    for key, value in fallback.items():
        if value is None:
            continue
        if entry.get(key) in {None, ""}:
            entry[key] = value


def _local_landed_change_fallback(
    ctx: RepoContext,
    *,
    snapshot_ids: list[str],
) -> dict[str, dict[str, Any]]:
    requested = {snapshot_id for snapshot_id in snapshot_ids if snapshot_id}
    if not requested:
        return {}
    fallback: dict[str, dict[str, Any]] = {}
    for change in local_control.list_workflow_changes(ctx):
        landed_snapshot_id = _normalize_text(change.get("landed_snapshot_id"))
        if landed_snapshot_id is None or landed_snapshot_id not in requested:
            continue
        existing = fallback.get(landed_snapshot_id)
        landed_at = _normalize_text(change.get("landed_at")) or ""
        if existing is not None and landed_at <= str(existing.get("_landed_at") or ""):
            continue
        fallback[landed_snapshot_id] = {
            "task_id": _normalize_text(change.get("task_id")),
            "change_id": _normalize_text(change.get("change_id")),
            "provenance_confidence": PROVENANCE_CONFIDENCE_LAND,
            "_landed_at": landed_at,
        }
    for value in fallback.values():
        value.pop("_landed_at", None)
    return fallback


def _remote_change_overlay(
    ctx: RepoContext,
    *,
    target: dict[str, Any],
    change_ids: list[str],
) -> dict[str, dict[str, Any]]:
    requested = [change_id for change_id in dict.fromkeys(change_ids) if change_id]
    if not requested:
        return {}
    remote_name = _normalize_text(target.get("remote_name"))
    try:
        remote = resolve_remote(ctx, remote_name)
    except KeyError:
        return {}
    repo_name = _normalize_text(target.get("repo_name")) or _normalize_text(remote.get("repo_name"))
    overlay: dict[str, dict[str, Any]] = {}
    for change_id in requested:
        try:
            detail = remote_get_change_detail(remote["url"], change_id, repo_name=repo_name)
            patchsets = remote_list_patchsets(remote["url"], change_id, repo_name=repo_name)
        except (KeyError, RemoteError, ValueError):
            continue
        task = detail.get("task") if isinstance(detail.get("task"), dict) else {}
        landing = detail.get("landing_summary") if isinstance(detail.get("landing_summary"), dict) else {}
        landing_result = landing.get("result") if isinstance(landing.get("result"), dict) else {}
        for patchset in patchsets:
            revision_snapshot_id = _normalize_text(patchset.get("revision_snapshot_id"))
            if revision_snapshot_id is None:
                continue
            row = overlay.setdefault(revision_snapshot_id, {})
            _apply_overlay_defaults(
                row,
                {
                    "task_id": _normalize_text(task.get("task_id")) or _normalize_text(detail.get("task_id")),
                    "change_id": _normalize_text(change_id),
                    "patchset_id": _normalize_text(patchset.get("patchset_id")),
                    "provenance_confidence": PROVENANCE_CONFIDENCE_PATCHSET,
                },
            )
        landed_snapshot_id = _normalize_text(landing_result.get("landed_snapshot_id"))
        if landed_snapshot_id is None:
            continue
        row = overlay.setdefault(landed_snapshot_id, {})
        _apply_overlay_defaults(
            row,
            {
                "task_id": _normalize_text(task.get("task_id")) or _normalize_text(detail.get("task_id")),
                "change_id": _normalize_text(change_id),
                "patchset_id": _normalize_text(landing.get("patchset_id")),
                "submission_id": _normalize_text(landing.get("submission_id")),
                "provenance_confidence": PROVENANCE_CONFIDENCE_LAND,
            },
        )
    return overlay


def _line_selection(
    *,
    total_lines: int,
    line: int | None = None,
    start_line: int | None = None,
    end_line: int | None = None,
) -> tuple[int, int]:
    if line is not None and (start_line is not None or end_line is not None):
        raise ValueError("Choose either --line or --start/--end.")
    if line is not None:
        start_line = line
        end_line = line
    if start_line is None and end_line is None:
        if total_lines == 0:
            return 0, 0
        return 1, total_lines
    if start_line is None or end_line is None:
        raise ValueError("Provide both --start and --end.")
    if start_line <= 0 or end_line <= 0:
        raise ValueError("Line selections are 1-based and must be positive.")
    if end_line < start_line:
        raise ValueError("The selected range must have end >= start.")
    if total_lines == 0:
        raise ValueError("The selected file is empty and has no blameable lines.")
    if end_line > total_lines:
        raise ValueError(f"Selected range {start_line}-{end_line} exceeds file length {total_lines}.")
    return start_line, end_line


def _apply_line_diff(
    old_lines: list[str],
    new_lines: list[str],
    old_owners: list[str],
    snapshot_id: str,
) -> list[str]:
    matcher = SequenceMatcher(a=old_lines, b=new_lines, autojunk=False)
    owners: list[str] = []
    for opcode, old_start, old_end, new_start, new_end in matcher.get_opcodes():
        if opcode == "equal":
            owners.extend(old_owners[old_start:old_end])
            continue
        if opcode in {"replace", "insert"}:
            owners.extend([snapshot_id] * (new_end - new_start))
            continue
        if opcode == "delete":
            continue
        raise ValueError(f"Unsupported diff opcode: {opcode}")
    return owners


def _plan_status_is_historical(status: Any) -> bool:
    return str(status or "").strip().lower() in {"archived", "superseded"}


def _plan_head_artifact_field(plan: dict[str, Any], field: str) -> str | None:
    head_revision = plan.get("head_revision") if isinstance(plan.get("head_revision"), dict) else {}
    nested_value = _normalize_text(head_revision.get(field))
    if nested_value is not None:
        return nested_value
    top_level_key = f"head_{field}"
    if top_level_key in plan:
        return _normalize_text(plan.get(top_level_key))
    return None


def _known_markdown_plan_refs(plans: list[dict[str, Any]]) -> list[str]:
    return sorted(
        {
            str(_plan_head_artifact_field(plan, "artifact_selector") or "").strip()
            for plan in plans
            if str(_plan_head_artifact_field(plan, "artifact_selector") or "").strip()
        }
    )


def _current_plan_for_artifact_path(
    ctx: RepoContext,
    rel_path: str,
    *,
    plan_id: str | None = None,
    plan_ref: str | None = None,
) -> dict[str, Any]:
    candidates = [
        plan
        for plan in list_local_plans(ctx)
        if _plan_head_artifact_field(plan, "artifact_path") == rel_path
    ]
    open_candidates = [plan for plan in candidates if not _plan_status_is_historical(plan.get("status"))]
    if not open_candidates:
        if candidates:
            raise ValueError(
                f"Lineage-only Markdown path {rel_path} is tracked only by historical plans. "
                "Use the repo root planning surface to inspect the historical record."
            )
        raise ValueError(
            f"Lineage-only Markdown path {rel_path} is not present in line snapshots and is not yet tracked in local plan lineage. "
            f"Run `ait plan sync {rel_path}` first."
        )
    if plan_id is not None:
        selected = [plan for plan in open_candidates if str(plan.get("plan_id") or "").strip() == str(plan_id).strip()]
        if not selected:
            raise ValueError(
                f"Lineage-only Markdown path {rel_path} is not tracked by current plan {plan_id}. "
                "Use a current plan id for this artifact path."
            )
        return get_local_plan(ctx, str(selected[0]["plan_id"]))
    if plan_ref is not None:
        resolved_ref = str(plan_ref).strip()
        selected = [
            plan
            for plan in open_candidates
            if str(_plan_head_artifact_field(plan, "artifact_selector") or "").strip() == resolved_ref
        ]
        if not selected:
            known_refs = _known_markdown_plan_refs(open_candidates)
            known_detail = f" Known tracked refs: {', '.join(known_refs)}." if known_refs else ""
            raise ValueError(
                f"Lineage-only Markdown path {rel_path} is not tracked by current plan ref {resolved_ref}.{known_detail}"
            )
        if len(selected) > 1:
            raise ValueError(
                f"Multiple current plans track lineage-only Markdown path {rel_path} with selector {resolved_ref}. "
                "Use `--plan-id` to choose one concrete plan."
            )
        return get_local_plan(ctx, str(selected[0]["plan_id"]))
    if len(open_candidates) > 1:
        selector_sample = _known_markdown_plan_refs(open_candidates)
        selector_detail = f" Known tracked refs: {', '.join(selector_sample)}." if selector_sample else ""
        raise ValueError(
            f"Multiple current plans track lineage-only Markdown path {rel_path}.{selector_detail} "
            "Use `--plan-ref` or `--plan-id` to choose one current tracked plan."
        )
    return get_local_plan(ctx, str(open_candidates[0]["plan_id"]))


def _repair_markdown_plan_revision_blob_bytes(
    ctx: RepoContext,
    *,
    plan: dict[str, Any],
    revision: dict[str, Any],
    rel_path: str,
) -> bytes | None:
    published_remote_name = _normalize_text(plan.get("published_remote_name"))
    published_plan_id = _normalize_text(plan.get("published_plan_id")) or _normalize_text(plan.get("plan_id"))
    published_revision_id = _normalize_text(revision.get("published_plan_revision_id"))
    expected_blob_id = _normalize_text(revision.get("artifact_blob_id"))
    if published_remote_name is None or published_plan_id is None or published_revision_id is None:
        return None
    remote = resolve_remote(ctx, published_remote_name)
    remote_revision = remote_get_plan_revision(remote["url"], published_plan_id, published_revision_id)
    artifact_body = remote_revision.get("artifact_body")
    if not isinstance(artifact_body, str):
        raise ValueError(
            f"Published remote plan revision {published_revision_id} for lineage-only Markdown path {rel_path} "
            "does not expose readable artifact body content."
        )
    materialized_blob_id = local_content.ensure_blob_bytes(
        ctx,
        artifact_body.encode("utf-8"),
        path_hint=_normalize_text(remote_revision.get("artifact_path")) or rel_path,
    )
    if expected_blob_id is not None and materialized_blob_id != expected_blob_id:
        raise ValueError(
            f"Published remote plan revision {published_revision_id} for lineage-only Markdown path {rel_path} "
            f"materialized blob {materialized_blob_id}, expected {expected_blob_id}."
        )
    return local_content._read_blob_bytes(ctx, materialized_blob_id)


def _plan_revision_body_bytes(
    ctx: RepoContext,
    *,
    plan: dict[str, Any],
    rel_path: str,
    revision: dict[str, Any],
    current_head_revision_id: str,
    current_bytes: bytes,
) -> bytes:
    blob_id = _normalize_text(revision.get("artifact_blob_id"))
    revision_id = str(revision.get("plan_revision_id") or "")
    if revision_id == current_head_revision_id:
        expected_blob_id = f"BLB-{hashlib.sha256(current_bytes).hexdigest()[:20]}"
        if blob_id is not None and blob_id != expected_blob_id:
            raise ValueError(
                f"Lineage-only Markdown path {rel_path} has unsynced local edits relative to local plan head {revision_id}. "
                f"Run `ait plan sync {rel_path}` first."
            )
        return current_bytes
    if blob_id is not None:
        try:
            return local_content._read_blob_bytes(ctx, blob_id)
        except KeyError:
            try:
                repaired = _repair_markdown_plan_revision_blob_bytes(
                    ctx,
                    plan=plan,
                    revision=revision,
                    rel_path=rel_path,
                )
            except (KeyError, RemoteError, ValueError):
                repaired = None
            if repaired is not None:
                return repaired
    raise MissingMarkdownRevisionBodyError(
        f"Plan revision {revision_id or '<unknown>'} for lineage-only Markdown path {rel_path} is missing readable artifact content locally."
    )


def _compute_markdown_plan_line_owners(
    ctx: RepoContext,
    *,
    plan: dict[str, Any],
    rel_path: str,
    revisions: list[dict[str, Any]],
    current_head_revision_id: str,
) -> tuple[list[str], list[str], list[str]]:
    if not revisions:
        raise ValueError(f"No local plan revisions exist for lineage-only Markdown path {rel_path}.")
    current_bytes = _current_repo_root_bytes(ctx, rel_path)
    current_lines = _decode_text_lines(current_bytes, label=f"Workspace file {rel_path}")

    previous_lines: list[str] = []
    previous_owners: list[str] = []
    skipped_revision_ids: list[str] = []
    for revision in revisions:
        revision_id = str(revision.get("plan_revision_id") or "").strip()
        if not revision_id:
            raise ValueError(f"Plan revision for lineage-only Markdown path {rel_path} is missing a revision id.")
        try:
            next_lines = _decode_text_lines(
                _plan_revision_body_bytes(
                    ctx,
                    plan=plan,
                    rel_path=rel_path,
                    revision=revision,
                    current_head_revision_id=current_head_revision_id,
                    current_bytes=current_bytes,
                ),
                label=f"Plan revision {revision_id}:{rel_path}",
            )
        except MissingMarkdownRevisionBodyError:
            skipped_revision_ids.append(revision_id)
            continue
        if not previous_lines:
            previous_lines = next_lines
            previous_owners = [revision_id] * len(next_lines)
            continue
        previous_owners = _apply_line_diff(previous_lines, next_lines, previous_owners, revision_id)
        previous_lines = next_lines
    if not previous_lines:
        raise MissingMarkdownRevisionBodyError(
            f"Lineage-only Markdown path {rel_path} does not have any readable plan revision bodies locally or from the published remote lineage."
        )
    return current_lines, previous_owners, skipped_revision_ids


def _compute_line_owners(
    conn,
    ctx: RepoContext,
    *,
    target_snapshot_id: str,
    rel_path: str,
) -> tuple[list[str], list[str]]:
    chain = local_content.collect_snapshot_chain(ctx, target_snapshot_id)
    if not chain:
        raise KeyError(f"Unknown snapshot: {target_snapshot_id}")
    file_rows = _snapshot_file_rows_for_path(conn, chain, rel_path)
    target_row = file_rows.get(target_snapshot_id)
    if target_row is None:
        raise KeyError(f"Path {rel_path} does not exist in snapshot {target_snapshot_id}.")
    blob_lines_cache: dict[str, list[str]] = {}
    target_lines = _blob_text_lines(
        conn,
        ctx,
        str(target_row["blob_id"]),
        cache=blob_lines_cache,
        label=f"Snapshot {target_snapshot_id}:{rel_path}",
    )

    previous_lines: list[str] = []
    previous_owners: list[str] = []
    previous_blob_id: str | None = None
    file_exists = False
    for snapshot_id in chain:
        row = file_rows.get(snapshot_id)
        if row is None:
            if file_exists:
                previous_lines = []
                previous_owners = []
                previous_blob_id = None
                file_exists = False
            continue
        blob_id = str(row["blob_id"])
        if not file_exists:
            next_lines = _blob_text_lines(
                conn,
                ctx,
                blob_id,
                cache=blob_lines_cache,
                label=f"Snapshot {snapshot_id}:{rel_path}",
            )
            previous_lines = next_lines
            previous_owners = [snapshot_id] * len(next_lines)
            previous_blob_id = blob_id
            file_exists = True
            continue
        if blob_id == previous_blob_id:
            continue
        next_lines = _blob_text_lines(
            conn,
            ctx,
            blob_id,
            cache=blob_lines_cache,
            label=f"Snapshot {snapshot_id}:{rel_path}",
        )
        previous_owners = _apply_line_diff(previous_lines, next_lines, previous_owners, snapshot_id)
        previous_lines = next_lines
        previous_blob_id = blob_id
    return target_lines, previous_owners


def _snapshot_overlay(
    ctx: RepoContext,
    *,
    snapshot_ids: list[str],
    target: dict[str, Any],
) -> dict[str, dict[str, Any]]:
    rows = local_control.list_workflow_snapshot_provenance(ctx, snapshot_ids=snapshot_ids)
    by_snapshot_id = {
        str(row["snapshot_id"]): dict(row)
        for row in rows
        if isinstance(row, dict) and str(row.get("snapshot_id") or "").strip()
    }
    landed_fallback = _local_landed_change_fallback(ctx, snapshot_ids=snapshot_ids)
    overlay: dict[str, dict[str, Any]] = {}
    patchset_revision_snapshot_id = _normalize_text(target.get("revision_snapshot_id"))
    target_kind = _normalize_text(target.get("kind")) or "current_line"
    for snapshot_id in snapshot_ids:
        direct = dict(by_snapshot_id.get(snapshot_id) or {})
        entry = {
            "task_id": _normalize_text(direct.get("task_id")),
            "change_id": _normalize_text(direct.get("change_id")),
            "patchset_id": None,
            "land_id": None,
            "submission_id": None,
            "session_id": _normalize_text(direct.get("session_id")),
            "checkpoint_id": _normalize_text(direct.get("checkpoint_id")),
            "author_mode": _normalize_text(direct.get("author_mode")),
            "model_name": _normalize_text(direct.get("model_name")),
            "worktree_name": _normalize_text(direct.get("worktree_name")),
            "provenance_confidence": PROVENANCE_CONFIDENCE_DIRECT if direct else PROVENANCE_CONFIDENCE_UNKNOWN,
        }
        landed_entry = landed_fallback.get(snapshot_id, {})
        _apply_overlay_defaults(entry, landed_entry)
        if entry["provenance_confidence"] == PROVENANCE_CONFIDENCE_UNKNOWN:
            landed_confidence = _normalize_text(landed_entry.get("provenance_confidence"))
            if landed_confidence is not None:
                entry["provenance_confidence"] = landed_confidence
        if patchset_revision_snapshot_id and snapshot_id == patchset_revision_snapshot_id:
            entry["patchset_id"] = _normalize_text(target.get("patchset_id"))
            if not entry.get("change_id"):
                entry["change_id"] = _normalize_text(target.get("change_id"))
            if not entry.get("task_id"):
                entry["task_id"] = _normalize_text(target.get("task_id"))
            if entry["provenance_confidence"] != PROVENANCE_CONFIDENCE_DIRECT:
                entry["provenance_confidence"] = PROVENANCE_CONFIDENCE_PATCHSET
        overlay[snapshot_id] = entry
    remote_overlay = {}
    if target_kind in {"patchset", "snapshot"}:
        remote_overlay = _remote_change_overlay(
            ctx,
            target=target,
            change_ids=[
                *(entry.get("change_id") for entry in overlay.values()),
                _normalize_text(target.get("change_id")),
            ],
        )
    for snapshot_id, remote_entry in remote_overlay.items():
        entry = overlay.get(snapshot_id)
        if entry is None:
            continue
        _apply_overlay_defaults(entry, remote_entry)
        if entry.get("provenance_confidence") == PROVENANCE_CONFIDENCE_UNKNOWN:
            confidence = _normalize_text(remote_entry.get("provenance_confidence"))
            if confidence is not None:
                entry["provenance_confidence"] = confidence
    return overlay


def _line_row_payload(
    *,
    rel_path: str,
    line_number: int,
    snapshot_row: dict[str, Any],
    overlay: dict[str, Any],
) -> dict[str, Any]:
    return {
        "path": rel_path,
        "start_line": line_number,
        "end_line": line_number,
        "snapshot_id": snapshot_row.get("snapshot_id"),
        "parent_snapshot_id": snapshot_row.get("parent_snapshot_id"),
        "line_name": snapshot_row.get("line_name"),
        "message": snapshot_row.get("message"),
        "created_at": snapshot_row.get("created_at"),
        "task_id": overlay.get("task_id"),
        "change_id": overlay.get("change_id"),
        "patchset_id": overlay.get("patchset_id"),
        "land_id": overlay.get("land_id"),
        "submission_id": overlay.get("submission_id"),
        "session_id": overlay.get("session_id"),
        "checkpoint_id": overlay.get("checkpoint_id"),
        "author_mode": overlay.get("author_mode"),
        "model_name": overlay.get("model_name"),
        "worktree_name": overlay.get("worktree_name"),
        "provenance_confidence": overlay.get("provenance_confidence") or PROVENANCE_CONFIDENCE_UNKNOWN,
    }


def _markdown_plan_row_map(plan: dict[str, Any], revisions: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    plan_id = str(plan.get("plan_id") or "")
    return {
        str(revision["plan_revision_id"]): {
            "plan_id": plan_id,
            "plan_revision_id": revision.get("plan_revision_id"),
            "parent_plan_revision_id": revision.get("parent_plan_revision_id"),
            "revision_number": revision.get("revision_number"),
            "title_snapshot": revision.get("title_snapshot"),
            "artifact_path": revision.get("artifact_path"),
            "artifact_selector": revision.get("artifact_selector"),
            "artifact_heading": revision.get("artifact_heading"),
            "created_at": revision.get("created_at"),
            "created_by": revision.get("created_by"),
            "actor_type": revision.get("actor_type"),
            "source_kind": revision.get("source_kind"),
            "source_session_id": revision.get("source_session_id"),
        }
        for revision in revisions
        if str(revision.get("plan_revision_id") or "").strip()
    }


def _markdown_line_row_payload(
    *,
    rel_path: str,
    line_number: int,
    plan_revision_row: dict[str, Any],
) -> dict[str, Any]:
    return {
        "path": rel_path,
        "start_line": line_number,
        "end_line": line_number,
        "plan_id": plan_revision_row.get("plan_id"),
        "plan_revision_id": plan_revision_row.get("plan_revision_id"),
        "parent_plan_revision_id": plan_revision_row.get("parent_plan_revision_id"),
        "revision_number": plan_revision_row.get("revision_number"),
        "title_snapshot": plan_revision_row.get("title_snapshot"),
        "artifact_path": plan_revision_row.get("artifact_path"),
        "artifact_selector": plan_revision_row.get("artifact_selector"),
        "artifact_heading": plan_revision_row.get("artifact_heading"),
        "created_at": plan_revision_row.get("created_at"),
        "created_by": plan_revision_row.get("created_by"),
        "actor_type": plan_revision_row.get("actor_type"),
        "source_kind": plan_revision_row.get("source_kind"),
        "source_session_id": plan_revision_row.get("source_session_id"),
        "provenance_confidence": PROVENANCE_CONFIDENCE_PLAN,
    }


def _collapse_line_rows(line_rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    hunks: list[dict[str, Any]] = []
    for row in line_rows:
        if not hunks:
            hunks.append(dict(row))
            continue
        previous = hunks[-1]
        previous_overlay = (
            previous.get("snapshot_id"),
            previous.get("plan_id"),
            previous.get("plan_revision_id"),
            previous.get("task_id"),
            previous.get("change_id"),
            previous.get("patchset_id"),
            previous.get("land_id"),
            previous.get("submission_id"),
            previous.get("session_id"),
            previous.get("checkpoint_id"),
            previous.get("author_mode"),
            previous.get("model_name"),
            previous.get("source_kind"),
            previous.get("provenance_confidence"),
        )
        current_overlay = (
            row.get("snapshot_id"),
            row.get("plan_id"),
            row.get("plan_revision_id"),
            row.get("task_id"),
            row.get("change_id"),
            row.get("patchset_id"),
            row.get("land_id"),
            row.get("submission_id"),
            row.get("session_id"),
            row.get("checkpoint_id"),
            row.get("author_mode"),
            row.get("model_name"),
            row.get("source_kind"),
            row.get("provenance_confidence"),
        )
        if previous_overlay == current_overlay and int(previous.get("end_line") or 0) + 1 == int(row.get("start_line") or 0):
            previous["end_line"] = row["end_line"]
            continue
        hunks.append(dict(row))
    return hunks


def compute_markdown_plan_blame(
    ctx: RepoContext,
    path_value: str | Path,
    *,
    line: int | None = None,
    start_line: int | None = None,
    end_line: int | None = None,
    plan_id: str | None = None,
    plan_ref: str | None = None,
) -> dict[str, Any]:
    rel_path = normalize_blame_path(ctx, path_value)
    if not _is_lineage_only_markdown_artifact_path(rel_path):
        raise ValueError(f"Path {rel_path} does not use lineage-only Markdown plan history.")
    plan = _current_plan_for_artifact_path(ctx, rel_path, plan_id=plan_id, plan_ref=plan_ref)
    revisions_desc = list_local_plan_revisions(ctx, str(plan["plan_id"]))
    revisions = sorted(revisions_desc, key=lambda row: int(row.get("revision_number") or 0))
    head_revision = plan.get("head_revision") if isinstance(plan.get("head_revision"), dict) else {}
    head_revision_id = str(head_revision.get("plan_revision_id") or "").strip()
    if not head_revision_id:
        raise ValueError(f"Current plan {plan['plan_id']} is missing a head revision for lineage-only Markdown path {rel_path}.")

    target_lines, owners, skipped_revision_ids = _compute_markdown_plan_line_owners(
        ctx,
        plan=plan,
        rel_path=rel_path,
        revisions=revisions,
        current_head_revision_id=head_revision_id,
    )
    selected_start, selected_end = _line_selection(
        total_lines=len(target_lines),
        line=line,
        start_line=start_line,
        end_line=end_line,
    )
    selected_owner_ids = owners[selected_start - 1:selected_end] if selected_start else []
    unique_owner_ids = list(dict.fromkeys(selected_owner_ids))
    revision_rows = _markdown_plan_row_map(plan, revisions)

    line_rows: list[dict[str, Any]] = []
    if selected_start:
        for index in range(selected_start - 1, selected_end):
            plan_revision_id = owners[index]
            revision_row = revision_rows.get(plan_revision_id)
            if revision_row is None:
                raise KeyError(f"Unknown plan revision: {plan_revision_id}")
            line_rows.append(
                _markdown_line_row_payload(
                    rel_path=rel_path,
                    line_number=index + 1,
                    plan_revision_row=revision_row,
                )
            )
    return {
        "target": {
            "kind": "markdown_plan",
            "plan_id": plan.get("plan_id"),
            "plan_ref": _plan_head_artifact_field(plan, "artifact_selector"),
            "artifact_path": rel_path,
            "resolved_plan_revision_id": head_revision_id,
        },
        "path": rel_path,
        "resolved_plan_revision_id": head_revision_id,
        "range": {
            "start": selected_start,
            "end": selected_end,
        },
        "warnings": [
            {
                "kind": "missing_markdown_revision_body",
                "plan_revision_id": revision_id,
                "message": (
                    f"Skipped unreadable historical plan revision {revision_id} while attributing lineage-only Markdown path {rel_path}."
                ),
            }
            for revision_id in skipped_revision_ids
        ],
        "hunks": _collapse_line_rows(line_rows),
        "lines": line_rows,
        "_internal": {
            "target_file_lines": target_lines,
            "selected_owner_plan_revision_ids": unique_owner_ids,
            "selected_owner_count": len(unique_owner_ids),
            "skipped_plan_revision_ids": skipped_revision_ids,
        },
    }


def compute_snapshot_blame(
    ctx: RepoContext,
    path_value: str | Path,
    *,
    target: dict[str, Any],
    line: int | None = None,
    start_line: int | None = None,
    end_line: int | None = None,
) -> dict[str, Any]:
    rel_path = normalize_blame_path(ctx, path_value)
    target_snapshot_id = str(target.get("resolved_snapshot_id") or "").strip()
    if not target_snapshot_id:
        raise ValueError("Resolved snapshot id is required.")

    conn = connect_sqlite(ctx.content_db_path)
    try:
        target_lines, owners = _compute_line_owners(
            conn,
            ctx,
            target_snapshot_id=target_snapshot_id,
            rel_path=rel_path,
        )
        selected_start, selected_end = _line_selection(
            total_lines=len(target_lines),
            line=line,
            start_line=start_line,
            end_line=end_line,
        )
        selected_owner_ids = owners[selected_start - 1:selected_end] if selected_start else []
        unique_owner_ids = list(dict.fromkeys(selected_owner_ids))
        snapshot_rows = _snapshot_row_map(conn, unique_owner_ids + [target_snapshot_id])
    finally:
        conn.close()

    overlay = _snapshot_overlay(ctx, snapshot_ids=unique_owner_ids, target=target)
    line_rows: list[dict[str, Any]] = []
    if selected_start:
        for index in range(selected_start - 1, selected_end):
            snapshot_id = owners[index]
            snapshot_row = snapshot_rows.get(snapshot_id)
            if snapshot_row is None:
                raise KeyError(f"Unknown snapshot: {snapshot_id}")
            line_rows.append(
                _line_row_payload(
                    rel_path=rel_path,
                    line_number=index + 1,
                    snapshot_row=snapshot_row,
                    overlay=overlay.get(snapshot_id, {}),
                )
            )
    target_snapshot_row = snapshot_rows.get(target_snapshot_id)
    if target_snapshot_row is None:
        raise KeyError(f"Unknown snapshot: {target_snapshot_id}")
    return {
        "target": dict(target),
        "path": rel_path,
        "resolved_snapshot_id": target_snapshot_id,
        "line_name": target_snapshot_row.get("line_name"),
        "range": {
            "start": selected_start,
            "end": selected_end,
        },
        "hunks": _collapse_line_rows(line_rows),
        "lines": line_rows,
        "_internal": {
            "target_file_lines": target_lines,
            "selected_owner_snapshot_ids": unique_owner_ids,
            "selected_owner_count": len(unique_owner_ids),
        },
    }


def _restore_preview_payload(
    *,
    rel_path: str,
    blame: dict[str, Any],
    current_lines: list[str],
) -> dict[str, Any]:
    selected_range = blame.get("range") if isinstance(blame.get("range"), dict) else {}
    start_line = int(selected_range.get("start") or 0)
    end_line = int(selected_range.get("end") or 0)
    if start_line <= 0 or end_line <= 0:
        raise ValueError("Scoped restore requires one selected line or range.")
    internal = blame.get("_internal") if isinstance(blame.get("_internal"), dict) else {}
    target_lines = list(internal.get("target_file_lines") or [])
    if end_line > len(target_lines):
        raise ValueError(f"Selected range {start_line}-{end_line} exceeds target file length {len(target_lines)}.")
    if end_line > len(current_lines):
        raise ValueError(
            f"Workspace file {rel_path} has only {len(current_lines)} lines, so range {start_line}-{end_line} cannot be restored safely."
        )
    owner_snapshot_ids = list(internal.get("selected_owner_snapshot_ids") or [])
    if len(owner_snapshot_ids) != 1:
        owner_list = ", ".join(owner_snapshot_ids) or "none"
        raise ValueError(
            f"Selected range {start_line}-{end_line} spans multiple owning snapshots ({owner_list}). Narrow the selection before using --restore."
        )
    target_selection = target_lines[start_line - 1:end_line]
    current_selection = current_lines[start_line - 1:end_line]
    return {
        "path": rel_path,
        "selected_range": {"start": start_line, "end": end_line},
        "restore_mode": "scoped_lines_only",
        "source_snapshot_id": owner_snapshot_ids[0],
        "resolved_snapshot_id": str(blame.get("resolved_snapshot_id") or ""),
        "unchanged_outside_selected_range": True,
        "would_overwrite_selected_local_edits": current_selection != target_selection,
        "applied": False,
        "_internal": {
            "restored_lines": current_lines[: start_line - 1] + target_selection + current_lines[end_line:],
        },
    }


def preview_scoped_restore(
    ctx: RepoContext,
    path_value: str | Path,
    *,
    target: dict[str, Any],
    line: int | None = None,
    start_line: int | None = None,
    end_line: int | None = None,
) -> dict[str, Any]:
    blame = compute_snapshot_blame(
        ctx,
        path_value,
        target=target,
        line=line,
        start_line=start_line,
        end_line=end_line,
    )
    rel_path = str(blame.get("path") or "")
    current_lines = _current_workspace_lines(ctx, rel_path)
    return _restore_preview_payload(rel_path=rel_path, blame=blame, current_lines=current_lines)


def apply_scoped_restore(
    ctx: RepoContext,
    path_value: str | Path,
    *,
    target: dict[str, Any],
    line: int | None = None,
    start_line: int | None = None,
    end_line: int | None = None,
) -> dict[str, Any]:
    blame = compute_snapshot_blame(
        ctx,
        path_value,
        target=target,
        line=line,
        start_line=start_line,
        end_line=end_line,
    )
    rel_path = str(blame.get("path") or "")
    current_lines = _current_workspace_lines(ctx, rel_path)
    preview = _restore_preview_payload(rel_path=rel_path, blame=blame, current_lines=current_lines)
    restored_lines = list((preview.get("_internal") or {}).get("restored_lines") or [])
    abs_path = ctx.root / rel_path
    abs_path.write_text("".join(restored_lines), encoding="utf-8")
    preview["applied"] = True
    return preview


def public_blame_payload(payload: dict[str, Any]) -> dict[str, Any]:
    public = dict(payload)
    public.pop("_internal", None)
    return public
