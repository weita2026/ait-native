from __future__ import annotations

import json

from . import local_control
from .repo_paths import RepoContext


def _local_plan_revision_view(row: dict | None) -> dict | None:
    if row is None:
        return None
    out = dict(row)
    try:
        out["items"] = json.loads(out.pop("items_json", "[]") or "[]")
    except Exception:
        out["items"] = []
    return out


def _local_plan_view(ctx: RepoContext, row: dict, *, include_head_revision: bool = True) -> dict:
    out = dict(row)
    if include_head_revision:
        head_revision_id = out.get("head_revision_id")
        out["head_revision"] = (
            _local_plan_revision_view(local_control.get_workflow_plan_revision(ctx, out["plan_id"], head_revision_id))
            if head_revision_id
            else None
        )
    return out


def _local_release_view(row: dict | None) -> dict | None:
    if row is None:
        return None
    out = dict(row)
    out["line"] = out.pop("line_name")
    package = {
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
    metadata = out.get("metadata") if isinstance(out.get("metadata"), dict) else {}
    metadata_package = metadata.get("package") if isinstance(metadata.get("package"), dict) else {}
    if metadata_package:
        package.update({key: value for key, value in metadata_package.items() if value is not None})
    out["package"] = package
    return out


def _local_change_view(row: dict) -> dict:
    out = dict(row)
    out["current_patchset_number"] = 0
    out["selected_patchset_number"] = None
    out["current_patchset_id"] = None
    out["selected_patchset_id"] = None
    out["stack_ids"] = []
    out["landed_at"] = out.get("landed_at")
    out["landed_snapshot_id"] = out.get("landed_snapshot_id")
    out["target_line"] = out.get("target_line")
    return out
