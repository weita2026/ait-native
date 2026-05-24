from __future__ import annotations

import re
from typing import Any, Mapping, Sequence

WORKFLOW_SEGMENT_CLASS_PLANNING = "planning"
WORKFLOW_SEGMENT_CLASS_TASK_EXECUTION = "task_execution"
WORKFLOW_SEGMENT_CLASS_CHANGE_LAND = "change_land"
WORKFLOW_BOUNDARY_CLOSE_AFTER = "close_after"

_WORKFLOW_CONTEXT_VERSION = 1
_ATTACHMENT_HINT_KEYS = (
    "plan_id",
    "planning_session_id",
    "task_id",
    "change_id",
    "plan_item_ref",
    "task_graph_json",
    "task_graph_id",
    "graph_id",
    "graph_run_id",
    "node_id",
    "node_key",
    "patchset_id",
    "selected_patchset_id",
    "land_id",
)
_GRAPH_ATTACHMENT_HINT_KEYS = {
    "plan_item_ref",
    "task_graph_json",
    "task_graph_id",
    "graph_id",
    "graph_run_id",
    "node_id",
    "node_key",
}
_SIGNAL_PRIORITY = {
    WORKFLOW_SEGMENT_CLASS_PLANNING: 1,
    WORKFLOW_SEGMENT_CLASS_TASK_EXECUTION: 2,
    WORKFLOW_SEGMENT_CLASS_CHANGE_LAND: 3,
}
_SIGNAL_SPECS: tuple[tuple[str, str, str, tuple[re.Pattern[str], ...]], ...] = (
    (
        "plan_sync",
        WORKFLOW_SEGMENT_CLASS_PLANNING,
        WORKFLOW_BOUNDARY_CLOSE_AFTER,
        (
            re.compile(r"\bait\s+plan\s+sync\b"),
            re.compile(r"\bplan\s+sync\b"),
            re.compile(r"\bremote\s+sync\b"),
            re.compile(r"\blocal\s+sync\b"),
        ),
    ),
    (
        "land",
        WORKFLOW_SEGMENT_CLASS_CHANGE_LAND,
        WORKFLOW_BOUNDARY_CLOSE_AFTER,
        (
            re.compile(r"\bait\s+workflow\s+land-local\b"),
            re.compile(r"\bait\s+workflow\s+land\b"),
            re.compile(r"\bait\s+land\s+submit\b"),
            re.compile(r"\bremote\s+land\b"),
            re.compile(r"\blocal\s+land\b"),
            re.compile(r"\bland\s+submit\b"),
        ),
    ),
)


def infer_workflow_boundary_signals(text: str | None) -> list[dict[str, Any]]:
    normalized = _normalize_text(text)
    if not normalized:
        return []
    signals: list[dict[str, Any]] = []
    for kind, segment_class, boundary_behavior, patterns in _SIGNAL_SPECS:
        if any(pattern.search(normalized) for pattern in patterns):
            signals.append(
                {
                    "kind": kind,
                    "segment_class": segment_class,
                    "boundary_behavior": boundary_behavior,
                    "source": "text",
                }
            )
    return signals


def infer_workflow_attachment_hints(session: Mapping[str, Any] | None) -> dict[str, str]:
    if not isinstance(session, Mapping):
        return {}
    hints: dict[str, str] = {}
    for key in ("plan_id", "planning_session_id", "task_id", "change_id"):
        value = _normalize_scalar(session.get(key))
        if value:
            hints[key] = value
    metadata = session.get("metadata")
    if isinstance(metadata, Mapping):
        for key in _ATTACHMENT_HINT_KEYS:
            if key in hints:
                continue
            value = _normalize_scalar(metadata.get(key))
            if value:
                hints[key] = value
    return hints


