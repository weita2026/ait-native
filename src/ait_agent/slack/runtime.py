from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from ait_agent.runtime_bindings import RuntimeSurfaceBindingStore, resolve_runtime_binding_state_path, utc_now_iso


DEFAULT_ENV_PATH = Path('.ait') / 'agent-runtime' / 'slack.env'
DEFAULT_RECENT_COMMAND_LIMIT = 64


def resolve_slack_env_path(repo_root: Path | None = None, value: str | os.PathLike[str] | None = None) -> Path:
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


def resolve_slack_sync_state_path(value: str | os.PathLike[str] | None = None) -> Path:
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


class SlackSyncStateStore:
    def __init__(self, path: Path | None = None):
        self.path = path or resolve_slack_sync_state_path()
        self.binding_store = RuntimeSurfaceBindingStore(self.path)

    def upsert_channel(
        self,
        channel_id: str | int,
        *,
        session_id: str,
        repo_name: str,
        channel_title: str | None = None,
        channel_kind: str | None = None,
        thread_id: str | int | None = None,
        canonical_session_id: str | None = None,
        branch_session_id: str | None = None,
        binding_role: str | None = None,
        last_synced_sequence: int | None = None,
        last_sync_at: str | None = None,
        **updates: Any,
    ) -> dict[str, Any]:
        return self.binding_store.upsert_binding(
            transport='slack',
            surface_id=channel_id,
            thread_id=thread_id,
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

    def get_channel(self, channel_id: str | int, *, thread_id: str | int | None = None) -> dict[str, Any] | None:
        return self.binding_store.get_binding(transport='slack', surface_id=channel_id, thread_id=thread_id)

    def patch_channel(
        self,
        channel_id: str | int,
        *,
        thread_id: str | int | None = None,
        **updates: Any,
    ) -> dict[str, Any] | None:
        return self.binding_store.patch_binding(transport='slack', surface_id=channel_id, thread_id=thread_id, **updates)

    def has_processed_command(
        self,
        channel_id: str | int,
        request_id: str | None,
        *,
        thread_id: str | int | None = None,
    ) -> bool:
        normalized = str(request_id or '').strip()
        if not normalized:
            return False
        binding = self.get_channel(channel_id, thread_id=thread_id) or {}
        recent = binding.get('slack_recent_request_ids') or []
        return normalized in {str(value).strip() for value in recent if str(value).strip()}

    def remember_command(
        self,
        channel_id: str | int,
        request_id: str,
        *,
        thread_id: str | int | None = None,
        source_user_id: str | None = None,
        team_id: str | None = None,
        command_name: str | None = None,
        last_synced_sequence: int | None = None,
        limit: int = DEFAULT_RECENT_COMMAND_LIMIT,
        **extra_updates: Any,
    ) -> dict[str, Any] | None:
        binding = self.get_channel(channel_id, thread_id=thread_id)
        if binding is None:
            return None
        recent = [
            str(value).strip()
            for value in (binding.get('slack_recent_request_ids') or [])
            if str(value).strip()
        ]
        normalized_request_id = str(request_id).strip()
        recent = [value for value in recent if value != normalized_request_id]
        recent.append(normalized_request_id)
        if limit > 0 and len(recent) > limit:
            recent = recent[-limit:]
        updates: dict[str, Any] = {
            'slack_recent_request_ids': recent,
            'slack_last_request_id': normalized_request_id,
            'slack_last_source_user_id': str(source_user_id).strip() if source_user_id else None,
            'slack_last_team_id': str(team_id).strip() if team_id else None,
            'slack_last_command_name': str(command_name).strip() if command_name else None,
            'last_sync_at': utc_now_iso(),
        }
        if last_synced_sequence is not None:
            updates['last_synced_sequence'] = max(int(last_synced_sequence), 0)
        updates.update(extra_updates)
        return self.patch_channel(channel_id, thread_id=thread_id, **updates)


__all__ = [
    'DEFAULT_ENV_PATH',
    'DEFAULT_RECENT_COMMAND_LIMIT',
    'SlackSyncStateStore',
    'load_simple_env_file',
    'resolve_slack_env_path',
    'resolve_slack_sync_state_path',
    'utc_now_iso',
]
