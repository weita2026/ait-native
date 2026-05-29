from __future__ import annotations

from datetime import datetime, timezone
import json
import shlex
import sys
import threading
import time
from collections import Counter
from pathlib import Path
from typing import TYPE_CHECKING, Any

from ait_protocol.common import workflow_id_matches_any_namespace_prefix

from .codex_app_server import CodexAppServerClient, CodexAppServerConfig, CodexAppServerError

if TYPE_CHECKING:
    from .session_reply import ReplyGenerationConfig


HELP_FLAGS = frozenset({"--help", "-h"})
DISCOVERY_TOKENS = frozenset({"ls", "find", "grep", "rg", "sed", "cat", "head", "tail"})
FILE_READ_TOKENS = frozenset({"cat", "head", "tail", "sed"})
MERGEABLE_INSPECTION_TOKENS = frozenset({"pwd", "ls", "find", "grep", "rg", "sed", "cat", "head", "tail"})
SHELL_CONTROL_SNIPPETS = ("&&", "||", ";", "|", "$(", "`")
AIT_TOP_LEVEL_COMMANDS = frozenset(
    {
        "attest",
        "change",
        "config",
        "doctor",
        "gc",
        "land",
        "line",
        "patchset",
        "policy",
        "queue",
        "ref",
        "remote",
        "repo",
        "review",
        "session",
        "snapshot",
        "stack",
        "status",
        "task",
        "workflow",
        "workspace",
        "worktree",
    }
)
CODEX_APP_SERVER_CONNECTION_CLOSED = "Codex app-server connection closed."
CODEX_APP_SERVER_CONNECTION_CLOSED_RETRY_LIMIT = 1
CODEX_APP_SERVER_RETRY_SAFETY_NOTE = (
    "Retry safety note: the previous Codex app-server websocket closed before a final reply. "
    "Some repository, workflow, or session mutations may already have happened. "
    "Before retrying mutations, inspect the current durable/session/repository state and continue from the present state. "
    "Do not blindly repeat non-idempotent actions."
)
RETRYABLE_CODEX_APP_SERVER_ERROR_MARKERS = (
    CODEX_APP_SERVER_CONNECTION_CLOSED.lower(),
    "codex app-server connection is not open",
    "failed to send payload to codex app-server",
    "failed to connect to codex app-server",
    "codex app-server exited",
    "reconnecting...",
    "unexpected status 503 service unavailable",
    "disconnect/reset before headers",
    "connection timeout",
    "timeout waiting for child process to exit",
)
CODEX_MODEL_CAPACITY_RETRY_SAFETY_NOTE = (
    "Model capacity retry note: the previous Codex turn failed because the selected model was at capacity. "
    "Some command execution or repository inspection may already have happened before the capacity failure. "
    "Continue from the current durable/session/repository state and avoid blindly repeating non-idempotent actions."
)
AIT_COMMAND_GROUPS_WITH_SUBCOMMAND = frozenset(
    {
        "attest",
        "change",
        "config",
        "doctor",
        "gc",
        "land",
        "line",
        "patchset",
        "policy",
        "queue",
        "ref",
        "remote",
        "repo",
        "review",
        "session",
        "snapshot",
        "stack",
        "task",
        "workflow",
        "workspace",
        "worktree",
    }
)
AIT_COMMAND_WRAPPER_TOKENS = frozenset(
    {
        "bash",
        "builtin",
        "command",
        "env",
        "exec",
        "gtimeout",
        "nice",
        "noglob",
        "nohup",
        "pipenv",
        "poetry",
        "python",
        "python3",
        "run",
        "sh",
        "stdbuf",
        "time",
        "timeout",
        "uv",
        "uvx",
        "zsh",
    }
)
AIT_MODULE_NAMES = frozenset({"ait", "ait.cli", "ait_native.cli"})
AIT_PREFIX_CONTROL_TOKENS = frozenset({"!", "do", "elif", "else", "then"})
AIT_INVENTORY_COMMAND_PATHS = frozenset({"change list", "change show", "task list", "task show"})
AIT_DUPLICATE_INVENTORY_COMMAND_PATHS = frozenset({"change list", "queue summary", "task audit", "task list"})
AIT_LAND_WORKFLOW_TOP_LEVELS = frozenset({"attest", "land", "patchset", "policy", "review", "snapshot"})
AIT_SHOW_COMMAND_PATHS = frozenset(
    {
        "change show",
        "land show",
        "line show",
        "patchset show",
        "policy show",
        "ref show",
        "repo show",
        "review show",
        "session checkpoint-show",
        "session show",
        "snapshot show",
        "stack show",
        "task show",
        "worktree show",
    }
)
TURN_ANALYSIS_SUMMARY_PRIORITY = (
    "duplicate_inventory_reads",
    "prefer_workflow_guide",
    "prefer_workflow_land",
    "consolidate_help_queries",
    "prefer_task_start",
    "prefer_task_audit",
    "queue_summary_for_inventory",
    "reuse_loaded_object_context",
    "reuse_file_read",
    "merge_inspection_commands",
    "avoid_repeated_commands",
    "consolidate_file_discovery",
    "batch_shell_inspection",
)
WORKFLOW_GUIDE_LAND_TOP_LEVELS = frozenset({"attest", "land", "patchset", "policy", "review", "snapshot", "task", "workspace", "worktree"})
WORKFLOW_GUIDE_INVENTORY_TOP_LEVELS = frozenset({"change", "queue", "task"})
SHELL_SEGMENT_SEPARATOR_TOKENS = frozenset({"&&", "||", "|"})


def _log_now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def _compact_log_value(value: Any, *, limit: int = 180) -> str:
    if isinstance(value, (dict, list, tuple)):
        try:
            text = json.dumps(value, ensure_ascii=False, sort_keys=True)
        except TypeError:
            text = str(value)
    else:
        text = str(value)
    text = text.replace("\n", "\\n").strip()
    if len(text) <= limit:
        return text
    return f"{text[: max(limit - 1, 0)]}…"


def _log_codex_reply(event: str, **fields: Any) -> None:
    rendered = " ".join(
        f"{key}={_compact_log_value(value)}"
        for key, value in fields.items()
        if value is not None and _compact_log_value(value)
    )
    line = f"{_log_now_iso()} ait codex reply {event}"
    if rendered:
        line = f"{line} {rendered}"
    print(line, file=sys.stderr, flush=True)


def _is_retryable_codex_app_server_error(error: CodexAppServerError) -> bool:
    message = str(error).lower()
    return any(phrase in message for phrase in RETRYABLE_CODEX_APP_SERVER_ERROR_MARKERS)


