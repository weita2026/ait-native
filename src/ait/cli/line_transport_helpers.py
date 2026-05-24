from __future__ import annotations

import importlib
from typing import Optional

from ait_protocol.common import StorageIngestMode

from ..remote_client import (
    RemoteError,
    get_remote_line as _default_get_remote_line,
    get_remote_snapshot as _default_get_remote_snapshot,
    get_remote_snapshots_existence as _default_get_remote_snapshots_existence,
    put_remote_snapshot as _default_put_remote_snapshot,
    update_remote_line as _default_update_remote_line,
)
from ..repo_paths import RepoContext
from ..store import (
    collect_snapshot_chain,
    export_snapshot_bundle,
    get_line,
    import_snapshot_bundle,
    load_config,
    set_line_head,
    snapshot_exists,
)
from .remote_repository_defaults import _remote_tuple, _sync_remote_repository_defaults, _verify_remote_repository
from .workflow_mode_config import _normalize_text_value


def _cli_app_override(name: str, default):
    cli_app = importlib.import_module("ait.cli.app")

    return getattr(cli_app, name, default)


def _verify_remote_line(remote_line: dict, repo_name: str, line_name: str, expected_head_snapshot_id: Optional[str]) -> None:
    if remote_line.get("repo_name") != repo_name:
        raise RemoteError(
            f"Remote line verification returned unexpected repository {remote_line.get('repo_name')!r} "
            f"(expected {repo_name!r})"
        )
    if remote_line.get("line_name") != line_name:
        raise RemoteError(
            f"Remote line verification returned unexpected line {remote_line.get('line_name')!r} "
            f"(expected {line_name!r})"
        )
    if remote_line.get("head_snapshot_id") != expected_head_snapshot_id:
        raise RemoteError(
            f"Remote line {line_name} head mismatch after push: "
            f"expected {expected_head_snapshot_id!r}, got {remote_line.get('head_snapshot_id')!r}"
        )


def _verify_remote_pull_line(remote_line: dict, repo_name: str, line_name: str) -> None:
    if remote_line.get("repo_name") != repo_name:
        raise RemoteError(
            f"Remote pull returned unexpected repository {remote_line.get('repo_name')!r} "
            f"(expected {repo_name!r})"
        )
    if remote_line.get("line_name") != line_name:
        raise RemoteError(
            f"Remote pull returned unexpected line {remote_line.get('line_name')!r} "
            f"(expected {line_name!r})"
        )


def _current_remote_line_head_snapshot_id(base_url: str, repo_name: str, line_name: str) -> str | None:
    get_remote_line = _cli_app_override("get_remote_line", _default_get_remote_line)
    try:
        remote_line = get_remote_line(base_url, repo_name, line_name)
    except RemoteError as exc:
        message = str(exc)
        if "failed: 404" in message or f"Unknown line {line_name}" in message:
            return None
        raise
    _verify_remote_pull_line(remote_line, repo_name, line_name)
    return _normalize_text_value(remote_line.get("head_snapshot_id"))


