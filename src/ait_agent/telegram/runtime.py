from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from ait_chat.runtime_config import DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS

from ait_agent.runtime_bindings import (
    binding_surface_id,
    binding_transport,
    RuntimeSurfaceBindingStore,
    default_runtime_binding_state_payload,
    load_runtime_binding_state,
    resolve_runtime_binding_state_path,
    save_runtime_binding_state,
    utc_now_iso,
)


DEFAULT_ENV_PATH = Path(".ait") / "agent-runtime" / "telegram.env"
DEFAULT_TELEGRAM_REQUEST_TIMEOUT_SECONDS: float | None = None
DEFAULT_TELEGRAM_POLL_TIMEOUT_SECONDS = 45
DEFAULT_TELEGRAM_OPENAI_TIMEOUT_SECONDS: float | None = None
DEFAULT_TELEGRAM_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS = 30.0
DEFAULT_TELEGRAM_CODEX_TURN_TIMEOUT_SECONDS: float | None = None
DEFAULT_TELEGRAM_CODEX_CHILD_REAP_TIMEOUT_SECONDS = DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS
PLACEHOLDER_OPENAI_API_KEYS = {
    "your-openai-api-key",
    "sk-your-openai-api-key",
    "your_openai_api_key",
    "replace-with-real-openai-api-key",
}


@dataclass(frozen=True)
class TelegramChatLink:
    chat_id: str
    session_id: str
    repo_name: str
    chat_type: str | None = None
    chat_title: str | None = None
    last_synced_sequence: int = 0
    last_sync_at: str | None = None
    workflow_notifications_enabled: bool = False
    last_queue_summary_digest: str | None = None
    last_queue_notification_at: str | None = None
    graph_watches: dict[str, dict[str, Any]] | None = None
    linked_at: str | None = None
    updated_at: str | None = None


@dataclass(frozen=True)
class TelegramSyncState:
    version: int
    last_update_id: int
    chats: dict[str, dict[str, Any]]
    bootstrap_auth: dict[str, Any]


def default_state_payload() -> dict[str, Any]:
    return default_runtime_binding_state_payload()


def resolve_telegram_env_path(repo_root: Path | None = None, value: str | os.PathLike[str] | None = None) -> Path:
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


def resolve_telegram_sync_state_path(value: str | os.PathLike[str] | None = None) -> Path:
    return resolve_runtime_binding_state_path(value)


