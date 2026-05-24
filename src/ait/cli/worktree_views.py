from __future__ import annotations

import os
import shlex
from pathlib import Path
from typing import Any

from rich import print as rprint
from rich.table import Table


def _worktree_runtime_paths(path_value: str) -> dict[str, str | None]:
    root = Path(path_value).expanduser().resolve()
    src_path = root / "src"
    venv_path = root / ".venv"
    venv_bin_path = venv_path / "bin"
    return {
        "src_path": str(src_path),
        "venv_path": str(venv_path) if venv_path.exists() else None,
        "venv_bin_path": str(venv_bin_path) if venv_bin_path.is_dir() else None,
    }


def _worktree_runtime_env(path_value: str, data: dict, base_env: dict[str, str] | None = None) -> dict[str, str]:
    env = dict(base_env or os.environ.copy())
    runtime_paths = _worktree_runtime_paths(path_value)
    env["AIT_WORKTREE_NAME"] = str(data.get("name") or "")
    env["AIT_WORKTREE_PATH"] = path_value
    if data.get("current_line"):
        env["AIT_WORKTREE_LINE"] = str(data["current_line"])
    src_path = runtime_paths.get("src_path")
    if src_path:
        existing = env.get("PYTHONPATH")
        env["PYTHONPATH"] = f"{src_path}{os.pathsep}{existing}" if existing else src_path
    venv_bin_path = runtime_paths.get("venv_bin_path")
    if venv_bin_path:
        existing_path = env.get("PATH")
        env["PATH"] = f"{venv_bin_path}{os.pathsep}{existing_path}" if existing_path else venv_bin_path
    return env


def _worktree_shell_command(path_value: str, data: dict) -> str:
    runtime_paths = _worktree_runtime_paths(path_value)
    commands = [f"cd {shlex.quote(path_value)}"]
    commands.append(f"export AIT_WORKTREE_NAME={shlex.quote(str(data.get('name') or ''))}")
    commands.append(f"export AIT_WORKTREE_PATH={shlex.quote(path_value)}")
    if data.get("current_line"):
        commands.append(f"export AIT_WORKTREE_LINE={shlex.quote(str(data['current_line']))}")
    src_path = runtime_paths.get("src_path")
    if src_path:
        commands.append(f"export PYTHONPATH={shlex.quote(src_path)}${{PYTHONPATH:+:$PYTHONPATH}}")
    venv_bin_path = runtime_paths.get("venv_bin_path")
    if venv_bin_path:
        commands.append(f"export PATH={shlex.quote(venv_bin_path)}:$PATH")
    return " && ".join(commands)


def _workspace_baseline_label(data: dict) -> str:
    source = data.get("baseline_source")
    baseline_snapshot_id = data.get("baseline_snapshot_id") or "empty"
    baseline_line_name = data.get("baseline_line_name")
    if source == "snapshot":
        return f"snapshot {baseline_snapshot_id}"
    if source == "line_head":
        return f"line {baseline_line_name} head ({baseline_snapshot_id})"
    if source == "current_line_head":
        return f"current line head {baseline_line_name} ({baseline_snapshot_id})"
    if baseline_line_name:
        return f"{baseline_line_name} ({baseline_snapshot_id})"
    return str(baseline_snapshot_id)


def _render_workspace_status(data: dict) -> None:
    state = "clean" if data.get("clean") else "dirty"
    ignore_policy = data.get("ignore_policy") if isinstance(data.get("ignore_policy"), dict) else {}
    operational_roots = [str(item) for item in ignore_policy.get("operational_roots", []) if str(item)]
    rule_files = [str(item) for item in ignore_policy.get("rule_files", []) if str(item)]
    summary = Table(title=f"ait workspace status ({state})")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("repo", str(data.get("repo_name") or ""))
    summary.add_row("workspace", str(data.get("workspace_root") or ""))
    if data.get("is_worktree"):
        summary.add_row("worktree", str(data.get("worktree_name") or ""))
    summary.add_row("current line", str(data.get("current_line") or ""))
    summary.add_row("baseline", _workspace_baseline_label(data))
    summary.add_row("changed files", str(data.get("changed_count", 0)))
    if operational_roots:
        summary.add_row("ignored roots", ", ".join(operational_roots))
    if rule_files:
        summary.add_row("ignore files", ", ".join(rule_files))
    rprint(summary)

    if data.get("clean"):
        rprint("[green]Workspace is clean.[/green]")
        return

    changed = Table(title="changed paths")
    changed.add_column("kind")
    changed.add_column("path")
    for kind, paths in (
        ("modified", data.get("modified_paths", [])),
        ("missing", data.get("missing_paths", [])),
        ("untracked", data.get("untracked_paths", [])),
    ):
        for path in paths:
            changed.add_row(kind, str(path))
    rprint(changed)