def _verify_remote_pushed_snapshot(remote_snapshot: dict, repo_name: str, snapshot_id: str, bundle: dict) -> None:
    if remote_snapshot.get("snapshot_id") != snapshot_id:
        raise RemoteError(
            f"Remote snapshot verification returned unexpected snapshot {remote_snapshot.get('snapshot_id')!r} "
            f"(expected {snapshot_id!r})"
        )
    if remote_snapshot.get("repo_name") != repo_name:
        raise RemoteError(
            f"Remote snapshot verification returned unexpected repository {remote_snapshot.get('repo_name')!r} "
            f"(expected {repo_name!r})"
        )
    expected_line_name = bundle.get("line_name") or "main"
    if remote_snapshot.get("line_name") != expected_line_name:
        raise RemoteError(
            f"Remote snapshot {snapshot_id} line mismatch after push: "
            f"expected {expected_line_name!r}, got {remote_snapshot.get('line_name')!r}"
        )
    expected_parent_snapshot_id = bundle.get("parent_snapshot_id")
    if remote_snapshot.get("parent_snapshot_id") != expected_parent_snapshot_id:
        raise RemoteError(
            f"Remote snapshot {snapshot_id} parent mismatch after push: "
            f"expected {expected_parent_snapshot_id!r}, got {remote_snapshot.get('parent_snapshot_id')!r}"
        )
    expected_message = bundle.get("message")
    if remote_snapshot.get("message") != expected_message:
        raise RemoteError(
            f"Remote snapshot {snapshot_id} message mismatch after push: "
            f"expected {expected_message!r}, got {remote_snapshot.get('message')!r}"
        )
    expected_file_count = bundle.get("file_count") or len(bundle.get("files", []))
    if remote_snapshot.get("file_count") != expected_file_count:
        raise RemoteError(
            f"Remote snapshot {snapshot_id} file_count mismatch after push: "
            f"expected {expected_file_count!r}, got {remote_snapshot.get('file_count')!r}"
        )
    expected_total_bytes = bundle.get("total_bytes")
    if expected_total_bytes is None:
        expected_total_bytes = sum(int(file_entry.get("size_bytes") or 0) for file_entry in bundle.get("files", []))
    if remote_snapshot.get("total_bytes") != expected_total_bytes:
        raise RemoteError(
            f"Remote snapshot {snapshot_id} total_bytes mismatch after push: "
            f"expected {expected_total_bytes!r}, got {remote_snapshot.get('total_bytes')!r}"
        )


def _verify_remote_snapshot_bundle(bundle: dict, repo_name: str, snapshot_id: str) -> None:
    if bundle.get("snapshot_id") != snapshot_id:
        raise RemoteError(
            f"Remote snapshot fetch returned unexpected snapshot {bundle.get('snapshot_id')!r} "
            f"(expected {snapshot_id!r})"
        )
    bundle_repo_name = bundle.get("repo_name")
    if bundle_repo_name and bundle_repo_name != repo_name:
        raise RemoteError(
            f"Remote snapshot fetch returned unexpected repository {bundle_repo_name!r} "
            f"(expected {repo_name!r})"
        )


def _remote_snapshot_exists(base_url: str, repo_name: str, snapshot_id: str) -> bool:
    get_remote_snapshot = _cli_app_override("get_remote_snapshot", _default_get_remote_snapshot)
    try:
        bundle = get_remote_snapshot(base_url, repo_name, snapshot_id, include_content=False)
    except RemoteError as exc:
        message = str(exc)
        if "failed: 404" in message or "Unknown snapshot" in message:
            return False
        raise
    _verify_remote_snapshot_bundle(bundle, repo_name, snapshot_id)
    return True


def _remote_snapshot_batch_check_unavailable(exc: RemoteError) -> bool:
    message = str(exc)
    return "failed: 404" in message or "failed: 405" in message


def _remote_existing_snapshot_ids(base_url: str, repo_name: str, snapshot_ids: list[str]) -> set[str] | None:
    if not snapshot_ids:
        return set()
    get_remote_snapshots_existence = _cli_app_override(
        "get_remote_snapshots_existence",
        _default_get_remote_snapshots_existence,
    )
    try:
        payload = get_remote_snapshots_existence(base_url, repo_name, snapshot_ids)
    except RemoteError as exc:
        if _remote_snapshot_batch_check_unavailable(exc):
            return None
        raise
    payload_repo_name = payload.get("repo_name")
    if payload_repo_name and payload_repo_name != repo_name:
        raise RemoteError(
            f"Remote snapshot existence check returned unexpected repository {payload_repo_name!r} "
            f"(expected {repo_name!r})"
        )
    requested = set(snapshot_ids)
    present = set(payload.get("present") or [])
    missing = set(payload.get("missing") or [])
    unexpected = (present | missing) - requested
    if unexpected:
        raise RemoteError(
            "Remote snapshot existence check returned unexpected snapshot ids: "
            + ", ".join(sorted(unexpected))
        )
    overlap = present & missing
    if overlap:
        raise RemoteError(
            "Remote snapshot existence check returned snapshots as both present and missing: "
            + ", ".join(sorted(overlap))
        )
    unaccounted = requested - present - missing
    if unaccounted:
        raise RemoteError(
            "Remote snapshot existence check did not account for snapshot ids: "
            + ", ".join(sorted(unaccounted))
        )
    return present