def load_simple_env_file(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    values: dict[str, str] = {}
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, raw_value = line.split("=", 1)
        key = key.strip()
        if not key:
            continue
        value = raw_value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        values[key] = value
    return values


def _format_env_seconds_value(value: float | int | None, *, none_token: str = "none") -> str:
    if value is None:
        return none_token
    if isinstance(value, float) and value.is_integer():
        return str(int(value))
    return str(value)


def telegram_worker_seed_env_defaults() -> dict[str, str]:
    return {
        "AIT_TELEGRAM_REQUEST_TIMEOUT_SECONDS": _format_env_seconds_value(
            DEFAULT_TELEGRAM_REQUEST_TIMEOUT_SECONDS,
            none_token="inf",
        ),
        "AIT_TELEGRAM_POLL_TIMEOUT_SECONDS": _format_env_seconds_value(DEFAULT_TELEGRAM_POLL_TIMEOUT_SECONDS),
        "AIT_TELEGRAM_OPENAI_TIMEOUT_SECONDS": _format_env_seconds_value(
            DEFAULT_TELEGRAM_OPENAI_TIMEOUT_SECONDS,
            none_token="inf",
        ),
        "AIT_TELEGRAM_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS": _format_env_seconds_value(
            DEFAULT_TELEGRAM_CODEX_APP_SERVER_READY_TIMEOUT_SECONDS
        ),
        "AIT_TELEGRAM_CODEX_TURN_TIMEOUT_SECONDS": _format_env_seconds_value(
            DEFAULT_TELEGRAM_CODEX_TURN_TIMEOUT_SECONDS,
            none_token="inf",
        ),
        "AIT_CHAT_CODEX_CHILD_REAP_TIMEOUT_SECONDS": _format_env_seconds_value(
            DEFAULT_TELEGRAM_CODEX_CHILD_REAP_TIMEOUT_SECONDS
        ),
        "AIT_TELEGRAM_CODEX_CHILD_REAP_TIMEOUT_SECONDS": _format_env_seconds_value(
            DEFAULT_TELEGRAM_CODEX_CHILD_REAP_TIMEOUT_SECONDS
        ),
        "AIT_TELEGRAM_STT_MODE": "off",
        "AIT_TELEGRAM_STT_MODEL": "mlx-community/whisper-large-v3-mlx",
        "AIT_TELEGRAM_STT_DEVICE": "auto",
        "AIT_TELEGRAM_STT_INCLUDE_AUDIO_UPLOADS": "false",
    }


def normalize_openai_api_key(value: str | None) -> str | None:
    raw = str(value or "").strip()
    if not raw:
        return None
    if raw.lower() in PLACEHOLDER_OPENAI_API_KEYS:
        return None
    return raw


def load_sync_state(path: Path | None = None) -> TelegramSyncState:
    state = load_runtime_binding_state(path or resolve_telegram_sync_state_path())
    return TelegramSyncState(
        version=int(state.version or 1),
        last_update_id=int(state.last_update_id or 0),
        chats={str(key): value for key, value in state.chats.items() if isinstance(value, dict)},
        bootstrap_auth={str(key): value for key, value in state.telegram_bootstrap_auth.items()},
    )


def save_sync_state(state: TelegramSyncState | dict[str, Any], path: Path | None = None) -> None:
    target = path or resolve_telegram_sync_state_path()
    existing = load_runtime_binding_state(target)
    payload = state if isinstance(state, dict) else {
        "version": int(state.version or existing.version or 1),
        "last_update_id": int(state.last_update_id or existing.last_update_id or 0),
        "chats": state.chats,
        "surface_bindings": existing.surface_bindings,
        "telegram_bootstrap_auth": state.bootstrap_auth,
    }
    if isinstance(payload, dict) and "surface_bindings" not in payload:
        payload = {
            **payload,
            "surface_bindings": existing.surface_bindings,
        }
    if isinstance(payload, dict) and "telegram_bootstrap_auth" not in payload:
        payload = {
            **payload,
            "telegram_bootstrap_auth": existing.telegram_bootstrap_auth,
        }
    save_runtime_binding_state(payload, target)


def recover_repo_local_sync_state_from_shared_runtime_root(
    target_path: Path,
    *,
    repo_name: str | None,
) -> bool:
    normalized_repo_name = str(repo_name or "").strip()
    if not normalized_repo_name:
        return False
    shared_path = resolve_runtime_binding_state_path()
    try:
        target_resolved = target_path.expanduser().resolve()
    except OSError:
        target_resolved = target_path.expanduser()
    try:
        shared_resolved = shared_path.expanduser().resolve()
    except OSError:
        shared_resolved = shared_path.expanduser()
    if target_resolved == shared_resolved:
        return False
    target_state = load_runtime_binding_state(target_path)
    if target_state.chats or target_state.surface_bindings or target_state.telegram_bootstrap_auth:
        return False
    shared_state = load_runtime_binding_state(shared_path)
    matching_chats = {
        chat_id: dict(link)
        for chat_id, link in shared_state.chats.items()
        if isinstance(link, Mapping) and str(link.get("repo_name") or "").strip() == normalized_repo_name
    }
    if not matching_chats:
        return False
    matching_chat_ids = set(matching_chats)
    matching_bindings = {
        binding_id: dict(binding)
        for binding_id, binding in shared_state.surface_bindings.items()
        if isinstance(binding, Mapping)
        and binding_transport(binding) == "telegram"
        and str(binding.get("repo_name") or "").strip() == normalized_repo_name
        and str(binding_surface_id(binding) or "").strip() in matching_chat_ids
    }
    shared_bootstrap_auth = dict(shared_state.telegram_bootstrap_auth)
    owner_chat_id = str(shared_bootstrap_auth.get("owner_chat_id") or "").strip()
    pending_chat_id = str(shared_bootstrap_auth.get("pending_chat_id") or "").strip()
    bootstrap_auth = (
        shared_bootstrap_auth
        if owner_chat_id in matching_chat_ids or pending_chat_id in matching_chat_ids
        else {}
    )
    save_runtime_binding_state(
        {
            "version": max(int(target_state.version or 0), int(shared_state.version or 0), 1),
            "last_update_id": max(int(target_state.last_update_id or 0), int(shared_state.last_update_id or 0)),
            "chats": matching_chats,
            "surface_bindings": matching_bindings,
            "telegram_bootstrap_auth": bootstrap_auth,
        },
        target_path,
    )
    return True


class TelegramSyncStateStore:
    def __init__(self, path: Path | None = None):
        self.path = path or resolve_telegram_sync_state_path()
        self.binding_store = RuntimeSurfaceBindingStore(self.path)

    def load(self) -> TelegramSyncState:
        return load_sync_state(self.path)

    def save(self, state: TelegramSyncState | dict[str, Any]) -> None:
        save_sync_state(state, self.path)

    def update_last_update_id(self, update_id: int) -> TelegramSyncState:
        state = self.binding_store.update_last_update_id(update_id)
        return TelegramSyncState(
            version=int(state.version or 1),
            last_update_id=int(state.last_update_id or 0),
            chats={str(key): value for key, value in state.chats.items() if isinstance(value, dict)},
            bootstrap_auth={str(key): value for key, value in state.telegram_bootstrap_auth.items()},
        )

    def upsert_chat(
        self,
        chat_id: str | int,
        *,
        session_id: str,
        repo_name: str,
        chat_type: str | None = None,
        chat_title: str | None = None,
        canonical_session_id: str | None = None,
        branch_session_id: str | None = None,
        binding_role: str | None = None,
        last_synced_sequence: int | None = None,
        last_sync_at: str | None = None,
        **updates: Any,
    ) -> dict[str, Any]:
        self.binding_store.upsert_binding(
            transport="telegram",
            surface_id=chat_id,
            repo_name=repo_name,
            surface_title=chat_title,
            surface_kind=chat_type,
            session_id=session_id,
            canonical_session_id=canonical_session_id,
            branch_session_id=branch_session_id,
            binding_role=binding_role,
            last_synced_sequence=last_synced_sequence,
            last_sync_at=last_sync_at,
            updates=updates,
        )
        return self.get_chat(chat_id) or {}

    def get_chat(self, chat_id: str | int) -> dict[str, Any] | None:
        state = self.load()
        return state.chats.get(str(chat_id))

    def patch_chat(self, chat_id: str | int, **updates: Any) -> dict[str, Any] | None:
        return self.binding_store.patch_binding(
            transport="telegram",
            surface_id=chat_id,
            **updates,
        )

    def linked_session_ids(self) -> set[str]:
        state = self.load()
        return {
            str(item.get("session_id"))
            for item in state.chats.values()
            if item.get("session_id")
        }

    def linkage_by_session(self) -> dict[str, dict[str, Any]]:
        state = self.load()
        links: dict[str, dict[str, Any]] = {}
        for item in state.chats.values():
            session_id = str(item.get("session_id") or "").strip()
            if session_id:
                links[session_id] = dict(item)
        return links

    def get_bootstrap_auth(self) -> dict[str, Any]:
        return dict(self.load().bootstrap_auth)

    def save_bootstrap_auth(self, payload: dict[str, Any]) -> dict[str, Any]:
        state = self.load()
        self.save(
            TelegramSyncState(
                version=state.version,
                last_update_id=state.last_update_id,
                chats=state.chats,
                bootstrap_auth=dict(payload),
            )
        )
        return self.get_bootstrap_auth()
