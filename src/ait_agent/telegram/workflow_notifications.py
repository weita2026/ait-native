from __future__ import annotations

import json
from typing import Any, Mapping


_GRAPH_WATCH_MISSING_COUNT_KEY = "missing_graph_count"
_GRAPH_WATCH_LAST_MISSING_AT_KEY = "last_missing_graph_at"
_PRIMARY_GATE_SECTION_TITLES = {
    "attestation": "Attestation",
    "ci": "CI",
    "policy": "Policy",
    "review": "Review",
    "freshness": "Stale base",
    "other": "Other blockers",
}
_PRIMARY_GATE_SECTION_ORDER = ("attestation", "ci", "policy", "review", "freshness", "other")


def _task_url(config: Any, task_id: str | None) -> str | None:
    if not getattr(config, "ait_web_url", None) or not task_id:
        return None
    return f"{config.ait_web_url}/tasks/{task_id}"


def _change_url(config: Any, change_id: str | None) -> str | None:
    if not getattr(config, "ait_web_url", None) or not change_id:
        return None
    return f"{config.ait_web_url}/changes/{change_id}"


def _task_queue_items(payload: dict[str, Any], *states: str) -> list[dict[str, Any]]:
    allowed = set(states)
    items = list(payload.get("items") or [])
    if not allowed:
        return items
    return [item for item in items if str((item.get("workflow") or {}).get("state") or "") in allowed]


def _task_list_lines(
    items: list[dict[str, Any]],
    *,
    limit: int = 3,
    include_state_and_next: bool = True,
) -> list[str]:
    lines: list[str] = []
    for item in items[:limit]:
        task = item.get("task") or {}
        workflow = item.get("workflow") or {}
        next_action = item.get("next_action") or {}
        detail = str(workflow.get("reason") or next_action.get("detail") or "").strip()
        lines.append(f"• {task.get('task_id')} · {task.get('title')}")
        if include_state_and_next:
            next_label = str(next_action.get("label") or next_action.get("code") or "inspect").strip()
            lines.append(f"  state={workflow.get('state')} · next={next_label}")
        ci_summary_line = _task_ci_summary_line(item)
        if ci_summary_line:
            lines.append(f"  {ci_summary_line}")
        if detail:
            lines.append(f"  {detail}")
    if len(items) > limit:
        lines.append(f"… and {len(items) - limit} more")
    return lines


def _primary_gate_key(item: Mapping[str, Any]) -> str:
    gate = str(item.get("primary_gate") or "").strip().lower()
    return gate if gate in _PRIMARY_GATE_SECTION_TITLES else "other"


def _tg1_count_label(summary: Mapping[str, Any]) -> str | None:
    live_count = summary.get("live_count")
    minimum_count = summary.get("minimum_count")
    if live_count is None and minimum_count is None:
        return None
    return f"{live_count if live_count is not None else '?'}/{minimum_count if minimum_count is not None else '?'}"


def _task_ci_summary_line(item: Mapping[str, Any]) -> str | None:
    ci_summary = item.get("ci_summary") if isinstance(item.get("ci_summary"), Mapping) else {}
    focus_change = item.get("focus_change") if isinstance(item.get("focus_change"), Mapping) else {}
    parts: list[str] = []
    patchset_id = str(ci_summary.get("patchset_id") or focus_change.get("patchset_id") or "").strip()
    if patchset_id:
        parts.append(f"patchset={patchset_id}")
    tg1_required = ci_summary.get("tg1_required") if isinstance(ci_summary.get("tg1_required"), Mapping) else {}
    tg1_status = str(tg1_required.get("status") or "").strip()
    if tg1_status:
        tg1_label = f"TG-1={tg1_status}"
        count_label = _tg1_count_label(tg1_required)
        if count_label:
            tg1_label += f" {count_label}"
        parts.append(tg1_label)
    else:
        tests_status = str(ci_summary.get("tests_status") or "").strip()
        if tests_status:
            parts.append(f"CI={tests_status}")
    remote_land_gate = str(ci_summary.get("remote_land_gate") or "").strip()
    if remote_land_gate:
        parts.append(f"land={remote_land_gate}")
    return " · ".join(parts) if parts else None