def _is_model_capacity_codex_app_server_error(error: CodexAppServerError) -> bool:
    message = str(error).lower()
    return "model is at capacity" in message or "selected model is at capacity" in message


def _developer_instructions_for_codex_attempt(
    instructions: str,
    *,
    retry_attempt: int,
    capacity_retry_attempt: int = 0,
) -> str:
    parts = [instructions.rstrip()]
    if retry_attempt > 0 and capacity_retry_attempt <= 0:
        parts.append(CODEX_APP_SERVER_RETRY_SAFETY_NOTE)
    if capacity_retry_attempt > 0:
        parts.append(CODEX_MODEL_CAPACITY_RETRY_SAFETY_NOTE)
    return "\n\n".join(part for part in parts if part).strip()


def _capacity_continue_messages(*, continue_text: str, failed_error: str, retry_attempt: int) -> list[dict[str, str]]:
    user_text = str(continue_text or "").strip() or "請繼續"
    detail = str(failed_error or "").strip()
    lines = [
        user_text,
        "",
        "[automatic capacity retry]",
        f"retry_attempt={retry_attempt}",
        "The previous turn failed because the selected model was at capacity.",
        "Continue from the current durable/session/repository state.",
        "Do not blindly repeat non-idempotent actions; inspect state first when needed.",
    ]
    if detail:
        lines.append(f"previous_error={detail}")
    return [{"role": "user", "content": "\n".join(lines).strip()}]


def _capacity_retry_exhausted_message(error: CodexAppServerError, *, capacity_retry_attempt: int) -> str:
    attempts = max(int(capacity_retry_attempt or 0), 0)
    if attempts <= 0:
        return (
            "Selected model is at capacity. "
            "Automatic continuation retry is disabled or unavailable. "
            "Telegram has been notified. Please try again later, send `請繼續`, "
            "or switch to a lower-effort / fallback model.\n"
            f"Last error: {error}"
        )
    suffix = "" if attempts == 1 else "s"
    return (
        "Selected model is at capacity after "
        f"{attempts} automatic continuation retry{suffix}. "
        "Telegram has been notified. Please try again later, send `請繼續`, "
        "or switch to a lower-effort / fallback model.\n"
        f"Last error: {error}"
    )


def _codex_worker_pool_key(
    *,
    strategy: str,
    session: dict,
    surface: str,
    chat_id: str | int | None,
    actor_identity: str | None,
) -> str:
    normalized_strategy = str(strategy or "session").strip().lower() or "session"
    if normalized_strategy == "bot":
        actor_key = str(actor_identity or "").strip()
        if actor_key:
            return f"bot:{actor_key}"
        normalized_strategy = "session"
    if normalized_strategy == "chat":
        normalized_chat_id = str(chat_id or "").strip()
        if normalized_chat_id:
            return f"chat:{normalized_chat_id}"
        normalized_strategy = "session"
    session_id = str(session.get("session_id") or "").strip()
    if session_id:
        return f"session:{session_id}"
    normalized_surface = str(surface or "").strip() or "session"
    if normalized_chat_id := str(chat_id or "").strip():
        return f"surface:{normalized_surface}:chat:{normalized_chat_id}"
    return f"surface:{normalized_surface}:anonymous"


def _codex_client_pool_key(config: CodexAppServerConfig) -> tuple[Any, ...]:
    return (
        str(config.repo_root),
        config.bin_path,
        config.model,
        config.reasoning_effort,
        config.sandbox,
        config.app_server_url or "",
        config.app_server_host,
        int(config.app_server_port or 0),
        float(config.ready_timeout_seconds),
        config.turn_timeout_seconds,
        float(config.child_kill_grace_seconds),
        float(config.child_reap_timeout_seconds),
        config.websocket_max_size_bytes,
    )


def _codex_thread_key(*, session: dict, surface: str, chat_id: str | int | None, chat_title: str | None) -> str:
    session_id = str(session.get("session_id") or "").strip()
    if session_id:
        return f"session:{session_id}"
    normalized_surface = str(surface or "").strip() or "session"
    if chat_id is not None and str(chat_id).strip():
        return f"{normalized_surface}:chat:{str(chat_id).strip()}"
    if chat_title and str(chat_title).strip():
        return f"{normalized_surface}:title:{str(chat_title).strip()}"
    return f"{normalized_surface}:anonymous"


class _PersistentCodexClientState:
    def __init__(self, config: CodexAppServerConfig):
        self.config = config
        self.lock = threading.RLock()
        self.client: CodexAppServerClient | None = None
        self.connection_generation = 0
        self.thread_records: dict[str, dict[str, Any]] = {}

    def ensure_client(self, trace_context: dict[str, Any] | None = None) -> CodexAppServerClient:
        if self.client is not None and bool(getattr(self.client, "is_started", True)):
            return self.client
        if self.client is not None:
            self.close_client(reason="stale_client_before_start")
        client = CodexAppServerClient(self.config)
        client.start()
        self.client = client
        self.connection_generation += 1
        _log_codex_reply(
            "persistent_client_ready",
            **(trace_context or {}),
            connection_generation=self.connection_generation,
        )
        return client

    def close_client(self, *, reason: str, trace_context: dict[str, Any] | None = None) -> None:
        client = self.client
        self.client = None
        if client is None:
            return
        _log_codex_reply(
            "persistent_client_reset",
            **(trace_context or {}),
            connection_generation=self.connection_generation,
            reason=reason,
        )
        try:
            client.close()
        except Exception as exc:  # pragma: no cover - defensive cleanup
            _log_codex_reply(
                "persistent_client_reset_failed",
                **(trace_context or {}),
                connection_generation=self.connection_generation,
                error_type=type(exc).__name__,
                error=str(exc),
            )

    def get_or_create_thread(
        self,
        client: CodexAppServerClient,
        *,
        thread_key: str,
        base_instructions: str,
        developer_instructions: str,
        trace_context: dict[str, Any],
    ) -> tuple[str, bool]:
        record = self.thread_records.get(thread_key)
        existing_thread_id = str((record or {}).get("thread_id") or "").strip()
        if existing_thread_id:
            if int((record or {}).get("connection_generation") or 0) == self.connection_generation:
                return existing_thread_id, True
            try:
                thread = client.resume_thread(
                    existing_thread_id,
                    base_instructions=base_instructions,
                    developer_instructions=developer_instructions,
                    persist_extended_history=True,
                    trace_context=trace_context,
                )
                resumed_thread_id = str(thread.get("id") or existing_thread_id).strip()
                self.thread_records[thread_key] = {
                    "thread_id": resumed_thread_id,
                    "connection_generation": self.connection_generation,
                }
                return resumed_thread_id, True
            except CodexAppServerError as exc:
                _log_codex_reply(
                    "thread_resume_failed",
                    **trace_context,
                    thread_key=thread_key,
                    thread_id=existing_thread_id,
                    connection_generation=self.connection_generation,
                    error=str(exc),
                )
        thread = client.start_thread(
            base_instructions=base_instructions,
            developer_instructions=developer_instructions,
            persist_extended_history=True,
            trace_context=trace_context,
        )
        thread_id = str(thread.get("id") or "").strip()
        self.thread_records[thread_key] = {
            "thread_id": thread_id,
            "connection_generation": self.connection_generation,
        }
        return thread_id, False