def infer_workflow_context(
    *,
    text: str | None,
    session: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    signals = infer_workflow_boundary_signals(text)
    attachment_hints = infer_workflow_attachment_hints(session)
    segment_class = _segment_class_from_signals(signals) or _segment_class_from_attachment_hints(attachment_hints)
    if not signals and not attachment_hints and not segment_class:
        return {}
    payload: dict[str, Any] = {
        "version": _WORKFLOW_CONTEXT_VERSION,
        "attachment_hints": attachment_hints,
        "durable_objects": _durable_objects_from_attachment_hints(attachment_hints),
        "signals": signals,
    }
    if segment_class:
        payload["segment_class"] = segment_class
        payload["classification_source"] = "boundary_signals" if signals else "attachment_hints"
    return payload


def summarize_workflow_segments(
    events: Sequence[Mapping[str, Any]],
    *,
    session: Mapping[str, Any] | None = None,
) -> list[dict[str, Any]]:
    segments: list[dict[str, Any]] = []
    current_events: list[dict[str, Any]] = []
    current_contexts: list[dict[str, Any]] = []

    for event in events:
        event_context = _workflow_context_from_event(event, session=session)
        if not current_events and not _event_can_open_segment(event, event_context):
            continue
        current_events.append(dict(event))
        current_contexts.append(event_context)
        if _has_boundary_behavior(event_context, WORKFLOW_BOUNDARY_CLOSE_AFTER):
            segments.append(_build_segment_summary(segments, current_events, current_contexts, status="closed"))
            current_events = []
            current_contexts = []

    if current_events:
        segments.append(_build_segment_summary(segments, current_events, current_contexts, status="open"))
    return segments


def resolve_workflow_segment_attachment(
    segment: Mapping[str, Any],
    *,
    plans: Sequence[Mapping[str, Any]] = (),
    tasks: Sequence[Mapping[str, Any]] = (),
    changes: Sequence[Mapping[str, Any]] = (),
) -> dict[str, Any]:
    durable_objects = [dict(row) for row in segment.get("durable_objects", []) if isinstance(row, Mapping)]
    segment_class = _normalize_scalar(segment.get("segment_class"))
    if durable_objects:
        primary_target = _primary_target_for_segment_class(segment_class, durable_objects) or durable_objects[0]
        return {
            "status": "attached",
            "confidence": "high",
            "primary_target": primary_target,
            "supporting_targets": [row for row in durable_objects if row != primary_target],
            "reason": "durable object hints already identify the workflow attachment",
        }

    candidates = _candidate_targets_for_segment_class(
        segment_class,
        plans=plans,
        tasks=tasks,
        changes=changes,
    )
    if len(candidates) == 1:
        return {
            "status": "attached",
            "confidence": "medium",
            "primary_target": candidates[0],
            "supporting_targets": [],
            "reason": "exactly one active workflow target matches the segment class",
        }
    if len(candidates) > 1:
        return {
            "status": "ambiguous",
            "confidence": "low",
            "primary_target": None,
            "supporting_targets": [],
            "reason": "multiple active workflow targets match the segment class",
            "clarifying_question": _clarifying_question_for_segment_class(segment_class),
            "candidate_targets": candidates,
        }
    return {
        "status": "unresolved",
        "confidence": "low",
        "primary_target": None,
        "supporting_targets": [],
        "reason": "no durable hints or active workflow targets matched the segment class",
    }


def _segment_class_from_signals(signals: list[dict[str, Any]]) -> str | None:
    best_class: str | None = None
    best_priority = -1
    for row in signals:
        candidate = _normalize_scalar(row.get("segment_class"))
        if not candidate:
            continue
        priority = _SIGNAL_PRIORITY.get(candidate, 0)
        if priority > best_priority:
            best_class = candidate
            best_priority = priority
    return best_class


def _segment_class_from_attachment_hints(attachment_hints: Mapping[str, Any]) -> str | None:
    if _normalize_scalar(attachment_hints.get("land_id")):
        return WORKFLOW_SEGMENT_CLASS_CHANGE_LAND
    if _normalize_scalar(attachment_hints.get("selected_patchset_id")):
        return WORKFLOW_SEGMENT_CLASS_CHANGE_LAND
    if _normalize_scalar(attachment_hints.get("patchset_id")):
        return WORKFLOW_SEGMENT_CLASS_CHANGE_LAND
    if _normalize_scalar(attachment_hints.get("change_id")):
        return WORKFLOW_SEGMENT_CLASS_TASK_EXECUTION
    if _normalize_scalar(attachment_hints.get("task_id")):
        return WORKFLOW_SEGMENT_CLASS_TASK_EXECUTION
    if any(_normalize_scalar(attachment_hints.get(key)) for key in _GRAPH_ATTACHMENT_HINT_KEYS):
        return WORKFLOW_SEGMENT_CLASS_TASK_EXECUTION
    if _normalize_scalar(attachment_hints.get("planning_session_id")):
        return WORKFLOW_SEGMENT_CLASS_PLANNING
    if _normalize_scalar(attachment_hints.get("plan_id")):
        return WORKFLOW_SEGMENT_CLASS_PLANNING
    return None


def _durable_objects_from_attachment_hints(attachment_hints: Mapping[str, Any]) -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    for key, object_type in (
        ("plan_id", "plan"),
        ("planning_session_id", "planning_session"),
        ("task_id", "task"),
        ("change_id", "change"),
        ("selected_patchset_id", "patchset"),
        ("patchset_id", "patchset"),
        ("land_id", "land"),
    ):
        object_id = _normalize_scalar(attachment_hints.get(key))
        if object_id is None:
            continue
        rows.append({"object_type": object_type, "object_id": object_id})
    return rows


def _primary_target_for_segment_class(
    segment_class: str | None,
    durable_objects: Sequence[Mapping[str, Any]],
) -> dict[str, str] | None:
    preferred_types = {
        WORKFLOW_SEGMENT_CLASS_PLANNING: ("planning_session", "plan"),
        WORKFLOW_SEGMENT_CLASS_TASK_EXECUTION: ("change", "task"),
        WORKFLOW_SEGMENT_CLASS_CHANGE_LAND: ("change", "patchset", "land"),
    }.get(segment_class, ())
    for object_type in preferred_types:
        for row in durable_objects:
            if _normalize_scalar(row.get("object_type")) == object_type:
                return {"object_type": object_type, "object_id": str(row.get("object_id") or "")}
    return None


def _candidate_targets_for_segment_class(
    segment_class: str | None,
    *,
    plans: Sequence[Mapping[str, Any]],
    tasks: Sequence[Mapping[str, Any]],
    changes: Sequence[Mapping[str, Any]],
) -> list[dict[str, str]]:
    if segment_class == WORKFLOW_SEGMENT_CLASS_PLANNING:
        return [
            {"object_type": "plan", "object_id": str(row.get("plan_id") or "")}
            for row in plans
            if str(row.get("plan_id") or "").strip()
        ]
    if segment_class == WORKFLOW_SEGMENT_CLASS_CHANGE_LAND:
        return [
            {"object_type": "change", "object_id": str(row.get("change_id") or "")}
            for row in changes
            if str(row.get("change_id") or "").strip() and str(row.get("status") or "").strip() not in {"landed", "archived"}
        ]
    if segment_class == WORKFLOW_SEGMENT_CLASS_TASK_EXECUTION:
        return [
            {"object_type": "task", "object_id": str(row.get("task_id") or "")}
            for row in tasks
            if str(row.get("task_id") or "").strip() and str(row.get("status") or "").strip() in {"active", "planned"}
        ]
    return []


def _clarifying_question_for_segment_class(segment_class: str | None) -> str:
    if segment_class == WORKFLOW_SEGMENT_CLASS_PLANNING:
        return "這段要接續哪個 plan?"
    if segment_class == WORKFLOW_SEGMENT_CLASS_CHANGE_LAND:
        return "這次要 land 哪個 change?"
    return "這段要接續哪個 task?"


def _workflow_context_from_event(
    event: Mapping[str, Any],
    *,
    session: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    payload = event.get("payload")
    if not isinstance(payload, Mapping):
        return {}
    existing = payload.get("workflow_context")
    if isinstance(existing, Mapping):
        return dict(existing)
    text = _normalize_scalar(payload.get("text"))
    if text is None:
        return {}
    return infer_workflow_context(text=text, session=session)


def _event_can_open_segment(event: Mapping[str, Any], event_context: Mapping[str, Any]) -> bool:
    if _has_explicit_workflow_context(event):
        return True
    return bool(event_context.get("signals"))


def _has_explicit_workflow_context(event: Mapping[str, Any]) -> bool:
    payload = event.get("payload")
    return isinstance(payload, Mapping) and isinstance(payload.get("workflow_context"), Mapping)


def _has_boundary_behavior(workflow_context: Mapping[str, Any], boundary_behavior: str) -> bool:
    signals = workflow_context.get("signals")
    if not isinstance(signals, Sequence):
        return False
    return any(
        isinstance(signal, Mapping) and _normalize_scalar(signal.get("boundary_behavior")) == boundary_behavior
        for signal in signals
    )


def _build_segment_summary(
    existing_segments: Sequence[Mapping[str, Any]],
    events: Sequence[Mapping[str, Any]],
    contexts: Sequence[Mapping[str, Any]],
    *,
    status: str,
) -> dict[str, Any]:
    first_event = events[0]
    last_event = events[-1]
    aggregated_signals = _aggregate_signals(contexts)
    attachment_hints = _merge_attachment_hints(contexts)
    segment_class = _segment_class_from_signals(aggregated_signals) or _segment_class_from_attachment_hints(attachment_hints)
    payload: dict[str, Any] = {
        "segment_index": len(existing_segments) + 1,
        "sequence_start": int(first_event.get("sequence") or 0),
        "sequence_end": int(last_event.get("sequence") or 0),
        "event_count": len(events),
        "event_types": [str(event.get("event_type") or "") for event in events],
        "signal_kinds": [str(signal.get("kind") or "") for signal in aggregated_signals],
        "signals": aggregated_signals,
        "attachment_hints": attachment_hints,
        "durable_objects": _durable_objects_from_attachment_hints(attachment_hints),
        "status": status,
    }
    if segment_class:
        payload["segment_class"] = segment_class
        payload["classification_source"] = "boundary_signals" if aggregated_signals else "attachment_hints"
    return payload


def _aggregate_signals(contexts: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    aggregated: list[dict[str, Any]] = []
    seen: set[tuple[str, str, str]] = set()
    for context in contexts:
        signals = context.get("signals")
        if not isinstance(signals, Sequence):
            continue
        for signal in signals:
            if not isinstance(signal, Mapping):
                continue
            key = (
                _normalize_scalar(signal.get("kind")) or "",
                _normalize_scalar(signal.get("segment_class")) or "",
                _normalize_scalar(signal.get("boundary_behavior")) or "",
            )
            if key in seen:
                continue
            seen.add(key)
            aggregated.append(dict(signal))
    return aggregated


def _merge_attachment_hints(contexts: Sequence[Mapping[str, Any]]) -> dict[str, str]:
    merged: dict[str, str] = {}
    for context in contexts:
        hints = context.get("attachment_hints")
        if not isinstance(hints, Mapping):
            continue
        for key, value in hints.items():
            normalized_value = _normalize_scalar(value)
            if normalized_value:
                merged[str(key)] = normalized_value
    return merged


def _normalize_text(value: str | None) -> str:
    return " ".join(str(value or "").strip().lower().split())


def _normalize_scalar(value: Any) -> str | None:
    text = str(value or "").strip()
    return text or None
