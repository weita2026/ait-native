from __future__ import annotations

from typing import Any

from rich import print as rprint
from rich.table import Table


def _render_task_dag_graph(payload: dict[str, Any]) -> None:
    summary = payload.get("workflow_summary") or payload.get("readiness_summary") or {}
    readiness_summary = payload.get("readiness_summary") or {}
    rprint(
        f"[bold]{payload.get('graph_id')}[/bold] "
        f"ready={summary.get('ready_nodes', 0)} dispatched={summary.get('dispatched_nodes', 0)} "
        f"running={summary.get('running_nodes', 0)} blocked={summary.get('blocked_nodes', 0)} "
        f"completed={summary.get('completed_nodes', 0)}"
    )
    if readiness_summary.get("stale_source_plan"):
        rprint("[yellow]source plan revision is stale[/yellow]")
    table = Table(title=f"task DAG graph for {payload.get('plan_id')}")
    table.add_column("node")
    table.add_column("state")
    table.add_column("workflow")
    table.add_column("deps")
    table.add_column("task")
    table.add_column("change")
    table.add_column("title")
    for row in payload.get("nodes") or []:
        table.add_row(
            str(row.get("node_id") or ""),
            str(row.get("state") or ""),
            str(row.get("workflow_state") or ""),
            ",".join(str(value) for value in row.get("depends_on") or []),
            str(row.get("task_id") or ""),
            str(row.get("change_id") or ""),
            str(row.get("title") or ""),
        )
    rprint(table)
    telegram_watch = payload.get("telegram_graph_watch") if isinstance(payload.get("telegram_graph_watch"), dict) else {}
    if telegram_watch:
        if telegram_watch.get("registered"):
            reason = "already registered" if telegram_watch.get("already_registered") else "auto-registered"
            rprint(
                "[bold]telegram graph watch:[/bold] "
                f"{reason} chat={telegram_watch.get('chat_id') or '?'} "
                f"mode={telegram_watch.get('resolution_mode') or 'unknown'}"
            )
        else:
            rprint(
                "[bold]telegram graph watch:[/bold] "
                f"not registered ({telegram_watch.get('reason') or 'unknown'})"
            )


def _render_task_dag_schedule(payload: dict[str, Any]) -> None:
    summary = payload.get("workflow_summary") or payload.get("readiness_summary") or {}
    readiness_summary = payload.get("readiness_summary") or {}
    execution_strategy = payload.get("execution_strategy") if isinstance(payload.get("execution_strategy"), dict) else {}
    rprint(
        f"[bold]next:[/bold] {summary.get('next_action') or 'none'} "
        f"ready={summary.get('ready_nodes', 0)} dispatched={summary.get('dispatched_nodes', 0)} "
        f"running={summary.get('running_nodes', 0)} blocked={summary.get('blocked_nodes', 0)}"
    )
    if execution_strategy:
        rprint(
            "[bold]strategy:[/bold] "
            f"{execution_strategy.get('default_mode')} + {execution_strategy.get('dispatch_model')} · "
            f"{execution_strategy.get('recommended_worker_sessions', 0)} fresh worker session(s); "
            "physical fan-out requires explicit opt-in"
        )
    if readiness_summary.get("stale_source_plan"):
        rprint("[yellow]source plan revision is stale[/yellow]")
    ready = Table(title=f"ready work for {payload.get('plan_id')}")
    ready.add_column("node")
    ready.add_column("state")
    ready.add_column("workflow")
    ready.add_column("plan_item_ref")
    ready.add_column("session")
    ready.add_column("title")
    for row in payload.get("ready") or []:
        recommendation = row.get("session_recommendation") if isinstance(row.get("session_recommendation"), dict) else {}
        ready.add_row(
            str(row.get("node_id") or ""),
            str(row.get("state") or ""),
            str(row.get("workflow_state") or ""),
            str(row.get("plan_item_ref") or ""),
            str(recommendation.get("action") or ""),
            str(row.get("title") or ""),
        )
    rprint(ready)
    dispatched = payload.get("dispatched") or []
    if dispatched:
        table = Table(title="dispatched planning")
        table.add_column("node")
        table.add_column("state")
        table.add_column("workflow")
        table.add_column("task")
        table.add_column("title")
        for row in dispatched:
            table.add_row(
                str(row.get("node_id") or ""),
                str(row.get("state") or ""),
                str(row.get("workflow_state") or ""),
                str(row.get("task_id") or ""),
                str(row.get("title") or ""),
            )
        rprint(table)
    running = payload.get("running") or []
    if running:
        table = Table(title="running workflow evidence")
        table.add_column("node")
        table.add_column("task")
        table.add_column("change")
        table.add_column("session")
        table.add_column("action")
        for row in running:
            table.add_row(
                str(row.get("node_id") or ""),
                str(row.get("task_id") or ""),
                str(row.get("change_id") or ""),
                str(row.get("session_id") or ""),
                str(row.get("action") or ""),
            )
        rprint(table)


def _render_task_dag_progress(payload: dict[str, Any]) -> None:
    progress = payload.get("progress") or {}
    summary = payload.get("workflow_summary") or {}
    readiness_summary = payload.get("readiness_summary") or {}
    estimated = progress.get("estimated_percent")
    estimate_text = f" (~{estimated}% active)" if estimated is not None else ""
    rprint(
        f"DAG {progress.get('completed_percent', 0)}% complete{estimate_text} · "
        f"done {progress.get('completed_nodes', 0)}/{progress.get('total_nodes', 0)} · "
        f"running {progress.get('running_nodes', 0)} · ready {progress.get('ready_nodes', 0)} · "
        f"dispatched {summary.get('dispatched_nodes', 0)} · blocked {progress.get('blocked_nodes', 0)} · "
        f"next: {progress.get('next_action') or 'none'}"
    )
    if readiness_summary.get("stale_source_plan"):
        rprint("[yellow]source plan revision is stale[/yellow]")
    blockers = payload.get("blockers") or []
    if blockers:
        table = Table(title="blockers")
        table.add_column("node")
        table.add_column("reason")
        for row in blockers:
            table.add_row(str(row.get("node_id") or ""), str(row.get("reason") or ""))
        rprint(table)