_PERSISTENT_CODEX_CLIENTS: dict[tuple[Any, ...], _PersistentCodexClientState] = {}
_PERSISTENT_CODEX_CLIENTS_LOCK = threading.Lock()


def _persistent_codex_state(config: CodexAppServerConfig, *, worker_key: str) -> _PersistentCodexClientState:
    key = (*_codex_client_pool_key(config), str(worker_key or ""))
    with _PERSISTENT_CODEX_CLIENTS_LOCK:
        state = _PERSISTENT_CODEX_CLIENTS.get(key)
        if state is None:
            state = _PersistentCodexClientState(config)
            _PERSISTENT_CODEX_CLIENTS[key] = state
        return state


def _reset_persistent_codex_clients_for_tests() -> None:
    with _PERSISTENT_CODEX_CLIENTS_LOCK:
        states = list(_PERSISTENT_CODEX_CLIENTS.values())
        _PERSISTENT_CODEX_CLIENTS.clear()
    for state in states:
        with state.lock:
            state.close_client(reason="test_reset")


def codex_base_instructions(repo_root: Path, *, surface: str = "telegram") -> str:
    normalized_surface = str(surface or "").strip() or "session"
    intro_line = (
        "You are Codex running behind ait's Telegram reply path."
        if normalized_surface == "telegram"
        else "You are Codex running behind ait's shared session reply path."
    )
    reply_line = (
        "Reply concisely and practically for a Telegram chat UI."
        if normalized_surface == "telegram"
        else "Reply concisely and practically for the active shared-session client."
    )
    if normalized_surface == "task_dag_compact_packet":
        bootstrap_line = (
            "This is a worker-only compact DAG packet session. Do not begin with repo-root governance discovery, "
            "general CLI help, raw git status/diff/log probes, or broad repository exploration. Start from the packet "
            "manifest path supplied in the session context, then follow the packet turn text before "
            "broadening scope."
        )
    else:
        bootstrap_line = (
            "When workflow or markdown governance matters, start with AGENTS.md, docs/plan.md, the applicable "
            "legal-layer governance docs, the active mode entrypoint named in AGENTS.md, and docs/ait.md before "
            "assuming behavior from code alone."
        )
    return "\n".join(
        [
            intro_line,
            f"You are working inside the repository at {repo_root}.",
            bootstrap_line,
            "You may inspect files, edit code, run commands, and complete workflow actions inside this workspace when the user asks for repository help.",
            "Before firing several small read-only shell probes, prefer one combined shell invocation or one broader read/search command when it keeps the result clear.",
            "For workflow inventory questions, prefer `ait queue summary` or `ait queue summary --all-changes` before stitching together separate task/change reads.",
            "For single-task readiness checks, prefer `ait task audit <task-id>` over manually stitching together `ait task show` and task-scoped `ait change list` reads.",
            "For one change's land path, prefer `ait workflow land <change-id>` over hopping through `patchset`, `attest`, `review`, `policy`, and `land` status manually.",
            "When opening a new task together with its first change, prefer `ait task start` over `ait task start --task-only` plus `ait change create`.",
            "For common workflow walkthroughs, prefer `ait workflow guide <topic>` such as `inventory` or `land` before chaining many separate `--help` calls.",
            reply_line,
        ]
    )


def _is_checkpoint_context_message(message: dict[str, str]) -> bool:
    return str(message.get("content") or "").strip().startswith("[durable checkpoint context]")


def _compact_messages(messages: list[dict[str, str]]) -> list[dict[str, str]]:
    compacted: list[dict[str, str]] = []
    for message in messages:
        content = str(message.get("content") or "").strip()
        if not content:
            continue
        compacted.append(
            {
                "role": str(message.get("role") or "").strip().lower(),
                "content": content,
            }
        )
    return compacted


def _persistent_thread_delta_messages(messages: list[dict[str, str]]) -> list[dict[str, str]]:
    compacted = _compact_messages(messages)
    while compacted and _is_checkpoint_context_message(compacted[0]):
        compacted = compacted[1:]
    last_assistant_index = -1
    for index, message in enumerate(compacted):
        if message["role"] == "assistant":
            last_assistant_index = index
    if last_assistant_index >= 0 and compacted[last_assistant_index + 1 :]:
        return compacted[last_assistant_index + 1 :]
    return compacted


def render_codex_turn_input(
    *,
    session_id: str,
    chat_id: str | int | None,
    chat_title: str | None,
    messages: list[dict[str, str]],
    surface: str = "telegram",
    context_mode: str = "transcript_replay",
) -> str:
    normalized_surface = str(surface or "").strip() or "session"
    normalized_context_mode = str(context_mode or "transcript_replay").strip().lower() or "transcript_replay"
    if normalized_context_mode == "thread_delta":
        active_messages = _persistent_thread_delta_messages(messages)
    else:
        active_messages = _compact_messages(messages)
    rendered_messages: list[str] = []
    for index, message in enumerate(active_messages, start=1):
        role = message["role"]
        content = message["content"]
        if content.startswith("[durable checkpoint context]"):
            label = "Shared checkpoint context"
        elif role == "assistant":
            label = "Assistant"
        else:
            label = "User"
        if normalized_context_mode == "thread_delta" or normalized_surface == "task_dag_compact_packet":
            rendered_messages.extend([f"{label}:", content, ""])
        else:
            rendered_messages.extend([f"{label} {index}:", content, ""])
    transcript = "\n".join(rendered_messages).strip()
    if normalized_context_mode == "thread_delta":
        header_lines = []
        trailer_lines = [
            "Continue the existing durable shared-session thread from this delta only.",
            "Treat the latest user message as the active request and perform repository work directly when practical.",
        ]
    elif normalized_surface == "telegram":
        header_lines = [
            "Shared Telegram-linked session transcript (oldest to newest):",
            f"session_id={session_id or '(unknown)'}",
            f"telegram_chat_id={chat_id or '(unknown)'}",
            f"telegram_chat_title={chat_title or '(unknown)'}",
        ]
        trailer_lines = [
            "Use the transcript as shared durable context.",
            "Treat the latest user message as the active request and perform repository work directly when practical.",
        ]
    elif normalized_surface == "task_dag_compact_packet":
        header_lines = []
        trailer_lines = [
            "Use the transcript as shared durable context.",
            "Treat the latest user message as the active request and perform repository work directly when practical.",
        ]
    else:
        header_lines = [
            "Shared session transcript (oldest to newest):",
            f"session_id={session_id or '(unknown)'}",
            f"session_surface={normalized_surface}",
            f"session_title={chat_title or '(unknown)'}",
        ]
        if chat_id is not None:
            header_lines.append(f"surface_context_id={chat_id}")
        trailer_lines = [
            "Use the transcript as shared durable context.",
            "Treat the latest user message as the active request and perform repository work directly when practical.",
        ]
    lines = [transcript or "(no transcript)", "", *trailer_lines]
    if header_lines:
        lines = [*header_lines, "", *lines]
    return "\n".join(
        lines
    )