def _local_line_state(ctx: RepoContext, line_name: str) -> tuple[bool, str | None]:
    try:
        line_row = get_line(ctx, line_name)
    except KeyError:
        return False, None
    return True, _normalize_text_value(line_row.get("head_snapshot_id"))


def _import_remote_snapshot_chain(
    ctx: RepoContext,
    base_url: str,
    repo_name: str,
    head_snapshot_id: str | None,
    *,
    initial_bundle: dict | None = None,
) -> tuple[int, list[str]]:
    get_remote_snapshot = _cli_app_override("get_remote_snapshot", _default_get_remote_snapshot)
    bundles: list[dict] = []
    cur = head_snapshot_id
    next_bundle = initial_bundle
    while cur and not snapshot_exists(ctx, cur):
        bundle = next_bundle if next_bundle is not None else get_remote_snapshot(base_url, repo_name, cur)
        _verify_remote_snapshot_bundle(bundle, repo_name, cur)
        bundles.append(bundle)
        cur = _normalize_text_value(bundle.get("parent_snapshot_id"))
        next_bundle = None
    imported_snapshot_ids: list[str] = []
    for bundle in reversed(bundles):
        import_snapshot_bundle(ctx, bundle)
        imported_snapshot_ids.append(str(bundle["snapshot_id"]))
    return len(imported_snapshot_ids), imported_snapshot_ids


def _push_line(
    ctx: RepoContext,
    remote_name: Optional[str],
    line_name: str,
    *,
    server_storage_mode: StorageIngestMode = StorageIngestMode.DEFAULT,
) -> dict:
    put_remote_snapshot = _cli_app_override("put_remote_snapshot", _default_put_remote_snapshot)
    update_remote_line = _cli_app_override("update_remote_line", _default_update_remote_line)
    remote, repo_name = _sync_remote_repository_defaults(ctx, remote_name)
    cfg = load_config(ctx)
    remote_repo = _verify_remote_repository(remote["url"], repo_name, cfg["default_line"])
    line_row = get_line(ctx, line_name)
    head_snapshot_id = line_row["head_snapshot_id"]
    expected_remote_head_snapshot_id = _current_remote_line_head_snapshot_id(remote["url"], repo_name, line_name)
    if not head_snapshot_id:
        remote_line = update_remote_line(
            remote["url"],
            repo_name,
            line_name,
            None,
            expected_head_snapshot_id=expected_remote_head_snapshot_id,
        )
        _verify_remote_line(remote_line, repo_name, line_name, None)
        return {
            "remote": remote["name"],
            "repo_name": repo_name,
            "line": line_name,
            "pushed_snapshots": 0,
            "checked_snapshots": 0,
            "uploaded_snapshots": 0,
            "skipped_snapshots": 0,
            "head_snapshot_id": None,
            "server_storage_mode": server_storage_mode.value,
            "remote_repository": remote_repo,
            "remote_line": remote_line,
        }
    chain = collect_snapshot_chain(ctx, head_snapshot_id)
    checked = 0
    uploaded = 0
    skipped = 0
    existing_snapshot_ids = _remote_existing_snapshot_ids(remote["url"], repo_name, chain)
    if existing_snapshot_ids is not None:
        checked = len(chain)
        for snapshot_id in chain:
            if snapshot_id in existing_snapshot_ids:
                skipped += 1
                continue
            bundle = export_snapshot_bundle(ctx, snapshot_id)
            ingest_mode = None if server_storage_mode == StorageIngestMode.DEFAULT else server_storage_mode.value
            remote_snapshot = put_remote_snapshot(remote["url"], repo_name, snapshot_id, bundle, storage_ingest_mode=ingest_mode)
            _verify_remote_pushed_snapshot(remote_snapshot, repo_name, snapshot_id, bundle)
            uploaded += 1
    else:
        for snapshot_id in chain:
            checked += 1
            if _remote_snapshot_exists(remote["url"], repo_name, snapshot_id):
                skipped += 1
                continue
            bundle = export_snapshot_bundle(ctx, snapshot_id)
            ingest_mode = None if server_storage_mode == StorageIngestMode.DEFAULT else server_storage_mode.value
            remote_snapshot = put_remote_snapshot(remote["url"], repo_name, snapshot_id, bundle, storage_ingest_mode=ingest_mode)
            _verify_remote_pushed_snapshot(remote_snapshot, repo_name, snapshot_id, bundle)
            uploaded += 1
    remote_line = update_remote_line(
        remote["url"],
        repo_name,
        line_name,
        head_snapshot_id,
        expected_head_snapshot_id=expected_remote_head_snapshot_id,
    )
    _verify_remote_line(remote_line, repo_name, line_name, head_snapshot_id)
    return {
        "remote": remote["name"],
        "repo_name": repo_name,
        "line": line_name,
        "pushed_snapshots": uploaded,
        "checked_snapshots": checked,
        "uploaded_snapshots": uploaded,
        "skipped_snapshots": skipped,
        "head_snapshot_id": head_snapshot_id,
        "server_storage_mode": server_storage_mode.value,
        "remote_repository": remote_repo,
        "remote_line": remote_line,
    }


