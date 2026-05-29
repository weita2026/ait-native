from __future__ import annotations

from pathlib import Path

from ait_chat.codex_reply import render_codex_turn_input
from ait_protocol.reply_runtime import DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS as PROTOCOL_REPLY_CHILD_REAP_TIMEOUT_SECONDS
from ait_chat.reply_context import session_assistant_instructions
from ait_chat.runtime_config import (
    DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS,
    load_runtime_env_file,
    resolve_reply_runtime_env_path,
)
from ait_chat.session_reply import AiReplyResult, _finalize_ai_reply_result, load_reply_generation_config


REPO_ROOT = Path(__file__).resolve().parents[1]


def _python_files_with_direct_import(root: Path, prefix: str) -> set[str]:
    matches: set[str] = set()
    for path in root.rglob("*.py"):
        text = path.read_text(encoding="utf-8")
        for line in text.splitlines():
            stripped = line.strip()
            if stripped.startswith(f"from {prefix}") or stripped.startswith(f"import {prefix}"):
                matches.add(path.relative_to(REPO_ROOT).as_posix())
                break
    return matches


def test_ait_chat_reply_runtime_stays_direct_import_free_from_telegram_runtime() -> None:
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_chat", "ait_agent.telegram") == set()


def test_reply_runtime_compatibility_constant_flows_through_protocol_seam() -> None:
    assert DEFAULT_REPLY_CODEX_CHILD_REAP_TIMEOUT_SECONDS == PROTOCOL_REPLY_CHILD_REAP_TIMEOUT_SECONDS


def test_reply_runtime_env_helpers_follow_chat_override(tmp_path: Path) -> None:
    repo_root = tmp_path / "repo"
    repo_root.mkdir(parents=True, exist_ok=True)
    custom_env = repo_root / ".config" / "reply.env"
    custom_env.parent.mkdir(parents=True, exist_ok=True)
    custom_env.write_text("AIT_CHAT_MODEL=gpt-test\n", encoding="utf-8")

    resolved = resolve_reply_runtime_env_path(repo_root, custom_env)
    assert resolved == custom_env
    assert load_runtime_env_file(resolved)["AIT_CHAT_MODEL"] == "gpt-test"


def test_load_reply_generation_config_accepts_chat_env_path_override(tmp_path: Path, monkeypatch) -> None:
    repo_root = tmp_path / "repo"
    repo_root.mkdir(parents=True, exist_ok=True)
    env_dir = repo_root / ".config"
    env_dir.mkdir(parents=True, exist_ok=True)
    env_path = env_dir / "reply.env"
    env_path.write_text("AIT_CHAT_MODEL=gpt-override\n", encoding="utf-8")

    monkeypatch.setenv("AIT_CHAT_ENV_PATH", str(env_path))
    monkeypatch.delenv("AIT_TELEGRAM_ENV_PATH", raising=False)

    config = load_reply_generation_config(repo_root=repo_root)

    assert config.openai_model == "gpt-override"


def test_finalize_ai_reply_result_extracts_discord_attachment_manifest(tmp_path: Path) -> None:
    export_path = tmp_path / "AIT_WHITEPAPER_DRAFT.md"
    export_path.write_text("# Whitepaper\n", encoding="utf-8")

    result = _finalize_ai_reply_result(
        AiReplyResult(
            text=(
                "這是白皮書草稿。\n\n"
                "```ait-attachments\n"
                '[{"local_path":"AIT_WHITEPAPER_DRAFT.md","caption":"ait whitepaper draft"}]\n'
                "```"
            ),
            model="gpt-test",
        ),
        surface="discord",
        repo_root=tmp_path,
    )

    assert result.text == "這是白皮書草稿。"
    assert result.attachments == (
        {
            "kind": "document",
            "local_path": str(export_path.resolve()),
            "file_name": "AIT_WHITEPAPER_DRAFT.md",
            "mime_type": "text/markdown",
            "caption": "ait whitepaper draft",
        },
    )


def test_finalize_ai_reply_result_extracts_telegram_image_attachment_manifest(tmp_path: Path) -> None:
    image_path = tmp_path / "diagram.png"
    image_path.write_bytes(b"png")

    result = _finalize_ai_reply_result(
        AiReplyResult(
            text=(
                "這是流程圖。\n\n"
                "```ait-attachments\n"
                '[{"local_path":"diagram.png","caption":"diagram export"}]\n'
                "```"
            ),
            model="gpt-test",
        ),
        surface="telegram",
        repo_root=tmp_path,
    )

    assert result.text == "這是流程圖。"
    assert result.attachments == (
        {
            "kind": "photo",
            "local_path": str(image_path.resolve()),
            "file_name": "diagram.png",
            "mime_type": "image/png",
            "caption": "diagram export",
        },
    )


