from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from ait_agent.runtime_bindings import RuntimeSurfaceBindingStore, resolve_runtime_binding_state_path, utc_now_iso


DEFAULT_ENV_PATH = Path('.ait') / 'agent-runtime' / 'discord.env'
DEFAULT_RECENT_INTERACTION_LIMIT = 64
DEFAULT_RECENT_MESSAGE_LIMIT = 64


def resolve_discord_env_path(repo_root: Path | None = None, value: str | os.PathLike[str] | None = None) -> Path:
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


def resolve_discord_sync_state_path(value: str | os.PathLike[str] | None = None) -> Path:
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


class DiscordSyncStateStore:
    def __init__(self, path: Path | None = None):
        self.path = path or resolve_discord_sync_state_path()
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
            transport='discord',
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
        return self.binding_store.get_binding(transport='discord', surface_id=channel_id, thread_id=thread_id)

    def patch_channel(
        self,
        channel_id: str | int,
        *,
        thread_id: str | int | None = None,
        **updates: Any,
    ) -> dict[str, Any] | None:
        return self.binding_store.patch_binding(transport='discord', surface_id=channel_id, thread_id=thread_id, **updates)

    def list_channels(
        self,
        *,
        repo_name: str | None = None,
        include_inactive: bool = False,
    ) -> list[dict[str, Any]]:
        return self.binding_store.list_bindings(
            repo_name=repo_name,
            transport='discord',
            include_inactive=include_inactive,
        )

    def has_processed_interaction(
        self,
        channel_id: str | int,
        interaction_id: str | None,
        *,
        thread_id: str | int | None = None,
    ) -> bool:
        normalized = str(interaction_id or '').strip()
        if not normalized:
            return False
        binding = self.get_channel(channel_id, thread_id=thread_id) or {}
        recent = binding.get('discord_recent_interaction_ids') or []
        return normalized in {str(value).strip() for value in recent if str(value).strip()}

    def remember_interaction(
        self,
        channel_id: str | int,
        interaction_id: str,
        *,
        thread_id: str | int | None = None,
        source_user_id: str | None = None,
        guild_id: str | None = None,
        command_name: str | None = None,
        last_synced_sequence: int | None = None,
        limit: int = DEFAULT_RECENT_INTERACTION_LIMIT,
        **extra_updates: Any,
    ) -> dict[str, Any] | None:
        binding = self.get_channel(channel_id, thread_id=thread_id)
        if binding is None:
            return None
        recent = [
            str(value).strip()
            for value in (binding.get('discord_recent_interaction_ids') or [])
            if str(value).strip()
        ]
        normalized_interaction_id = str(interaction_id).strip()
        recent = [value for value in recent if value != normalized_interaction_id]
        recent.append(normalized_interaction_id)
        if limit > 0 and len(recent) > limit:
            recent = recent[-limit:]
        updates: dict[str, Any] = {
            'discord_recent_interaction_ids': recent,
            'discord_last_interaction_id': normalized_interaction_id,
            'discord_last_source_user_id': str(source_user_id).strip() if source_user_id else None,
            'discord_last_guild_id': str(guild_id).strip() if guild_id else None,
            'discord_last_command_name': str(command_name).strip() if command_name else None,
            'last_sync_at': utc_now_iso(),
        }
        if last_synced_sequence is not None:
            updates['last_synced_sequence'] = max(int(last_synced_sequence), 0)
        updates.update(extra_updates)
        return self.patch_channel(channel_id, thread_id=thread_id, **updates)

    def has_processed_message(
        self,
        channel_id: str | int,
        message_id: str | None,
        *,
        thread_id: str | int | None = None,
    ) -> bool:
        normalized = str(message_id or '').strip()
        if not normalized:
            return False
        binding = self.get_channel(channel_id, thread_id=thread_id) or {}
        recent = binding.get('discord_recent_message_ids') or []
        return normalized in {str(value).strip() for value in recent if str(value).strip()}

    def remember_message(
        self,
        channel_id: str | int,
        message_id: str,
        *,
        thread_id: str | int | None = None,
        source_user_id: str | None = None,
        guild_id: str | None = None,
        last_synced_sequence: int | None = None,
        limit: int = DEFAULT_RECENT_MESSAGE_LIMIT,
        **extra_updates: Any,
    ) -> dict[str, Any] | None:
        binding = self.get_channel(channel_id, thread_id=thread_id)
        if binding is None:
            return None
        recent = [
            str(value).strip()
            for value in (binding.get('discord_recent_message_ids') or [])
            if str(value).strip()
        ]
        normalized_message_id = str(message_id).strip()
        recent = [value for value in recent if value != normalized_message_id]
        recent.append(normalized_message_id)
        if limit > 0 and len(recent) > limit:
            recent = recent[-limit:]
        updates: dict[str, Any] = {
            'discord_recent_message_ids': recent,
            'discord_last_message_id': normalized_message_id,
            'discord_last_source_user_id': str(source_user_id).strip() if source_user_id else None,
            'discord_last_guild_id': str(guild_id).strip() if guild_id else None,
            'last_sync_at': utc_now_iso(),
        }
        if last_synced_sequence is not None:
            updates['last_synced_sequence'] = max(int(last_synced_sequence), 0)
        updates.update(extra_updates)
        return self.patch_channel(channel_id, thread_id=thread_id, **updates)


__all__ = [
    'DEFAULT_ENV_PATH',
    'DEFAULT_RECENT_INTERACTION_LIMIT',
    'DEFAULT_RECENT_MESSAGE_LIMIT',
    'DiscordSyncStateStore',
    'load_simple_env_file',
    'resolve_discord_env_path',
    'resolve_discord_sync_state_path',
    'utc_now_iso',
]
