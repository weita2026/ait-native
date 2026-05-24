from __future__ import annotations

from typing import Any, Mapping

from ait_protocol.common import (
    workflow_id_matches,
    workflow_id_matches_any_namespace_prefix,
)

from .. import local_control
from ..remote_client import (
    list_changes as remote_list_changes,
    list_tasks as remote_list_tasks,
)
from .runtime_defaults import _normalize_text_value


def _require_remote_identity(entity_type: str, requested_id: str, remote_data: dict) -> None:
    remote_id = remote_data.get(f"{entity_type}_id")
    if remote_id != requested_id:
        raise ValueError(
            f"Remote server returned {entity_type}_id {remote_id!r} while publishing local {entity_type} {requested_id}. "
            f"Upgrade ait-server before publishing local short sequence IDs."
        )


def _local_workflow_identity_requests_explicit_remote_id(row: Mapping[str, Any] | None) -> bool:
    if not isinstance(row, Mapping):
        return False
    identity_source = _normalize_text_value(row.get("identity_source"))
    return identity_source != local_control.LOCAL_IDENTITY_SOURCE_SEQUENCE


def _workflow_sequence_for_row(row: Mapping[str, Any] | None, *, family: str) -> int | None:
    if not isinstance(row, Mapping):
        return None
    resolved_family = str(family or "").strip().upper()
    sequence_key = {"T": "task_seq", "C": "change_seq"}.get(resolved_family)
    if sequence_key is None:
        raise ValueError(f"Unsupported workflow sequence family: {family!r}")
    raw_sequence = row.get(sequence_key)
    if raw_sequence is not None:
        try:
            return int(raw_sequence)
        except (TypeError, ValueError):
            pass
    id_key = {"T": "task_id", "C": "change_id"}[resolved_family]
    return local_control.workflow_sequence_from_id(_normalize_text_value(row.get(id_key)), family=family)


def _remote_workflow_max_sequence(base_url: str, repo_name: str, *, family: str) -> int:
    resolved_family = str(family or "").strip().upper()
    rows = remote_list_tasks(base_url, repo_name) if resolved_family == "T" else remote_list_changes(base_url, repo_name)
    max_sequence = 0
    for row in rows:
        sequence = _workflow_sequence_for_row(row, family=resolved_family)
        if sequence is not None and sequence > max_sequence:
            max_sequence = sequence
    return max_sequence


def _aligned_remote_publish_identity_request(
    base_url: str,
    repo_name: str,
    row: Mapping[str, Any] | None,
    *,
    entity_type: str,
    namespace_prefix: str | None,
) -> str | None:
    del base_url, repo_name, namespace_prefix
    if not isinstance(row, Mapping):
        return None
    if entity_type not in {"task", "change"}:
        raise ValueError(f"Unsupported workflow publish entity: {entity_type}")
    row_id = _normalize_text_value(row.get(f"{entity_type}_id"))
    if row_id is None:
        return None
    family = {"task": "T", "change": "C"}[entity_type]
    if _local_workflow_identity_requests_explicit_remote_id(row):
        return row_id
    return row_id if _workflow_sequence_for_row(row, family=family) is not None else None


def _is_remote_publish_identity_conflict(exc: Exception, *, entity_type: str, requested_id: str | None) -> bool:
    if requested_id is None:
        return False
    text = str(exc or "")
    entity_label = entity_type.capitalize()
    return f"{entity_label} {requested_id} already exists with different fields" in text


def _require_remote_workflow_identity_family(
    entity_type: str,
    remote_data: Mapping[str, Any],
    *,
    namespace_prefix: str | None,
    requested_id: str | None = None,
) -> str:
    remote_id = _normalize_text_value(remote_data.get(f"{entity_type}_id"))
    if remote_id is None:
        raise ValueError(f"Remote server did not return {entity_type}_id while publishing local {entity_type}.")
    family = {
        "task": "T",
        "change": "C",
        "plan": "PL",
        "release": "RL",
    }.get(entity_type)
    if family is None:
        raise ValueError(f"Unsupported workflow entity type: {entity_type}")
    matches_expected_family = workflow_id_matches(remote_id, family, namespace_prefix, include_legacy=False)
    if family in {"T", "C", "P"}:
        matches_expected_family = workflow_id_matches_any_namespace_prefix(
            remote_id,
            family,
            namespace_prefix,
            include_task_change_origins=True,
        )
    if not matches_expected_family:
        raise ValueError(
            f"Remote server returned {entity_type}_id {remote_id!r} with an unexpected namespace prefix. "
            "Upgrade ait-server or resync the repository id namespace prefix before publishing."
        )
    if requested_id is not None and remote_id != requested_id:
        raise ValueError(
            f"Remote server returned {entity_type}_id {remote_id!r} while publishing local {entity_type} {requested_id}. "
            f"Upgrade ait-server before publishing local short sequence IDs."
        )
    return remote_id