def _render_worktrees(rows: list[dict]) -> None:
    table = Table(title="ait worktrees")
    table.add_column("name")
    table.add_column("path")
    table.add_column("line")
    table.add_column("status")
    table.add_column("cleanup")
    table.add_column("exists")
    table.add_column("current")
    for row in rows:
        status = str(row.get("workspace_status") or "")
        status_source = str(row.get("status_source") or "")
        changed_count = row.get("changed_count")
        if status == "dirty" and changed_count is not None:
            status = f"dirty ({changed_count})"
        elif status == "unknown" and status_source == "unverified":
            status = "unknown"
        if status and status_source == "cached":
            status = f"{status} [cached]"
        cleanup_class = str(row.get("cleanup_class") or "")
        cleanup_note = str(row.get("cleanup_policy") or "")
        if cleanup_class:
            cleanup_note = f"{cleanup_class}:{cleanup_note}" if cleanup_note else cleanup_class
        table.add_row(
            str(row.get("name") or ""),
            str(row.get("path") or ""),
            str(row.get("current_line") or row.get("registered_line_name") or ""),
            status,
            cleanup_note,
            "yes" if row.get("exists") else "no",
            "yes" if row.get("is_current") else "",
        )
    rprint(table)


def _render_worktree_doctor(data: dict) -> None:
    summary = Table(title="ait worktree doctor")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("total", str(data.get("total_count", 0)))
    summary.add_row("current", str(data.get("current_count", 0)))
    summary.add_row("clean", str(data.get("clean_count", 0)))
    summary.add_row("dirty", str(data.get("dirty_count", 0)))
    summary.add_row("missing", str(data.get("missing_count", 0)))
    summary.add_row("detached", str(data.get("detached_count", 0)))
    summary.add_row("protected", str(data.get("protected_count", 0)))
    summary.add_row("safe_auto_remove", str(data.get("safe_auto_remove_count", 0)))
    summary.add_row("safe_cleanup_candidate", str(data.get("safe_cleanup_candidate_count", 0)))
    summary.add_row("manual_review_candidate", str(data.get("manual_review_candidate_count", 0)))
    rprint(summary)
    if data.get("cleanup_candidate_rows"):
        rprint("[cyan]Cleanup candidates[/cyan]")
        _render_worktrees(data["cleanup_candidate_rows"])
    if data.get("manual_review_rows"):
        rprint("[yellow]Manual review cleanup candidates[/yellow]")
        _render_worktrees(data["manual_review_rows"])
    stale_rows = data.get("stale_rows", [])
    if not stale_rows:
        rprint("[green]No stale worktree registrations found.[/green]")
        return
    _render_worktrees(stale_rows)


def _render_worktree_cleanup_candidates(data: dict) -> None:
    summary = Table(title="ait worktree cleanup candidates")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("older_than", str(data.get("older_than") or ""))
    summary.add_row("policy", str(data.get("cleanup_policy") or "all"))
    summary.add_row("allow_manual_only", "yes" if data.get("allow_manual_only") else "no")
    summary.add_row("inspected", str(data.get("inspected_count", 0)))
    summary.add_row("candidates", str(data.get("candidate_count", 0)))
    summary.add_row("protected", str(data.get("protected_count", 0)))
    summary.add_row("stale", str(data.get("stale_count", 0)))
    rprint(summary)

    candidates = data.get("candidates", [])
    if candidates:
        table = Table(title="candidate rows")
        table.add_column("name")
        table.add_column("class")
        table.add_column("policy")
        table.add_column("last_used_at")
        table.add_column("reason")
        for row in candidates:
            table.add_row(
                str(row.get("name") or ""),
                str(row.get("cleanup_class") or ""),
                str(row.get("cleanup_policy") or ""),
                str(row.get("last_used_at") or ""),
                str(row.get("cleanup_reason") or ""),
            )
        rprint(table)
    else:
        rprint("[green]No cleanup candidates matched the current filters.[/green]")

    protected = data.get("protected", [])
    if protected:
        table = Table(title="protected rows")
        table.add_column("name")
        table.add_column("policy")
        table.add_column("reason")
        for row in protected:
            table.add_row(
                str(row.get("name") or ""),
                str(row.get("cleanup_policy") or ""),
                str(row.get("protected_reason") or ""),
            )
        rprint(table)