def _attention_sections(attention_items: list[dict[str, Any]]) -> list[tuple[str, list[str]]]:
    sections: list[tuple[str, list[str]]] = []
    grouped = {key: [] for key in _PRIMARY_GATE_SECTION_ORDER}
    for item in attention_items:
        grouped[_primary_gate_key(item)].append(item)
    for gate in _PRIMARY_GATE_SECTION_ORDER:
        gate_items = grouped[gate]
        if gate_items:
            sections.append((_PRIMARY_GATE_SECTION_TITLES[gate], _task_list_lines(gate_items, include_state_and_next=False)))
    return sections


def _workflow_notification_body_lines(payload: dict[str, Any]) -> list[str]:
    sections: list[tuple[str, list[str]]] = []
    attention_items = _task_queue_items(payload, "attention_required")
    ready_land_items = _task_queue_items(payload, "ready_to_land")
    ready_complete_items = _task_queue_items(payload, "ready_to_complete")
    if attention_items:
        sections.extend(_attention_sections(attention_items))
    if ready_land_items:
        sections.append(("Ready to land", _task_list_lines(ready_land_items, include_state_and_next=False)))
    if ready_complete_items:
        sections.append(("Ready to complete", _task_list_lines(ready_complete_items, include_state_and_next=False)))
    lines: list[str] = []
    for index, (title, item_lines) in enumerate(sections):
        if index:
            lines.append("")
        lines.append(title)
        lines.extend(item_lines)
    return lines


def _queue_digest(payload: dict[str, Any]) -> str:
    body_lines = _workflow_notification_body_lines(payload)
    digest_payload = {
        "actionable": bool(body_lines),
        "lines": body_lines,
    }
    return json.dumps(digest_payload, ensure_ascii=False, sort_keys=True)


def _queue_digest_actionable(raw: str | None) -> bool:
    if not raw:
        return False
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError:
        return False
    return bool(payload.get("actionable"))


def _graph_watches(link: dict[str, Any] | None) -> dict[str, dict[str, Any]]:
    raw = (link or {}).get("graph_watches")
    if not isinstance(raw, dict):
        return {}
    return {str(key): dict(value) for key, value in raw.items() if isinstance(value, dict)}


def _graph_watch_missing_file_digest(graph_path: str) -> str:
    return f"error:Task graph JSON not found: {graph_path}"


def _graph_watch_missing_file_count(watch: Mapping[str, Any] | None) -> int:
    if not isinstance(watch, Mapping):
        return 0
    try:
        value = int(watch.get(_GRAPH_WATCH_MISSING_COUNT_KEY) or 0)
    except (TypeError, ValueError):
        return 0
    return max(value, 0)


def _graph_watch_clear_missing_file_state(watch: Mapping[str, Any]) -> dict[str, Any]:
    updated = dict(watch)
    updated.pop(_GRAPH_WATCH_MISSING_COUNT_KEY, None)
    updated.pop(_GRAPH_WATCH_LAST_MISSING_AT_KEY, None)
    return updated


def _graph_watch_mark_missing_file(
    watch: Mapping[str, Any],
    *,
    graph_path: str,
    observed_at: str,
) -> dict[str, Any]:
    updated = dict(watch)
    updated[_GRAPH_WATCH_MISSING_COUNT_KEY] = _graph_watch_missing_file_count(watch) + 1
    updated[_GRAPH_WATCH_LAST_MISSING_AT_KEY] = observed_at
    updated["last_progress_digest"] = _graph_watch_missing_file_digest(graph_path)
    updated["last_progress_notification_at"] = observed_at
    return updated