def _render_task_dag_dispatch(payload: dict[str, Any]) -> None:
    mode = payload.get("mode") or "advisory"
    dispatch_scope = payload.get("dispatch_scope") or "ready_nodes"
    rprint(f"[bold]dispatch mode:[/bold] {mode}")
    rprint(f"[bold]dispatch scope:[/bold] {dispatch_scope}")
    summary = payload.get("workflow_summary") or {}
    readiness_summary = payload.get("readiness_summary") or {}
    execution_strategy = payload.get("execution_strategy") if isinstance(payload.get("execution_strategy"), dict) else {}
    rprint(
        f"ready={summary.get('ready_nodes', 0)} dispatched={summary.get('dispatched_nodes', 0)} "
        f"running={summary.get('running_nodes', 0)} blocked={summary.get('blocked_nodes', 0)} "
        f"next: {summary.get('next_action') or 'none'}"
    )
    if execution_strategy:
        rprint(
            "[bold]strategy:[/bold] "
            f"{execution_strategy.get('default_mode')} + {execution_strategy.get('dispatch_model')} · "
            f"{execution_strategy.get('recommended_worker_sessions', 0)} fresh worker session(s); "
            "physical fan-out requires explicit opt-in"
        )
    if readiness_summary.get("stale_source_plan"):
        rprint("[yellow]source plan revision is stale[/yellow]")
    rows = payload.get("created_tasks") or payload.get("dispatchable") or []
    table = Table(title="task DAG dispatch")
    table.add_column("node")
    table.add_column("state")
    table.add_column("task")
    table.add_column("title")
    for row in rows:
        table.add_row(
            str(row.get("node_id") or ""),
            str(row.get("state") or ""),
            str(row.get("task_id") or ""),
            str(row.get("title") or ""),
        )
    rprint(table)
    created_batch_sessions = payload.get("created_batch_sessions") or []
    if created_batch_sessions:
        session_table = Table(title="compact-worker sessions")
        session_table.add_column("batch")
        session_table.add_column("session")
        session_table.add_column("nodes")
        session_table.add_column("title")
        for row in created_batch_sessions:
            session_table.add_row(
                str(row.get("batch_id") or ""),
                str(row.get("session_id") or ""),
                ",".join(str(node_id) for node_id in row.get("node_ids") or []),
                str(row.get("title") or ""),
            )
        rprint(session_table)
    dispatched = payload.get("dispatched") or []
    if dispatched:
        table = Table(title="dispatched planning")
        table.add_column("node")
        table.add_column("state")
        table.add_column("workflow")
        table.add_column("task")
        table.add_column("title")
        for row in dispatched:
            table.add_row(
                str(row.get("node_id") or ""),
                str(row.get("state") or ""),
                str(row.get("workflow_state") or ""),
                str(row.get("task_id") or ""),
                str(row.get("title") or ""),
            )
        rprint(table)
    skipped = payload.get("skipped") or []
    if skipped:
        skip_table = Table(title="skipped")
        skip_table.add_column("node")
        skip_table.add_column("reason")
        for row in skipped:
            skip_table.add_row(str(row.get("node_id") or ""), str(row.get("reason") or ""))
        rprint(skip_table)


def _render_task_dag_execute(payload: dict[str, Any]) -> None:
    mode = payload.get("mode") or "advisory"
    summary = payload.get("workflow_summary") or {}
    readiness_summary = payload.get("readiness_summary") or {}
    contract = payload.get("execute_run_contract") if isinstance(payload.get("execute_run_contract"), dict) else {}
    rprint(f"[bold]execute mode:[/bold] {mode}")
    rprint(
        f"ready={summary.get('ready_nodes', 0)} dispatched={summary.get('dispatched_nodes', 0)} "
        f"running={summary.get('running_nodes', 0)} blocked={summary.get('blocked_nodes', 0)} "
        f"completed={summary.get('completed_nodes', 0)} next: {summary.get('next_action') or 'none'}"
    )
    if readiness_summary.get("stale_source_plan"):
        rprint("[yellow]source plan revision is stale[/yellow]")
    if contract:
        rprint(
            "[bold]capability:[/bold] "
            f"{contract.get('capability_stage')} · auto-continue={contract.get('auto_continue_supported')} · "
            f"final-land={contract.get('final_land_disposition') or ('remote' if contract.get('final_remote_disposition_default') else 'local')} · "
            f"final-remote-disposition={contract.get('final_remote_disposition_default')}"
        )
        boundary = str(contract.get("current_boundary") or "").strip()
        if boundary:
            rprint(f"[bold]boundary:[/bold] {boundary}")
        next_focus_node_id = str(contract.get("next_focus_node_id") or "").strip()
        next_focus_task_id = str(contract.get("next_focus_task_id") or "").strip()
        next_focus_change_id = str(contract.get("next_focus_change_id") or "").strip()
        if next_focus_node_id or next_focus_task_id or next_focus_change_id:
            focus_label = (
                f"change {next_focus_change_id} · node {next_focus_node_id or '-'}"
                if next_focus_change_id
                else (
                    f"task {next_focus_task_id} · node {next_focus_node_id or '-'}"
                    if next_focus_task_id
                    else f"node {next_focus_node_id}"
                )
            )
            rprint(f"[bold]focus cut:[/bold] keep one active reviewable focus; next focus {focus_label}")
