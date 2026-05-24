from __future__ import annotations

from ..plan_markdown_authoring import (
    _guard_execution_worktree_plan_sync,
    _resolve_plan_artifact_input,
)
from ..task_dag_graph_artifacts import (
    _TASK_DAG_CANONICAL_PATH_HINT,
    _load_task_dag_graph_for_plan,
)
from ..workflow_authoring import _plan_uses_local_store
from ..plan_sync_matching import _select_sync_existing_plan_with_continuity
from ..plan_sync_scope import (
    _load_plan_sync_existing_plans,
    _prune_missing_plan_artifacts,
    _publish_plan_sync_paired_artifacts,
    _resolve_plan_sync_paired_artifacts,
    _resolve_plan_sync_target,
    _sync_single_plan_artifact,
    _tracked_missing_markdown_artifact_paths,
)
from ..shared import export_app_namespace

export_app_namespace(globals())

_TASK_DAG_FROM_JSON_HELP = "Task DAG JSON artifact. When omitted, scan docs/sprints."


@plan_app.command("list")
def plan_list(
    local: bool = typer.Option(False, "--local", help="List local draft plans from .ait/control.db. This is the default when --remote is omitted."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        use_local = _plan_uses_local_store(local, remote)
        if use_local:
            rows = list_local_plans(ctx)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            rows = remote_list_plans(remote_row["url"], repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    _render_plan_summary(rows)


@plan_app.command("show", help="Inspect one synced plan and optionally one specific revision.", short_help="Inspect one plan or revision.")
def plan_show(
    plan_id: str,
    revision: Optional[str] = typer.Option(None, "--revision", help="Show a specific plan revision instead of the current head."),
    local: bool = typer.Option(False, "--local", help="Read the plan from the local draft store. This is the default when --remote is omitted."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        use_local = _plan_uses_local_store(local, remote)
        if use_local:
            plan = get_local_plan(ctx, plan_id)
            revision_data = get_local_plan_revision(ctx, plan_id, revision) if revision else None
        else:
            remote_row, _ = _remote_tuple(ctx, remote)
            plan = remote_get_plan(remote_row["url"], plan_id)
            revision_data = remote_get_plan_revision(remote_row["url"], plan_id, revision) if revision else None
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        if revision_data is not None:
            _emit({"plan": plan, "revision": revision_data}, True)
        else:
            _emit(plan, True)
        return
    _render_plan_detail(plan, revision=revision_data)


@plan_app.command("revisions")
def plan_revisions(
    plan_id: str,
    local: bool = typer.Option(False, "--local", help="List local plan revisions from .ait/control.db. This is the default when --remote is omitted."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        use_local = _plan_uses_local_store(local, remote)
        if use_local:
            rows = list_local_plan_revisions(ctx, plan_id)
        else:
            remote_row, _ = _remote_tuple(ctx, remote)
            rows = remote_list_plan_revisions(remote_row["url"], plan_id)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    _render_plan_revisions(plan_id, rows)


@plan_app.command("items", help="List stable plan item refs from the current or selected plan revision.", short_help="List stable plan items.")
def plan_items(
    plan_id: str,
    revision: Optional[str] = typer.Option(None, "--revision", help="Read plan items from a specific revision instead of the current head."),
    local: bool = typer.Option(False, "--local", help="Read plan items from the local draft plan store. This is the default when --remote is omitted."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        use_local = _plan_uses_local_store(local, remote)
        if use_local:
            plan = get_local_plan(ctx, plan_id)
            revision_data = get_local_plan_revision(ctx, plan_id, revision) if revision else None
        else:
            remote_row, _ = _remote_tuple(ctx, remote)
            plan = remote_get_plan(remote_row["url"], plan_id)
            revision_data = remote_get_plan_revision(remote_row["url"], plan_id, revision) if revision else None
        payload = _plan_items_payload(plan, revision=revision_data)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    _render_plan_items(payload)


@plan_app.command("candidates", help="List currently taskable plan items across open plans before execution.", short_help="List taskable plan items.")
def plan_candidates(
    local: bool = typer.Option(False, "--local", help="Read candidate plans from the local draft plan store. This is the default when --remote is omitted."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    include_all: bool = typer.Option(False, "--all", help="Include open plans even when they have no currently taskable items."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        payload = _plan_dispatch_scope_payload(ctx, local=local, remote=remote, include_all=include_all)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    _render_plan_candidates(payload)


@plan_app.command("inspect", help="Inspect one plan's DAG state, linked tasks, and unpublished local-vs-remote lineage.", short_help="Inspect one plan's DAG state.")
def plan_inspect(
    plan_id: str,
    revision: Optional[str] = typer.Option(None, "--revision", help="Inspect a specific plan revision instead of the current head."),
    local: bool = typer.Option(False, "--local", help="Read the plan from the local draft plan store. This is the default when --remote is omitted."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        use_local = _plan_uses_local_store(local, remote)
        if use_local:
            plan = get_local_plan(ctx, plan_id)
            revision_data = get_local_plan_revision(ctx, plan_id, revision) if revision else None
            tasks = list_local_tasks(ctx)
            remote_name = None
            repo_name = (load_config(ctx).get("repo_name") or ctx.root.name)
            local_shadow = None
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            plan = remote_get_plan(remote_row["url"], plan_id)
            revision_data = remote_get_plan_revision(remote_row["url"], plan_id, revision) if revision else None
            tasks = remote_list_tasks(remote_row["url"], repo_name)
            remote_name = remote_row.get("name") or remote
            local_shadow = _local_plan_publish_shadow_index(ctx).get(plan_id)
        task_links_by_item, tasks_by_plan = _plan_task_link_indexes(tasks)
        summary = _plan_dispatch_summary(
            plan,
            revision=revision_data,
            task_links_by_item=task_links_by_item,
            tasks_by_plan=tasks_by_plan,
            local_shadow=local_shadow,
        )
        payload = {
            "scope": "local" if use_local else "remote",
            "remote": remote_name,
            "repo_name": repo_name,
            "plan": summary,
        }
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    _render_plan_inspect(payload)


@plan_app.command(
    "sync",
    help=(
        "Sync Markdown plan artifacts into the local draft store. "
        "Use `--remote origin` when this sync should update shared Markdown lineage. "
        "With --remote, sync shared plan state fast-forward-only and archive missing tracked Markdown in scope; "
        "use --rebase (or legacy --reconcile) only as an explicit divergent-head retry that "
        "replays the current local head on top of the current remote head."
    ),
)
def plan_sync(
    target: Path = typer.Argument(..., help="Markdown file or directory to sync into durable ait plans."),
    plan_ref: Optional[str] = typer.Option(
        None,
        "--plan-ref",
        help="For a single Markdown file, choose which `[plan-ref: ...]` section to sync when the file exposes multiple plan roots.",
    ),
    prune: bool = typer.Option(
        False,
        "--prune",
        help="Archive tracked plans under the selected sync scope when their Markdown artifact file has been deleted. With --remote, the same delete scan is already part of normal command success.",
    ),
    local: bool = typer.Option(False, "--local", help="Sync against the local draft plan store only. This is the default when --remote is omitted."),
    remote: Optional[str] = typer.Option(
        None,
        "--remote",
        help=(
            "After local sync, publish touched local plan revisions to the selected remote. "
            "Use `--remote origin` for the explicit shared-lineage Markdown sync boundary; "
            "omitting `--remote` keeps the sync local-only."
        ),
    ),
    source_session: Optional[str] = typer.Option(None, "--source-session", help="Override the hidden session provenance binding used when publishing to a remote."),
    rebase: bool = typer.Option(
        False,
        "--rebase",
        help="With --remote, explicitly retry a divergent remote plan head by replaying the current local head on top of the remote head. This is the preferred spelling for the current-local-head retry path.",
    ),
    reconcile: bool = typer.Option(
        False,
        "--reconcile",
        help="Compatibility alias for the current-local-head divergent retry path. With --remote, explicitly replay the current local head on top of the remote head instead of failing fast-forward-only publish.",
    ),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    session_target: dict[str, Any] | None = None
    sync_target: dict[str, Any] = {"scope": "unknown"}
    results: list[dict[str, Any]] = []
    adoptions: list[dict[str, Any]] = []
    publish_results: list[dict[str, Any]] = []
    artifact_results: list[dict[str, Any]] = []

    def _plan_sync_payload(status: str, *, error: dict[str, Any] | None = None) -> dict[str, Any]:
        payload = {
            "status": status,
            "target": str(target),
            "scope": sync_target["scope"],
            "mode": "local_publish" if remote is not None else "local",
            "results": results,
            "adoptions": adoptions,
            "publish_results": publish_results,
            "artifact_results": artifact_results,
            "summary": {
                "created_count": sum(1 for row in results if row.get("action") == "created"),
                "updated_count": sum(1 for row in results if row.get("action") == "updated"),
                "unchanged_count": sum(1 for row in results if row.get("action") == "unchanged"),
                "pruned_count": sum(1 for row in results if row.get("action") == "pruned"),
                "adopted_count": len(adoptions),
                "processed_count": len(results),
                "published_count": len(publish_results),
                "artifact_count": len(artifact_results),
            },
        }
        if error is not None:
            payload["error"] = error
        return payload

    try:
        if local and remote is not None:
            raise typer.BadParameter("`--local` cannot be combined with `--remote`; omit `--local` to sync locally and publish.")
        if rebase and reconcile:
            raise typer.BadParameter("Choose either `--rebase` or `--reconcile`, not both.")
        if (rebase or reconcile) and remote is None:
            raise typer.BadParameter("`--rebase` and `--reconcile` require `--remote <name>` because the divergent-head retry path only applies to shared publish.")
        _guard_execution_worktree_plan_sync(ctx, target=target)
        divergent_retry_mode = "rebase" if rebase else "reconcile" if reconcile else None
        publish_remote = remote is not None
        prune_missing = prune or publish_remote
        sync_target = _resolve_plan_sync_target(ctx, target, allow_missing=prune_missing)
        if plan_ref is not None and sync_target["scope"] != "file":
            raise typer.BadParameter("`--plan-ref` can only be used when syncing one Markdown file.")

        artifacts = [_resolve_plan_artifact_input(ctx, file_path, plan_ref, allow_generic_markdown=True) for file_path in sync_target["files"]]
        if not artifacts and not prune_missing:
            raise typer.BadParameter(f"No Markdown plan artifacts found under {target}.")
        paired_artifacts_by_markdown_path = _resolve_plan_sync_paired_artifacts(
            ctx,
            sync_target=sync_target,
            markdown_artifacts=artifacts,
            publish_remote=publish_remote,
        )

        plans, _, _ = _load_plan_sync_existing_plans(ctx, local=True, remote_name=None)
        indexed_plans = _index_plans_by_artifact_path(plans)
        indexed_plans_by_identity = _index_plans_by_artifact_identity(plans)
        remote_row: dict[str, Any] | None = None
        repo_name: str | None = None
        remote_indexed_plans: dict[str, list[dict[str, Any]]] = {}
        remote_indexed_plans_by_identity: dict[tuple[str, str | None], list[dict[str, Any]]] = {}
        remote_plan_cache: dict[str, dict[str, Any]] = {}
        remote_revisions_by_plan_id: dict[str, list[dict[str, Any]]] = {}
        remote_revision_cache: dict[tuple[str, str], dict[str, Any]] = {}
        remote_full_plans_cache: list[dict[str, Any]] | None = None
        remote_full_index_by_identity: dict[tuple[str, str | None], list[dict[str, Any]]] | None = None
        if publish_remote:
            session_target = _resolve_remote_workflow_boundary_session(
                ctx,
                remote_name=remote,
                command_name="plan sync",
                source_surface="cli.plan.sync",
                session_id=source_session,
            )
            remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote)
            remote_list_artifact_path = str(sync_target["target_path"]) if sync_target["scope"] == "file" else None
            remote_plans = [
                _remote_plan_summary_to_plan(row)
                for row in remote_list_plans(
                    remote_row["url"],
                    repo_name,
                    artifact_path=remote_list_artifact_path,
                )
            ]
            remote_plan_cache = {
                str(plan.get("plan_id") or ""): plan
                for plan in remote_plans
                if str(plan.get("plan_id") or "").strip()
            }
            remote_indexed_plans = _index_plans_by_artifact_path(remote_plans)
            remote_indexed_plans_by_identity = _index_plans_by_artifact_identity(remote_plans)
        synced_artifact_paths: set[str] = set()

        def load_remote_plan(plan_id: str) -> dict[str, Any]:
            cached = remote_plan_cache.get(plan_id)
            if cached is not None and isinstance(cached.get("head_revision"), dict) and cached["head_revision"].get("artifact_blob_id") is not None:
                return cached
            assert remote_row is not None
            fetched = remote_get_plan(remote_row["url"], plan_id)
            remote_plan_cache[plan_id] = fetched
            return fetched

        def load_remote_revisions(plan_id: str) -> list[dict[str, Any]]:
            cached = remote_revisions_by_plan_id.get(plan_id)
            if cached is not None:
                return cached
            assert remote_row is not None
            rows = remote_list_plan_revisions(remote_row["url"], plan_id)
            remote_revisions_by_plan_id[plan_id] = rows
            return rows

        def load_remote_revision(plan_id: str, plan_revision_id: str) -> dict[str, Any]:
            cache_key = (plan_id, plan_revision_id)
            cached = remote_revision_cache.get(cache_key)
            if cached is not None:
                return cached
            assert remote_row is not None
            row = remote_get_plan_revision(remote_row["url"], plan_id, plan_revision_id)
            remote_revision_cache[cache_key] = row
            return row

        def load_full_remote_plan_candidates() -> tuple[list[dict[str, Any]], dict[tuple[str, str | None], list[dict[str, Any]]]]:
            nonlocal remote_full_plans_cache, remote_full_index_by_identity
            if remote_full_plans_cache is None or remote_full_index_by_identity is None:
                assert remote_row is not None
                assert repo_name is not None
                remote_full_plans_cache = [
                    _remote_plan_summary_to_plan(row)
                    for row in remote_list_plans(
                        remote_row["url"],
                        repo_name,
                    )
                ]
                remote_full_index_by_identity = _index_plans_by_artifact_identity(remote_full_plans_cache)
                for plan in remote_full_plans_cache:
                    plan_id = str(plan.get("plan_id") or "").strip()
                    if plan_id:
                        remote_plan_cache[plan_id] = plan
            return remote_full_plans_cache, remote_full_index_by_identity

        for artifact in artifacts:
            continuity_match: dict[str, Any] | None = None
            if publish_remote:
                existing_plan, adoption, continuity_match = _resolve_local_sync_plan_candidate(
                    ctx,
                    artifact,
                    local_plans=plans,
                    local_indexed_plans=indexed_plans_by_identity,
                    remote_plans=remote_plans,
                    remote_indexed_plans=remote_indexed_plans_by_identity,
                    remote_name=remote,
                    repo_name=repo_name,
                    load_full_remote_plan_candidates=load_full_remote_plan_candidates,
                    load_remote_plan=load_remote_plan,
                    load_remote_revisions=load_remote_revisions,
                    load_remote_revision=load_remote_revision,
                )
                if adoption is not None:
                    adoptions.append(adoption)
            else:
                existing_plan, continuity_match = _select_sync_existing_plan_with_continuity(
                    ctx,
                    artifact,
                    indexed_plans_by_identity=indexed_plans_by_identity,
                    plans=plans,
                )
            result = _sync_single_plan_artifact(
                ctx,
                artifact,
                local=True,
                remote_row=None,
                repo_name=None,
                existing_plan=existing_plan,
                continuity_match=continuity_match,
            )
            results.append(result)
            synced_artifact_paths.add(str(artifact["artifact_path"]))

        if prune_missing:
            plans, _, _ = _load_plan_sync_existing_plans(ctx, local=True, remote_name=None)
            indexed_plans = _index_plans_by_artifact_path(plans)
            indexed_plans_by_identity = _index_plans_by_artifact_identity(plans)
            deleted_artifact_paths = _tracked_missing_markdown_artifact_paths(
                ctx,
                sync_target=sync_target,
                artifact_paths=set(indexed_plans) | set(remote_indexed_plans),
                synced_artifact_paths=synced_artifact_paths,
            )
            if publish_remote:
                for artifact_path in sorted(deleted_artifact_paths & set(remote_indexed_plans)):
                    identity_keys = sorted(
                        {
                            _plan_artifact_identity_key(
                                artifact_path,
                                _normalize_text_value((candidate.get("head_revision") or {}).get("artifact_selector")),
                            )
                            for candidate in remote_indexed_plans.get(artifact_path, [])
                        },
                        key=lambda item: (item[0], item[1] or ""),
                    )
                    for _, artifact_selector in identity_keys:
                        _, adoption = _resolve_local_sync_plan_candidate(
                            ctx,
                            artifact_path,
                            artifact_selector=artifact_selector,
                            local_indexed_plans=indexed_plans_by_identity,
                            remote_indexed_plans=remote_indexed_plans_by_identity,
                            remote_name=remote,
                            repo_name=repo_name,
                            load_remote_plan=load_remote_plan,
                            load_remote_revisions=load_remote_revisions,
                            load_remote_revision=load_remote_revision,
                        )
                        if adoption is not None:
                            adoptions.append(adoption)
                plans, _, _ = _load_plan_sync_existing_plans(ctx, local=True, remote_name=None)
                indexed_plans = _index_plans_by_artifact_path(plans)
            pruned_results = _prune_missing_plan_artifacts(
                ctx,
                local=True,
                remote_row=None,
                sync_target=sync_target,
                indexed_plans=indexed_plans,
                synced_artifact_paths=synced_artifact_paths,
            )
            results.extend(pruned_results)
        if publish_remote:
            publish_results = _publish_synced_local_plan_results(
                ctx,
                results,
                remote,
                divergent_retry_mode=divergent_retry_mode,
                source_session_id=None if session_target is None else str(session_target["session_id"]),
            )
        artifact_results = _publish_plan_sync_paired_artifacts(
            ctx,
            results=results,
            paired_artifacts_by_markdown_path=paired_artifacts_by_markdown_path,
            remote_name=remote,
        )
        if publish_remote and session_target is not None:
            unique_plan_ids = sorted(
                {
                    str(row.get("plan_id") or "").strip()
                    for row in results
                    if str(row.get("plan_id") or "").strip()
                }
            )
            unique_revision_ids = sorted(
                {
                    str(row.get("published_head_revision_id") or row.get("head_revision_id") or row.get("plan_revision_id") or "").strip()
                    for row in (publish_results or results)
                    if str(row.get("published_head_revision_id") or row.get("head_revision_id") or row.get("plan_revision_id") or "").strip()
                }
            )
            attachment_hints: dict[str, Any] = {}
            if len(unique_plan_ids) == 1:
                attachment_hints["plan_id"] = unique_plan_ids[0]
            if len(unique_revision_ids) == 1:
                attachment_hints["plan_revision_id"] = unique_revision_ids[0]
            _append_remote_workflow_boundary_event(
                ctx,
                session_target=session_target,
                command_text=f"ait plan sync {str(target)} --remote {remote or session_target['remote_name']}",
                boundary_kind="plan_sync",
                attachment_hints=attachment_hints,
                extra_payload={
                    "target_path": str(target),
                    "result_count": len(results),
                    "published_count": len(publish_results),
                },
            )
    except (KeyError, RemoteError, ValueError) as exc:
        if json_output:
            status = "partial_success" if publish_results or artifact_results else "failed"
            _emit(
                _plan_sync_payload(
                    status,
                    error={"message": str(exc), "stage": "sync"},
                ),
                True,
            )
            raise typer.Exit(code=1) from exc
        raise typer.BadParameter(str(exc)) from exc

    payload = _plan_sync_payload("ok")
    if json_output:
        _emit(payload, True)
        return
    _render_plan_sync_summary(payload)


@plan_app.command("graph")
def plan_graph_cmd(
    plan_id: str,
    from_json: Optional[Path] = typer.Option(None, "--from-json", help=_TASK_DAG_FROM_JSON_HELP),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        graph, graph_path = _load_task_dag_graph_for_plan(ctx, plan_id, from_json)
        readiness = _task_dag_readiness_payload(ctx, graph, remote)
        payload = _task_dag_graph_payload(plan_id, graph, graph_path, readiness, ctx)
        remote_row = None
        repo_name = None
        try:
            remote_row, repo_name = _remote_tuple(ctx, remote)
        except (KeyError, ValueError):
            pass
        payload["telegram_graph_watch"] = _maybe_auto_register_task_dag_telegram_watch(
            ctx=ctx,
            remote_row=remote_row,
            repo_name=repo_name,
            plan_id=plan_id,
            graph_artifact_path=_task_dag_relative_path(ctx, graph_path),
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    _render_task_dag_graph(payload)


@plan_app.command("ready")
def plan_ready_cmd(
    plan_id: str,
    from_json: Optional[Path] = typer.Option(None, "--from-json", help=_TASK_DAG_FROM_JSON_HELP),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        graph, graph_path = _load_task_dag_graph_for_plan(ctx, plan_id, from_json)
        readiness = _task_dag_readiness_payload(ctx, graph, remote)
        payload = _task_dag_ready_payload(plan_id, graph, graph_path, readiness, ctx)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    _render_task_dag_schedule(payload)


@plan_app.command("schedule")
def plan_schedule_cmd(
    plan_id: str,
    from_json: Optional[Path] = typer.Option(None, "--from-json", help=_TASK_DAG_FROM_JSON_HELP),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        graph, graph_path = _load_task_dag_graph_for_plan(ctx, plan_id, from_json)
        readiness = _task_dag_readiness_payload(ctx, graph, remote)
        payload = _task_dag_schedule_payload(plan_id, graph, graph_path, readiness, ctx)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    _render_task_dag_schedule(payload)


@plan_app.command("progress")
def plan_progress_cmd(
    plan_id: str,
    from_json: Optional[Path] = typer.Option(None, "--from-json", help=_TASK_DAG_FROM_JSON_HELP),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        graph, graph_path = _load_task_dag_graph_for_plan(ctx, plan_id, from_json)
        readiness = _task_dag_readiness_payload(ctx, graph, remote)
        payload = _task_dag_progress_summary_payload(plan_id, graph, graph_path, readiness, ctx)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    _render_task_dag_progress(payload)


@plan_app.command("execute")
def plan_execute_cmd(
    plan_id: str,
    from_json: Optional[Path] = typer.Option(None, "--from-json", help=_TASK_DAG_FROM_JSON_HELP),
    auto_compact_worker: bool = typer.Option(
        False,
        "--auto-compact-worker",
        help="Create a guarded remote graph-run scaffold, generate a compact packet artifact, open one fresh worker session, and run the packet through local compact-worker reply generation.",
    ),
    comparison_evidence_report: Optional[Path] = typer.Option(
        None,
        "--comparison-evidence-report",
        help="Package one benchmark report JSON into the compact worker packet so the worker can complete bounded token comparisons without broadening scope.",
    ),
    comparison_evidence_workload_id: Optional[str] = typer.Option(
        None,
        "--comparison-evidence-workload-id",
        help="Select which workload row to package from a multi-workload comparison evidence report.",
    ),
    pause_run: Optional[str] = typer.Option(
        None,
        "--pause-run",
        help="Pause a recorded graph-run session for one explicit reason: review, attestation, policy, land, or manual.",
    ),
    resume_run: bool = typer.Option(
        False,
        "--resume-run",
        help="Resume a recorded graph-run session after an operator pause or cleared gate.",
    ),
    retry_run: bool = typer.Option(
        False,
        "--retry-run",
        help="Retry a recorded graph-run session after a recoverable stop or gate reset.",
    ),
    abort_run: bool = typer.Option(
        False,
        "--abort-run",
        help="Abort a recorded graph-run session.",
    ),
    run_session: Optional[str] = typer.Option(
        None,
        "--run-session",
        help="Inspect one recorded graph-run session for this plan and graph.",
    ),
    latest_run: bool = typer.Option(
        False,
        "--latest-run",
        help="Inspect the latest recorded graph-run session for this plan and graph.",
    ),
    yes: bool = typer.Option(False, "--yes", help="Confirm guarded graph-run scaffolding."),
    allow_stale: bool = typer.Option(False, "--allow-stale", help="Allow guarded execute-run scaffolding from a stale source plan revision."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        open_execute_run = auto_compact_worker
        if comparison_evidence_workload_id and comparison_evidence_report is None:
            raise ValueError("--comparison-evidence-workload-id requires --comparison-evidence-report.")
        if comparison_evidence_report is not None and not auto_compact_worker:
            raise ValueError("--comparison-evidence-report currently requires --auto-compact-worker.")
        operator_actions = [name for name, enabled in (("pause", pause_run is not None), ("resume", resume_run), ("retry", retry_run), ("abort", abort_run)) if enabled]
        if run_session and latest_run:
            raise ValueError("Choose either --run-session or --latest-run, not both.")
        if (run_session or latest_run) and open_execute_run:
            raise ValueError("Opening a graph-run scaffold cannot be combined with --run-session or --latest-run.")
        if operator_actions and open_execute_run:
            raise ValueError("Graph-run operator controls cannot be combined with graph-run scaffolding.")
        if len(operator_actions) > 1:
            raise ValueError("Choose only one graph-run operator control at a time.")
        if operator_actions and not (run_session or latest_run):
            raise ValueError("Graph-run operator controls require --run-session or --latest-run.")
        if operator_actions and not yes:
            raise ValueError("Graph-run operator controls require --yes.")
        graph, graph_path = _load_task_dag_graph_for_plan(ctx, plan_id, from_json)
        readiness = _task_dag_readiness_payload(ctx, graph, remote)
        payload = _task_dag_execute_payload(plan_id, graph, graph_path, readiness, ctx)
        payload["compact_packet_surface"] = _task_dag_compact_packet_surface_payload(
            ctx,
            graph_path=graph_path,
            final_remote_disposition_default=bool(
                (payload.get("execute_run_contract") if isinstance(payload.get("execute_run_contract"), dict) else {}).get(
                    "final_remote_disposition_default"
                )
            ),
            change_focus_policy=payload.get("change_focus_policy")
            if isinstance(payload.get("change_focus_policy"), dict)
            else {},
        )
        execute_contract = payload.get("execute_run_contract") if isinstance(payload.get("execute_run_contract"), dict) else {}
        remote_row = None
        repo_name = None
        resolved_run_session = run_session
        if operator_actions or run_session or latest_run or open_execute_run:
            remote_row, repo_name = _remote_tuple(ctx, remote)
        if operator_actions:
            action = operator_actions[0]
            if latest_run and not resolved_run_session:
                latest = _task_dag_latest_execute_run_session(
                    remote_row=remote_row,
                    repo_name=repo_name,
                    plan_id=plan_id,
                    graph=graph,
                )
                if latest is None:
                    raise ValueError("No recorded task_graph_run session exists yet for this plan and graph.")
                resolved_run_session = str(latest.get("session_id") or "")
            payload["controlled_run"] = _task_dag_control_execute_run(
                ctx=ctx,
                remote_row=remote_row,
                repo_name=repo_name,
                plan_id=plan_id,
                graph=graph,
                graph_path=graph_path,
                readiness=readiness,
                session_id=str(resolved_run_session or ""),
                action=action,
                pause_reason=pause_run,
            )
            payload["mode"] = f"{action}_run"
        elif run_session or latest_run:
            if latest_run:
                latest = _task_dag_latest_execute_run_session(
                    remote_row=remote_row,
                    repo_name=repo_name,
                    plan_id=plan_id,
                    graph=graph,
                )
                if latest is None:
                    raise ValueError("No recorded task_graph_run session exists yet for this plan and graph.")
                resolved_run_session = str(latest.get("session_id") or "")
            payload["recorded_run"] = _task_dag_load_execute_run(
                ctx=ctx,
                remote_row=remote_row,
                repo_name=repo_name,
                plan_id=plan_id,
                graph=graph,
                graph_path=graph_path,
                readiness=readiness,
                session_id=str(resolved_run_session or ""),
            )
            payload["mode"] = "inspect_run"
        elif open_execute_run:
            if not yes:
                raise ValueError("Guarded execute-run scaffolding requires --yes.")
            if readiness.get("stale_source_plan") and not allow_stale:
                raise ValueError("Task graph source plan revision is stale; pass --allow-stale to override guarded execute-run scaffolding.")
            contract = (
                payload.get("execute_run_contract")
                if isinstance(payload.get("execute_run_contract"), dict)
                else payload.get("contract")
                if isinstance(payload.get("contract"), dict)
                else {}
            )
            if (
                contract.get("worker_execution_mode") == "worker_only_compact_packet"
                and comparison_evidence_report is None
            ):
                _guard_task_dag_implementation_authoring_workspace(
                    ctx,
                    remote_row=remote_row,
                    compact_packet_surface=payload.get("compact_packet_surface") if isinstance(payload.get("compact_packet_surface"), dict) else {},
                    command_name="plan execute --auto-compact-worker",
                )
            payload["recorded_run"] = _task_dag_open_execute_run(
                ctx=ctx,
                remote_row=remote_row,
                repo_name=repo_name,
                plan_id=plan_id,
                graph=graph,
                graph_path=graph_path,
                readiness=readiness,
                payload=payload,
            )
            if auto_compact_worker:
                payload["auto_compact_worker"] = _task_dag_start_auto_compact_worker(
                    ctx=ctx,
                    remote_row=remote_row,
                    repo_name=repo_name,
                    plan_id=plan_id,
                    graph=graph,
                    graph_path=graph_path,
                    recorded_run=payload["recorded_run"],
                    compact_packet_surface=payload.get("compact_packet_surface") if isinstance(payload.get("compact_packet_surface"), dict) else {},
                    comparison_evidence_report=comparison_evidence_report,
                    comparison_evidence_workload_id=comparison_evidence_workload_id,
                )
            payload["mode"] = "record_run"
        if payload.get("mode") in {"bootstrap_node", "advance_run", "pause_run", "resume_run", "retry_run", "abort_run", "record_run"}:
            payload["telegram_graph_watch"] = _maybe_auto_register_task_dag_telegram_watch(
                ctx=ctx,
                remote_row=remote_row,
                repo_name=repo_name,
                plan_id=plan_id,
                graph_artifact_path=_task_dag_relative_path(ctx, graph_path),
            )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(payload, True)
        return
    _render_task_dag_execute(payload)


@plan_app.command(
    "dispatch",
    hidden=True,
    context_settings={"allow_extra_args": True, "ignore_unknown_options": True},
)
def plan_dispatch_removed_cmd(ctx: typer.Context):
    typer.echo(
        "`ait plan dispatch` has been removed. Use `ait plan execute <plan-id> --from-json <task-graph-json> "
        "--auto-compact-worker --yes` as the default DAG path.",
        err=True,
    )
    raise typer.Exit(2)