def _progress_payload(payload: dict[str, Any]) -> dict[str, Any]:
    progress = payload.get("progress") if isinstance(payload, dict) else {}
    return progress if isinstance(progress, dict) else {}


def _graph_run_payload(payload: dict[str, Any]) -> dict[str, Any]:
    latest_graph_run = payload.get("latest_graph_run") if isinstance(payload, dict) else {}
    return latest_graph_run if isinstance(latest_graph_run, dict) else {}


def _format_percent(value: Any) -> str:
    if value is None:
        return "?"
    try:
        number = float(value)
    except (TypeError, ValueError):
        return str(value)
    if number.is_integer():
        return str(int(number))
    return f"{number:.1f}".rstrip("0").rstrip(".")


def _graph_next_action_from_progress(progress: dict[str, Any]) -> str:
    next_action = str(progress.get("next_action") or "").strip()
    if next_action:
        return next_action
    try:
        completed = float(progress.get("completed_percent") or 0) >= 100
    except (TypeError, ValueError):
        completed = False
    if completed:
        return "complete task graph"
    return "inspect graph"


def _graph_count_value(value: Any) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _graph_latest_graph_run_action_is_stale(progress: dict[str, Any], latest_graph_run: dict[str, Any]) -> bool:
    execution_state = str(latest_graph_run.get("execution_state") or "").strip().lower()
    if execution_state and execution_state != "active":
        return False
    if str(latest_graph_run.get("pause_reason") or "").strip():
        return False
    gate_handoff = latest_graph_run.get("gate_handoff")
    if isinstance(gate_handoff, dict) and gate_handoff:
        return False
    workflow_summary = latest_graph_run.get("workflow_summary") if isinstance(latest_graph_run.get("workflow_summary"), dict) else {}
    if not workflow_summary:
        return False
    for key in ("completed_nodes", "ready_nodes", "running_nodes", "blocked_nodes", "total_nodes"):
        progress_value = _graph_count_value(progress.get(key))
        summary_value = _graph_count_value(workflow_summary.get(key))
        if progress_value is not None and summary_value is not None and progress_value != summary_value:
            return True
    return False


def _graph_effective_next_action_from_payload(payload: dict[str, Any]) -> str:
    progress = _progress_payload(payload)
    progress_action = _graph_next_action_from_progress(progress)
    latest_graph_run = _graph_run_payload(payload)
    run_action = str(latest_graph_run.get("next_action") or "").strip()
    if not run_action:
        return progress_action
    if not progress_action or progress_action == run_action:
        return run_action
    if _graph_latest_graph_run_action_is_stale(progress, latest_graph_run):
        return progress_action
    return run_action


def _graph_next_action_from_payload(payload: dict[str, Any]) -> str:
    return _graph_effective_next_action_from_payload(payload)


def _graph_progress_digest(payload: dict[str, Any]) -> str:
    progress = _progress_payload(payload)
    latest_graph_run = _graph_run_payload(payload)
    node_states = progress.get("node_states") if isinstance(progress.get("node_states"), dict) else {}
    digest_nodes = {
        str(node_id): str((node or {}).get("state") or "")
        for node_id, node in sorted(node_states.items())
        if isinstance(node, dict)
    }
    digest_payload = {
        "completed_percent": progress.get("completed_percent"),
        "estimated_percent": progress.get("estimated_percent"),
        "completed_nodes": progress.get("completed_nodes"),
        "ready_nodes": progress.get("ready_nodes"),
        "running_nodes": progress.get("running_nodes"),
        "blocked_nodes": progress.get("blocked_nodes"),
        "next_action": _graph_next_action_from_payload(payload),
        "node_states": digest_nodes,
        "latest_graph_run": {
            "session_id": latest_graph_run.get("session_id"),
            "session_local_id": latest_graph_run.get("session_local_id"),
            "repo_name": latest_graph_run.get("repo_name"),
            "repo_id": latest_graph_run.get("repo_id"),
            "graph_run_id": latest_graph_run.get("graph_run_id"),
            "execution_state": latest_graph_run.get("execution_state"),
            "pause_reason": latest_graph_run.get("pause_reason"),
            "next_action": latest_graph_run.get("next_action"),
            "gate_handoff": latest_graph_run.get("gate_handoff"),
        }
        if latest_graph_run
        else None,
    }
    return json.dumps(digest_payload, ensure_ascii=False, sort_keys=True)