def test_finalize_ai_reply_result_ignores_non_image_telegram_attachment_manifest(tmp_path: Path) -> None:
    export_path = tmp_path / "notes.md"
    export_path.write_text("# Notes\n", encoding="utf-8")

    result = _finalize_ai_reply_result(
        AiReplyResult(
            text=(
                "先給你路徑。\n\n"
                "```ait-attachments\n"
                '[{"local_path":"notes.md","caption":"notes"}]\n'
                "```"
            ),
            model="gpt-test",
        ),
        surface="telegram",
        repo_root=tmp_path,
    )

    assert result.text == "先給你路徑。"
    assert result.attachments == ()


def test_session_assistant_instructions_packet_worker_surface_guidance(tmp_path: Path) -> None:
    repo_root = tmp_path / "repo"
    repo_root.mkdir(parents=True, exist_ok=True)
    config = load_reply_generation_config(repo_root=repo_root)

    instructions = session_assistant_instructions(
        config,
        {
            "session_id": "S-demo",
            "title": "Compact DAG worker",
            "metadata": {
                "session_policy": "task_dag_compact_packet_worker",
                "packet_root_manifest_path": ".ait/generated/task_dag_compact_packets/demo/packet_root/packet_root_manifest.json",
                "packet_turn_artifact_path": ".ait/generated/task_dag_compact_packets/demo/compact_worker_turn.txt",
                "packet_root_path": ".ait/generated/task_dag_compact_packets/demo/packet_root",
                "workspace_root": "/tmp/lt-1218",
            },
        },
        surface="task_dag_compact_packet",
        surface_title="Compact DAG worker",
    )

    assert "worker-only compact DAG packet turn" in instructions
    assert "Start with `cat .ait/generated/task_dag_compact_packets/demo/packet_root/packet_root_manifest.json`." in instructions
    assert "Then read `.ait/generated/task_dag_compact_packets/demo/compact_worker_turn.txt` before any broader inspection." in instructions
    assert "runtime digest" not in instructions
    assert "`/tmp/lt-1218`" in instructions
    assert "repo-root `AGENTS.md`, `docs/plan.md`, `docs/ait.md`" in instructions
    assert "raw `git status`/`git diff`/`git log`" in instructions


def test_render_codex_turn_input_omits_shared_session_header_for_compact_dag_packet() -> None:
    rendered = render_codex_turn_input(
        session_id="S-compact",
        chat_id="S-compact",
        chat_title="Compact DAG worker: demo",
        surface="task_dag_compact_packet",
        messages=[{"role": "user", "content": "trim the packet prompt"}],
    )

    assert "Shared session transcript (oldest to newest):" not in rendered
    assert "session_id=S-compact" not in rendered
    assert "session_surface=task_dag_compact_packet" not in rendered
    assert "session_title=Compact DAG worker: demo" not in rendered
    assert "surface_context_id=S-compact" not in rendered
    assert rendered.startswith("User:\ntrim the packet prompt")
    assert "User 1:" not in rendered
    assert "Use the transcript as shared durable context." in rendered


def test_render_codex_turn_input_keeps_shared_session_header_for_non_packet_surface() -> None:
    rendered = render_codex_turn_input(
        session_id="S-editor",
        chat_id="C-editor",
        chat_title="VSCode Codex",
        surface="vscode",
        messages=[{"role": "user", "content": "inspect the repo"}],
    )

    assert "Shared session transcript (oldest to newest):" in rendered
    assert "session_id=S-editor" in rendered
    assert "session_surface=vscode" in rendered
    assert "session_title=VSCode Codex" in rendered
    assert "surface_context_id=C-editor" in rendered


def test_render_codex_turn_input_uses_delta_mode_for_reused_shared_thread() -> None:
    rendered = render_codex_turn_input(
        session_id="S-editor",
        chat_id="C-editor",
        chat_title="VSCode Codex",
        surface="vscode",
        context_mode="thread_delta",
        messages=[
            {"role": "user", "content": "older request"},
            {"role": "assistant", "content": "older answer"},
            {"role": "user", "content": "[web note from alice@example.com] Policy is still pending."},
            {"role": "user", "content": "inspect the repo"},
        ],
    )

    assert "Shared session transcript (oldest to newest):" not in rendered
    assert "session_id=S-editor" not in rendered
    assert "session_surface=vscode" not in rendered
    assert "surface_context_id=C-editor" not in rendered
    assert "older request" not in rendered
    assert "older answer" not in rendered
    assert "[web note from alice@example.com] Policy is still pending." in rendered
    assert "inspect the repo" in rendered
    assert "Continue the existing durable shared-session thread from this delta only." in rendered
