from __future__ import annotations

from typing import Any

from . import local_content, local_content_pack_runtime, local_control
from .repo_paths import RepoContext
from .store_repo_config import load_config


def content_storage_stats(ctx: RepoContext) -> dict:
    return local_content_pack_runtime.storage_stats(ctx)



def pack_content(ctx: RepoContext, *, max_members: int | None = None, repack: bool = False) -> dict:
    row = local_content_pack_runtime.create_pack(ctx, max_members=max_members, repack=repack)
    if row.get("created"):
        local_control.record_event(
            ctx,
            "content.pack.created",
            "pack",
            row["pack_id"],
            {
                "pack_id": row["pack_id"],
                "member_count": row["member_count"],
                "total_bytes": row["total_bytes"],
                "repack": repack,
            },
        )
    return row



def gc_content(ctx: RepoContext, *, prune_unreferenced: bool = True, prune_orphan_packs: bool = True) -> dict:
    row = local_content_pack_runtime.gc_content(
        ctx,
        prune_unreferenced=prune_unreferenced,
        prune_orphan_packs=prune_orphan_packs,
    )
    local_control.record_event(
        ctx,
        "content.gc.completed",
        "repository",
        load_config(ctx).get("repo_name") or ctx.root.name,
        {
            "removed_unreferenced_blob_count": row["removed_unreferenced_blob_count"],
            "removed_unreachable_tree_count": row["removed_unreachable_tree_count"],
            "removed_unreachable_tree_entry_count": row["removed_unreachable_tree_entry_count"],
            "removed_orphan_pack_count": row["removed_orphan_pack_count"],
        },
    )
    return row



def optimize_content(ctx: RepoContext) -> dict[str, Any]:
    initial_storage = content_storage_stats(ctx)
    current_storage = initial_storage
    steps: list[dict[str, Any]] = []

    if current_storage["pack_count"] > 1:
        repack_result = pack_content(ctx, repack=True)
        current_storage = repack_result["stats"]
        steps.append(
            {
                "action": "repack",
                "reason": "multi_pack_layout",
                "result": {
                    "created": repack_result.get("created", False),
                    "pack_id": repack_result.get("pack_id"),
                    "member_count": repack_result.get("member_count", 0),
                },
            }
        )

    if (
        current_storage["pack_count"] > 1
        or current_storage["unreachable_blob_count"] > 0
        or current_storage.get("unreachable_tree_count", 0) > 0
    ):
        gc_result = gc_content(ctx)
        current_storage = gc_result["stats"]
        steps.append(
            {
                "action": "gc",
                "reason": "pack_cleanup_or_unreferenced_storage_metadata",
                "result": {
                    "removed_unreferenced_blob_count": gc_result.get("removed_unreferenced_blob_count", 0),
                    "removed_unreachable_tree_count": gc_result.get("removed_unreachable_tree_count", 0),
                    "removed_unreachable_tree_entry_count": gc_result.get("removed_unreachable_tree_entry_count", 0),
                    "removed_orphan_pack_count": gc_result.get("removed_orphan_pack_count", 0),
                },
            }
        )

    repo_name = load_config(ctx).get("repo_name") or ctx.root.name
    local_control.record_event(
        ctx,
        "content.optimize.completed",
        "repository",
        repo_name,
        {
            "repo_name": repo_name,
            "step_count": len(steps),
            "initial_pack_count": initial_storage["pack_count"],
            "final_pack_count": current_storage["pack_count"],
            "initial_unreachable_tree_count": initial_storage.get("unreachable_tree_count", 0),
            "final_unreachable_tree_count": current_storage.get("unreachable_tree_count", 0),
            "final_packed_delta_blob_count": current_storage["packed_delta_blob_count"],
        },
    )
    return {
        "repo_name": repo_name,
        "initial_storage": initial_storage,
        "final_storage": current_storage,
        "steps": steps,
        "executed_step_count": len(steps),
    }



def export_snapshot_bundle(ctx: RepoContext, snapshot_id: str) -> dict:
    repo_name = local_control.get_meta(ctx, "repo_name") or ctx.root.name
    return local_content.export_snapshot_bundle(ctx, snapshot_id, repo_name)



def import_snapshot_bundle(ctx: RepoContext, bundle: dict) -> dict:
    row = local_content.import_snapshot_bundle(ctx, bundle)
    local_control.record_event(
        ctx,
        "snapshot.imported",
        "snapshot",
        row["snapshot_id"],
        {"snapshot_id": row["snapshot_id"], "parent_snapshot_id": row["parent_snapshot_id"]},
    )
    return row


__all__ = [
    "content_storage_stats",
    "export_snapshot_bundle",
    "gc_content",
    "import_snapshot_bundle",
    "optimize_content",
    "pack_content",
]