def _graph_last_next_action_from_watch(watch: dict[str, Any]) -> str:
    last_action = str(watch.get("last_next_action") or "").strip()
    if last_action:
        return last_action
    digest = str(watch.get("last_progress_digest") or "").strip()
    if digest.startswith("{"):
        try:
            payload = json.loads(digest)
        except json.JSONDecodeError:
            payload = {}
        if isinstance(payload, dict):
            digest_action = str(payload.get("next_action") or "").strip()
            if digest_action:
                return digest_action
    return "update graph"


def _format_graph_progress_percent(progress: dict[str, Any]) -> str:
    label = _format_percent(progress.get("completed_percent"))
    suffix = "" if label.endswith("%") else "%"
    return f"{label}{suffix}"


def _format_graph_progress_lines(progress: dict[str, Any]) -> list[str]:
    completed_percent = progress.get("completed_percent")
    estimated_percent = progress.get("estimated_percent")
    lines = [
        "completed_percent="
        f"{_format_percent(completed_percent)} "
        f"completed={progress.get('completed_nodes', 0)} "
        f"ready={progress.get('ready_nodes', 0)} "
        f"running={progress.get('running_nodes', 0)} "
        f"blocked={progress.get('blocked_nodes', 0)}"
    ]
    if estimated_percent is not None:
        lines.append(f"estimated_percent={_format_percent(estimated_percent)}")
    next_action = str(progress.get("next_action") or "").strip()
    if next_action:
        lines.append(f"next_action={next_action}")
    try:
        completed = float(completed_percent or 0) >= 100
    except (TypeError, ValueError):
        completed = False
    if completed:
        lines.append("Graph completed.")
    return lines


def _format_graph_run_lines(payload: dict[str, Any]) -> list[str]:
    latest_graph_run = _graph_run_payload(payload)
    if not latest_graph_run:
        return []
    lines = [f"execution_state={latest_graph_run.get('execution_state') or 'active'}"]
    run_session = str(latest_graph_run.get("session_id") or "").strip()
    if run_session:
        lines.append(f"run_session={run_session}")
    session_local_id = str(latest_graph_run.get("session_local_id") or "").strip()
    if session_local_id:
        lines.append(f"run_session_local_id={session_local_id}")
    pause_reason = str(latest_graph_run.get("pause_reason") or "").strip()
    if pause_reason:
        lines.append(f"pause_reason={pause_reason}")
    next_action = str(latest_graph_run.get("next_action") or "").strip()
    if next_action:
        lines.append(f"run_next_action={next_action}")
    gate_handoff = latest_graph_run.get("gate_handoff") if isinstance(latest_graph_run.get("gate_handoff"), dict) else {}
    if gate_handoff:
        lines.append(f"gate_handoff={gate_handoff.get('kind') or 'pending'}")
        required_gates = [str(value).strip() for value in gate_handoff.get("required_gates") or [] if str(value).strip()]
        if required_gates:
            lines.append(f"required_gates={','.join(required_gates)}")
        if gate_handoff.get("promotion_required"):
            lines.append("promotion_required=true")
    return lines


def _format_graph_run_suffix(payload: dict[str, Any]) -> str:
    latest_graph_run = _graph_run_payload(payload)
    if not latest_graph_run:
        return ""
    parts = [f"state={latest_graph_run.get('execution_state') or 'active'}"]
    pause_reason = str(latest_graph_run.get("pause_reason") or "").strip()
    if pause_reason:
        parts.append(f"pause={pause_reason}")
    gate_handoff = latest_graph_run.get("gate_handoff") if isinstance(latest_graph_run.get("gate_handoff"), dict) else {}
    if gate_handoff:
        parts.append(f"gate={gate_handoff.get('kind') or 'pending'}")
    return " · " + " · ".join(parts)