def _upload_snapshot_chain(
    ctx: RepoContext,
    remote_name: Optional[str],
    snapshot_id: str,
    *,
    line_name: str | None = None,
    server_storage_mode: StorageIngestMode = StorageIngestMode.DEFAULT,
    reason: str | None = None,
) -> dict:
    put_remote_snapshot = _cli_app_override("put_remote_snapshot", _default_put_remote_snapshot)
    remote, repo_name = _sync_remote_repository_defaults(ctx, remote_name)
    cfg = load_config(ctx)
    remote_repo = _verify_remote_repository(remote["url"], repo_name, cfg["default_line"])
    chain = collect_snapshot_chain(ctx, snapshot_id)
    checked = 0
    uploaded = 0
    skipped = 0
    existing_snapshot_ids = _remote_existing_snapshot_ids(remote["url"], repo_name, chain)
    if existing_snapshot_ids is not None:
        checked = len(chain)
        for chain_snapshot_id in chain:
            if chain_snapshot_id in existing_snapshot_ids:
                skipped += 1
                continue
            bundle = export_snapshot_bundle(ctx, chain_snapshot_id)
            ingest_mode = None if server_storage_mode == StorageIngestMode.DEFAULT else server_storage_mode.value
            remote_snapshot = put_remote_snapshot(
                remote["url"],
                repo_name,
                chain_snapshot_id,
                bundle,
                storage_ingest_mode=ingest_mode,
            )
            _verify_remote_pushed_snapshot(remote_snapshot, repo_name, chain_snapshot_id, bundle)
            uploaded += 1
    else:
        for chain_snapshot_id in chain:
            checked += 1
            if _remote_snapshot_exists(remote["url"], repo_name, chain_snapshot_id):
                skipped += 1
                continue
            bundle = export_snapshot_bundle(ctx, chain_snapshot_id)
            ingest_mode = None if server_storage_mode == StorageIngestMode.DEFAULT else server_storage_mode.value
            remote_snapshot = put_remote_snapshot(
                remote["url"],
                repo_name,
                chain_snapshot_id,
                bundle,
                storage_ingest_mode=ingest_mode,
            )
            _verify_remote_pushed_snapshot(remote_snapshot, repo_name, chain_snapshot_id, bundle)
            uploaded += 1
    return {
        "remote": remote["name"],
        "repo_name": repo_name,
        "line": line_name,
        "line_updated": False,
        "line_update_skipped_reason": reason,
        "pushed_snapshots": uploaded,
        "checked_snapshots": checked,
        "uploaded_snapshots": uploaded,
        "skipped_snapshots": skipped,
        "head_snapshot_id": snapshot_id,
        "server_storage_mode": server_storage_mode.value,
        "remote_repository": remote_repo,
        "remote_line": None,
    }


