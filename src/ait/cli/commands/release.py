from __future__ import annotations

import base64
from pathlib import Path
from typing import Any, Optional

from ...repo_paths import RepoContext
from ...release_ops import (
    build_release_candidate,
    create_release_candidate,
    generate_release_formula,
    get_release_candidate,
    run_release_checks,
)
from ...store import update_local_release
from ..workflow_identity_helpers import _require_remote_identity
from ..shared import export_app_namespace

export_app_namespace(globals())

candidate_app = typer.Typer(help="Create local release candidate records")
release_app.add_typer(candidate_app, name="candidate")


def _artifact_display_path(artifact: dict[str, Any]) -> str:
    return str(
        artifact.get("download_url")
        or artifact.get("download_path")
        or artifact.get("path")
        or artifact.get("url")
        or ""
    )


def _formula_display_path(formula: dict[str, Any]) -> str:
    return str(formula.get("download_url") or formula.get("download_path") or formula.get("path") or "")


def _relative_release_artifact_path(ctx: RepoContext, path: Path) -> str:
    try:
        return path.relative_to(ctx.root).as_posix()
    except ValueError:
        if path.parent.name:
            return Path(path.parent.name, path.name).as_posix()
        return path.name


def _release_publish_artifacts(ctx: RepoContext, record: dict[str, Any]) -> list[dict[str, Any]]:
    uploads: list[dict[str, Any]] = []
    for artifact in record.get("artifacts", []):
        if not isinstance(artifact, dict):
            continue
        kind = str(artifact.get("kind") or "").strip()
        source_path = Path(str(artifact.get("absolute_path") or artifact.get("path") or ""))
        if not kind:
            continue
        if not source_path.exists():
            raise ValueError(f"Release artifact {kind!r} is missing on disk: {source_path}")
        uploads.append(
            {
                "kind": kind,
                "path": _relative_release_artifact_path(ctx, source_path),
                "sha256": str(artifact.get("sha256") or ""),
                "size_bytes": int(artifact.get("size_bytes") or source_path.stat().st_size),
                "content_b64": base64.b64encode(source_path.read_bytes()).decode("ascii"),
            }
        )
    return uploads


def _release_publish_metadata(record: dict[str, Any]) -> dict[str, Any]:
    metadata = record.get("metadata") if isinstance(record.get("metadata"), dict) else {}
    safe: dict[str, Any] = {}
    if metadata.get("source_snapshot_created_at"):
        safe["source_snapshot_created_at"] = metadata["source_snapshot_created_at"]
    if isinstance(record.get("package"), dict):
        safe["package"] = record["package"]
    if isinstance(record.get("check_summary"), dict):
        safe["check_summary"] = record["check_summary"]
    build = metadata.get("build") if isinstance(metadata.get("build"), dict) else {}
    if build:
        safe["build"] = {
            key: build[key]
            for key in ("built_at", "source_date_epoch", "builder")
            if key in build
        }
    return safe


def _assert_release_publish_ready(record: dict[str, Any]) -> None:
    checks = record.get("checks") if isinstance(record.get("checks"), list) else []
    if not checks:
        raise ValueError(f"Release {record['release_id']} has no recorded checks. Run `ait release check {record['release_id']}` first.")
    blocking = [row for row in checks if isinstance(row, dict) and bool(row.get("blocking"))]
    if blocking:
        raise ValueError(
            f"Release {record['release_id']} still has blocking checks. Resolve them and rerun `ait release check {record['release_id']}`."
        )
    artifact_kinds = {str(row.get("kind") or "") for row in record.get("artifacts", []) if isinstance(row, dict)}
    missing = sorted({"sdist", "wheel", "manifest", "checksum"} - artifact_kinds)
    if missing:
        raise ValueError(
            f"Release {record['release_id']} is missing built artifacts: {', '.join(missing)}. Run `ait release build {record['release_id']}` first."
        )


def _assert_remote_release_snapshot(remote_row: dict[str, Any], repo_name: str, record: dict[str, Any]) -> None:
    try:
        get_remote_snapshot(remote_row["url"], repo_name, str(record["snapshot_id"]), include_content=False)
    except RemoteError as exc:
        if "failed: 404" in str(exc):
            raise ValueError(
                f"Remote repository {repo_name} is missing source snapshot {record['snapshot_id']}. Run `ait push --line {record['line']}` first."
            ) from exc
        raise