def _render_worktree_cleanup_report(data: dict) -> None:
    summary = Table(title="ait worktree cleanup")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("older_than", str(data.get("older_than") or ""))
    summary.add_row("policy", str(data.get("cleanup_policy") or "all"))
    summary.add_row("allow_manual_only", "yes" if data.get("allow_manual_only") else "no")
    summary.add_row("candidate_count", str(data.get("candidate_count", 0)))
    summary.add_row("planned_count", str(data.get("planned_count", 0)))
    if data.get("dry_run"):
        summary.add_row("mode", "dry_run")
    else:
        summary.add_row("removed_count", str(data.get("removed_count", 0)))
    rprint(summary)
    rows = data.get("planned_rows") if data.get("dry_run") else data.get("removed_rows")
    if rows:
        _render_worktrees(
            rows if data.get("dry_run") else [dict(row, cleanup_reason="removed by cleanup") for row in rows]
        )


def _render_line_cleanup_candidates(data: dict) -> None:
    summary = Table(title="ait line cleanup candidates")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("older_than", str(data.get("older_than") or ""))
    summary.add_row("kind", str(data.get("cleanup_kind") or "all"))
    summary.add_row("inspected", str(data.get("inspected_count", 0)))
    summary.add_row("candidates", str(data.get("candidate_count", 0)))
    summary.add_row("protected", str(data.get("protected_count", 0)))
    rprint(summary)
    candidates = data.get("candidates") or []
    if candidates:
        table = Table(title="line candidates")
        table.add_column("line")
        table.add_column("kind")
        table.add_column("policy")
        table.add_column("last_activity")
        table.add_column("reason")
        for row in candidates:
            table.add_row(
                str(row.get("line_name") or ""),
                str(row.get("lifecycle_kind") or ""),
                str(row.get("cleanup_policy") or ""),
                str(row.get("last_activity_at") or ""),
                str(row.get("cleanup_reason") or ""),
            )
        rprint(table)
    protected = data.get("protected") or []
    if protected:
        table = Table(title="protected lines")
        table.add_column("line")
        table.add_column("kind")
        table.add_column("policy")
        table.add_column("reason")
        for row in protected:
            table.add_row(
                str(row.get("line_name") or ""),
                str(row.get("lifecycle_kind") or ""),
                str(row.get("cleanup_policy") or ""),
                str(row.get("protected_reason") or ""),
            )
        rprint(table)


def _render_line_cleanup_report(data: dict) -> None:
    summary = Table(title="ait line cleanup")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("older_than", str(data.get("older_than") or ""))
    summary.add_row("kind", str(data.get("cleanup_kind") or "all"))
    summary.add_row("candidate_count", str(data.get("candidate_count", 0)))
    summary.add_row("planned_count", str(data.get("planned_count", 0)))
    if data.get("dry_run"):
        summary.add_row("mode", "dry_run")
    else:
        summary.add_row("archived_count", str(data.get("archived_count", 0)))
    rprint(summary)
    rows = data.get("planned_rows") if data.get("dry_run") else data.get("archived_rows")
    if rows:
        table = Table(title="line cleanup rows")
        table.add_column("line")
        table.add_column("detail")
        for row in rows:
            table.add_row(
                str(row.get("line_name") or ""),
                str(row.get("cleanup_reason") or row.get("status") or ""),
            )
        rprint(table)


