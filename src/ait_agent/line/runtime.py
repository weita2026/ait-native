from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from ait_agent.runtime_bindings import RuntimeSurfaceBindingStore, resolve_runtime_binding_state_path, utc_now_iso


DEFAULT_ENV_PATH = Path('.ait') / 'agent-runtime' / 'line.env'
DEFAULT_RECENT_EVENT_LIMIT = 64


def resolve_line_env_path(repo_root: Path | None = None, value: str | os.PathLike[str] | None = None) -> Path:
    root = repo_root or Path.cwd()
    default_path = root / DEFAULT_ENV_PATH
    if value:
        candidate = Path(value).expanduser()
        if repo_root is not None and default_path.exists():
            try:
                resolved_root = root.resolve()
            except OSError:
                resolved_root = root
            try:
                resolved_candidate = candidate.resolve()
            except OSError:
                resolved_candidate = candidate
            if resolved_candidate != default_path.resolve() and resolved_root not in resolved_candidate.parents:
                return default_path
        return candidate
    return default_path


def resolve_line_sync_state_path(value: str | os.PathLike[str] | None = None) -> Path:
    return resolve_runtime_binding_state_path(value)


def load_simple_env_file(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding='utf-8').splitlines():
        line = raw_line.strip()
        if not line or line.startswith('#') or '=' not in line:
            continue
        key, raw_value = line.split('=', 1)
        key = key.strip()
        if not key:
            continue
        value = raw_value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        values[key] = value
    return values


class LineSyncStateStore:
    def __init__(self, path: Path | None = None):
        self.path = path or resolve_line_sync_state_path()
        self.binding_store = RuntimeSurfaceBindingStore(self.path)

    def upsert_channel(
        self,
        channel_id: str | int,
        *,
        session_id: str,
        repo_name: str,
        channel_title: str | None = None,
        channel_kind: str | None = None,
        canonical_session_id: str | None = None,
        branch_session_id: str | None = None,
        binding_role: str | None = None,
        last_synced_sequence: int | None = None,
        last_sync_at: str | None = None,
        **updates: Any,
    ) -> dict[str, Any]:
        return self.binding_store.upsert_binding(
            transport='line',
            surface_id=channel_id,
            repo_name=repo_name,
            surface_title=channel_title,
            surface_kind=channel_kind,
            session_id=session_id,
            canonical_session_id=canonical_session_id,
            branch_session_id=branch_session_id,
            binding_role=binding_role,
            last_synced_sequence=last_synced_sequence,
            last_sync_at=last_sync_at,
            updates=updates,
        )

    def get_channel(self, channel_id: str | int) -> dict[str, Any] | None:
        return self.binding_store.get_binding(transport='line', surface_id=channel_id)

    def patch_channel(self, channel_id: str | int, **updates: Any) -> dict[str, Any] | None:
        return self.binding_store.patch_binding(transport='line', surface_id=channel_id, **updates)

    def linked_session_ids(self) -> set[str]:
        return {
            str(item.get('session_id'))
            for item in self.binding_store.list_bindings(transport='line', include_inactive=True)
            if item.get('session_id')
        }

    def linkage_by_session(self) -> dict[str, dict[str, Any]]:
        links: dict[str, dict[str, Any]] = {}
        for item in self.binding_store.list_bindings(transport='line', include_inactive=True):
            session_id = str(item.get('session_id') or '').strip()
            if session_id:
                links[session_id] = dict(item)
        return links

    def has_processed_event(self, channel_id: str | int, event_id: str | None) -> bool:
        normalized = str(event_id or '').strip()
        if not normalized:
            return False
        binding = self.get_channel(channel_id) or {}
        recent = binding.get('line_recent_webhook_event_ids') or []
        return normalized in {str(value).strip() for value in recent if str(value).strip()}

    def remember_processed_event(
        self,
        channel_id: str | int,
        event_id: str,
        *,
        source_user_id: str | None = None,
        message_id: str | int | None = None,
        reply_token_present: bool = False,
        last_synced_sequence: int | None = None,
        limit: int = DEFAULT_RECENT_EVENT_LIMIT,
        **extra_updates: Any,
    ) -> dict[str, Any] | None:
        binding = self.get_channel(channel_id)
        if binding is None:
            return None
        recent = [
            str(value).strip()
            for value in (binding.get('line_recent_webhook_event_ids') or [])
            if str(value).strip()
        ]
        normalized_event_id = str(event_id).strip()
        recent = [value for value in recent if value != normalized_event_id]
        recent.append(normalized_event_id)
        if limit > 0 and len(recent) > limit:
            recent = recent[-limit:]
        updates: dict[str, Any] = {
            'line_recent_webhook_event_ids': recent,
            'line_last_webhook_event_id': normalized_event_id,
            'line_last_message_id': str(message_id) if message_id is not None else None,
            'line_last_source_user_id': str(source_user_id).strip() if source_user_id else None,
            'line_last_reply_token_seen_at': utc_now_iso() if reply_token_present else binding.get('line_last_reply_token_seen_at'),
            'last_sync_at': utc_now_iso(),
        }
        if last_synced_sequence is not None:
            updates['last_synced_sequence'] = max(int(last_synced_sequence), 0)
        updates.update(extra_updates)
        return self.patch_channel(channel_id, **updates)


__all__ = [
    'DEFAULT_ENV_PATH',
    'DEFAULT_RECENT_EVENT_LIMIT',
    'LineSyncStateStore',
    'load_simple_env_file',
    'resolve_line_env_path',
    'resolve_line_sync_state_path',
    'utc_now_iso',
]