def _render_release_summary(record: dict[str, Any]) -> None:
    header = Table(title=f"ait release {record['release_id']}")
    header.add_column("field")
    header.add_column("value")
    header.add_row("status", str(record.get("status") or ""))
    header.add_row("version", str(record.get("version") or ""))
    header.add_row("line", str(record.get("line") or ""))
    header.add_row("snapshot", str(record.get("snapshot_id") or ""))
    header.add_row("profile", str(record.get("profile") or ""))
    package = record.get("package") if isinstance(record.get("package"), dict) else {}
    header.add_row("package", str(package.get("name") or ""))
    header.add_row("next", str((record.get("next_action") or {}).get("detail") or ""))
    rprint(header)

    checks = record.get("checks") if isinstance(record.get("checks"), list) else []
    if checks:
        check_table = Table(title="release checks")
        check_table.add_column("check")
        check_table.add_column("status")
        check_table.add_column("details")
        for row in checks:
            check_table.add_row(str(row.get("check_id") or ""), str(row.get("status") or ""), str(row.get("details") or ""))
        rprint(check_table)

    artifacts = record.get("artifacts") if isinstance(record.get("artifacts"), list) else []
    if artifacts:
        artifact_table = Table(title="release artifacts")
        artifact_table.add_column("kind")
        artifact_table.add_column("path")
        artifact_table.add_column("sha256")
        for row in artifacts:
            artifact_table.add_row(str(row.get("kind") or ""), _artifact_display_path(row), str(row.get("sha256") or ""))
        rprint(artifact_table)

    formula = record.get("formula") if isinstance(record.get("formula"), dict) else {}
    formula_display = _formula_display_path(formula)
    if formula_display:
        typer.echo(f"formula draft: {formula_display}")


@candidate_app.command("create", help="Create a durable local release candidate record from one native line head.")
def release_candidate_create(
    version: str = typer.Option(..., "--version"),
    line: str = typer.Option(..., "--line"),
    profile: str = typer.Option(..., "--profile"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = create_release_candidate(ctx, version=version, line_name=line, profile=profile)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_release_summary(data)


@release_app.command("check", help="Run structured readiness checks against one local release candidate.")
def release_check(
    release_id: str,
    tests_command: Optional[str] = typer.Option(None, "--tests-command", help="Shell command to run as the release test step inside the exported snapshot."),
    skip_tests_reason: Optional[str] = typer.Option(None, "--skip-tests-reason", help="Record an explicit test waiver instead of running tests."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = run_release_checks(
            ctx,
            release_id,
            tests_command=tests_command,
            skip_tests_reason=skip_tests_reason,
        )
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_release_summary(data)


@release_app.command("build", help="Build deterministic release artifacts for one local release candidate.")
def release_build(
    release_id: str,
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = build_release_candidate(ctx, release_id)
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_release_summary(data)


@release_app.command("formula", help="Generate a Homebrew formula draft from a built local release candidate.")
def release_formula(
    release_id: str,
    name: str = typer.Option(..., "--name"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = generate_release_formula(ctx, release_id, name=name)
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_release_summary(data)


@release_app.command("show", help="Inspect one local release candidate, including checks, artifacts, and next action.")
def release_show(
    release_id: str,
    remote: Optional[str] = typer.Option(None, "--remote", help="Read the shared release record from the selected remote instead of the local draft record."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        if remote:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = remote_get_release(remote_row["url"], release_id, repo_name=repo_name)
        else:
            data = get_release_candidate(ctx, release_id)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_release_summary(data)


@release_app.command("publish", help="Upload one built local release candidate to the selected ait-server remote.")
def release_publish(
    release_id: str,
    remote: Optional[str] = typer.Option(None, "--remote", help="Publish to the selected remote; defaults to the configured default remote."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        record = get_release_candidate(ctx, release_id)
        _assert_release_publish_ready(record)
        remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote)
        if str(record.get("repo_name") or "") != repo_name:
            raise KeyError(f"Local release {release_id} belongs to repository {record.get('repo_name')}, not {repo_name}")
        _assert_remote_release_snapshot(remote_row, repo_name, record)
        remote_release = remote_publish_release(
            remote_row["url"],
            repo_name,
            record["release_id"],
            str(record["version"]),
            str(record["line"]),
            str(record["snapshot_id"]),
            str(record["manifest_hash"]),
            str(record["profile"]),
            package=record.get("package") if isinstance(record.get("package"), dict) else {},
            checks=record.get("checks") if isinstance(record.get("checks"), list) else [],
            artifacts=_release_publish_artifacts(ctx, record),
            formula=record.get("formula") if isinstance(record.get("formula"), dict) else {},
            metadata=_release_publish_metadata(record),
        )
        _require_remote_identity("release", release_id, remote_release)
        metadata = dict(record.get("metadata") or {})
        metadata["remote_publish"] = {
            "remote_name": str(remote_row.get("name") or remote or ""),
            "repo_name": repo_name,
            "release_id": str(remote_release.get("release_id") or release_id),
            "published_at": str(remote_release.get("updated_at") or remote_release.get("created_at") or ""),
            "status": str(remote_release.get("status") or ""),
            "artifact_count": len(remote_release.get("artifacts") or []),
        }
        update_local_release(
            ctx,
            release_id,
            status="published",
            metadata=metadata,
            event_type="release.published",
        )
        data = get_release_candidate(ctx, release_id)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_release_summary(data)