def _command_preview(command_execution: dict) -> str:
    actions = command_execution.get("commandActions")
    if isinstance(actions, list):
        for row in actions:
            if not isinstance(row, dict):
                continue
            command = str(row.get("command") or "").strip()
            if command:
                return command
    raw = str(command_execution.get("command") or "").strip()
    if not raw:
        return "(unknown command)"
    try:
        tokens = shlex.split(raw)
    except ValueError:
        return raw
    if len(tokens) >= 3 and tokens[1] in {"-lc", "-c"}:
        return tokens[2]
    return raw


def _split_preview_tokens(preview: str) -> list[str]:
    text = str(preview or "").strip()
    if not text:
        return []
    try:
        return shlex.split(text)
    except ValueError:
        return text.split()


def _common_token_prefix(token_groups: list[list[str]]) -> list[str]:
    if not token_groups:
        return []
    prefix = list(token_groups[0])
    for group in token_groups[1:]:
        shared = 0
        for left, right in zip(prefix, group):
            if left != right:
                break
            shared += 1
        prefix = prefix[:shared]
        if not prefix:
            break
    return prefix


def _help_target_tokens(tokens: list[str]) -> list[str] | None:
    filtered = [token for token in tokens if token not in HELP_FLAGS]
    if len(filtered) == len(tokens):
        return None
    return filtered or None


def _help_entrypoint(help_targets: list[list[str]]) -> str | None:
    prefix = _common_token_prefix(help_targets)
    if not prefix:
        return None
    return " ".join([*prefix, "--help"])


def _read_target(preview_tokens: list[str]) -> str | None:
    if not preview_tokens:
        return None
    if preview_tokens[0] not in FILE_READ_TOKENS:
        return None
    for token in reversed(preview_tokens[1:]):
        if token and not token.startswith("-"):
            return token
    return None


def _dedupe_preserving_order(values: list[str]) -> list[str]:
    seen: set[str] = set()
    deduped: list[str] = []
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        deduped.append(value)
    return deduped


def _is_ait_wrapper_token(token: str) -> bool:
    normalized = str(token or "").rsplit("/", 1)[-1]
    return normalized in AIT_COMMAND_WRAPPER_TOKENS or normalized.startswith("python")


def _is_shell_env_assignment(token: str) -> bool:
    text = str(token or "").strip()
    if "=" not in text or text.startswith("="):
        return False
    name, _, _value = text.partition("=")
    if not name:
        return False
    if not (name[0].isalpha() or name[0] == "_"):
        return False
    return all(char.isalnum() or char == "_" for char in name[1:])


def _is_shell_duration_or_number_token(token: str) -> bool:
    text = str(token or "").strip()
    if not text:
        return False
    if text[0] in {"+", "-"}:
        text = text[1:]
    if not text:
        return False
    if text.isdigit():
        return True
    if len(text) >= 2 and text[:-1].isdigit() and text[-1] in {"s", "m", "h", "d"}:
        return True
    return False


def _is_ignorable_ait_prefix_token(token: str) -> bool:
    text = str(token or "").strip()
    if not text:
        return False
    return (
        _is_ait_wrapper_token(text)
        or _is_shell_env_assignment(text)
        or _is_shell_duration_or_number_token(text)
        or text in AIT_PREFIX_CONTROL_TOKENS
    )


def _is_ait_command_token(token: str) -> bool:
    return str(token or "").rsplit("/", 1)[-1] == "ait"


def _split_shell_command_segments(tokens: list[str]) -> list[list[str]]:
    segments: list[list[str]] = []
    current: list[str] = []
    for token in tokens:
        if token in SHELL_SEGMENT_SEPARATOR_TOKENS:
            if current:
                segments.append(current)
                current = []
            continue
        stripped = token.rstrip(";")
        if stripped:
            current.append(stripped)
        if stripped != token:
            if current:
                segments.append(current)
                current = []
    if current:
        segments.append(current)
    return segments or [tokens]


def _find_wrapped_ait_token(tokens: list[str]) -> int | None:
    for idx, token in enumerate(tokens):
        if not _is_ait_command_token(token):
            continue
        prefix = [item for item in tokens[:idx] if item and not item.startswith("-")]
        if all(_is_ignorable_ait_prefix_token(item) for item in prefix):
            return idx
    return None


def _find_wrapped_ait_module(tokens: list[str]) -> int | None:
    for idx in range(len(tokens) - 1):
        if tokens[idx] != "-m":
            continue
        module_name = tokens[idx + 1]
        prefix = [item for item in tokens[:idx] if item and not item.startswith("-")]
        if module_name in AIT_MODULE_NAMES and all(_is_ignorable_ait_prefix_token(item) for item in prefix):
            return idx
    return None


def _extract_ait_command_from_preview(preview: str, tokens: list[str]) -> dict[str, Any] | None:
    if not tokens:
        return None
    for segment in _split_shell_command_segments(tokens):
        if not segment:
            continue
        remainder: list[str]
        if segment[0] in AIT_TOP_LEVEL_COMMANDS:
            remainder = segment
        else:
            module_idx = _find_wrapped_ait_module(segment)
            if module_idx is not None:
                remainder = segment[module_idx + 2 :]
            else:
                ait_idx = _find_wrapped_ait_token(segment)
                if ait_idx is None:
                    continue
                remainder = segment[ait_idx + 1 :]
        if not remainder:
            continue
        top_level = remainder[0]
        if top_level not in AIT_TOP_LEVEL_COMMANDS:
            continue
        command_path_tokens = [top_level]
        if top_level in AIT_COMMAND_GROUPS_WITH_SUBCOMMAND and len(remainder) > 1 and not remainder[1].startswith("-"):
            command_path_tokens.append(remainder[1])
        command_path = " ".join(command_path_tokens)
        trailing_tokens = remainder[len(command_path_tokens) :]
        target = next((token for token in trailing_tokens if token != "--" and not token.startswith("-")), None)
        signature = f"ait {command_path}" + (f" {target}" if target else "")
        return {
            "command": preview,
            "tokens": list(segment),
            "top_level": top_level,
            "command_path": command_path,
            "target": target,
            "signature": signature,
        }
    return None


