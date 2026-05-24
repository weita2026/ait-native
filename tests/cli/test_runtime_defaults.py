from __future__ import annotations

import importlib

import pytest
import typer

from ait.cli import runtime_defaults
from ait.repo_paths import RepoContext
from ait_protocol.common import AuthorMode

from ._shared import app, runner

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_runtime_default_helpers() -> None:
    helper_names = [
        "_detect_actor_identity",
        "_detect_model_name",
        "_effective_actor_identity",
        "_effective_author_mode",
        "_effective_checkpoint_id",
        "_effective_model_name",
        "_effective_reviewer_identity",
        "_effective_session_id",
        "_normalize_model_name",
        "_parse_json_object_option",
        "_parse_key_value_options",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(runtime_defaults, name)


def test_runtime_default_resolution_contract(tmp_path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-runtime-defaults"
    repo.mkdir()
    monkeypatch.chdir(repo)
    monkeypatch.delenv("AIT_MODEL", raising=False)
    monkeypatch.delenv("CODEX_MODEL", raising=False)
    monkeypatch.delenv("OPENAI_MODEL", raising=False)
    monkeypatch.delenv("AIT_NATIVE_ACTOR", raising=False)
    monkeypatch.delenv("AIT_ACTOR", raising=False)
    monkeypatch.delenv("AIT_SESSION_ID", raising=False)
    monkeypatch.delenv("AIT_CHECKPOINT_ID", raising=False)

    init_out = runner.invoke(
        app,
        [
            "init",
            "--name",
            "housekeeper",
            "--default-author-mode",
            "human_with_ai_assist",
            "--default-model",
            "codex",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert init_out.exit_code == 0, init_out.stdout
    ctx = RepoContext.discover()

    assert runtime_defaults._effective_author_mode(ctx) == "human_with_ai_assist"
    assert runtime_defaults._effective_author_mode(ctx, AuthorMode.HUMAN_ONLY) == "human_only"
    assert runtime_defaults._effective_model_name(ctx) == "codex"
    assert runtime_defaults._effective_reviewer_identity(ctx) is None
    assert runtime_defaults._effective_actor_identity(ctx) is None
    assert runtime_defaults._effective_session_id() is None
    assert runtime_defaults._effective_checkpoint_id() is None

    monkeypatch.setenv("AIT_MODEL", "env-model")
    monkeypatch.setenv("AIT_NATIVE_ACTOR", "env-reviewer@example.com")
    monkeypatch.setenv("AIT_SESSION_ID", "S-ENV")
    monkeypatch.setenv("AIT_CHECKPOINT_ID", "K-ENV")

    assert runtime_defaults._detect_model_name() == "env-model"
    assert runtime_defaults._effective_model_name(ctx) == "env-model"
    assert runtime_defaults._effective_model_name(ctx, " explicit-model ") == "explicit-model"
    assert runtime_defaults._detect_actor_identity() == "env-reviewer@example.com"
    assert runtime_defaults._effective_actor_identity(ctx) == "env-reviewer@example.com"
    assert runtime_defaults._effective_reviewer_identity(ctx) == "env-reviewer@example.com"
    assert runtime_defaults._effective_session_id() == "S-ENV"
    assert runtime_defaults._effective_session_id(" S-EXPLICIT ") == "S-EXPLICIT"
    assert runtime_defaults._effective_checkpoint_id() == "K-ENV"
    assert runtime_defaults._effective_checkpoint_id(" K-EXPLICIT ") == "K-EXPLICIT"

    config_out = runner.invoke(
        app,
        ["config", "set", "--user-name", "Alice Example", "--user-email", "alice@example.com", "--json"],
        catch_exceptions=False,
    )
    assert config_out.exit_code == 0, config_out.stdout

    assert runtime_defaults._effective_reviewer_identity(ctx) == "Alice Example <alice@example.com>"
    assert runtime_defaults._effective_reviewer_identity(ctx, "Bob Example <bob@example.com>") == "Bob Example <bob@example.com>"
    assert runtime_defaults._effective_actor_identity(ctx) == "env-reviewer@example.com"


def test_runtime_default_option_parsers_validate_contract() -> None:
    assert runtime_defaults._parse_json_object_option(None, "--payload-json") == {}
    assert runtime_defaults._parse_json_object_option('{"command":"policy eval"}', "--payload-json") == {
        "command": "policy eval"
    }
    with pytest.raises(typer.BadParameter, match="must decode to a JSON object"):
        runtime_defaults._parse_json_object_option('["not","an","object"]', "--payload-json")
    with pytest.raises(typer.BadParameter, match="must be valid JSON"):
        runtime_defaults._parse_json_object_option("{oops", "--payload-json")

    assert runtime_defaults._parse_key_value_options(["command=policy eval", "result=pass"], "--field") == {
        "command": "policy eval",
        "result": "pass",
    }
    with pytest.raises(typer.BadParameter, match="must use key=value syntax"):
        runtime_defaults._parse_key_value_options(["command"], "--field")
    with pytest.raises(typer.BadParameter, match="must include a non-empty key"):
        runtime_defaults._parse_key_value_options([" =pass"], "--field")
