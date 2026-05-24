from __future__ import annotations

import json
import os
import shlex
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping

from .runtime import TelegramSyncStateStore, utc_now_iso
from .workflow_notifications import (
    _graph_next_action_from_payload,
    _graph_progress_digest,
    _graph_watch_clear_missing_file_state,
    _graph_watch_mark_missing_file,
    _graph_watch_missing_file_count,
    _graph_watches,
    format_graph_progress_notification,
    format_graph_start_notification,
    format_graph_watch_missing_file_notification,
    format_graph_watch_status,
)


def _bot_runtime_error_type() -> type[Exception]:
    from .app import BotRuntimeError

    return BotRuntimeError


def _telegram_api_client_type() -> type[Any]:
    from .app import TelegramApiClient

    return TelegramApiClient


def _runtime_repo_root(config: Any) -> Path:
    if (
        config.env_path.name == "telegram.env"
        and config.env_path.parent.name == "agent-runtime"
        and config.env_path.parent.parent.name == ".ait"
    ):
        return config.env_path.parent.parent.parent
    env_root = str(os.environ.get("AIT_REPO_ROOT") or "").strip()
    if env_root:
        return Path(env_root)
    return Path.cwd()


def _load_graph_watch_json_for_config(config: Any, graph_path: str) -> tuple[dict[str, Any], Path]:
    runtime_error = _bot_runtime_error_type()
    raw = str(graph_path or "").strip()
    if not raw:
        raise runtime_error("Graph JSON path is required.")
    path = Path(raw).expanduser()
    if not path.is_absolute():
        path = _runtime_repo_root(config) / path
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise runtime_error(f"Task graph JSON not found: {graph_path}") from exc
    except json.JSONDecodeError as exc:
        raise runtime_error(f"Task graph JSON is invalid: {graph_path}: {exc}") from exc
    if not isinstance(payload, dict):
        raise runtime_error("Task graph JSON must be an object.")
    return payload, path


def _graph_watch_state_path(path: Path, *, config: Any | None = None) -> str:
    repo_root = _runtime_repo_root(config) if config is not None else Path.cwd()
    try:
        return path.resolve().relative_to(repo_root.resolve()).as_posix()
    except ValueError:
        return str(path)


def _normalized_graph_watch_plan_id(plan_id: str) -> str:
    runtime_error = _bot_runtime_error_type()
    normalized = str(plan_id or "").strip().upper()
    if not normalized:
        raise runtime_error("Plan id is required for graph watch registration.")
    return normalized


def _normalized_graph_watch_plan_ids(plan_ids: Iterable[str] | None) -> set[str] | None:
    if plan_ids is None:
        return None
    normalized: set[str] = set()
    for value in plan_ids:
        text = str(value or "").strip()
        if text:
            normalized.add(_normalized_graph_watch_plan_id(text))
    return normalized or None