def _format_graph_progress_action_transition(current_action: str, next_action: str) -> str:
    current_text = str(current_action or "").strip()
    next_text = str(next_action or "").strip()
    if current_text.lower().startswith("start ") and next_text.lower().startswith("start "):
        current_text = current_text[6:].strip() or current_text
        next_text = next_text[6:].strip() or next_text
    return f"{current_text} —> {next_text}"


def format_graph_start_notification(config: Any, watch: dict[str, Any], payload: dict[str, Any]) -> str:
    progress = _progress_payload(payload)
    plan_id = str(watch.get("plan_id") or "").strip()
    graph_id = str(watch.get("graph_id") or progress.get("graph_id") or "").strip()
    lines = [
        "ait graph start",
        f"repo={getattr(config, 'repo_name', None) or 'unknown'}",
        f"plan={plan_id or 'unknown'}",
        f"graph={graph_id or 'unknown'}",
    ]
    lines.extend(_format_graph_progress_lines(progress))
    lines.extend(_format_graph_run_lines(payload))
    return "\n".join(lines)


def format_graph_progress_notification(_config: Any, watch: dict[str, Any], payload: dict[str, Any]) -> str:
    progress = _progress_payload(payload)
    current_action = _graph_last_next_action_from_watch(watch)
    next_action = _graph_next_action_from_payload(payload)
    percent = _format_graph_progress_percent(progress)
    transition = _format_graph_progress_action_transition(current_action, next_action)
    return f"{transition} ({percent}){_format_graph_run_suffix(payload)}"


def format_graph_watch_status(config: Any, link: dict[str, Any]) -> str:
    watches = _graph_watches(link)
    lines = [
        f"graph_watches={len(watches)}",
        "delivery=ait_server_triggered",
        f"background_graph_sweep={'on' if getattr(config, 'graph_watch_background_sweep_enabled', False) else 'off'}",
        "missing_file_behavior=warn_once_per_streak",
    ]
    if not watches:
        lines.append("No graph watches are enabled for this chat.")
    for watch in watches.values():
        plan_id = watch.get("plan_id") or "unknown"
        graph_path = watch.get("graph_path") or "missing path"
        lines.append(f"- {plan_id} · {graph_path}")
        missing_count = _graph_watch_missing_file_count(watch)
        if missing_count > 0:
            last_missing_at = str(watch.get(_GRAPH_WATCH_LAST_MISSING_AT_KEY) or "").strip() or "unknown"
            lines.append(f"  missing_streak={missing_count} last_missing_at={last_missing_at}")
    lines.append("ait-server workflow mutations send delivery when the progress digest changes.")
    if not getattr(config, "graph_watch_background_sweep_enabled", False):
        lines.append("Telegram background sync does not poll graph progress by default.")
    return "\n".join(lines)


def format_graph_watch_missing_file_notification(
    config: Any,
    watch: Mapping[str, Any],
    *,
    graph_path: str,
    missing_count: int,
) -> str:
    return "\n".join(
        [
            f"ait graph watch missing · repo={getattr(config, 'repo_name', '')}",
            f"plan={watch.get('plan_id') or 'unknown'}",
            f"file={graph_path}",
            f"missing_streak={missing_count}",
            "The watch stays registered; restore the graph file or disable the watch when it is no longer needed.",
        ]
    )


