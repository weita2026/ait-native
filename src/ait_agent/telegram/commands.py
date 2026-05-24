from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable, Mapping, Sequence

from .session_views import (
    _notification_mode_label,
    _sync_mode_label,
    format_session_events,
    format_session_status,
)
from .workflow_notifications import (
    _queue_digest,
    _queue_digest_actionable,
    format_attention_summary,
    format_change_land_summary,
    format_change_summary,
    format_queue_summary,
    format_ready_summary,
    format_task_audit_summary,
    format_task_summary,
    format_workflow_notification,
)


@dataclass(frozen=True)
class TelegramCommandSpec:
    name: str
    usage: str
    description: str
    category: str
    handler_name: str
    aliases: tuple[str, ...] = ()


TELEGRAM_COMMAND_SPECS: tuple[TelegramCommandSpec, ...] = (
    TelegramCommandSpec(
        name="help",
        usage="/start / /help",
        description="show workflow guidance and realistic examples",
        category="Learn and session",
        handler_name="handle_help_command",
        aliases=("start",),
    ),
    TelegramCommandSpec(
        name="status",
        usage="/status",
        description="show the current linked session and sync mode",
        category="Learn and session",
        handler_name="handle_status_command",
    ),
    TelegramCommandSpec(
        name="sync",
        usage="/sync",
        description="pull newer shared session events from web or CLI",
        category="Learn and session",
        handler_name="handle_sync_command",
    ),
    TelegramCommandSpec(
        name="session",
        usage="/session",
        description="show or create the linked session record",
        category="Learn and session",
        handler_name="handle_session_command",
    ),
    TelegramCommandSpec(
        name="queue",
        usage="/queue",
        description="show the active task queue summary",
        category="Workflow queries",
        handler_name="handle_queue_command",
    ),
    TelegramCommandSpec(
        name="attention",
        usage="/attention",
        description="show active tasks that currently need attention",
        category="Workflow queries",
        handler_name="handle_attention_command",
    ),
    TelegramCommandSpec(
        name="ready",
        usage="/ready",
        description="show tasks that are ready to land or complete",
        category="Workflow queries",
        handler_name="handle_ready_command",
    ),
    TelegramCommandSpec(
        name="task",
        usage="/task AITT-...",
        description="show task detail for a task id",
        category="Workflow queries",
        handler_name="handle_task_command",
    ),
    TelegramCommandSpec(
        name="audit",
        usage="/audit AITT-...",
        description="show task readiness and target-line audit summary",
        category="Workflow queries",
        handler_name="handle_audit_command",
    ),
    TelegramCommandSpec(
        name="change",
        usage="/change AITC-...",
        description="show change detail for a change id",
        category="Workflow queries",
        handler_name="handle_change_command",
    ),
    TelegramCommandSpec(
        name="land",
        usage="/land AITC-...",
        description="show change land-readiness summary",
        category="Workflow queries",
        handler_name="handle_land_command",
    ),
    TelegramCommandSpec(
        name="notify",
        usage="/notify on|off|status",
        description="toggle optional workflow queue notifications for this chat",
        category="Workflow notifications",
        handler_name="handle_notify_command",
    ),
    TelegramCommandSpec(
        name="watchgraph",
        usage="/watchgraph <plan-id> <graph-json> | status | off [plan-id]",
        description="subscribe this chat to task graph progress triggers",
        category="Workflow notifications",
        handler_name="handle_watchgraph_command",
        aliases=("graphwatch",),
    ),
    TelegramCommandSpec(
        name="ping",
        usage="/ping",
        description="health check",
        category="Utility",
        handler_name="handle_ping_command",
    ),
)

TELEGRAM_COMMANDS_BY_NAME = {
    alias: spec
    for spec in TELEGRAM_COMMAND_SPECS
    for alias in (spec.name, *spec.aliases)
}

WORKFLOW_QUERY_EXAMPLES: tuple[str, ...] = (
    "queue",
    "attention",
    "ready",
    "task AITT-0010",
    "audit AITT-0010",
    "change AITC-0011",
    "land AITC-0011",
    "watchgraph PL-... docs/sprints/demo.task_graph.json",
    "what should land next",
)