def _render_worktree_sync_report(data: dict) -> None:
    summary = Table(title="ait worktree sync")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("requested", str(data.get("requested_count", 0)))
    summary.add_row("synced", str(data.get("synced_count", 0)))
    summary.add_row("skipped", str(data.get("skipped_count", 0)))
    summary.add_row("errors", str(data.get("error_count", 0)))
    rprint(summary)
    if data.get("synced_rows"):
        _render_worktrees(data["synced_rows"])
    if data.get("skipped_rows"):
        rprint("[yellow]Skipped stale worktree registrations.[/yellow]")
        _render_worktrees(data["skipped_rows"])
    if data.get("error_rows"):
        table = Table(title="sync errors")
        table.add_column("name")
        table.add_column("line")
        table.add_column("status")
        table.add_column("error")
        for row in data["error_rows"]:
            table.add_row(
                str(row.get("name") or ""),
                str(row.get("current_line") or ""),
                str(row.get("workspace_status") or ""),
                str(row.get("error") or ""),
            )
        rprint(table)


def _render_worktree_prune_report(data: dict) -> None:
    if data.get("dry_run"):
        rprint(f"Would prune {data.get('pruned_count', 0)} stale worktree registrations.")
    else:
        rprint(f"Pruned {data.get('pruned_count', 0)} stale worktree registrations.")
    if data.get("pruned_rows"):
        _render_worktrees(data["pruned_rows"])


def _render_repo_status(data: dict) -> None:
    ignore_policy = data.get("ignore_policy") if isinstance(data.get("ignore_policy"), dict) else {}
    operational_roots = [str(item) for item in ignore_policy.get("operational_roots", []) if str(item)]
    rule_files = [str(item) for item in ignore_policy.get("rule_files", []) if str(item)]
    worktree_hygiene = data.get("worktree_hygiene") if isinstance(data.get("worktree_hygiene"), dict) else {}
    line_hygiene = data.get("line_hygiene") if isinstance(data.get("line_hygiene"), dict) else {}
    summary = Table(title="ait status")
    summary.add_column("field")
    summary.add_column("value")
    summary.add_row("repo", str(data.get("repo_name") or ""))
    if data.get("is_worktree"):
        summary.add_row("worktree", str(data.get("worktree_name") or ""))
    summary.add_row("current line", str(data.get("current_line") or ""))
    summary.add_row("head snapshot", str(data.get("head_snapshot_id") or ""))
    summary.add_row("default remote", str(data.get("default_remote") or "none"))
    summary.add_row("workspace", f"{data.get('workspace_status', 'unknown')} ({data.get('workspace_changed_count', 0)} changed)")
    summary.add_row("snapshots", str(data.get("snapshot_count", 0)))
    summary.add_row("packs", str(data.get("pack_count", 0)))
    summary.add_row("packed blobs", str(data.get("packed_blob_count", 0)))
    summary.add_row("remotes", str(data.get("remote_count", 0)))
    if worktree_hygiene:
        summary.add_row(
            "worktree hygiene",
            (
                f"{worktree_hygiene.get('cleanup_candidate_count', 0)} cleanup, "
                f"{worktree_hygiene.get('manual_review_candidate_count', 0)} manual-review, "
                f"{worktree_hygiene.get('stale_count', 0)} stale"
            ),
        )
    if line_hygiene:
        summary.add_row(
            "line hygiene",
            (
                f"{line_hygiene.get('candidate_count', 0)} cleanup, "
                f"{line_hygiene.get('protected_count', 0)} protected "
                f"(older_than {line_hygiene.get('older_than') or '7d'})"
            ),
        )
    if operational_roots:
        summary.add_row("ignored roots", ", ".join(operational_roots))
    if rule_files:
        summary.add_row("ignore files", ", ".join(rule_files))
    rprint(summary)

    if data.get("workspace_dirty"):
        sample = ", ".join(str(path) for path in data.get("workspace_changed_paths_sample", []))
        if sample:
            rprint(f"[yellow]Workspace dirty:[/yellow] {sample}")
        else:
            rprint("[yellow]Workspace dirty.[/yellow]")
        rprint("Run [bold]ait workspace status[/bold] for path-level details.")
    if int(worktree_hygiene.get("manual_review_candidate_count", 0)) > 0:
        rprint("[yellow]Manual worktree backlog detected.[/yellow] Review with [bold]ait worktree cleanup-candidates --allow-manual-only --include-protected[/bold].")
    if int(line_hygiene.get("candidate_count", 0)) > 0:
        rprint("[yellow]Line cleanup candidates detected.[/yellow] Review with [bold]ait line cleanup-candidates --include-protected[/bold].")