def _workflow_guide_topic_for_help_targets(help_targets: list[list[str]]) -> str | None:
    top_levels: set[str] = set()
    for tokens in help_targets:
        parsed = _extract_ait_command_from_preview(" ".join(tokens), tokens)
        if parsed is None:
            continue
        top_level = str(parsed.get("top_level") or "").strip()
        if top_level:
            top_levels.add(top_level)
    if len(top_levels & WORKFLOW_GUIDE_LAND_TOP_LEVELS) >= 3:
        return "land"
    if top_levels and top_levels.issubset(WORKFLOW_GUIDE_INVENTORY_TOP_LEVELS) and len(top_levels) >= 2:
        return "inventory"
    return None


def _workflow_land_target_ids(ait_commands: list[dict[str, Any]]) -> tuple[list[str], list[str]]:
    change_ids: list[str] = []
    patchset_ids: list[str] = []
    for row in ait_commands:
        tokens = list(row.get("tokens") or [])
        candidates = [
            str(row.get("target") or "").strip().upper(),
            str(_command_flag_value(tokens, "--change") or "").strip().upper(),
            str(_command_flag_value(tokens, "--patchset") or "").strip().upper(),
        ]
        for candidate in candidates:
            if _looks_like_change_id(candidate) and candidate not in change_ids:
                change_ids.append(candidate)
            if _looks_like_patchset_id(candidate) and candidate not in patchset_ids:
                patchset_ids.append(candidate)
    return change_ids, patchset_ids


def _workflow_land_suggested_command(change_id: str | None = None, patchset_id: str | None = None) -> str:
    if _looks_like_change_id(change_id):
        return " ".join(["ait", "workflow", "land", str(change_id).upper()])
    return "ait workflow land <change-id>"


def _command_flag_value(tokens: list[str], flag: str) -> str | None:
    for idx, token in enumerate(tokens):
        if token != flag:
            continue
        if idx + 1 >= len(tokens):
            return None
        value = tokens[idx + 1]
        if value.startswith("-"):
            return None
        return value
    return None


def _task_start_suggested_command(task_start_rows: list[dict[str, Any]], change_create_rows: list[dict[str, Any]]) -> str:
    matched_rows = [*task_start_rows, *change_create_rows]
    parts = ["ait", "task", "start"]
    if any("--local" in row["tokens"] for row in matched_rows):
        parts.append("--local")
    base_line = next((_command_flag_value(row["tokens"], "--base-line") for row in change_create_rows), None)
    if base_line and base_line != "main":
        parts.extend(["--base-line", base_line])
    return " ".join(shlex.quote(part) for part in parts)


def _looks_like_task_id(value: str | None) -> bool:
    return workflow_id_matches_any_namespace_prefix(value, "T", "", include_task_change_origins=True)


def _looks_like_change_id(value: str | None) -> bool:
    return workflow_id_matches_any_namespace_prefix(value, "C", "", include_task_change_origins=True)


def _looks_like_patchset_id(value: str | None) -> bool:
    return workflow_id_matches_any_namespace_prefix(value, "P", "", include_task_change_origins=True)


def _task_audit_suggested_command(task_id: str) -> str:
    return " ".join(["ait", "task", "audit", task_id])


def _task_audit_hint(ait_commands: list[dict[str, Any]]) -> dict[str, Any] | None:
    if any(row["command_path"] == "task audit" for row in ait_commands):
        return None
    candidate_targets = _dedupe_preserving_order(
        [
            str(row["target"]).upper()
            for row in ait_commands
            if row["command_path"] in {"task show", "change list"} and _looks_like_task_id(str(row.get("target") or ""))
        ]
    )
    for task_id in candidate_targets:
        task_show_rows = [row for row in ait_commands if row["command_path"] == "task show" and str(row.get("target") or "").upper() == task_id]
        change_list_rows = [row for row in ait_commands if row["command_path"] == "change list" and str(row.get("target") or "").upper() == task_id]
        matched_rows = [*task_show_rows, *change_list_rows]
        if not task_show_rows or not change_list_rows:
            continue
        if any("--local" in row["tokens"] for row in matched_rows):
            continue
        return {
            "code": "prefer_task_audit",
            "summary": "This task-readiness turn could likely use `ait task audit`.",
            "detail": f"When the goal is to understand one task's readiness or target-line status, `ait task audit {task_id}` can replace separate `ait task show` and task-scoped `ait change list` reads.",
            "task_id": task_id,
            "suggested_command": _task_audit_suggested_command(task_id),
            "matched_commands": [row["command"] for row in matched_rows[:5]],
        }
    return None


def _is_mergeable_inspection_preview(preview: str, preview_tokens: list[str]) -> bool:
    if not preview_tokens:
        return False
    if preview_tokens[0] not in MERGEABLE_INSPECTION_TOKENS:
        return False
    if any(token in HELP_FLAGS for token in preview_tokens[1:]):
        return False
    return not any(snippet in preview for snippet in SHELL_CONTROL_SNIPPETS)


def _mergeable_inspection_run(previews: list[str], preview_tokens: list[list[str]]) -> list[str]:
    best_run: list[str] = []
    current_run: list[str] = []
    for preview, tokens in zip(previews, preview_tokens):
        if _is_mergeable_inspection_preview(preview, tokens):
            current_run.append(preview)
            if len(current_run) > len(best_run):
                best_run = list(current_run)
            continue
        current_run = []
    unique_run = _dedupe_preserving_order(best_run)
    if len(unique_run) < 2:
        return []
    return unique_run


def _select_turn_optimization_summary(hints: list[dict[str, Any]]) -> str:
    if not hints:
        return "No obvious command-churn optimization stood out in this turn."
    by_code = {str(hint.get("code") or ""): hint for hint in hints}
    for code in TURN_ANALYSIS_SUMMARY_PRIORITY:
        selected = by_code.get(code)
        if selected and str(selected.get("summary") or "").strip():
            return str(selected["summary"])
    first = hints[0]
    return str(first.get("summary") or "No obvious command-churn optimization stood out in this turn.")