def format_queue_summary(config: Any, payload: dict[str, Any]) -> str:
    summary = payload.get("summary") or {}
    attention_items = _task_queue_items(payload, "attention_required")
    ready_land_items = _task_queue_items(payload, "ready_to_land")
    ready_complete_items = _task_queue_items(payload, "ready_to_complete")
    other_items = [
        item
        for item in _task_queue_items(payload)
        if item not in attention_items and item not in ready_land_items and item not in ready_complete_items
    ]
    lines = [
        f"ait queue · repo={getattr(config, 'repo_name', '')}",
        f"active={summary.get('active', 0)} attention={summary.get('attention_required', 0)} ready_to_land={summary.get('ready_to_land', 0)} ready_to_complete={summary.get('ready_to_complete', 0)}",
    ]
    if not (attention_items or ready_land_items or ready_complete_items or other_items):
        lines.append("No active tasks.")
        return "\n".join(lines)
    if attention_items:
        for title, item_lines in _attention_sections(attention_items):
            lines.extend(["", title, *item_lines])
    if ready_land_items:
        lines.extend(["", "Ready to land", *_task_list_lines(ready_land_items)])
    if ready_complete_items:
        lines.extend(["", "Ready to complete", *_task_list_lines(ready_complete_items)])
    if other_items:
        lines.extend(["", "Other active tasks", *_task_list_lines(other_items, limit=2)])
    return "\n".join(lines)


def format_attention_summary(config: Any, payload: dict[str, Any]) -> str:
    items = _task_queue_items(payload, "attention_required")
    lines = [
        f"ait attention · repo={getattr(config, 'repo_name', '')}",
        f"attention={len(items)}",
    ]
    if not items:
        lines.append("No active tasks currently need attention.")
        return "\n".join(lines)
    for title, item_lines in _attention_sections(items):
        lines.extend(["", title, *item_lines])
    return "\n".join(lines)


def format_ready_summary(config: Any, payload: dict[str, Any]) -> str:
    ready_land_items = _task_queue_items(payload, "ready_to_land")
    ready_complete_items = _task_queue_items(payload, "ready_to_complete")
    lines = [
        f"ait ready · repo={getattr(config, 'repo_name', '')}",
        f"ready_to_land={len(ready_land_items)} ready_to_complete={len(ready_complete_items)}",
    ]
    if not (ready_land_items or ready_complete_items):
        lines.append("No active tasks are ready to land or complete.")
        return "\n".join(lines)
    if ready_land_items:
        lines.extend(["", "Ready to land", *_task_list_lines(ready_land_items)])
    if ready_complete_items:
        lines.extend(["", "Ready to complete", *_task_list_lines(ready_complete_items)])
    return "\n".join(lines)


def format_task_summary(config: Any, detail: dict[str, Any]) -> str:
    task = detail.get("task") or {}
    workflow = detail.get("workflow") or {}
    changes = detail.get("changes") or []
    next_action = detail.get("next_action") or {}
    lines = [
        f"{task.get('task_id')} · {task.get('title')}",
        f"status={task.get('status')} risk={task.get('risk_tier')} workflow={workflow.get('state')}",
        f"intent={task.get('intent')}",
        f"linked_changes={len(changes)} · next={next_action.get('code') or 'open_task'}",
    ]
    url = _task_url(config, task.get("task_id"))
    if url:
        lines.append(url)
    return "\n".join(lines)


def format_change_summary(config: Any, detail: dict[str, Any]) -> str:
    change = detail.get("change") or {}
    task = detail.get("task") or {}
    current_patchset = detail.get("current_patchset") or {}
    policy = detail.get("policy_summary") or {}
    reviews = detail.get("review_summary") or {}
    lines = [
        f"{change.get('change_id')} · {change.get('title')}",
        f"status={change.get('status')} lane={change.get('lane')} risk={change.get('risk_tier')}",
        f"task={task.get('task_id')} · patchset={current_patchset.get('patchset_id') or 'none'} · policy={policy.get('decision', 'pending')}",
        f"approvals={reviews.get('approvals', 0)} blocking={reviews.get('blocking', 0)} comments={reviews.get('comments', 0)}",
    ]
    url = _change_url(config, change.get("change_id"))
    if url:
        lines.append(url)
    return "\n".join(lines)