def _build_graph_watch(
    config: Any,
    *,
    plan_id: str,
    graph_path: str,
    progress_reader: Callable[[dict[str, Any]], dict[str, Any]],
    created_at: str | None = None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    runtime_error = _bot_runtime_error_type()
    normalized_plan_id = _normalized_graph_watch_plan_id(plan_id)
    graph, resolved_path = _load_graph_watch_json_for_config(config, graph_path)
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    graph_plan_id = str(source_plan.get("plan_id") or "").strip().upper()
    if graph_plan_id and graph_plan_id != normalized_plan_id:
        raise runtime_error(f"Graph source_plan.plan_id is {graph_plan_id}, not {normalized_plan_id}.")
    progress_payload = progress_reader(graph)
    watch = {
        "plan_id": normalized_plan_id,
        "graph_path": _graph_watch_state_path(resolved_path, config=config),
        "graph_id": str(graph.get("graph_id") or ""),
        "last_progress_digest": _graph_progress_digest(progress_payload),
        "last_next_action": _graph_next_action_from_payload(progress_payload),
        "last_progress_notification_at": utc_now_iso(),
        "created_at": created_at or utc_now_iso(),
    }
    return watch, progress_payload


def upsert_graph_watch_for_chat(
    config: Any,
    *,
    chat_id: str | int,
    plan_id: str,
    graph_path: str,
    progress_reader: Callable[[dict[str, Any]], dict[str, Any]],
    state_store: TelegramSyncStateStore | None = None,
    telegram_api: Any | None = None,
    notify_policy: str = "always",
) -> dict[str, Any]:
    runtime_error = _bot_runtime_error_type()
    normalized_policy = str(notify_policy or "always").strip().lower().replace("-", "_")
    if normalized_policy not in {"always", "create_or_retarget", "never"}:
        raise runtime_error(f"Unsupported graph watch notify policy: {notify_policy!r}")
    store = state_store or TelegramSyncStateStore(config.sync_state_path)
    link = store.get_chat(chat_id)
    if not link:
        raise runtime_error(f"Chat {chat_id} is not linked for graph watches.")
    watches = _graph_watches(link)
    normalized_plan_id = _normalized_graph_watch_plan_id(plan_id)
    existing_watch = watches.get(normalized_plan_id) if isinstance(watches.get(normalized_plan_id), dict) else None
    if existing_watch and normalized_policy == "create_or_retarget":
        existing_path = str(existing_watch.get("graph_path") or "").strip()
        existing_graph_id = str(existing_watch.get("graph_id") or "").strip()
        graph, resolved_path = _load_graph_watch_json_for_config(config, graph_path)
        candidate_path = _graph_watch_state_path(resolved_path, config=config)
        candidate_graph_id = str(graph.get("graph_id") or "").strip()
        if existing_path == candidate_path and existing_graph_id == candidate_graph_id:
            return {
                "registered": True,
                "already_registered": True,
                "created": False,
                "retargeted": False,
                "notification_sent": False,
                "resolution_mode": "chat_id",
                "chat_id": str(chat_id),
                "chat_title": str(link.get("chat_title") or ""),
                "plan_id": normalized_plan_id,
                "graph_id": existing_graph_id,
                "graph_path": existing_path,
            }
    watch, progress_payload = _build_graph_watch(
        config,
        plan_id=normalized_plan_id,
        graph_path=graph_path,
        progress_reader=progress_reader,
        created_at=str(existing_watch.get("created_at") or "") if existing_watch else None,
    )
    watches[normalized_plan_id] = watch
    store.patch_chat(chat_id, graph_watches=watches)
    retargeted = bool(
        existing_watch
        and (
            str(existing_watch.get("graph_path") or "").strip() != str(watch.get("graph_path") or "").strip()
            or str(existing_watch.get("graph_id") or "").strip() != str(watch.get("graph_id") or "").strip()
        )
    )
    notification_sent = False
    if normalized_policy == "always" or (
        normalized_policy == "create_or_retarget" and (existing_watch is None or retargeted)
    ):
        sender = telegram_api or _telegram_api_client_type()(config)
        sender.send_message(chat_id, format_graph_start_notification(config, watch, progress_payload))
        notification_sent = True
    return {
        "registered": True,
        "already_registered": False,
        "created": existing_watch is None,
        "retargeted": retargeted,
        "notification_sent": notification_sent,
        "resolution_mode": "chat_id",
        "chat_id": str(chat_id),
        "chat_title": str(link.get("chat_title") or ""),
        "plan_id": normalized_plan_id,
        "graph_id": str(watch.get("graph_id") or ""),
        "graph_path": str(watch.get("graph_path") or ""),
    }


def auto_register_graph_watch(
    config: Any,
    *,
    plan_id: str,
    graph_path: str,
    progress_reader: Callable[[dict[str, Any]], dict[str, Any]],
    repo_name: str | None = None,
    linked_session_id: str | None = None,
    chat_id: str | int | None = None,
    state_store: TelegramSyncStateStore | None = None,
    telegram_api: Any | None = None,
) -> dict[str, Any]:
    store = state_store or TelegramSyncStateStore(config.sync_state_path)
    state = store.load()
    normalized_repo = str(repo_name or "").strip() or str(config.repo_name or "").strip()
    normalized_session_id = str(linked_session_id or "").strip()
    explicit_chat_id = str(chat_id).strip() if chat_id is not None else ""

    def _link_matches(link: dict[str, Any]) -> bool:
        link_repo = str(link.get("repo_name") or "").strip()
        link_session_id = str(link.get("session_id") or "").strip()
        if normalized_repo and link_repo and link_repo != normalized_repo:
            return False
        if normalized_session_id:
            return bool(link_session_id) and link_session_id == normalized_session_id
        return True

    candidates: list[tuple[str, dict[str, Any]]] = []
    for current_chat_id, current_link in state.chats.items():
        if not isinstance(current_link, dict):
            continue
        if explicit_chat_id and str(current_chat_id) != explicit_chat_id:
            continue
        if not _link_matches(current_link):
            continue
        candidates.append((str(current_chat_id), dict(current_link)))

    if explicit_chat_id:
        if not candidates:
            return {
                "registered": False,
                "reason": "chat_not_linked",
                "chat_id": explicit_chat_id,
                "plan_id": _normalized_graph_watch_plan_id(plan_id),
            }
        resolved_chat_id, _resolved_link = candidates[0]
        result = upsert_graph_watch_for_chat(
            config,
            chat_id=resolved_chat_id,
            plan_id=plan_id,
            graph_path=graph_path,
            progress_reader=progress_reader,
            state_store=store,
            telegram_api=telegram_api,
            notify_policy="create_or_retarget",
        )
        result["resolution_mode"] = "chat_id"
        return result

    if normalized_session_id:
        if not candidates:
            return {
                "registered": False,
                "reason": "session_not_linked",
                "session_id": normalized_session_id,
                "plan_id": _normalized_graph_watch_plan_id(plan_id),
            }
        if len(candidates) > 1:
            return {
                "registered": False,
                "reason": "ambiguous_session_link",
                "session_id": normalized_session_id,
                "candidate_chat_ids": [chat for chat, _ in candidates],
                "plan_id": _normalized_graph_watch_plan_id(plan_id),
            }
        resolved_chat_id, _resolved_link = candidates[0]
        result = upsert_graph_watch_for_chat(
            config,
            chat_id=resolved_chat_id,
            plan_id=plan_id,
            graph_path=graph_path,
            progress_reader=progress_reader,
            state_store=store,
            telegram_api=telegram_api,
            notify_policy="create_or_retarget",
        )
        result["resolution_mode"] = "session_id"
        result["session_id"] = normalized_session_id
        return result

    if not candidates:
        return {
            "registered": False,
            "reason": "no_repo_linked_chat",
            "repo_name": normalized_repo,
            "plan_id": _normalized_graph_watch_plan_id(plan_id),
        }
    if len(candidates) > 1:
        return {
            "registered": False,
            "reason": "ambiguous_repo_link",
            "repo_name": normalized_repo,
            "candidate_chat_ids": [chat for chat, _ in candidates],
            "plan_id": _normalized_graph_watch_plan_id(plan_id),
        }
    resolved_chat_id, _resolved_link = candidates[0]
    result = upsert_graph_watch_for_chat(
        config,
        chat_id=resolved_chat_id,
        plan_id=plan_id,
        graph_path=graph_path,
        progress_reader=progress_reader,
        state_store=store,
        telegram_api=telegram_api,
        notify_policy="create_or_retarget",
    )
    result["resolution_mode"] = "repo_unique_chat"
    return result


def trigger_graph_watch_notifications(
    config: Any,
    *,
    progress_reader: Callable[[dict[str, Any]], dict[str, Any]],
    repo_name: str | None = None,
    plan_ids: Iterable[str] | None = None,
    state_store: TelegramSyncStateStore | None = None,
    telegram_api: Any | None = None,
) -> dict[str, Any]:
    runtime_error = _bot_runtime_error_type()
    store = state_store or TelegramSyncStateStore(config.sync_state_path)
    sender = telegram_api or _telegram_api_client_type()(config)
    state = store.load()
    checked = 0
    sent = 0
    errors = 0
    missing_files = 0
    skipped_repo = 0
    allowed_plan_ids = _normalized_graph_watch_plan_ids(plan_ids)

    for chat_id, link in state.chats.items():
        link_repo = str(link.get("repo_name") or config.repo_name or "").strip()
        if repo_name and link_repo and link_repo != repo_name:
            skipped_repo += 1
            continue
        watches = _graph_watches(link)
        if not watches:
            continue
        next_watches = dict(watches)
        state_changed = False
        for key, watch in watches.items():
            watch_plan_id = str(watch.get("plan_id") or key or "").strip().upper()
            if allowed_plan_ids is not None and watch_plan_id not in allowed_plan_ids:
                continue
            checked += 1
            graph_path = str(watch.get("graph_path") or "").strip()
            current_watch = dict(watch)
            try:
                graph, _ = _load_graph_watch_json_for_config(config, graph_path)
                current_watch = _graph_watch_clear_missing_file_state(watch)
                graph_repo = str(graph.get("repo_name") or "").strip()
                if repo_name and graph_repo and graph_repo != repo_name:
                    skipped_repo += 1
                    continue
                payload = progress_reader(graph)
                current_digest = _graph_progress_digest(payload)
                if current_digest == str(current_watch.get("last_progress_digest") or ""):
                    if current_watch != watch:
                        next_watches[key] = current_watch
                        store.patch_chat(chat_id, graph_watches=next_watches)
                        state_changed = True
                    continue
                updated = dict(current_watch)
                updated["last_progress_digest"] = current_digest
                updated["last_next_action"] = _graph_next_action_from_payload(payload)
                updated["last_progress_notification_at"] = utc_now_iso()
                next_watches[key] = updated
                store.patch_chat(chat_id, graph_watches=next_watches)
                state_changed = True
                sender.send_message(chat_id, format_graph_progress_notification(config, watch, payload))
                sent += 1
            except runtime_error as exc:
                if str(exc).startswith("Task graph JSON not found:"):
                    missing_files += 1
                    observed_at = utc_now_iso()
                    updated = _graph_watch_mark_missing_file(
                        watch,
                        graph_path=graph_path,
                        observed_at=observed_at,
                    )
                    missing_count = _graph_watch_missing_file_count(updated)
                    next_watches[key] = updated
                    store.patch_chat(chat_id, graph_watches=next_watches)
                    state_changed = True
                    if _graph_watch_missing_file_count(watch) == 0:
                        sender.send_message(
                            chat_id,
                            format_graph_watch_missing_file_notification(
                                config,
                                updated,
                                graph_path=graph_path,
                                missing_count=missing_count,
                            ),
                        )
                        sent += 1
                    continue
                errors += 1
                current_digest = "error:" + str(exc)
                if current_digest == str(current_watch.get("last_progress_digest") or ""):
                    continue
                updated = dict(current_watch)
                updated["last_progress_digest"] = current_digest
                updated["last_progress_notification_at"] = utc_now_iso()
                next_watches[key] = updated
                store.patch_chat(chat_id, graph_watches=next_watches)
                state_changed = True
                sender.send_message(
                    chat_id,
                    f"ait graph watch error · repo={config.repo_name}\n"
                    f"plan={watch.get('plan_id') or key}\n"
                    f"{exc}",
                )
                sent += 1
            except Exception as exc:
                errors += 1
                current_digest = "error:" + str(exc)
                if current_digest == str(current_watch.get("last_progress_digest") or ""):
                    continue
                updated = dict(current_watch)
                updated["last_progress_digest"] = current_digest
                updated["last_progress_notification_at"] = utc_now_iso()
                next_watches[key] = updated
                store.patch_chat(chat_id, graph_watches=next_watches)
                state_changed = True
                sender.send_message(
                    chat_id,
                    f"ait graph watch error · repo={config.repo_name}\n"
                    f"plan={watch.get('plan_id') or key}\n"
                    f"{exc}",
                )
        if state_changed:
            state = store.load()
    return {
        "checked": checked,
        "sent": sent,
        "errors": errors,
        "missing_files": missing_files,
        "skipped_repo": skipped_repo,
    }


class TelegramGraphWatchManager:
    def __init__(
        self,
        *,
        config: Any,
        ait_api: Any,
        telegram_api: Any,
        state_store: TelegramSyncStateStore,
        ensure_session_link: Callable[..., dict[str, Any] | None],
        missing_link_text: Callable[[], str],
        state_patch_chat: Callable[..., dict[str, Any] | None],
        watchgraph_usage: str,
        plan_id_pattern: Any,
        runtime_repo_root: Callable[[Any], Path],
    ) -> None:
        self._config = config
        self._ait_api = ait_api
        self._telegram_api = telegram_api
        self._state_store = state_store
        self._ensure_session_link = ensure_session_link
        self._missing_link_text = missing_link_text
        self._state_patch_chat = state_patch_chat
        self._watchgraph_usage = watchgraph_usage
        self._plan_id_pattern = plan_id_pattern
        self._runtime_repo_root = runtime_repo_root

    def _resolve_graph_watch_path(self, graph_path: str) -> Path:
        raw = str(graph_path or "").strip()
        if not raw:
            raise _bot_runtime_error_type()("Graph JSON path is required.")
        path = Path(raw).expanduser()
        if not path.is_absolute():
            path = self._runtime_repo_root(self._config) / path
        return path

    def _load_graph_watch_json(self, graph_path: str) -> tuple[dict[str, Any], Path]:
        runtime_error = _bot_runtime_error_type()
        path = self._resolve_graph_watch_path(graph_path)
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except FileNotFoundError as exc:
            raise runtime_error(f"Task graph JSON not found: {graph_path}") from exc
        except json.JSONDecodeError as exc:
            raise runtime_error(f"Task graph JSON is invalid: {graph_path}: {exc}") from exc
        if not isinstance(payload, dict):
            raise runtime_error("Task graph JSON must be an object.")
        return payload, path

    def _parse_watchgraph_args(self, args: str) -> tuple[str, str | None, str | None]:
        runtime_error = _bot_runtime_error_type()
        try:
            parts = shlex.split(str(args or ""))
        except ValueError as exc:
            raise runtime_error(f"Could not parse /watchgraph arguments: {exc}") from exc
        if not parts:
            return ("status", None, None)
        mode = parts[0].lower()
        if mode in {"status", "show", "list"}:
            return ("status", None, None)
        if mode in {"off", "disable", "disabled", "clear"}:
            plan_id = parts[1].upper() if len(parts) > 1 else None
            return ("off", plan_id, None)
        if mode in {"on", "watch", "enable", "enabled"}:
            parts = parts[1:]
        if len(parts) < 2:
            raise runtime_error(f"Usage: {self._watchgraph_usage}")
        match = self._plan_id_pattern.fullmatch(parts[0])
        if not match:
            raise runtime_error("First /watchgraph argument must be a PL-... plan id.")
        return ("on", match.group(1).upper(), parts[1])

    def handle_watchgraph_command(
        self,
        chat_id: str | int,
        chat: dict[str, Any],
        chat_title: str,
        args: str,
    ) -> None:
        runtime_error = _bot_runtime_error_type()
        link = self._ensure_session_link(chat_id, chat, chat_title, create_if_missing=False)
        if not link:
            self._telegram_api.send_message(chat_id, self._missing_link_text())
            return
        try:
            mode, plan_id, graph_path = self._parse_watchgraph_args(args)
        except runtime_error as exc:
            self._telegram_api.send_message(chat_id, str(exc))
            return
        if mode == "status":
            self._telegram_api.send_message(chat_id, format_graph_watch_status(self._config, link))
            return
        watches = _graph_watches(link)
        if mode == "off":
            if plan_id:
                watches.pop(plan_id, None)
                self._state_patch_chat(chat_id, graph_watches=watches)
                self._telegram_api.send_message(chat_id, f"Graph watch disabled for {plan_id}.")
            else:
                self._state_patch_chat(chat_id, graph_watches={})
                self._telegram_api.send_message(chat_id, "All graph watches disabled for this chat.")
            return

        assert plan_id is not None and graph_path is not None
        try:
            upsert_graph_watch_for_chat(
                self._config,
                chat_id=chat_id,
                plan_id=plan_id,
                graph_path=graph_path,
                progress_reader=self._ait_api.read_task_dag_progress,
                state_store=self._state_store,
                telegram_api=self._telegram_api,
                notify_policy="always",
            )
        except runtime_error as exc:
            self._telegram_api.send_message(chat_id, str(exc))

    def run_graph_notifications_for_chat(self, chat_id: str, link: dict[str, Any]) -> bool:
        runtime_error = _bot_runtime_error_type()
        watches = _graph_watches(link)
        if not watches:
            return False
        sent_any = False
        next_watches = dict(watches)
        for key, watch in watches.items():
            graph_path = str(watch.get("graph_path") or "").strip()
            current_watch = dict(watch)
            try:
                graph, _ = self._load_graph_watch_json(graph_path)
                current_watch = _graph_watch_clear_missing_file_state(watch)
                payload = self._ait_api.read_task_dag_progress(graph)
                current_digest = _graph_progress_digest(payload)
            except runtime_error as exc:
                if str(exc).startswith("Task graph JSON not found:"):
                    updated = _graph_watch_mark_missing_file(
                        watch,
                        graph_path=graph_path,
                        observed_at=utc_now_iso(),
                    )
                    next_watches[key] = updated
                    self._state_patch_chat(chat_id, graph_watches=next_watches)
                    if _graph_watch_missing_file_count(watch) == 0:
                        self._telegram_api.send_message(
                            chat_id,
                            format_graph_watch_missing_file_notification(
                                self._config,
                                updated,
                                graph_path=graph_path,
                                missing_count=_graph_watch_missing_file_count(updated),
                            ),
                        )
                        sent_any = True
                    continue
                current_digest = "error:" + str(exc)
                if current_digest == str(current_watch.get("last_progress_digest") or ""):
                    continue
                updated = dict(current_watch)
                updated["last_progress_digest"] = current_digest
                updated["last_progress_notification_at"] = utc_now_iso()
                next_watches[key] = updated
                self._state_patch_chat(chat_id, graph_watches=next_watches)
                self._telegram_api.send_message(
                    chat_id,
                    f"ait graph watch error · repo={self._config.repo_name}\n"
                    f"plan={watch.get('plan_id') or key}\n"
                    f"{exc}",
                )
                sent_any = True
                continue
            except Exception as exc:
                current_digest = "error:" + str(exc)
                if current_digest == str(current_watch.get("last_progress_digest") or ""):
                    continue
                updated = dict(current_watch)
                updated["last_progress_digest"] = current_digest
                updated["last_progress_notification_at"] = utc_now_iso()
                next_watches[key] = updated
                self._state_patch_chat(chat_id, graph_watches=next_watches)
                self._telegram_api.send_message(
                    chat_id,
                    f"ait graph watch error · repo={self._config.repo_name}\n"
                    f"plan={watch.get('plan_id') or key}\n"
                    f"{exc}",
                )
                sent_any = True
                continue
            if current_digest == str(current_watch.get("last_progress_digest") or ""):
                if current_watch != watch:
                    next_watches[key] = current_watch
                    self._state_patch_chat(chat_id, graph_watches=next_watches)
                continue
            updated = dict(current_watch)
            updated["last_progress_digest"] = current_digest
            updated["last_next_action"] = _graph_next_action_from_payload(payload)
            updated["last_progress_notification_at"] = utc_now_iso()
            next_watches[key] = updated
            self._state_patch_chat(chat_id, graph_watches=next_watches)
            self._telegram_api.send_message(chat_id, format_graph_progress_notification(self._config, watch, payload))
            sent_any = True
        return sent_any