def _turn_analysis_for_commands(command_executions: tuple[dict[str, object], ...] | list[dict]) -> dict | None:
    commands = [dict(row) for row in command_executions if isinstance(row, dict)]
    if not commands:
        return None
    previews = [_command_preview(row) for row in commands]
    preview_tokens = [_split_preview_tokens(preview) for preview in previews]
    preview_counts = Counter(previews)
    ait_commands = [
        parsed
        for preview, tokens in zip(previews, preview_tokens)
        if (parsed := _extract_ait_command_from_preview(preview, tokens)) is not None
    ]
    hints: list[dict[str, Any]] = []
    burst_clusters: list[dict[str, Any]] = []
    repeated = [command for command, count in preview_counts.items() if count > 1]
    if repeated:
        repeated_preview = repeated[0]
        hints.append(
            {
                "code": "avoid_repeated_commands",
                "summary": "The same command was rerun in this turn.",
                "detail": f"Reuse earlier output instead of repeating `{repeated_preview}` unless state changed.",
            }
        )
    repeated_inventory_signatures = Counter(
        row["signature"]
        for row in ait_commands
        if row["command_path"] in AIT_DUPLICATE_INVENTORY_COMMAND_PATHS
    )
    repeated_inventory_signature = next((signature for signature, count in repeated_inventory_signatures.items() if count > 1), None)
    if repeated_inventory_signature:
        matched_commands = [row["command"] for row in ait_commands if row["signature"] == repeated_inventory_signature]
        hints.append(
            {
                "code": "duplicate_inventory_reads",
                "summary": "The same workflow inventory command was rerun in this turn.",
                "detail": "Reuse the earlier queue or list output unless workflow state changed in between.",
                "matched_commands": matched_commands[:5],
            }
        )
    task_start_task_only_commands = [
        row
        for row in ait_commands
        if row["command_path"] == "task start" and "--task-only" in (row.get("tokens") or [])
    ]
    change_create_commands = [row for row in ait_commands if row["command_path"] == "change create"]
    if task_start_task_only_commands and change_create_commands:
        suggested_command = _task_start_suggested_command(task_start_task_only_commands, change_create_commands)
        hints.append(
            {
                "code": "prefer_task_start",
                "summary": "This task bootstrap turn could likely use `ait task start`.",
                "detail": "When the goal is to open a task plus its first change, `ait task start` without `--task-only` can replace `ait task start --task-only` plus `ait change create`.",
                "suggested_command": suggested_command,
                "matched_commands": [row["command"] for row in [*task_start_task_only_commands, *change_create_commands][:5]],
            }
        )
    task_audit_hint = _task_audit_hint(ait_commands)
    if task_audit_hint is not None:
        hints.append(task_audit_hint)
    inventory_commands = [row for row in ait_commands if row["command_path"] in AIT_INVENTORY_COMMAND_PATHS]
    inventory_paths = {row["command_path"] for row in inventory_commands}
    inventory_cluster_suggested_command: str | None = None
    inventory_cluster_summary = "This turn revisited workflow inventory several times."
    inventory_cluster_detail = "Prefer one inventory summary first, then drill down only where it points."
    inventory_cluster_commands: list[str] = []
    if task_audit_hint is not None:
        inventory_cluster_suggested_command = str(task_audit_hint.get("suggested_command") or "").strip() or None
        inventory_cluster_summary = "This turn rebuilt one task's readiness from several inventory reads."
        inventory_cluster_detail = "Use one `ait task audit` view instead of separate task and change reads when the goal is one task's readiness."
        inventory_cluster_commands = list(task_audit_hint.get("matched_commands") or [])[:5]
    if (
        len(inventory_paths) >= 2
        and any(row["command_path"] in {"task list", "change list"} for row in inventory_commands)
        and not any(row["command_path"] == "queue summary" for row in ait_commands)
        and not (task_audit_hint is not None and not any(row["command_path"] == "task list" for row in inventory_commands))
    ):
        suggested_command = "ait queue summary --all-changes" if any(row["command_path"] == "change list" for row in inventory_commands) else "ait queue summary"
        hints.append(
            {
                "code": "queue_summary_for_inventory",
                "summary": "This workflow inventory turn could likely start with one queue summary command.",
                "detail": f"Prefer `{suggested_command}` before stitching together separate `ait task ...` and `ait change ...` reads.",
                "suggested_command": suggested_command,
                "matched_commands": [row["command"] for row in inventory_commands[:5]],
            }
        )
        inventory_cluster_suggested_command = suggested_command
        inventory_cluster_summary = "This turn stitched workflow inventory together from several commands."
        inventory_cluster_detail = "Start with one queue summary view, then open a task or change only if the queue shows a real gap."
        inventory_cluster_commands = [row["command"] for row in inventory_commands[:5]]
    elif repeated_inventory_signature and not inventory_cluster_suggested_command:
        inventory_cluster_suggested_command = matched_commands[0] if matched_commands else None
        inventory_cluster_summary = "This turn reran the same workflow inventory command."
        inventory_cluster_detail = "Reuse the earlier queue or list output unless workflow state changed in between."
        inventory_cluster_commands = matched_commands[:5]
    if inventory_cluster_suggested_command or inventory_cluster_commands:
        burst_clusters.append(
            {
                "code": "inventory_burst",
                "summary": inventory_cluster_summary,
                "detail": inventory_cluster_detail,
                "suggested_command": inventory_cluster_suggested_command,
                "matched_commands": inventory_cluster_commands,
                "matched_count": len(inventory_cluster_commands),
            }
        )
    show_signature_counts = Counter(row["signature"] for row in ait_commands if row["command_path"] in AIT_SHOW_COMMAND_PATHS)
    repeated_show_signature = next((signature for signature, count in show_signature_counts.items() if count > 1), None)
    if repeated_show_signature:
        matched_commands = [row["command"] for row in ait_commands if row["signature"] == repeated_show_signature]
        hints.append(
            {
                "code": "reuse_loaded_object_context",
                "summary": "The same workflow object was opened multiple times.",
                "detail": "Keep the fetched task, change, or session detail in working context instead of calling `show` again.",
                "signature": repeated_show_signature,
                "matched_commands": matched_commands[:5],
            }
        )
    help_commands = [
        {
            "command": preview,
            "target_tokens": help_target,
        }
        for preview, tokens in zip(previews, preview_tokens)
        if (help_target := _help_target_tokens(tokens)) is not None
    ]
    if len(help_commands) >= 2:
        help_targets = [row["target_tokens"] for row in help_commands]
        workflow_topic = _workflow_guide_topic_for_help_targets(help_targets)
        help_suggested_command: str | None = None
        help_detail = "Reuse one broader help entry point before drilling into subcommands."
        if workflow_topic:
            help_suggested_command = f"ait workflow guide {workflow_topic}"
            help_detail = f"Prefer `{help_suggested_command}` before walking several separate `--help` screens for the same flow."
            hints.append(
                {
                    "code": "prefer_workflow_guide",
                    "summary": "This help burst could likely start with one workflow guide.",
                    "detail": help_detail,
                    "suggested_command": help_suggested_command,
                    "matched_commands": [row["command"] for row in help_commands[:5]],
                }
            )
        else:
            suggested_help = _help_entrypoint(help_targets)
            detail = help_detail
            if suggested_help:
                detail = f"Start with `{suggested_help}` once before drilling into narrower help commands."
            help_suggested_command = suggested_help
            help_detail = detail
            hints.append(
                {
                    "code": "consolidate_help_queries",
                    "summary": "Several help commands were used in this turn.",
                    "detail": detail,
                    "suggested_command": suggested_help,
                    "matched_commands": [row["command"] for row in help_commands[:5]],
                }
            )
        burst_clusters.append(
            {
                "code": "help_burst",
                "summary": "This turn reopened several help screens for the same workflow.",
                "detail": help_detail,
                "suggested_command": help_suggested_command,
                "matched_commands": [row["command"] for row in help_commands[:5]],
                "matched_count": len(help_commands),
            }
        )
    land_commands = [
        row
        for row in ait_commands
        if row["top_level"] in AIT_LAND_WORKFLOW_TOP_LEVELS and not any(token in HELP_FLAGS for token in row.get("tokens") or [])
    ]
    distinct_land_top_levels = sorted({row["top_level"] for row in land_commands})
    if len(distinct_land_top_levels) >= 3 and not any(row["command_path"] == "workflow land" for row in ait_commands):
        change_ids, patchset_ids = _workflow_land_target_ids(land_commands)
        suggested_workflow_land = _workflow_land_suggested_command(
            change_ids[0] if len(change_ids) == 1 else None,
            patchset_ids[0] if len(patchset_ids) == 1 else None,
        )
        land_detail = f"Prefer `{suggested_workflow_land}` when one turn keeps hopping across patchset, attestation, review, policy, and land status for the same change."
        hints.append(
            {
                "code": "prefer_workflow_land",
                "summary": "This land workflow turn could likely start with one workflow land helper.",
                "detail": land_detail,
                "suggested_command": suggested_workflow_land,
                "matched_commands": [row["command"] for row in land_commands[:5]],
            }
        )
        land_cluster = {
            "code": "land_workflow_burst",
            "summary": "This turn crossed several land-workflow steps.",
            "detail": "A single `ait workflow land` view can summarize the next gate instead of rediscovering patchset, attestation, review, policy, and land status one command at a time.",
            "suggested_command": suggested_workflow_land,
            "matched_commands": [row["command"] for row in land_commands[:5]],
            "matched_count": len(land_commands),
            "top_levels": distinct_land_top_levels,
        }
        if len(change_ids) == 1:
            land_cluster["change_id"] = change_ids[0]
        if len(patchset_ids) == 1:
            land_cluster["patchset_id"] = patchset_ids[0]
        burst_clusters.append(land_cluster)
    read_targets = Counter(target for tokens in preview_tokens if (target := _read_target(tokens)))
    repeated_read_target = next((target for target, count in read_targets.items() if count > 1), None)
    if repeated_read_target:
        matched_reads = [preview for preview, tokens in zip(previews, preview_tokens) if _read_target(tokens) == repeated_read_target]
        hints.append(
            {
                "code": "reuse_file_read",
                "summary": "The same file was inspected multiple times.",
                "detail": f"Read `{repeated_read_target}` once with a wider range instead of reopening it across several shell commands.",
                "target": repeated_read_target,
                "matched_commands": matched_reads[:5],
            }
        )
    mergeable_run = _mergeable_inspection_run(previews, preview_tokens)
    if mergeable_run:
        suggested_command = " && ".join(mergeable_run[:4])
        detail = "Batch adjacent read-only inspection commands into one shell invocation when possible."
        if suggested_command:
            detail = f"These adjacent read-only checks could likely be batched into one shell call, e.g. `{suggested_command}`."
        hints.append(
            {
                "code": "merge_inspection_commands",
                "summary": "Several read-only shell probes could have been merged into one command.",
                "detail": detail,
                "suggested_command": suggested_command,
                "matched_commands": mergeable_run[:5],
            }
        )
    if len(previews) >= 4:
        hints.append(
            {
                "code": "batch_shell_inspection",
                "summary": "This turn used several shell commands.",
                "detail": "Batch adjacent inspection commands when possible so the reply reaches an answer with fewer tool hops.",
            }
        )
    discovery_count = sum(1 for tokens in preview_tokens if tokens and tokens[0] in DISCOVERY_TOKENS)
    if discovery_count >= 3:
        hints.append(
            {
                "code": "consolidate_file_discovery",
                "summary": "The turn spent several commands on file discovery or inspection.",
                "detail": "A broader search or one combined read can often replace multiple small probes.",
            }
        )
    optimization_summary = _select_turn_optimization_summary(hints)
    return {
        "command_count": len(commands),
        "distinct_command_count": len(preview_counts),
        "commands": previews[:8],
        "top_commands": [
            {"command": command, "count": count}
            for command, count in sorted(preview_counts.items(), key=lambda item: (-item[1], item[0]))[:5]
        ],
        "burst_clusters": burst_clusters,
        "optimization_hints": hints,
        "optimization_summary": optimization_summary,
    }