def format_task_audit_summary(config: Any, detail: dict[str, Any]) -> str:
    task = detail.get("task") or {}
    workflow = detail.get("workflow") or {}
    summary = detail.get("summary") or {}
    target = detail.get("target") or {}
    recommended = detail.get("recommended_action") or {}
    changes = list(detail.get("changes") or [])
    lines = [
        f"{task.get('task_id')} · {task.get('title')}",
        f"workflow={workflow.get('state')} verdict={summary.get('verdict')} target={target.get('line_name') or 'main'}",
        f"open_changes={summary.get('open_change_count', 0)} landed={summary.get('landed_change_count', 0)} on_target={summary.get('effective_on_target_change_count', 0)}",
        f"recommended={recommended.get('label') or recommended.get('code') or 'inspect'}",
    ]
    detail_text = str(recommended.get("detail") or workflow.get("reason") or "").strip()
    if detail_text:
        lines.append(detail_text)
    if changes:
        lines.extend(["", "Linked changes"])
        for row in changes[:3]:
            change = row.get("change") or {}
            lines.append(f"• {change.get('change_id')} · status={change.get('status')} · target={row.get('target_state')}")
    url = _task_url(config, task.get("task_id"))
    if url:
        lines.append(url)
    return "\n".join(lines)


def _change_land_readiness(detail: dict[str, Any]) -> tuple[str, str]:
    change = detail.get("change") or {}
    current_patchset = detail.get("current_patchset") or {}
    policy = detail.get("policy_summary") or {}
    reviews = detail.get("review_summary") or {}
    freshness = detail.get("freshness") or {}
    if change.get("status") == "landed":
        return ("landed", "Change is already landed.")
    if not current_patchset:
        return ("no_patchset", "Publish and select a patchset before landing.")
    if not bool(freshness.get("base_is_fresh")):
        return ("stale_base", "Refresh or restack onto the current base head before landing.")
    if change.get("status") == "blocked" or int(reviews.get("blocking") or 0) > 0:
        return ("blocked", "Resolve blocking review feedback before landing.")
    if str(policy.get("decision") or "pending") != "pass":
        return ("policy_pending", "Wait for required policy or validation checks to pass.")
    if change.get("status") in {"review", "gated", "approved", "landable"}:
        return ("ready_to_land", "Selected patchset looks landable on the current base.")
    return ("not_ready", "Move the change toward review and approval first.")


def format_change_land_summary(config: Any, detail: dict[str, Any]) -> str:
    change = detail.get("change") or {}
    task = detail.get("task") or {}
    current_patchset = detail.get("current_patchset") or {}
    reviews = detail.get("review_summary") or {}
    policy = detail.get("policy_summary") or {}
    freshness = detail.get("freshness") or {}
    readiness, reason = _change_land_readiness(detail)
    lines = [
        f"{change.get('change_id')} · {change.get('title')}",
        f"land_state={readiness} status={change.get('status')} task={task.get('task_id')}",
        f"patchset={current_patchset.get('patchset_id') or 'none'} policy={policy.get('decision', 'pending')} base_fresh={bool(freshness.get('base_is_fresh'))}",
        f"approvals={reviews.get('approvals', 0)} blocking={reviews.get('blocking', 0)} comments={reviews.get('comments', 0)}",
        reason,
    ]
    url = _change_url(config, change.get("change_id"))
    if url:
        lines.append(url)
    return "\n".join(lines)


def format_workflow_notification(config: Any, payload: dict[str, Any]) -> str:
    lines = [f"workflow ({getattr(config, 'repo_name', '')})"]
    body_lines = _workflow_notification_body_lines(payload)
    if body_lines:
        lines.extend(["", *body_lines])
    else:
        lines.extend(["", "Complete"])
    return "\n".join(lines)