def _fetch_line(ctx: RepoContext, remote_name: Optional[str], line_name: str) -> dict:
    get_remote_line = _cli_app_override("get_remote_line", _default_get_remote_line)
    remote, repo_name = _remote_tuple(ctx, remote_name)
    line_info = get_remote_line(remote["url"], repo_name, line_name)
    _verify_remote_pull_line(line_info, repo_name, line_name)
    local_line_present, local_line_head_snapshot_id = _local_line_state(ctx, line_name)
    head_snapshot_id = _normalize_text_value(line_info.get("head_snapshot_id"))
    imported, imported_snapshot_ids = _import_remote_snapshot_chain(ctx, remote["url"], repo_name, head_snapshot_id)
    return {
        "remote": remote["name"],
        "repo_name": repo_name,
        "mode": "line",
        "line": line_name,
        "remote_line": line_info,
        "local_line_present": local_line_present,
        "local_line_head_snapshot_id": local_line_head_snapshot_id,
        "imported_snapshots": imported,
        "imported_snapshot_ids": imported_snapshot_ids,
        "head_snapshot_id": head_snapshot_id,
        "line_head_updated": False,
        "workspace_restored": False,
    }


def _fetch_snapshot(ctx: RepoContext, remote_name: Optional[str], snapshot_id: str) -> dict:
    get_remote_snapshot = _cli_app_override("get_remote_snapshot", _default_get_remote_snapshot)
    remote, repo_name = _remote_tuple(ctx, remote_name)
    needs_content = not snapshot_exists(ctx, snapshot_id)
    remote_snapshot = get_remote_snapshot(remote["url"], repo_name, snapshot_id, include_content=needs_content)
    _verify_remote_snapshot_bundle(remote_snapshot, repo_name, snapshot_id)
    imported, imported_snapshot_ids = _import_remote_snapshot_chain(
        ctx,
        remote["url"],
        repo_name,
        snapshot_id,
        initial_bundle=remote_snapshot if needs_content else None,
    )
    return {
        "remote": remote["name"],
        "repo_name": repo_name,
        "mode": "snapshot",
        "snapshot_id": snapshot_id,
        "remote_snapshot": remote_snapshot,
        "imported_snapshots": imported,
        "imported_snapshot_ids": imported_snapshot_ids,
        "head_snapshot_id": snapshot_id,
        "line_head_updated": False,
        "workspace_restored": False,
    }


def _pull_line(ctx: RepoContext, remote_name: Optional[str], line_name: str) -> dict:
    get_remote_line = _cli_app_override("get_remote_line", _default_get_remote_line)
    remote, repo_name = _remote_tuple(ctx, remote_name)
    line_info = get_remote_line(remote["url"], repo_name, line_name)
    _verify_remote_pull_line(line_info, repo_name, line_name)
    head_snapshot_id = _normalize_text_value(line_info.get("head_snapshot_id"))
    imported, _imported_snapshot_ids = _import_remote_snapshot_chain(ctx, remote["url"], repo_name, head_snapshot_id)
    set_line_head(ctx, line_name, head_snapshot_id)
    return {
        "remote": remote["name"],
        "repo_name": repo_name,
        "line": line_name,
        "imported_snapshots": imported,
        "head_snapshot_id": head_snapshot_id,
    }