def generate_codex_session_reply(
    config: ReplyGenerationConfig,
    *,
    session: dict,
    messages: list[dict[str, str]],
    chat_id: str | int | None,
    chat_title: str | None,
    assistant_instructions: str,
    surface: str = "telegram",
    actor_identity: str | None = None,
):
    from .session_reply import AiReplyResult

    repo_root = Path(config.repo_root or Path.cwd())
    trace_context = {
        "session_id": str(session.get("session_id") or "").strip() or None,
        "surface": str(surface or "").strip() or "session",
        "chat_id": str(chat_id).strip() if chat_id is not None else None,
        "chat_title": str(chat_title or "").strip() or None,
    }
    client_config = CodexAppServerConfig(
        repo_root=repo_root,
        bin_path=config.codex_bin,
        model=config.codex_model,
        reasoning_effort=config.codex_reasoning_effort,
        sandbox=config.codex_sandbox,
        app_server_url=config.codex_app_server_url,
        app_server_host=config.codex_app_server_host,
        app_server_port=config.codex_app_server_port,
        ready_timeout_seconds=config.codex_app_server_ready_timeout_seconds,
        turn_timeout_seconds=config.codex_turn_timeout_seconds,
        child_kill_grace_seconds=config.codex_child_kill_grace_seconds,
        child_reap_timeout_seconds=config.codex_child_reap_timeout_seconds,
        websocket_max_size_bytes=config.codex_websocket_max_size_bytes,
    )
    started_at = time.monotonic()
    _log_codex_reply(
        "start",
        **trace_context,
        model=config.codex_model,
        reasoning_effort=config.codex_reasoning_effort,
        sandbox=config.codex_sandbox,
        message_count=len(messages),
        persistent_client=getattr(config, "codex_persistent_client", True),
    )
    retry_attempt = 0
    capacity_retry_attempt = 0
    active_messages = messages
    worker_key = _codex_worker_pool_key(
        strategy=getattr(config, "codex_worker_pool_strategy", "session"),
        session=session,
        surface=surface,
        chat_id=chat_id,
        actor_identity=actor_identity,
    )
    while True:
        try:
            if getattr(config, "codex_persistent_client", True):
                state = _persistent_codex_state(client_config, worker_key=worker_key)
                thread_key = _codex_thread_key(
                    session=session,
                    surface=surface,
                    chat_id=chat_id,
                    chat_title=chat_title,
                )
                with state.lock:
                    client = state.ensure_client(trace_context=trace_context)
                    thread_id, reused_thread = state.get_or_create_thread(
                        client,
                        thread_key=thread_key,
                        base_instructions=codex_base_instructions(repo_root, surface=surface),
                        developer_instructions=_developer_instructions_for_codex_attempt(
                            assistant_instructions,
                            retry_attempt=retry_attempt,
                            capacity_retry_attempt=capacity_retry_attempt,
                        ),
                        trace_context={**trace_context, "retry_attempt": retry_attempt},
                    )
                    _log_codex_reply(
                        "thread_ready",
                        **trace_context,
                        thread_key=thread_key,
                        thread_id=thread_id,
                        retry_attempt=retry_attempt,
                        reused_thread=reused_thread,
                        connection_generation=state.connection_generation,
                    )
                    turn = client.run_turn(
                        thread_id=thread_id,
                        input_text=render_codex_turn_input(
                            session_id=str(session.get("session_id") or ""),
                            chat_id=chat_id,
                            chat_title=chat_title,
                            messages=active_messages,
                            surface=surface,
                            context_mode="thread_delta" if reused_thread else "transcript_replay",
                        ),
                        trace_context={**trace_context, "thread_id": thread_id, "retry_attempt": retry_attempt},
                    )
                break
            with CodexAppServerClient(client_config) as client:
                thread = client.start_thread(
                    base_instructions=codex_base_instructions(repo_root, surface=surface),
                    developer_instructions=_developer_instructions_for_codex_attempt(
                        assistant_instructions,
                        retry_attempt=retry_attempt,
                        capacity_retry_attempt=capacity_retry_attempt,
                    ),
                    persist_extended_history=False,
                    trace_context=trace_context,
                )
                thread_id = str(thread.get("id") or "").strip()
                _log_codex_reply("thread_ready", **trace_context, thread_id=thread_id, retry_attempt=retry_attempt)
                turn = client.run_turn(
                    thread_id=thread_id,
                    input_text=render_codex_turn_input(
                        session_id=str(session.get("session_id") or ""),
                        chat_id=chat_id,
                        chat_title=chat_title,
                        messages=active_messages,
                        surface=surface,
                    ),
                    trace_context={**trace_context, "thread_id": thread_id, "retry_attempt": retry_attempt},
                )
                break
        except CodexAppServerError as exc:
            retryable = _is_retryable_codex_app_server_error(exc)
            capacity_error = _is_model_capacity_codex_app_server_error(exc)
            if getattr(config, "codex_persistent_client", True) and retryable:
                state = _persistent_codex_state(client_config, worker_key=worker_key)
                with state.lock:
                    state.close_client(reason=str(exc), trace_context=trace_context)
            if capacity_error and capacity_retry_attempt < max(int(getattr(config, "codex_capacity_retry_limit", 0) or 0), 0):
                capacity_retry_attempt += 1
                retry_attempt += 1
                active_messages = _capacity_continue_messages(
                    continue_text=str(getattr(config, "codex_capacity_continue_text", "") or ""),
                    failed_error=str(exc),
                    retry_attempt=capacity_retry_attempt,
                )
                _log_codex_reply(
                    "capacity_retrying",
                    **trace_context,
                    elapsed_sec=f"{max(time.monotonic() - started_at, 0.0):.2f}",
                    retry_attempt=retry_attempt,
                    capacity_retry_attempt=capacity_retry_attempt,
                    capacity_retry_limit=int(getattr(config, "codex_capacity_retry_limit", 0) or 0),
                    reason=str(exc),
                )
                continue
            if (
                retry_attempt < CODEX_APP_SERVER_CONNECTION_CLOSED_RETRY_LIMIT
                and retryable
            ):
                retry_attempt += 1
                _log_codex_reply(
                    "retrying",
                    **trace_context,
                    elapsed_sec=f"{max(time.monotonic() - started_at, 0.0):.2f}",
                    retry_attempt=retry_attempt,
                    reason=str(exc),
                )
                continue
            _log_codex_reply(
                "failed",
                **trace_context,
                elapsed_sec=f"{max(time.monotonic() - started_at, 0.0):.2f}",
                retry_attempt=retry_attempt,
                error=str(exc),
            )
            if capacity_error:
                raise RuntimeError(
                    _capacity_retry_exhausted_message(exc, capacity_retry_attempt=capacity_retry_attempt)
                ) from exc
            raise RuntimeError(str(exc)) from exc
    _log_codex_reply(
        "completed",
        **trace_context,
        thread_id=thread_id,
        turn_id=turn.turn_id,
        elapsed_sec=f"{max(time.monotonic() - started_at, 0.0):.2f}",
        retry_attempt=retry_attempt,
        command_count=len(turn.command_executions),
        output_chars=len(turn.text or ""),
    )
    return AiReplyResult(
        text=turn.text,
        model=config.codex_model,
        response_id=turn.turn_id,
        usage=turn.usage,
        source="codex",
        turn_analysis=_turn_analysis_for_commands(turn.command_executions),
    )
