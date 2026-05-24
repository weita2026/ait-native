from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from .codex_app_server import CodexAppServerClient, CodexAppServerConfig, CodexAppServerError
from .session_reply import (
    DEFAULT_OPENAI_BASE_URL,
    ReplyGenerationConfig,
    ReplyGenerationError,
    _json_request,
    response_output_text,
)


UTILITY_BASE_INSTRUCTIONS = (
    "You are ait's utility worker for Telegram event-trigger classification and "
    "context-compression support. Do not inspect files, run commands, or use tools."
)
UTILITY_DEVELOPER_INSTRUCTIONS = (
    "Return valid JSON only. Keep values concise, stable, and implementation-facing."
)


def _strip_json_fence(text: str) -> str:
    raw = str(text or "").strip()
    if raw.startswith("```"):
        lines = raw.splitlines()
        if lines:
            lines = lines[1:]
        if lines and lines[-1].strip().startswith("```"):
            lines = lines[:-1]
        raw = "\n".join(lines).strip()
    return raw


def extract_json_object(text: str) -> dict[str, Any] | None:
    raw = _strip_json_fence(text)
    if not raw:
        return None
    try:
        parsed = json.loads(raw)
        return parsed if isinstance(parsed, dict) else None
    except json.JSONDecodeError:
        pass
    start = raw.find("{")
    end = raw.rfind("}")
    if start < 0 or end <= start:
        return None
    try:
        parsed = json.loads(raw[start : end + 1])
    except json.JSONDecodeError:
        return None
    return parsed if isinstance(parsed, dict) else None


def _run_openai_utility_prompt(
    config: ReplyGenerationConfig,
    *,
    prompt: str,
) -> dict[str, Any]:
    api_key = str(config.openai_api_key or "").strip()
    if not api_key:
        raise ReplyGenerationError("OpenAI API key is required for the utility prompt path.")
    payload: dict[str, Any] = {
        "model": config.openai_model,
        "store": False,
        "instructions": UTILITY_BASE_INSTRUCTIONS,
        "input": [{"role": "user", "content": prompt}],
        "text": {"format": {"type": "text"}},
        "max_output_tokens": min(max(int(config.openai_max_output_tokens or 0), 128), 400),
        "metadata": {"source": "ait_telegram_utility_prompt"},
    }
    if config.openai_reasoning_effort:
        payload["reasoning"] = {"effort": config.openai_reasoning_effort}
    response = _json_request(
        f"{config.openai_base_url or DEFAULT_OPENAI_BASE_URL}/responses",
        method="POST",
        payload=payload,
        headers={"Authorization": f"Bearer {api_key}"},
        timeout=config.openai_timeout_seconds,
    )
    if not isinstance(response, dict):
        raise ReplyGenerationError("Utility prompt returned an invalid OpenAI response.")
    text = response_output_text(response)
    parsed = extract_json_object(text)
    if parsed is None:
        raise ReplyGenerationError("Utility prompt did not return valid JSON.")
    return parsed


def _run_codex_utility_prompt(
    config: ReplyGenerationConfig,
    *,
    prompt: str,
) -> dict[str, Any]:
    repo_root = Path(config.repo_root or Path.cwd())
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
    try:
        with CodexAppServerClient(client_config) as client:
            thread = client.start_thread(
                base_instructions=UTILITY_BASE_INSTRUCTIONS,
                developer_instructions=UTILITY_DEVELOPER_INSTRUCTIONS,
                persist_extended_history=False,
            )
            thread_id = str(thread.get("id") or "").strip()
            if not thread_id:
                raise ReplyGenerationError("Codex utility prompt did not return a thread id.")
            turn = client.run_turn(thread_id=thread_id, input_text=prompt)
    except CodexAppServerError as exc:
        raise ReplyGenerationError(str(exc)) from exc
    parsed = extract_json_object(turn.text)
    if parsed is None:
        raise ReplyGenerationError("Codex utility prompt did not return valid JSON.")
    return parsed


def run_utility_json_prompt(
    config: ReplyGenerationConfig,
    *,
    prompt: str,
) -> dict[str, Any]:
    last_error: Exception | None = None
    try:
        return _run_codex_utility_prompt(config, prompt=prompt)
    except ReplyGenerationError as exc:
        last_error = exc
    if str(config.openai_api_key or "").strip():
        try:
            return _run_openai_utility_prompt(config, prompt=prompt)
        except ReplyGenerationError as exc:
            last_error = exc
    if last_error is not None:
        raise last_error
    raise ReplyGenerationError("Utility prompt backend is unavailable.")