class TelegramCommandRuntime:
    def __init__(
        self,
        *,
        config: Any,
        ait_api: Any,
        telegram_api: Any,
        ensure_session_link: Callable[..., dict[str, Any] | None],
        sync_session: Callable[[str | int, dict[str, Any]], list[dict[str, Any]]],
        state_patch_chat: Callable[..., dict[str, Any] | None],
        ait_api_call: Callable[..., Any],
        runtime_snapshot: Callable[[], Any],
        command_specs: Sequence[Any],
        commands_by_name: Mapping[str, Any],
        workflow_query_examples: Sequence[str],
        task_id_pattern: Any,
        change_id_pattern: Any,
        now_iso: Callable[[], str],
        runtime_error_type: type[Exception],
        custom_handlers: Mapping[str, Callable[..., None]] | None = None,
    ) -> None:
        self._config = config
        self._ait_api = ait_api
        self._telegram_api = telegram_api
        self._ensure_session_link = ensure_session_link
        self._sync_session = sync_session
        self._state_patch_chat = state_patch_chat
        self._ait_api_call = ait_api_call
        self._runtime_snapshot = runtime_snapshot
        self._command_specs = tuple(command_specs)
        self._commands_by_name = dict(commands_by_name)
        self._workflow_query_examples = tuple(workflow_query_examples)
        self._task_id_pattern = task_id_pattern
        self._change_id_pattern = change_id_pattern
        self._now_iso = now_iso
        self._runtime_error_type = runtime_error_type
        self._custom_handlers = dict(custom_handlers or {})

    def dispatch(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
        from_user: Mapping[str, Any],
        chat_title: str,
        name: str,
        args: str,
    ) -> None:
        spec = self._commands_by_name.get(name)
        if spec is None:
            self._telegram_api.send_message(chat_id, self.unknown_command_text(name))
            return
        handler = getattr(self, spec.handler_name, None)
        if handler is None:
            handler = self._custom_handlers.get(spec.name)
        if handler is None:
            self._telegram_api.send_message(chat_id, self.unknown_command_text(name))
            return
        handler(chat_id, chat, from_user, chat_title, args)

    def help_text(self, chat_id: str | int, chat: Mapping[str, Any], chat_title: str) -> str:
        link = self._ensure_session_link(chat_id, chat, chat_title, create_if_missing=False)
        chat_type = str(chat.get("type") or "").strip()
        lines = [
            "ait Telegram bot",
            "",
            "Thin Telegram transport for a shared ait session.",
            "",
            "Durable state: shared session history stays in ait session events.",
            "Runtime-only state: Telegram linkage and sync cursor stay in telegram-sync.json.",
            f"Sync mode: {_sync_mode_label(self._config)}.",
            "Optional workflow notifications stay runtime-only and are scoped per linked chat.",
        ]
        current_category = None
        for spec in self._command_specs:
            if spec.category != current_category:
                lines.extend(["", spec.category])
                current_category = spec.category
            lines.append(f"{spec.usage} - {spec.description}")
        lines.extend(["", "Workflow query examples", *self._workflow_query_examples])
        if chat_type in {"group", "supergroup"}:
            mention_target = f"@{self._config.username}" if self._config.username else "@your_bot"
            lines.extend(
                [
                    "",
                    f"Group chat tip: start free text with {mention_target} so the bot knows the turn is for ait.",
                    f"Example: {mention_target} summarize what should land next",
                ]
            )
        lines.extend(
            [
                "",
                "Any other text is normalized, appended to the linked ait session, and answered through the shared AI reply flow.",
            ]
        )
        if link:
            lines.extend(["", "Current linked session", self.linked_session_status_text(link)])
        else:
            lines.extend(["", "Current linked session", self.missing_link_text()])
        return "\n".join(lines)

    def unknown_command_text(self, name: str) -> str:
        suggestions = [
            "/help",
            self._commands_by_name["queue"].usage,
            self._commands_by_name["attention"].usage,
            self._commands_by_name["ready"].usage,
            self._commands_by_name["task"].usage,
            self._commands_by_name["change"].usage,
            self._commands_by_name["notify"].usage,
            self._commands_by_name["watchgraph"].usage,
            self._commands_by_name["sync"].usage,
        ]
        return f"Unknown command /{name}. Send /help for examples like {', '.join(suggestions)}."

    def missing_link_text(self) -> str:
        return "No linked session yet. Send a message to start one immediately."

    def linked_session_detail(self, link: dict[str, Any] | None) -> dict[str, Any] | None:
        session_id = str((link or {}).get("session_id") or "").strip()
        if not session_id:
            return None
        try:
            return self._ait_api_call("get_session", session_id, runtime_snapshot=self._runtime_snapshot())
        except self._runtime_error_type:
            return None

    def linked_session_status_text(self, link: dict[str, Any]) -> str:
        return format_session_status(self._config, link, session=self.linked_session_detail(link))

    def handle_help_command(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        chat_title: str,
        _args: str,
    ) -> None:
        self._telegram_api.send_message(chat_id, self.help_text(chat_id, chat, chat_title))

    def handle_status_command(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        chat_title: str,
        _args: str,
    ) -> None:
        link = self._ensure_session_link(chat_id, chat, chat_title, create_if_missing=False)
        if not link:
            self._telegram_api.send_message(chat_id, self.missing_link_text())
            return
        self._telegram_api.send_message(chat_id, self.linked_session_status_text(link))

    def handle_sync_command(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        chat_title: str,
        _args: str,
    ) -> None:
        link = self._ensure_session_link(chat_id, chat, chat_title, create_if_missing=False)
        if not link:
            self._telegram_api.send_message(chat_id, self.missing_link_text())
            return
        if self.linked_session_detail(link) is None:
            self._telegram_api.send_message(chat_id, self.linked_session_status_text(link))
            return
        events = self._sync_session(chat_id, link)
        self._telegram_api.send_message(chat_id, format_session_events(events))

    def handle_session_command(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        chat_title: str,
        _args: str,
    ) -> None:
        link = self._ensure_session_link(chat_id, chat, chat_title)
        if link is None:
            self._telegram_api.send_message(chat_id, self.missing_link_text())
            return
        self._telegram_api.send_message(chat_id, self.linked_session_status_text(link))

    def handle_queue_command(
        self,
        chat_id: str | int,
        _chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        _chat_title: str,
        _args: str,
    ) -> None:
        self._telegram_api.send_message(chat_id, format_queue_summary(self._config, self._ait_api.read_task_queue()))

    def handle_attention_command(
        self,
        chat_id: str | int,
        _chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        _chat_title: str,
        _args: str,
    ) -> None:
        self._telegram_api.send_message(chat_id, format_attention_summary(self._config, self._ait_api.read_task_queue()))

    def handle_ready_command(
        self,
        chat_id: str | int,
        _chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        _chat_title: str,
        _args: str,
    ) -> None:
        self._telegram_api.send_message(chat_id, format_ready_summary(self._config, self._ait_api.read_task_queue()))

    def handle_task_command(
        self,
        chat_id: str | int,
        _chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        _chat_title: str,
        args: str,
    ) -> None:
        match = self._task_id_pattern.search(args or "")
        task_id = match.group(1).upper() if match else None
        if not task_id:
            self._telegram_api.send_message(chat_id, f"Usage: {self._commands_by_name['task'].usage}")
            return
        self._telegram_api.send_message(chat_id, format_task_summary(self._config, self._ait_api.read_task(task_id)))

    def handle_audit_command(
        self,
        chat_id: str | int,
        _chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        _chat_title: str,
        args: str,
    ) -> None:
        match = self._task_id_pattern.search(args or "")
        task_id = match.group(1).upper() if match else None
        if not task_id:
            self._telegram_api.send_message(chat_id, f"Usage: {self._commands_by_name['audit'].usage}")
            return
        self._telegram_api.send_message(
            chat_id,
            format_task_audit_summary(self._config, self._ait_api.read_task_audit(task_id)),
        )

    def handle_change_command(
        self,
        chat_id: str | int,
        _chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        _chat_title: str,
        args: str,
    ) -> None:
        match = self._change_id_pattern.search(args or "")
        change_id = match.group(1).upper() if match else None
        if not change_id:
            self._telegram_api.send_message(chat_id, f"Usage: {self._commands_by_name['change'].usage}")
            return
        self._telegram_api.send_message(
            chat_id,
            format_change_summary(self._config, self._ait_api.read_change(change_id)),
        )

    def handle_land_command(
        self,
        chat_id: str | int,
        _chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        _chat_title: str,
        args: str,
    ) -> None:
        match = self._change_id_pattern.search(args or "")
        change_id = match.group(1).upper() if match else None
        if not change_id:
            self._telegram_api.send_message(chat_id, f"Usage: {self._commands_by_name['land'].usage}")
            return
        self._telegram_api.send_message(
            chat_id,
            format_change_land_summary(self._config, self._ait_api.read_change(change_id)),
        )

    def handle_notify_command(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        chat_title: str,
        args: str,
    ) -> None:
        link = self._ensure_session_link(chat_id, chat, chat_title, create_if_missing=False)
        if not link:
            self._telegram_api.send_message(chat_id, self.missing_link_text())
            return
        mode = str(args or "").strip().lower() or "status"
        if mode in {"status", "show"}:
            self._telegram_api.send_message(chat_id, self.notification_status_text(link))
            return
        if mode in {"on", "enable", "enabled"}:
            payload = self._ait_api.read_task_queue()
            digest = _queue_digest(payload)
            self._state_patch_chat(
                chat_id,
                workflow_notifications_enabled=True,
                last_queue_summary_digest=digest,
                last_queue_notification_at=self._now_iso() if _queue_digest_actionable(digest) else None,
            )
            self._telegram_api.send_message(chat_id, self.notification_enabled_text(link, payload))
            return
        if mode in {"off", "disable", "disabled"}:
            self._state_patch_chat(
                chat_id,
                workflow_notifications_enabled=False,
                last_queue_summary_digest=None,
                last_queue_notification_at=None,
            )
            self._telegram_api.send_message(chat_id, "Workflow notifications disabled for this chat.")
            return
        self._telegram_api.send_message(chat_id, f"Usage: {self._commands_by_name['notify'].usage}")

    def handle_ping_command(
        self,
        chat_id: str | int,
        _chat: Mapping[str, Any],
        _from_user: Mapping[str, Any],
        _chat_title: str,
        _args: str,
    ) -> None:
        self._telegram_api.send_message(chat_id, "pong")

    def notification_status_text(self, link: dict[str, Any]) -> str:
        lines = [
            f"workflow_notifications={_notification_mode_label(self._config, link)}",
            f"sync_mode={_sync_mode_label(self._config)}",
        ]
        last_queue_notification_at = str(link.get("last_queue_notification_at") or "").strip()
        if last_queue_notification_at:
            lines.append(f"last_queue_notification_at={last_queue_notification_at}")
        else:
            lines.append("No workflow queue notification delivered yet.")
        if not self._config.background_sync_enabled:
            lines.append("Background sync is disabled, so automatic delivery is currently paused.")
        return "\n".join(lines)

    def notification_enabled_text(self, _link: dict[str, Any], payload: dict[str, Any]) -> str:
        lines = ["Workflow notifications enabled for this chat."]
        if self._config.background_sync_enabled:
            lines.append("Background sync is active, so queue updates can arrive automatically.")
        else:
            lines.append("Background sync is currently disabled, so automatic delivery will wait until it is enabled.")
        if _queue_digest_actionable(_queue_digest(payload)):
            lines.extend(["", format_workflow_notification(self._config, payload)])
        else:
            lines.append("Complete")
        return "\n".join(lines)


__all__ = [
    "TELEGRAM_COMMANDS_BY_NAME",
    "TELEGRAM_COMMAND_SPECS",
    "TelegramCommandRuntime",
    "TelegramCommandSpec",
    "WORKFLOW_QUERY_EXAMPLES",
]
