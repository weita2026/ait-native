from __future__ import annotations

import json
from pathlib import Path
from typing import Callable

import pytest

from ait_python import cli
from ait_python.runtime import NativeRuntime
from ait_python.errors import NativeResolutionError


CACHE_PATH_ARGV = ["--namespace-root", r"cache\namespace", "--core-repo-root", r"C:\source\core", "--ext-suffix", ".so"]
CACHE_PATH_VALUES = (r"cache\namespace", r"C:\source\core")
CACHE_COMMON = {"ext_suffix": ".so", "rustflags": "", "worker_id": "shared"}
RELEASE_NATIVE_ARGV = ["--target", "aarch64-apple-darwin", "--source-dir", r"source\target", "--runner", "runner",
                       "--runner-image", "image", "--rust-toolchain", "stable", "--rustc-path", r"C:\bin\rustc.exe"]


@pytest.mark.parametrize("arguments,target", [
    (["help"], []),
    (["help", "task"], ["task"]),
    (["task", "help"], ["task"]),
    (["help", "task", "finish"], ["task", "finish"]),
    (["task", "help", "finish"], ["task", "finish"]),
    (["help", "branch", "list"], ["line", "list"]),
    (["branch", "help", "list"], ["line", "list"]),
    (["help", "worktree", "open"], ["worktree", "path"]),
    (["worktree", "help", "open"], ["worktree", "path"]),
    (["help", "review", "task", "approve"], ["review", "task", "approve"]),
    (["review", "help", "task", "approve"], ["review", "task", "approve"]),
    (["review", "task", "help", "approve"], ["review", "task", "approve"]),
])
def test_help_commands_match_target_help_without_loading_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
    arguments: list[str], target: list[str],
) -> None:
    def unavailable():
        raise AssertionError("help must not construct the native runtime")

    monkeypatch.setattr(cli, "NativeRuntime", unavailable)
    with pytest.raises(SystemExit) as expected:
        cli.main([*target, "--help"])
    assert expected.value.code == 0
    reference = capsys.readouterr()
    with pytest.raises(SystemExit) as actual:
        cli.main(arguments)
    assert actual.value.code == 0
    assert capsys.readouterr() == reference


@pytest.mark.parametrize("arguments,invalid", [
    (["help", "missing"], "missing"),
    (["help", "task", "missing"], "missing"),
    (["task", "help", "missing"], "missing"),
    (["help", "task", "finish", "extra"], "extra"),
    (["help", "--help"], "--help"),
    (["help", "task", "--json"], "--json"),
    (["help", "task", "help", "finish"], "finish"),
    (["branch", "help", "missing"], "missing"),
    (["worktree", "help", "open", "extra"], "extra"),
])
def test_invalid_help_paths_fail_before_native_execution(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
    arguments: list[str], invalid: str,
) -> None:
    monkeypatch.setattr(cli, "NativeRuntime", lambda: pytest.fail("help invoked a domain runtime"))
    with pytest.raises(SystemExit) as error:
        cli.main(arguments)
    assert error.value.code == 2
    output = capsys.readouterr()
    assert output.out == ""
    assert f"unrecognized subcommand {invalid!r}" in output.err


@pytest.mark.parametrize("arguments,usage", [
    (["help", "help"], "ait help"),
    (["help", "task", "help"], "ait task help"),
    (["task", "help", "help"], "ait task help"),
])
def test_generated_help_commands_have_their_own_help(
    capsys: pytest.CaptureFixture[str], arguments: list[str], usage: str,
) -> None:
    with pytest.raises(SystemExit) as result:
        cli.main(arguments)
    assert result.value.code == 0
    output = capsys.readouterr()
    assert f"usage: {usage} [COMMAND ...]" in output.out
    assert "Print this message or the help of the given subcommand(s)" in output.out
    assert output.err == ""


def test_help_is_still_a_literal_positional_argument_for_leaf_commands() -> None:
    assert cli.build_parser().parse_args(["task", "show", "help"]).task_id == "help"
    assert cli.build_parser().parse_args(["diff", "help"]).paths == ["help"]


def native_presentation(name: str, args: tuple[object, ...], kwargs: dict[str, object]) -> object:
    if name == "encode_json":
        if kwargs.get("pretty"):
            return json.dumps(args[0], ensure_ascii=False, indent=2, sort_keys=True)
        return json.dumps(args[0], ensure_ascii=False, separators=(",", ":"), sort_keys=True)
    if name == "format_domain_error":
        return f"Error: {args[0]}"
    if name == "domain_error_exit_code":
        return 1
    raise AssertionError(f"unexpected native function {name}")


def use_native_calls(monkeypatch: pytest.MonkeyPatch, invoke: Callable[..., object]) -> None:
    monkeypatch.setattr(NativeRuntime, "call", invoke)
    monkeypatch.setattr(
        NativeRuntime, "resolve_callable",
        lambda runtime, name: lambda *args, **kwargs: invoke(runtime, name, *args, **kwargs),
    )


def test_terminal_parameters_become_typed_function_inputs(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path
) -> None:
    calls: list[tuple[object, ...]] = []

    def record(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append((name, args, kwargs))
        if name == "configuration_error":
            return None
        if name == "configure_repository":
            return {"saved": True}
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, record)
    assert cli.main(["config", "set", "--sprint", "on", "--plan-style", "concise", "--json"]) == 0
    assert calls == [("configure_repository", (str(tmp_path),), {
        "payload": {"sprint": "on", "plan_style": "concise"},
    }), ("encode_json", ({"saved": True},), {"pretty": True}),
        ("configuration_error", ({"saved": True},), {})]
    assert json.loads(capsys.readouterr().out) == {"saved": True}


def test_named_scope_is_passed_without_selecting_a_remote(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    calls: list[tuple[object, ...]] = []

    def record(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append((name, args, kwargs))
        if name == "task_workflow_task_list":
            return []
        if name == "project_task_list":
            return args[0]
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, record)
    assert cli.main(["task", "list", "--remote", "named", "--all", "--json"]) == 0
    assert calls == [("task_workflow_task_list", (str(tmp_path),), {
        "local": False, "remote_name": "named", "all": True,
    }), ("project_task_list", ([],), {"all": True}),
        ("encode_json", ([],), {"pretty": True})]


@pytest.mark.parametrize("argv", [
    ["task", "list", "--local", "--remote", "named"],
    ["remote", "add", "named"],
    ["config", "set", "--not-an-option", "value"],
    ["line", "list", "--all", "--archived"],
    ["task", "start", "--title", "title", "--from", "card.md#ref", "--intent", "intent"],
    ["task", "start", "--intent", "intent"],
    ["snapshot", "create", "LPT-0001", "--message", "message", "--resume", "SNP-111111111111"],
    ["status", "--full"],
    ["plan", "list", "--all", "--json"],
    ["plan", "revisions", "PR-1", "--all", "--json"],
    ["plan", "show", "PR-1", "--local", "--remote", "named"],
    ["plan", "sync"],
    ["workflow", "ready", "LPT-1", "--summary", "summary"],
    ["workflow", "ready", "LPT-1", "--run-ci-once"],
    ["workflow", "finish", "LPT-1", "--reviewer", "Reviewer"],
    ["workflow", "finish", "LPT-1", "--full"],
    ["workflow", "reconcile", "--scheduled"],
    ["workflow", "reconcile", "--apply", "--dry-run"],
    ["workflow", "reconcile", "--limit", "-1"],
    ["workflow", "guide", "--json"],
    ["config", "unset", "sprint"],
    ["config", "set", "--agent-teams", "on"],
    ["config", "set", "--plan-style", "verbose"],
    ["config", "set", "--default-author-mode", "ai_only"],
    ["config", "set", "--task-worktree-main-seed-ram-max-bytes", "9223372036854775808"],
    ["snapshot", "ancestry", "tag", "--max-depth", "0"],
    ["snapshot", "ancestry", "tag", "--limit", "-1"],
    ["snapshot", "ancestry", "tag", "--ancestors", "--descendants"],
    ["line", "create", "feature", "--from", "SNP-111111111111"],
    ["line", "switch", "feature", "--force"],
    ["line", "delete", "feature"],
    ["line", "merge", "feature", "--continue"],
    ["line", "merge", "--abort", "--message", "message"],
    ["repo", "show", "--local"],
    ["repo", "jobs", "--worker-job-index", "-1"],
    ["repo", "jobs", "--worker-job-index", "4294967296"],
    ["repo", "jobs", "--worker-job-index", "0", "--state", "queued"],
    ["repo", "jobs", "--worker-job-index", "3", "--limit", "50"],
    ["repo", "jobs", "--state", "unknown"],
    ["repo", "jobs", "--limit", "0"],
    ["repo", "jobs", "--limit", "1001"],
    ["change", "create", "LPT-1"],
    ["change", "create", "LPT-1", "--title", "revision", "--local", "--remote", "named"],
    ["change", "show", "LPT-1", "--local", "--remote", "named"],
    ["change", "publish", "LPT-1", "--local"],
    ["patchset", "publish", "LPT-1"],
    ["patchset", "publish", "LPT-1", "--summary", "summary", "--author-mode", "unknown"],
    ["patchset", "publish", "LPT-1", "--summary", "summary", "--local"],
    ["patchset", "list", "123"],
    ["patchset", "list", "  "],
    ["patchset", "show", "123"],
    ["patchset", "select", "  "],
    ["snapshot", "diff", "old", "new", "--max-bytes", "10"],
    ["snapshot", "diff", "old", "new", "--include-text", "--max-bytes", "0"],
    ["snapshot", "diff", "old", "new", "--include-text", "--max-bytes", "-1"],
    ["snapshot", "diff", "old", "new", "--include-text", "--max-bytes", "18446744073709551616"],
    ["snapshot", "restore-lines", "SNP-111111111111", "file.py"],
    ["snapshot", "restore-lines", "SNP-111111111111", "file.py", "--end", "2"],
    ["snapshot", "restore-lines", "SNP-111111111111", "file.py", "--start", "1"],
    ["snapshot", "restore-lines", "SNP-111111111111", "file.py", "--line", "1", "--start", "1", "--end", "2"],
    ["snapshot", "restore-lines", "SNP-111111111111", "file.py", "--line", "1", "--end", "2"],
    ["snapshot", "restore-lines", "SNP-111111111111", "file.py", "--line", "0"],
    ["auth", "grant", "--role", "reviewer"],
    ["auth", "grant", "actor"],
    ["auth", "grant", "--actor", "actor", "--role"],
    ["auth", "whoami", "--local"],
    ["stash", "save", "-m", "message"],
    ["stash", "list", "--all"],
    ["stash", "show"],
    ["stash", "apply", "stash-id", "--remote", "named"],
    ["stash", "pop", "stash-id", "--dry-run"],
    ["stash", "drop", "stash-id", "--force"],
    ["tag", "create", "release"],
    ["tag", "create", "release", "--message", "reason", "--snapshot"],
    ["tag", "list", "--all"],
    ["tag", "show", "release", "--remote", "named"],
    ["tag", "delete", "release", "--yes"],
    ["review", "show", "LPT-1", "--full", "--json"],
    ["review", "team", "request", "LPT-1"],
    ["review", "task", "approve", "LPT-1", "--message", "evidence"],
    ["review", "task", "approve", "LPT-1", "--patchset", "LPT-1/P-01"],
    ["review", "task", "comment", "LPT-1", "--reviewer", "Reviewer"],
    ["review", "code", "submit", "LPT-1", "--message", "evidence"],
    ["review", "code", "submit", "LPT-1", "--patchset", "LPT-1/P-01"],
    ["review", "code", "template", "--style", "unknown"],
    ["attest", "put"],
    ["attest", "put", "LPT-1/P-01", "--change", "LPT-1/C-01"],
    ["attest", "show", "LPT-1/P-01", "--repo", "repository"],
    ["policy", "show", "LPT-1/P-01", "--repo", "repository"],
    ["policy", "waive", "LPT-1/P-01", "--reason", "reason"],
    ["policy", "waive", "LPT-1/P-01", "--rule", "rule"],
    ["diff", "--stat", "--json"],
    ["diff", "--name-only", "--json"],
    ["diff", "--name-only", "--stat"],
    ["diff", "first", "--unsupported", "second"],
    ["blame", "file.py", "--line", "0"],
    ["blame", "file.py", "--start", "1"],
    ["blame", "file.py", "--end", "2"],
    ["blame", "file.py", "--line", "1", "--end", "2"],
    ["blame", "file.py", "--via-parent", "SNP-111111111111"],
    ["blame", "file.py", "--remote", "named"],
    ["blame", "file.py", "--snapshot", "SNP-111111111111", "--patchset", "LPT-1/P-01"],
    ["blame", "file.py", "--plan-id", "PR-1", "--plan-ref", "card#item"],
    ["pull", "--merge"],
    ["pull", "--force"],
    ["pull", "--restore", "--merge", "--force"],
    ["push", "--force"],
    ["push", "line-name"],
    ["worktree", "cleanup", "--limit", "-1"],
    ["worktree", "status", "--remote", "named"],
    ["worktree", "remove", "first", "--unknown", "second"],
    ["git", "migrate"],
    ["git", "export", "target", "--git-object", "commit"],
    ["git", "restore", "--generation", "generation", "--git-object", "commit"],
    ["git", "mirror", "endpoint"],
    ["git", "mirror", "endpoint", "--direction", "unknown"],
    ["gc", "prune", "--dry-run"],
    ["gc", "prune", "--yes"],
    ["gc", "validate", "--all"],
    ["current-source-cache", "run-cli", "status"],
    ["current-source-cache", "contract", "--namespace-root", "namespace"],
    ["current-source-cache", "activate", *CACHE_PATH_ARGV, "--source-mtime-ns", "1", "--owner-pid", "0"],
    ["current-source-cache", "activate", *CACHE_PATH_ARGV, "--source-mtime-ns", "1", "--register-lease", "--owner-pid", "4294967296"],
    ["current-source-cache", "mark-building", *CACHE_PATH_ARGV, "--source-mtime-ns", "18446744073709551616"],
    ["current-source-cache", "prune", "--namespace-root", "namespace", "--max-bytes", "-1"],
    ["current-source-cache", "prune", "--namespace-root", "namespace", "--now", "not-a-float"],
    ["current-source-cache", "prune", "--namespace-root", "namespace", "--now", "1_0"],
    ["current-source-cache", "prune", "--namespace-root", "namespace", "--now", " 1.5 "],
    ["current-source-cache", "prune", "--namespace-root", "namespace", "--now", "１.５"],
    ["release", "adapter", "check"],
    ["release", "candidate", "create", "--version", "1.2.0", "--channel", "unknown"],
    ["release", "native-source", "release-id", "--target", "target"],
    ["release", "native-bundle", "release-id"],
    ["release", "package", "release-id", "--channel", "unknown"],
    ["release", "formula", "release-id"],
    ["release", "promote", "release-id", "--channel", "pypi"],
    ["release", "publish", "release-id", "--public-source-root", "source"],
    ["external", "update", "dependency"],
    ["external", "update", "--latest"],
    ["external", "update", "--to", "SNP-111111111111"],
    ["external", "update", "dependency", "--latest", "--to", "SNP-111111111111"],
    ["external", "update", "dependency", "--latest", "--locked"],
    ["external", "link", "dependency"],
    ["team", "session", "ensure"],
    ["team", "session", "ensure", "--agent", "-1"],
    ["team", "session", "ensure", "--agent", "4294967296"],
    ["team", "session", "ensure", "--agent", "1", "--role", "4294967296"],
    ["team", "session", "acknowledge"],
    ["binary-db", "upgrade-u64-seconds", "--generation-root", "generation"],
    ["binary-db", "agent-pool50"],
    ["binary-db", "agent-pool-50", "--generation-root", "generation"],
    ["doctor", "plan-authority", "--json", "--json"],
    ["status", "--full", "--full", "--json"],
    ["snapshot", "create", "LPT-1", "-m", "first", "--message", "second"],
    ["task", "quick", "--intent", "first", "--intent", "second"],
    ["diff", "first", "--stat", "second", "--stat"],
    ["remote", "recover-head", "--jobs", "0"],
    ["remote", "recover-head", "--jobs", "65"],
    ["remote", "recover-head", "--jobs", "not-an-integer"],
])
def test_invalid_syntax_never_calls_rust(
    monkeypatch: pytest.MonkeyPatch, argv: list[str]
) -> None:
    def unexpected(*_args: object, **_kwargs: object) -> object:
        raise AssertionError("invalid syntax reached Rust")

    use_native_calls(monkeypatch, unexpected)
    with pytest.raises(SystemExit) as error:
        cli.main(argv)
    assert error.value.code == 2


def test_rust_errors_keep_their_message_and_failure_exit(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    calls: list[str] = []

    def failure(_self: NativeRuntime, name: str, *args: object, **_kwargs: object) -> object:
        calls.append(name)
        if name == "task_workflow_status":
            raise ValueError("exact Rust domain failure")
        if name == "format_domain_error":
            assert args == ("exact Rust domain failure",)
            return "Native-normalized public failure"
        assert name == "domain_error_exit_code" and not args
        return 17

    use_native_calls(monkeypatch, failure)
    assert cli.main(["status", "--json"]) == 17
    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == "Native-normalized public failure\n"
    assert calls == ["task_workflow_status", "format_domain_error", "domain_error_exit_code"]


def test_python_cli_uses_reviewed_native_services(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)
    assert cli.main(["init", "--json"]) == 0
    capsys.readouterr()
    assert (tmp_path / ".ait").is_dir()

    assert cli.main(["config", "set", "--plan-style", "detailed", "--json"]) == 0
    capsys.readouterr()
    assert cli.main(["config", "show", "--json"]) == 0
    configured = json.loads(capsys.readouterr().out)
    assert "detailed" in json.dumps(configured)

    assert cli.main(["remote", "add", "isolated", "http://127.0.0.1:1", "--json"]) == 1
    assert "Actor-aware Remote mutation requires effective AIT config user_name" in capsys.readouterr().err
    assert cli.main(["remote", "list", "--json"]) == 0
    remotes = json.loads(capsys.readouterr().out)
    assert isinstance(remotes, (dict, list))


@pytest.mark.parametrize("argv,export,arguments,options,projector", [
    (["task", "start", "--from", "docs/card.md#item", "--intent", "intent", "--edit-root", r"\\?\C:\work\task", "--local", "--json"],
     "task_start_operation", ("intent",), {"source": "docs/card.md#item", "edit_root": r"\\?\C:\work\task", "local": True, "human_progress": False}, "project_task_start"),
    (["task", "quick", "--intent", "intent", "--remote", "named", "--json", "--full"],
     "task_quick_operation", ("intent",), {"local": False, "remote_name": "named", "human_progress": False}, "project_task_start"),
    (["task", "finish", "LPT-0001", "--message", "message", "--sync-plan-markdown", "--local", "--json"],
     "task_finish_operation", ("LPT-0001",), {"local": True, "message": "message", "sync_plan_markdown": True}, "project_task_finish"),
    (["snapshot", "create", "LPT-0001/C-01", "--resume", "SNP-111111111111", "--json"],
     "snapshot_capture", ("LPT-0001/C-01",), {"resume_snapshot_id": "SNP-111111111111"}, "project_snapshot_capture"),
    (["commit", "LPT-0001", "-m", "message", "--json"],
     "snapshot_capture", ("LPT-0001",), {"message": "message"}, "project_snapshot_capture"),
])
def test_operation_batch_uses_typed_services_and_native_projections(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], export: str, arguments: tuple[str, ...], options: dict[str, object], projector: str,
) -> None:
    calls: list[tuple[object, ...]] = []
    original = {"service": "native", "nested": {"preserve": True}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append((name, args, kwargs))
        if name == "parse_snapshot_authoring_reference":
            return args[0]
        if name == export:
            return original
        if name == projector:
            assert args[0] is original
            return {"projected": True}
        if name == "encode_json":
            return native_presentation(name, args, kwargs)
        assert name == "task_finish_exit_code"
        return 0

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(argv) == 0
    if export == "snapshot_capture":
        assert calls.pop(0) == ("parse_snapshot_authoring_reference", (arguments[0],), {})
    assert calls[0] == (export, (str(tmp_path), *arguments), options)
    assert calls[1] == (projector, (original,), {"full": "--full" in argv})
    assert json.loads(capsys.readouterr().out) == {"projected": True}


def test_finish_prints_native_text_and_uses_native_partial_closeout_exit(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"status": "completed", "cleanup": "needs-recovery"}
    calls: list[str] = []

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append(name)
        if name == "task_finish_operation":
            return payload
        if name == "format_task_finish":
            assert args == (payload, "LPT-0001")
            return "exact native finish text"
        assert name == "task_finish_exit_code" and args == (payload,)
        return 75

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["task", "finish", "LPT-0001", "--local"]) == 75
    assert calls == ["task_finish_operation", "format_task_finish", "task_finish_exit_code"]
    assert capsys.readouterr().out == "exact native finish text\n"


def test_configuration_post_save_error_follows_the_native_payload(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "configure_repository":
            return {"saved": True}
        if name == "configuration_error":
            assert args == ({"saved": True},)
            return "original native setup error"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["config", "set", "--agent-teams", "enabled", "--json"]) == 1
    output = capsys.readouterr()
    assert json.loads(output.out) == {"saved": True}
    assert output.err == "Error: original native setup error\n"


def test_status_human_output_comes_from_its_native_formatter(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"native": "status"}
    calls: list[str] = []

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append(name)
        if name == "task_workflow_status":
            return payload
        assert name == "format_status" and args == (payload,)
        return "Exact native status\nnext: native action"

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["status"]) == 0
    assert capsys.readouterr().out == "Exact native status\nnext: native action\n"
    assert calls == ["task_workflow_status", "format_status"]


def test_json_codec_receives_uint64_and_projected_values_unchanged(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"value": 2**64 - 1, "text": "資料"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_workflow_status":
            return payload
        if name == "project_status":
            assert args[0] is payload and kwargs == {"full": True}
            return payload
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["status", "--json", "--full"]) == 0
    assert json.loads(capsys.readouterr().out) == payload


@pytest.mark.parametrize("argv,export,arguments,options", [
    (["plan", "list", "--json"], "plan_list_operation", (), {"local": False}),
    (["plan", "revisions", "PR-1", "--remote", "named", "--json"],
     "plan_revisions_operation", ("PR-1",), {"local": False, "remote_name": "named"}),
    (["plan", "show", "artifact:docs/card.md", "--revision", "revision-number:2", "--local", "--json"],
     "plan_show_operation", ("artifact:docs/card.md",), {"local": True, "revision": "revision-number:2"}),
    (["plan", "items", "PR-1", "--json"], "plan_items_operation", ("PR-1",), {"local": False}),
    (["plan", "inspect", "title:Plan title", "--json"],
     "plan_inspect_operation", ("title:Plan title",), {"local": False}),
    (["plan", "candidates", "--all", "--contains", "Team,release", "--json"],
     "plan_candidates_operation", (), {"local": False, "include_all": True, "contains": "Team,release"}),
    (["plan", "sync", "docs/card.md", "--plan-ref", "stable/ref", "--prune", "--reconcile", "--local", "--json"],
     "plan_sync_operation", ("docs/card.md",), {"local": True, "plan_ref": "stable/ref", "prune": True, "reconcile": True}),
])
def test_plan_cli_passes_only_user_selection_to_repository_aware_services(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], export: str, arguments: tuple[str, ...], options: dict[str, object],
) -> None:
    calls: list[tuple[object, ...]] = []
    payload: object = [] if export in {"plan_list_operation", "plan_revisions_operation"} else {"native": True}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append((name, args, kwargs))
        if name == export:
            return payload
        if name == "plan_sync_error":
            assert args[0] is payload
            return None
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(argv) == 0
    assert calls[0] == (export, (str(tmp_path), *arguments), options)
    assert json.loads(capsys.readouterr().out) == payload
    assert all(name not in {"plan_command_execute_list", "plan_command_execute_sync"} for name, *_ in calls)


def test_plan_sync_emits_partial_payload_before_native_terminal_error(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"status": "partial_success", "preserved": ["PR-1"]}
    calls: list[str] = []

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append(name)
        if name == "plan_sync_operation":
            return payload
        if name == "plan_sync_error":
            assert args[0] is payload
            return "original native terminal error"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["plan", "sync", "docs", "--local", "--json"]) == 1
    output = capsys.readouterr()
    assert json.loads(output.out) == payload
    assert output.err == "Error: original native terminal error\n"
    assert calls == ["plan_sync_operation", "encode_json", "plan_sync_error", "format_domain_error", "domain_error_exit_code"]


def test_unavailable_formatter_stops_before_mutating_operation(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def resolve(_self: NativeRuntime, name: str) -> Callable[..., object]:
        if name == "format_task_finish":
            raise NativeResolutionError("required formatter unavailable")
        return lambda *_args, **_kwargs: None

    def unexpected(*_args: object, **_kwargs: object) -> object:
        raise AssertionError("operation ran before presentation was available")

    monkeypatch.setattr(NativeRuntime, "call", unexpected)
    monkeypatch.setattr(NativeRuntime, "resolve_callable", resolve)
    assert cli.main(["task", "finish", "LPT-1", "--local"]) == 1
    assert capsys.readouterr().err == "Error: required formatter unavailable\n"


@pytest.mark.parametrize("verb", ["ready", "finish"])
def test_workflow_phase_status_and_repository_projection_stay_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path, verb: str
) -> None:
    calls: list[tuple[object, ...]] = []
    payload = {"status": "completed", "opaque": {"preserve": True}}
    canonical = str(tmp_path / "authority")

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append((name, args, kwargs))
        if name == "repository_authority_root":
            assert args == (str(tmp_path),)
            return canonical
        if name == "workflow_" + verb + "_operation":
            return {"payload": payload, "exit_code": 75}
        if name == "project_workflow_" + verb:
            assert args == (canonical, payload, "LPT-1")
            assert kwargs == {"remote_name": "named", "full": True}
            return {"native_projection": True}
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    argv = ["workflow", verb, "LPT-1", "--apply", "--remote", "named", "--expected-identity", "identity.json", "--json", "--full"]
    if verb == "ready":
        argv += ["--snapshot-message", "snapshot", "--summary", "summary", "--tests", "pass", "--author-mode", "ai_only_experimental", "--run-ci-once"]
    else:
        argv += ["--reviewer", "Reviewer", "--review-message", "review"]
    assert cli.main(argv) == 75
    assert calls[0][0] == "repository_authority_root"
    operation = calls[1]
    assert operation[0] == "workflow_" + verb + "_operation"
    assert operation[1] == (str(tmp_path), "LPT-1")
    assert operation[2]["expected_identity_path"] == "identity.json"
    assert operation[2]["apply"] is True
    assert json.loads(capsys.readouterr().out) == {"native_projection": True}


def test_workflow_finish_keeps_native_authority_after_invoking_worktree_disappears(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path
) -> None:
    invoking = tmp_path / "managed"
    invoking.mkdir()
    payload = {"accepted": True}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "repository_authority_root":
            assert invoking.exists()
            return str(tmp_path)
        if name == "workflow_finish_operation":
            assert args == (str(invoking), "LPT-1")
            monkeypatch.chdir(tmp_path)
            invoking.rmdir()
            return {"payload": payload, "exit_code": 0}
        if name == "project_workflow_finish":
            assert not invoking.exists()
            assert args == (str(tmp_path), payload, "LPT-1")
            return payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(invoking)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["workflow", "finish", "LPT-1", "--apply", "--json"]) == 0
    assert json.loads(capsys.readouterr().out) == payload


@pytest.mark.parametrize("outcome", [
    {"payload": {}}, {"payload": {}, "exit_code": False}, {"payload": {}, "exit_code": 256},
])
def test_invalid_native_phase_outcome_fails_without_emitting_success(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], outcome: object
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "repository_authority_root":
            return "/native-authority"
        if name == "workflow_ready_operation":
            return outcome
        raise AssertionError("invalid result reached a projector")

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["workflow", "ready", "LPT-1", "--json"]) == 1
    output = capsys.readouterr()
    assert output.out == "" and output.err.startswith("Error: native")


def test_plan_revision_text_uses_native_bounding_and_scope_options(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = [{"revision": "native"}]

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "plan_revisions_operation":
            return payload
        assert name == "format_plan_revisions" and args == (payload, "PR-1")
        assert kwargs == {"all": True, "local": False, "remote_name": "named"}
        return "native history output"

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["plan", "revisions", "PR-1", "--all", "--remote", "named"]) == 0
    assert capsys.readouterr().out == "native history output\n"


def test_reconcile_does_not_reimplement_scheduled_policy(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "workflow_reconcile_operation":
            assert kwargs == {"remote_name": "named", "task_id": "LPT-1", "apply": True,
                              "safe_only": False, "scheduled": True, "limit": 10}
            return {"native": True}
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["workflow", "reconcile", "--remote", "named", "--task", "LPT-1", "--apply", "--scheduled", "--limit", "10", "--json"]) == 0
    assert json.loads(capsys.readouterr().out) == {"native": True}


@pytest.mark.parametrize("verb", ["start", "quick"])
@pytest.mark.parametrize("terminal", [False, True])
@pytest.mark.parametrize("json_output", [False, True])
def test_task_progress_is_native_and_only_enabled_for_terminal_text(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
    verb: str, terminal: bool, json_output: bool,
) -> None:
    payload = {"task_id": "LPT-1"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == f"task_{verb}_operation":
            assert kwargs == {"local": True, "title": "title",
                              "human_progress": terminal and not json_output}
            return payload
        if name == "project_task_start":
            assert args == (payload,) and kwargs == {"full": False}
            return payload
        if name == f"format_task_{verb}":
            assert args == (payload,) and not kwargs
            return "native Task result"
        return native_presentation(name, args, kwargs)

    monkeypatch.setattr(cli.sys.stdout, "isatty", lambda: terminal)
    use_native_calls(monkeypatch, invoke)
    argv = ["task", verb, "--title", "title", "--intent", "intent", "--local"]
    assert cli.main(argv + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native Task result\n"


def test_quick_uses_the_combined_native_card_renderer(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"quick_card": {"artifact_path": "docs/sprints/quick.md"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_quick_operation":
            return payload
        assert name == "format_task_quick" and args == (payload,)
        return "native Task start\ncard: docs/sprints/quick.md"

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["task", "quick", "--intent", "intent", "--local"]) == 0
    assert capsys.readouterr().out == "native Task start\ncard: docs/sprints/quick.md\n"


def test_abandon_uses_native_package_phase_status(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"status": "accepted", "opaque": "native package outcome"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_abandon_operation":
            assert args[1] == "@/package/identity.json"
            assert kwargs == {"local": False, "remote_name": "named"}
            return {"payload": payload, "exit_code": 75}
        if name == "format_task_abandon":
            assert args == (payload, "@/package/identity.json")
            return "native accepted package with recovery pending"
        raise AssertionError(f"unexpected call {name}")

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["task", "abandon", "@/package/identity.json", "--remote", "named"]) == 75
    assert capsys.readouterr().out == "native accepted package with recovery pending\n"


def test_config_text_reuses_the_identical_original_domain_request(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    original: list[object] = []
    payload = {"saved": True, "team_bootstrap": {"status": "pending"}}
    calls: list[str] = []

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append(name)
        if name == "configure_repository":
            original.append(kwargs["payload"])
            return payload
        if name == "format_configuration":
            assert args[0] is payload and args[1] is original[0]
            assert args[1] == {"agent_teams": "enabled", "plan_language": "zh-TW"}
            return "native updated keys\nnative Team condition"
        assert name == "configuration_error" and args == (payload,)
        return None

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["config", "set", "--agent-teams", "enabled", "--plan-language", "zh-TW"]) == 0
    assert capsys.readouterr().out == "native updated keys\nnative Team condition\n"
    assert calls == ["configure_repository", "format_configuration", "configuration_error"]


@pytest.mark.parametrize("argv,operation,formatter,arguments,options,requested", [
    (["init", "--repair-existing"], "initialize_repository", "format_initialize_repository", (),
     {"policy_profile": "prototype", "repair_existing": True}, None),
    (["config", "show"], "task_workflow_config_show", "format_configuration_show", (), {}, None),
    (["config", "unset", "plan-style"], "unset_configuration", "format_configuration_unset", ("plan-style",), {}, "plan-style"),
    (["remote", "list"], "task_workflow_remote_list", "format_remote_list", (), {}, None),
    (["queue", "summary", "--remote", "named"], "task_workflow_queue_summary", "format_queue_summary", (), {"remote_name": "named"}, None),
    (["doctor", "memory-root"], "doctor_memory_root_operation", "format_doctor_memory_root", (), {}, None),
    (["doctor", "runtime-root"], "task_workflow_doctor_runtime_root", "format_doctor_runtime_root", (), {}, None),
    (["doctor", "plan-authority"], "doctor_plan_authority_operation", "format_doctor_plan_authority", (), {}, None),
])
def test_basic_native_text_batch_has_no_generic_text_substitute(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, formatter: str, arguments: tuple[str, ...],
    options: dict[str, object], requested: str | None,
) -> None:
    payload = {"native": "opaque"}
    calls: list[str] = []

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append(name)
        if name == operation:
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        assert name == formatter
        assert args == ((payload, requested) if requested else (payload,))
        return "exact native text"

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(argv) == 0
    assert capsys.readouterr().out == "exact native text\n"
    assert calls == [operation, formatter]


def test_snapshot_ancestry_passes_tag_and_typed_bounds_to_native_query(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path
) -> None:
    payload = {"native": "ancestry"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "snapshot_ancestry_operation":
            assert args == (str(tmp_path), "release-tag")
            assert kwargs == {"descendants": True, "first_parent": True, "max_depth": 5, "limit": 7}
            return payload
        assert name == "format_snapshot_ancestry" and args == (payload,)
        assert kwargs == {"all": True}
        return "native bounded ancestry and continuation"

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["snapshot", "ancestry", "release-tag", "--descendants", "--first-parent", "--max-depth", "5", "--limit", "7", "--all"]) == 0
    assert capsys.readouterr().out == "native bounded ancestry and continuation\n"


def test_snapshot_ancestor_does_not_infer_status_from_payload_fields(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"ancestor": True, "distance": 0}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "snapshot_is_ancestor_operation":
            assert args[1:] == ("older-tag", "newer-tag")
            return {"payload": payload, "exit_code": 1}
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["snapshot", "is-ancestor", "older-tag", "newer-tag", "--json"]) == 1
    assert json.loads(capsys.readouterr().out) == payload


def test_empty_merge_base_text_emits_no_stdout(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"merge_bases": []}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "snapshot_merge_base_operation":
            assert args[1:] == ("left-tag", "right-tag") and kwargs == {"all": True}
            return {"payload": payload, "exit_code": 1}
        assert name == "format_snapshot_merge_base" and args == (payload,)
        return ""

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["snapshot", "merge-base", "left-tag", "right-tag", "--all"]) == 1
    assert capsys.readouterr().out == ""


@pytest.mark.parametrize("arguments", [
    ["ancestry", "missing"], ["is-ancestor", "missing", "tag", "--json"], ["merge-base", "missing", "tag"],
])
def test_snapshot_lookup_errors_use_the_query_renderer_and_status_two(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], arguments: list[str]
) -> None:
    calls: list[str] = []

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        calls.append(name)
        if name in {"snapshot_ancestry_operation", "snapshot_is_ancestor_operation", "snapshot_merge_base_operation"}:
            raise ValueError("native missing Snapshot/tag")
        if name == "format_snapshot_query_error":
            assert args == ("native missing Snapshot/tag",)
            return "native query failure text"
        assert name == "snapshot_query_error_exit_code" and not args
        return 2

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["snapshot", *arguments]) == 2
    output = capsys.readouterr()
    assert output.out == "" and output.err == "native query failure text\n"
    assert calls[-2:] == ["format_snapshot_query_error", "snapshot_query_error_exit_code"]


@pytest.mark.parametrize("json_output", [False, True])
def test_task_inventory_fetches_complete_raw_rows_before_native_selection(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], json_output: bool
) -> None:
    raw = [{"status": "completed", "opaque": 1}, {"status": "in_progress", "opaque": 2}]
    projected = [{"native_selected": True}]

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_workflow_task_list":
            assert kwargs == {"local": False, "remote_name": "named", "all": True}
            return raw
        if name == "project_task_list":
            assert args[0] is raw and kwargs == {"all": False}
            return projected
        if name == "format_task_list":
            assert args[0] is raw
            assert kwargs == {"local": False, "remote_name": "named", "all": False}
            return "native total=2 terminal=1 and scoped expansion"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    arguments = ["task", "list", "--remote", "named"] + (["--json"] if json_output else [])
    assert cli.main(arguments) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == projected
    else:
        assert output == "native total=2 terminal=1 and scoped expansion\n"


def test_snapshot_list_json_uses_native_order_and_all_projection(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    raw = [{"opaque": "unsorted-raw"}]
    selected = [{"opaque": "native-order"}]

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_workflow_snapshot_list":
            return raw
        if name == "project_snapshot_list":
            assert args[0] is raw and kwargs == {"all": True}
            return selected
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["snapshot", "list", "--all", "--json"]) == 0
    assert json.loads(capsys.readouterr().out) == selected


@pytest.mark.parametrize("json_output", [False, True])
def test_snapshot_show_keeps_native_inspection_and_emits_only_payload_in_json(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], json_output: bool
) -> None:
    inspection = {"payload": {"snapshot": "native-resolved"}, "delta": {"evidence": "native-parent-diff"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "inspect_snapshot_operation":
            assert args[1] == "release-tag" and kwargs == {"include_delta": not json_output}
            return inspection
        if name == "format_snapshot_show":
            assert args[0] is inspection and kwargs == {"files": True}
            return "native identity/files/parent evidence"
        assert name == "encode_json" and args[0] is inspection["payload"]
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    arguments = ["snapshot", "show", "release-tag", "--files"] + (["--json"] if json_output else [])
    assert cli.main(arguments) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == inspection["payload"]
    else:
        assert output == "native identity/files/parent evidence\n"


@pytest.mark.parametrize("json_output", [False, True])
def test_line_inventory_preserves_native_current_line_priority_context(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], json_output: bool
) -> None:
    inventory = {"payload": [{"name": "native-order"}], "current_line": "native-current"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "line_inventory_operation":
            assert kwargs == {"include_all": True, "archived": False, "remote_name": "named"}
            return inventory
        if name == "format_line_list":
            assert args[0] is inventory
            assert kwargs == {"include_all": True, "archived": False, "remote_name": "named"}
            return "native inventory priority and counts"
        assert name == "encode_json" and args[0] is inventory["payload"]
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    argv = ["branch", "list", "--all", "--remote", "named"] + (["--json"] if json_output else [])
    assert cli.main(argv) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == inventory["payload"]
    else:
        assert output == "native inventory priority and counts\n"


@pytest.mark.parametrize("argv,operation,arguments,options,formatter", [
    (["line", "create", "feature", "--from-snapshot", "SNP-111111111111", "--switch"],
     "task_workflow_line_create", ("feature",), {"from_snapshot": "SNP-111111111111", "switch": True}, "format_line_create"),
    (["line", "switch", "feature", "--restore", "--force"],
     "task_workflow_line_switch", ("feature",), {"restore": True, "force": True}, "format_line_switch"),
    (["line", "show", "feature", "--remote", "named"],
     "line_show_operation", (), {"name": "feature", "remote_name": "named"}, "format_line_show"),
    (["line", "archive", "feature"], "task_workflow_line_archive", ("feature",), {}, "format_line_archive"),
    (["line", "rename", "old", "new", "--remote", "named"],
     "task_workflow_line_rename", ("old", "new"), {"remote_name": "named"}, "format_line_rename"),
    (["line", "delete", "feature", "--yes"],
     "task_workflow_line_delete", ("feature",), {"confirm": True}, "format_line_delete"),
    (["line", "merge", "--continue", "--message", "resolved"],
     "line_merge_operation", (), {"message": "resolved", "continue_merge": True, "abort_merge": False}, "format_line_merge"),
])
def test_line_operations_use_existing_typed_mutations_and_native_text(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object], formatter: str,
) -> None:
    payload = {"native": "line result"}

    def invoke(_self: NativeRuntime, name: str, /, *args: object, **kwargs: object) -> object:
        if name == operation:
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        assert name == formatter and args == (payload,)
        return "native line result text"

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(argv) == 0
    assert capsys.readouterr().out == "native line result text\n"


def test_line_cleanup_keeps_native_selection_and_expansion_fields(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"opaque": "cleanup protections and candidates"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_workflow_line_cleanup":
            assert kwargs == {"idle_for": "12h", "cleanup_kind": "review", "limit": 3,
                              "include_protected": True, "apply": True}
            return payload
        assert name == "format_line_cleanup" and args == (payload, "12h")
        assert kwargs == {"all": True, "include_protected": True, "cleanup_kind": "review", "limit": 3}
        return "native protection groups/examples/follow-up"

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["line", "cleanup", "--idle-for", "12h", "--kind", "review", "--limit", "3", "--include-protected", "--all", "--yes"]) == 0
    assert capsys.readouterr().out == "native protection groups/examples/follow-up\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,options,presentation_options", [
    (["show"], "show", {}, {}),
    (["show", "--remote", "named"], "show", {"remote_name": "named"}, {"remote_name": "named"}),
    (["ci-capabilities", "--remote", "named"], "ci_capabilities",
     {"remote_name": "named"}, {"remote_name": "named"}),
    (["retire"], "retire", {"abort": False}, {}),
    (["retire", "--remote", "named", "--abort"], "retire", {"remote_name": "named", "abort": True}, {}),
    (["restore", "--remote", "named"], "restore", {"remote_name": "named"}, {}),
    (["jobs"], "jobs", {}, {}),
    (["jobs", "--remote", "named", "--state", "failed", "--limit", "1000"], "jobs",
     {"remote_name": "named", "state": "failed", "limit": 1000},
     {"remote_name": "named", "state": "failed", "limit": 1000}),
    (["jobs", "--worker-job-index", "4294967295"], "jobs",
     {"worker_job_index": 4294967295}, {"worker_job_index": 4294967295}),
])
def test_repository_routes_preserve_typed_selection_and_native_results(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, options: dict[str, object],
    presentation_options: dict[str, object], json_output: bool,
) -> None:
    payload = {"native": {"repository_index": 2**64 - 1, "archive": "native identity"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == f"repository_{operation}_operation":
            assert args == (str(tmp_path),) and kwargs == options
            return payload
        if name == f"format_repository_{operation}":
            assert args == (payload,) and kwargs == presentation_options
            return "native Repository result and guidance"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["repo", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native Repository result and guidance\n"


def test_repository_retirement_requires_native_presentation_before_mutation(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def resolve(_self: NativeRuntime, name: str) -> Callable[..., object]:
        if name == "format_repository_retire":
            raise NativeResolutionError("retirement formatter unavailable")
        return lambda *args, **kwargs: None

    def invoke(*args: object, **kwargs: object) -> object:
        pytest.fail("retirement cannot run before all required exports resolve")

    monkeypatch.setattr(NativeRuntime, "resolve_callable", resolve)
    monkeypatch.setattr(NativeRuntime, "call", invoke)
    assert cli.main(["repo", "retire", "--remote", "named"]) == 1
    assert capsys.readouterr().err == "Error: retirement formatter unavailable\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options,formatter", [
    (["change", "create", "LPT-1", "--title", "revision", "--base-line", "main",
      "--fork-snapshot", "SNP-111111111111", "--local"],
     "change_create_operation", ("LPT-1", "revision"),
     {"local": True, "base_line": "main", "fork_snapshot": "SNP-111111111111"}, "format_change_create"),
    (["change", "show", "LPT-1", "--remote", "named"], "change_show_operation", ("LPT-1",),
     {"local": False, "remote_name": "named"}, "format_change_show"),
    (["change", "revert", "LPT-1/C-01", "--dry-run", "--remote", "named"],
     "change_revert_operation", ("LPT-1/C-01",),
     {"local": False, "remote_name": "named", "force": False, "dry_run": True}, "format_change_revert"),
    (["change", "replay", "LPT-1", "--local"], "change_replay_operation", ("LPT-1",),
     {"local": True, "force": False, "dry_run": False}, "format_change_replay"),
    (["change", "replay", "LPT-1/C-01", "--onto", "current-line", "--force", "--dry-run"],
     "change_replay_operation", ("LPT-1/C-01",),
     {"local": False, "onto": "current-line", "force": True, "dry_run": True}, "format_change_replay"),
    (["change", "close", "LPT-1/C-01", "--local"], "change_close_operation", ("LPT-1/C-01",),
     {"local": True}, "format_change_close"),
    (["change", "publish", "LPT-1", "--remote", "named"], "change_publish_operation", ("LPT-1",),
     {"remote_name": "named"}, "format_change_publish"),
    (["patchset", "publish", " LPT-1 ", "--summary", "publication", "--author-mode",
      "ai_only_experimental", "--remote", "named"],
     "patchset_publish_operation", ("LPT-1", "publication"),
     {"author_mode": "ai_only_experimental", "remote_name": "named"}, "format_patchset_publish"),
    (["patchset", "list", " LPT-1 ", "--remote", "named"], "patchset_list_operation", ("LPT-1",),
     {"remote_name": "named"}, "format_patchset_list"),
    (["patchset", "show", " LPT-1/P-01 "], "task_workflow_patchset_show", ("LPT-1/P-01",),
     {}, "format_patchset_show"),
    (["patchset", "select", "LPT-1/P-01", "--remote", "named"],
     "task_workflow_patchset_select", ("LPT-1/P-01",), {"remote_name": "named"}, "format_patchset_select"),
    (["patchset", "ci-status", "native-selector"], "task_workflow_patchset_ci_status", ("native-selector",),
     {}, "format_patchset_ci_status"),
    (["patchset", "rerun-ci", "LPT-1/P-01", "--remote", "named"],
     "task_workflow_patchset_rerun_ci", ("LPT-1/P-01",), {"remote_name": "named"}, "format_patchset_rerun_ci"),
])
def test_change_and_patchset_routes_keep_resolution_and_output_in_rust(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object],
    formatter: str, json_output: bool,
) -> None:
    payload = {"native": {"readable_summary": "retained", "close_reconciliation": "opaque"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == operation:
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == formatter:
            assert args == (payload,) and not kwargs
            return "native resolution, summary and CI guidance"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(argv + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native resolution, summary and CI guidance\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("include_all", [False, True])
def test_change_inventory_projects_the_full_native_result(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
    json_output: bool, include_all: bool,
) -> None:
    payload = [{"state": "closed"}, {"state": "open"}]

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_workflow_change_list":
            assert kwargs == {"local": False, "remote_name": "named"}
            return payload
        if name == "project_change_list":
            assert args[0] is payload and kwargs == {"all": include_all}
            return {"native_view": include_all}
        if name == "format_change_list":
            assert args[0] is payload
            assert kwargs == {"local": False, "remote_name": "named", "all": include_all}
            return "native bounded Change inventory and guidance"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    argv = ["change", "list", "--remote", "named"]
    assert cli.main(argv + (["--all"] if include_all else []) + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == {"native_view": include_all}
    else:
        assert output == "native bounded Change inventory and guidance\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options,formatter", [
    (["diff", "old-tag", "new-tag"], "snapshot_diff_operation", ("old-tag", "new-tag"),
     {"include_text": False}, "format_snapshot_diff"),
    (["diff", "old-tag", "SNP-111111111111", "--include-text", "--max-bytes", "4096"],
     "snapshot_diff_operation", ("old-tag", "SNP-111111111111"),
     {"include_text": True, "max_bytes": 4096}, "format_snapshot_diff"),
    (["diff", "old-tag", "new-tag", "--include-text"], "snapshot_diff_operation", ("old-tag", "new-tag"),
     {"include_text": True}, "format_snapshot_diff"),
    (["revert", "head-tag", "--force", "--dry-run"], "snapshot_revert_operation", ("head-tag",),
     {"force": True, "dry_run": True}, "format_snapshot_revert"),
    (["replay", "source-tag"], "snapshot_replay_operation", ("source-tag",),
     {"force": False, "dry_run": False}, "format_snapshot_replay"),
    (["replay", "source-tag", "--onto", "current-line", "--dry-run"], "snapshot_replay_operation", ("source-tag",),
     {"onto": "current-line", "force": False, "dry_run": True}, "format_snapshot_replay"),
    (["restore-lines", "SNP-111111111111", "src/file.py", "--line", "7"],
     "task_workflow_snapshot_restore_lines", ("SNP-111111111111", "src/file.py"),
     {"line": 7, "yes": False}, "format_snapshot_restore_lines"),
    (["restore-lines", "SNP-111111111111", r"src\file.py", "--start", "3", "--end", "9", "--yes"],
     "task_workflow_snapshot_restore_lines", ("SNP-111111111111", r"src\file.py"),
     {"start_line": 3, "end_line": 9, "yes": True}, "format_snapshot_restore_lines"),
])
def test_snapshot_workspace_routes_keep_resolution_locks_and_text_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object],
    formatter: str, json_output: bool,
) -> None:
    payload = {"native": {"delta": "opaque", "overwrite_risk": "guarded", "line": "native-current"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == operation:
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == formatter:
            assert args == (payload,) and not kwargs
            return "native Snapshot plan, bounded diff and evidence"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["snapshot", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native Snapshot plan, bounded diff and evidence\n"


@pytest.mark.parametrize("argv,operation", [
    (["diff", "old-tag", "new-tag"], "snapshot_diff_operation"),
    (["revert", "head-tag"], "snapshot_revert_operation"),
    (["replay", "source-tag"], "snapshot_replay_operation"),
    (["restore-lines", "SNP-111111111111", "src/file.py", "--start", "9", "--end", "3"],
     "task_workflow_snapshot_restore_lines"),
])
def test_snapshot_workspace_errors_keep_domain_status_one(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], argv: list[str], operation: str,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == operation:
            raise ValueError("native Snapshot selection or workspace guard failed")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["snapshot", *argv, "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native Snapshot selection or workspace guard failed\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,options,payload", [
    (["whoami"], "whoami", {}, {"identity": "native-local", "mode": "local"}),
    (["whoami", "--remote", "named", "--repo", "selected"], "whoami",
     {"remote_name": "named", "repo_name": "selected"}, {"identity": "native-remote"}),
    (["bindings", "--remote", "named", "--repo", "selected"], "bindings",
     {"remote_name": "named", "repo_name": "selected"}, [{"actor": "native", "roles": ["owner"]}]),
    (["grant", "--actor", " actor ", "--role", "reviewer", "--role", "developer", "--role", "reviewer",
      "--repo", "selected", "--remote", "named"], "grant",
     {"actor_identity": " actor ", "roles": ["reviewer", "developer", "reviewer"],
      "repo_name": "selected", "remote_name": "named"}, {"native_role_binding": "opaque"}),
])
def test_auth_routes_preserve_native_identity_roles_and_output(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, options: dict[str, object], payload: object, json_output: bool,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_workflow_auth_" + operation:
            assert args == (str(tmp_path),) and kwargs == options
            return payload
        if name == "format_auth_" + operation:
            assert args == (payload,) and not kwargs
            return "native identity, binding columns and roles"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["auth", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native identity, binding columns and roles\n"


def test_auth_grant_keeps_role_validation_and_denial_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_workflow_auth_grant":
            assert kwargs == {"actor_identity": "actor", "roles": []}
            raise ValueError("native role or authorization validation failed")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["auth", "grant", "--actor", "actor", "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native role or authorization validation failed\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options,formatter", [
    (["save"], "stash_save_operation", (), {"keep_workspace": False}, "format_stash_save"),
    (["save", "--message", "stash message", "--keep-workspace"], "stash_save_operation", (),
     {"message": "stash message", "keep_workspace": True}, "format_stash_save"),
    (["list"], "task_workflow_stash_list", (), {}, "format_stash_list"),
    (["show", "stash-id"], "task_workflow_stash_show", ("stash-id",), {}, "format_stash_show"),
    (["apply", "stash-id"], "stash_apply_operation", ("stash-id",), {"force": False}, "format_stash_apply"),
    (["pop", "stash-id", "--force"], "stash_pop_operation", ("stash-id",), {"force": True}, "format_stash_pop"),
    (["drop", "stash-id"], "stash_drop_operation", ("stash-id",), {}, "format_stash_drop"),
])
def test_stash_routes_use_locked_operations_and_native_inventory_text(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object],
    formatter: str, json_output: bool,
) -> None:
    payload = {"native": {"source_line": "opaque", "stash_record": "guarded"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == operation:
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == formatter:
            assert args == (payload,) and not kwargs
            return "native Stash result and conflict guidance"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["stash", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native Stash result and conflict guidance\n"


@pytest.mark.parametrize("verb", ["apply", "pop"])
def test_stash_unsaved_work_conflict_guidance_stays_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], verb: str,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "stash_" + verb + "_operation":
            assert args[1] == "stash-id" and kwargs == {"force": False}
            raise ValueError("native unsaved-work conflict; save or use the native force option")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["stash", verb, "stash-id", "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native unsaved-work conflict; save or use the native force option\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options,payload", [
    (["create", "release", "--message", "reason"], "create", ("release", "reason"), {},
     {"name": "release", "source_line": "native-current"}),
    (["create", "release", "--message", "reason", "--snapshot", "SNP-111111111111"],
     "create", ("release", "reason"), {"snapshot_id": "SNP-111111111111"}, {"name": "release"}),
    (["list"], "list", (), {}, [{"name": "native-order"}]),
    (["show", "release"], "show", ("release",), {}, {"name": "release", "message": "native reason"}),
    (["delete", "release"], "delete", ("release",), {}, {"deleted": "release"}),
])
def test_tag_routes_keep_head_selection_reference_semantics_and_text_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object],
    payload: object, json_output: bool,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "tag_" + operation + "_operation":
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == "format_tag_" + operation:
            assert args == (payload,) and not kwargs
            return "native Tag identity, reason and source Line"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["tag", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native Tag identity, reason and source Line\n"


@pytest.mark.parametrize("argv,operation,arguments", [
    (["create", "release", "--message", ""], "create", ("release", "")),
    (["create", "release", "--message", "reason\nsecond line"], "create", ("release", "reason\nsecond line")),
    (["delete", "missing"], "delete", ("missing",)),
])
def test_tag_validation_and_missing_record_failures_stay_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
    argv: list[str], operation: str, arguments: tuple[str, ...],
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "tag_" + operation + "_operation":
            assert args[1:] == arguments and not kwargs
            raise ValueError("native Tag validation or missing-record error")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["tag", *argv, "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native Tag validation or missing-record error\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options", [
    (["team", "request", "LPT-1", "--group", "alpha", "--group", "beta", "--group", "alpha",
      "--patchset", " LPT-1/P-01 ", "--note", "request note", "--remote", "named"],
     "team_request", ("LPT-1", ["alpha", "beta", "alpha"]),
     {"patchset_id": "LPT-1/P-01", "note": "request note", "remote_name": "named"}),
    (["team", "approve", "LPT-1", "--patchset", "LPT-1/P-01", "--reviewer", "Reviewer",
      "--message", "evidence", "--remote", "named"], "team_approve", ("LPT-1",),
     {"patchset_id": "LPT-1/P-01", "reviewer": "Reviewer", "message": "evidence", "remote_name": "named"}),
    (["team", "request-changes", "LPT-1", "--message", "required fix"],
     "team_request_changes", ("LPT-1",), {"message": "required fix"}),
    (["team", "comment", "LPT-1", "--reviewer", "Reviewer"],
     "team_comment", ("LPT-1",), {"reviewer": "Reviewer"}),
    (["team", "defer", "LPT-1"], "team_defer", ("LPT-1",), {}),
    (["code", "submit", "LPT-1", "--patchset", "LPT-1/P-01", "--message", "structured\nevidence",
      "--reviewer", "Reviewer", "--remote", "named"], "code_submit", ("LPT-1", "LPT-1/P-01", "structured\nevidence"),
     {"reviewer": "Reviewer", "remote_name": "named"}),
])
def test_review_team_and_code_routes_preserve_native_identity_and_candidate_rules(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[object, ...], options: dict[str, object], json_output: bool,
) -> None:
    payload = {"native": {"review": "opaque", "automatic_task_review": "native decision"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "validate_public_patchset_argument":
            return str(args[0]).strip()
        if name == "review_" + operation + "_operation":
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == "format_review_" + operation:
            assert args == (payload,) and not kwargs
            return "native review lanes, requests and acting Reviewer"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["review", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native review lanes, requests and acting Reviewer\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("package_review", [False, True])
@pytest.mark.parametrize("verb", ["approve", "request-changes", "comment", "defer"])
def test_review_task_passes_complete_envelope_to_native_projector_or_formatter(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
    json_output: bool, package_review: bool, verb: str,
) -> None:
    operation = verb.replace("-", "_")
    selector = "@/package/identity.json" if package_review else "LPT-1/P-01"
    payload = {"payload": {"native_result": "opaque"}, "package_review": package_review}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "validate_review_patchset_selector":
            assert args == (selector,) and not kwargs
            return selector
        if name == "review_task_" + operation + "_operation":
            if verb == "approve":
                assert args[1:] == ("LPT-1", selector, "evidence") and kwargs == {"remote_name": "named"}
            else:
                assert args[1:] == ("LPT-1",)
                assert kwargs == {"patchset_id": selector, "message": "evidence", "remote_name": "named"}
            return payload
        if name == "project_review_task_operation":
            assert args[0] is payload and not kwargs
            return {"native_projected_package": package_review}
        if name == "format_review_task_" + operation:
            assert args[0] is payload and not kwargs
            return "native package title" if package_review else "native ordinary review title"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    argv = ["review", "task", verb, "LPT-1", "--patchset", selector, "--message", "evidence", "--remote", "named"]
    assert cli.main(argv + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == {"native_projected_package": package_review}
    else:
        assert output == ("native package title\n" if package_review else "native ordinary review title\n")


@pytest.mark.parametrize("json_output", [False, True])
def test_review_show_preserves_native_full_text_and_compact_json(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], json_output: bool,
) -> None:
    payload = {"native": "full review and request evidence"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "review_show_operation":
            assert args[1:] == ("LPT-1",) and kwargs == {"remote_name": "named"}
            return payload
        if name == "project_review_show":
            assert args == (payload,) and not kwargs
            return {"native": "compact view"}
        if name == "format_review_show":
            assert args == (payload,) and not kwargs
            return "native full review and request evidence"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["review", "show", "LPT-1", "--remote", "named"] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == {"native": "compact view"}
    else:
        assert output == "native full review and request evidence\n"


@pytest.mark.parametrize("style", [None, "inline"])
@pytest.mark.parametrize("json_output", [False, True])
def test_review_code_template_does_not_discover_a_repository(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], style: str | None, json_output: bool,
) -> None:
    payload = {"native_template": style or "numbered"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "task_workflow_review_code_template":
            assert not args and kwargs == {"style": style or "numbered"}
            return payload
        if name == "format_review_code_template":
            assert args == (payload,) and not kwargs
            return "native required review template"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    argv = ["review", "code", "template"] + (["--style", style] if style else [])
    assert cli.main(argv + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native required review template\n"


@pytest.mark.parametrize("argv,operation", [
    (["task", "approve", "LPT-1", "--patchset", "@/package/identity.json", "--message", "evidence"],
     "review_task_approve_operation"),
    (["code", "submit", "LPT-1", "--patchset", "LPT-1/P-01", "--message", "evidence", "--reviewer", "other"],
     "review_code_submit_operation"),
])
def test_review_identity_validation_remains_a_native_domain_error(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], argv: list[str], operation: str,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name in ("validate_review_patchset_selector", "validate_public_patchset_argument"):
            return args[0]
        if name == operation:
            raise ValueError("native identity-file or acting-Reviewer validation failed")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["review", *argv, "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native identity-file or acting-Reviewer validation failed\n"


@pytest.mark.parametrize("argv,validator,value", [
    (["task", "comment", "LPT-1", "--patchset", "@relative/identity.json"],
     "validate_review_patchset_selector", "@relative/identity.json"),
    (["task", "comment", "LPT-1", "--patchset", " @/package/identity.json "],
     "validate_review_patchset_selector", " @/package/identity.json "),
    (["task", "comment", "LPT-1", "--patchset", "123"], "validate_review_patchset_selector", "123"),
    (["task", "comment", "LPT-1", "--patchset", ""], "validate_review_patchset_selector", ""),
    (["team", "approve", "LPT-1", "--patchset", "123"], "validate_public_patchset_argument", "123"),
    (["code", "submit", "LPT-1", "--patchset", " ", "--message", "evidence"],
     "validate_public_patchset_argument", " "),
])
def test_review_scalar_validation_errors_use_parser_status_two_before_execution(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
    argv: list[str], validator: str, value: str,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        assert name == validator and args == (value,) and not kwargs
        raise ValueError("native frozen scalar selector error")

    use_native_calls(monkeypatch, invoke)
    with pytest.raises(SystemExit) as error:
        cli.main(["review", *argv, "--json"])
    assert error.value.code == 2
    output = capsys.readouterr()
    assert not output.out
    assert "native frozen scalar selector error" in output.err


@pytest.mark.parametrize("selector", [None, " native selector "])
def test_review_task_uses_native_scalar_result_or_omits_the_default_selector(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], selector: str | None,
) -> None:
    payload = {"payload": {"review": "native"}, "package_review": False}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "validate_review_patchset_selector":
            assert args == (selector,) and not kwargs
            return "LPT-1/P-09"
        if name == "review_task_comment_operation":
            assert args[1:] == ("LPT-1",)
            assert kwargs == ({"patchset_id": "LPT-1/P-09"} if selector is not None else {})
            return payload
        if name == "project_review_task_operation":
            assert args[0] is payload
            return payload["payload"]
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    argv = ["review", "task", "comment", "LPT-1", "--json"]
    assert cli.main(argv + (["--patchset", selector] if selector is not None else [])) == 0
    assert json.loads(capsys.readouterr().out) == payload["payload"]


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options", [
    (["attest", "put", " LPT-1/P-01 "], "attest_put", (), {"patchset_id": "LPT-1/P-01"}),
    (["attest", "put", "LPT-1/P-01", "--tests", "pass", "--author-mode", "ai_with_human_review", "--remote", "named"],
     "attest_put", (), {"patchset_id": "LPT-1/P-01", "tests": "pass", "author_mode": "ai_with_human_review",
                           "remote_name": "named"}),
    (["attest", "show", "LPT-1/P-01", "--remote", "named"], "attest_show", ("LPT-1/P-01",),
     {"remote_name": "named"}),
    (["policy", "eval", "LPT-1/P-01", "--remote", "named"], "policy_eval", ("LPT-1/P-01",),
     {"remote_name": "named"}),
    (["policy", "waive", "LPT-1/P-01", "--rule", "rule", "--reason", " typed reason ",
      "--expires-at", "2026-12-01T00:00:00Z", "--remote", "named"],
     "policy_waive", ("LPT-1/P-01",), {"rule_name": "rule", "reason": " typed reason ",
      "expires_at": "2026-12-01T00:00:00Z", "remote_name": "named"}),
])
def test_attestation_and_policy_inputs_and_success_status_remain_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object], json_output: bool,
) -> None:
    payload = {"passed": False, "native": {"ci_state": "derived", "author_mode": "immutable"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "validate_public_patchset_argument":
            return str(args[0]).strip()
        if name == "task_workflow_" + operation:
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == "format_" + operation:
            assert args == (payload,) and not kwargs
            return "native evidence, Policy evaluation and waiver result"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(argv + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native evidence, Policy evaluation and waiver result\n"


@pytest.mark.parametrize("json_output", [False, True])
def test_policy_show_passes_full_evidence_to_native_text_and_native_projection_to_json(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], json_output: bool,
) -> None:
    payload = {"checks": [{"native_evidence": "complete"}], "passed": False}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "validate_public_patchset_argument":
            return args[0]
        if name == "task_workflow_policy_show":
            assert args[1:] == ("LPT-1/P-01",) and kwargs == {"remote_name": "named"}
            return payload
        if name == "project_policy_show":
            assert args[0] is payload and not kwargs
            return {"native": "compact Policy"}
        if name == "format_policy_show":
            assert args[0] is payload and not kwargs
            return "native full Policy check evidence"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["policy", "show", "LPT-1/P-01", "--remote", "named"] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == {"native": "compact Policy"}
    else:
        assert output == "native full Policy check evidence\n"


@pytest.mark.parametrize("field", ["lint", "security", "license", "model"])
def test_attestation_unsupported_manual_fields_are_rejected_by_the_native_operation(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], field: str,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "validate_public_patchset_argument":
            return args[0]
        if name == "task_workflow_attest_put":
            assert kwargs == {"patchset_id": "LPT-1/P-01", field: "raw-value"}
            raise ValueError("native compact Attestation rejects this explicit field")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["attest", "put", "LPT-1/P-01", "--" + field, "raw-value", "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native compact Attestation rejects this explicit field\n"


@pytest.mark.parametrize("argv", [
    ["attest", "put", "123"], ["policy", "eval", "123"],
])
def test_attestation_and_policy_native_scalar_errors_keep_parser_status_two(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], argv: list[str],
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        assert name == "validate_public_patchset_argument" and args == ("123",) and not kwargs
        raise ValueError("public Patchset reference required; use TASK_ID/P-##")

    use_native_calls(monkeypatch, invoke)
    with pytest.raises(SystemExit) as error:
        cli.main(argv)
    assert error.value.code == 2
    assert "public Patchset reference required" in capsys.readouterr().err


@pytest.mark.parametrize("rendered", ["", "native first line\n\nlast native line\n"])
@pytest.mark.parametrize("argv,paths,renderer", [
    ([], [], "render_workspace_diff_text"),
    (["first", "--stat", "second"], ["first", "second"], "render_workspace_diff_stat"),
    (["--name-only", "first", "second"], ["first", "second"], "render_workspace_diff_names"),
    (["first", "second", "--stat"], ["first", "second"], "render_workspace_diff_stat"),
    (["--", "--stat", "-name"], ["--stat", "-name"], "render_workspace_diff_text"),
])
def test_workspace_diff_preserves_path_order_and_exact_native_stdout(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], paths: list[str], renderer: str, rendered: str,
) -> None:
    payload = {"native": "workspace diff"}
    resolved: list[str] = []

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "workspace_diff_operation":
            assert args == (str(tmp_path), paths) and not kwargs
            return payload
        assert name == renderer and args == (payload,) and not kwargs
        return rendered

    def resolve(_self: NativeRuntime, name: str) -> Callable[..., object]:
        resolved.append(name)
        if name.startswith("render_workspace_diff_"):
            assert name == renderer
        return lambda *args, **kwargs: None

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    monkeypatch.setattr(NativeRuntime, "resolve_callable", resolve)
    assert cli.main(["diff", *argv]) == 0
    assert renderer in resolved
    assert capsys.readouterr().out == rendered


def test_workspace_diff_json_keeps_the_complete_native_payload(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"files": [{"native": "text and binary evidence"}], "changed_paths": ["first", "second"]}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "workspace_diff_operation":
            assert args[1:] == (["first", "second"],) and not kwargs
            return payload
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["diff", "first", "--json", "second"]) == 0
    assert json.loads(capsys.readouterr().out) == payload


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,options", [
    (["src/file.py"], {}),
    (["src/file.py", "--line", "5", "--snapshot", "SNP-111111111111", "--via-parent", "SNP-222222222222"],
     {"line": 5, "snapshot_id": "SNP-111111111111", "via_parent_snapshot_id": "SNP-222222222222"}),
    (["src/file.py", "--start", "3", "--end", "7", "--patchset", " LPT-1/P-01 ", "--remote", "named"],
     {"start_line": 3, "end_line": 7, "patchset_id": "LPT-1/P-01", "remote_name": "named"}),
    (["docs/card.md", "--plan-id", "PR-67", "--line", "1"], {"line": 1, "plan_id": "PR-67"}),
    (["docs/card.md", "--plan-ref", "docs/card.md#ref"], {"plan_ref": "docs/card.md#ref"}),
])
def test_blame_preserves_native_selection_and_evidence_output(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], options: dict[str, object], json_output: bool,
) -> None:
    payload = {"native": {"parent": "selected", "hunks": "complete", "warning": "retained"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "validate_public_patchset_argument":
            return str(args[0]).strip()
        if name == "task_workflow_blame":
            assert args == (str(tmp_path), argv[0]) and kwargs == options
            return payload
        if name == "format_blame":
            assert args[0] is payload and not kwargs
            return "native Blame target, parent, range, hunks and warnings"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["blame", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native Blame target, parent, range, hunks and warnings\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,verb,options", [
    (["pull"], "pull", {"merge": False, "restore": False, "force": False}),
    (["pull", "--remote", "named", "--line", "selected", "--merge", "--restore"], "pull",
     {"remote_name": "named", "line_name": "selected", "merge": True, "restore": True, "force": False}),
    (["pull", "--restore", "--force"], "pull", {"merge": False, "restore": True, "force": True}),
    (["push"], "push", {}),
    (["push", "--remote", "named", "--line", "selected"], "push", {"remote_name": "named", "line_name": "selected"}),
])
def test_remote_sync_routes_preserve_native_locking_selection_and_projection(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], verb: str, options: dict[str, object], json_output: bool,
) -> None:
    payload = {"native": {"relationship": "opaque", "restore_applied": False},
               "phase_timings_ms": {"native": 123}, "remote_sync_metrics": {"native": 456}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == verb + "_operation":
            assert args == (str(tmp_path),) and kwargs == options
            return payload
        if name == "project_remote_sync":
            assert args[0] is payload and not kwargs
            return {"native_projection": "sync payload without metrics"}
        if name == "format_" + verb:
            assert args[0] is payload and not kwargs
            return "native remote synchronization result"
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(argv + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == {"native_projection": "sync payload without metrics"}
    else:
        assert output == "native remote synchronization result\n"


@pytest.mark.parametrize("verb", ["pull", "push"])
def test_remote_sync_domain_errors_are_emitted_without_frontend_retries(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], verb: str,
) -> None:
    operations: list[str] = []

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == verb + "_operation":
            operations.append(name)
            raise ValueError("native remote capability or synchronization policy failure")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main([verb, "--remote", "named", "--json"]) == 1
    assert operations == [verb + "_operation"]
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native remote capability or synchronization policy failure\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options,formatter,presentation_options", [
    (["status", "selected", "--snapshot", "SNP-111111111111", "--verbose"],
     "task_workflow_worktree_status", (), {"name": "selected", "snapshot_id": "SNP-111111111111"},
     "format_worktree_status", {"verbose": True}),
    (["show"], "worktree_show_operation", (), {}, "format_worktree_show", {}),
    (["list"], "task_workflow_worktree_list", (), {"refresh_status": False}, "format_worktree_list", {}),
    (["list", "--refresh"], "task_workflow_worktree_list", (), {"refresh_status": True}, "format_worktree_list", {}),
    (["doctor"], "task_workflow_worktree_doctor", (), {"refresh_status": False}, "format_worktree_doctor", {}),
    (["restore", "selected", "--snapshot", "SNP-111111111111", "--path", "src", "--path", "README.md", "--force", "--dry-run"],
     "worktree_restore_operation", (), {"name": "selected", "snapshot_id": "SNP-111111111111", "paths": ["src", "README.md"],
     "force": True, "dry_run": True}, "format_worktree_restore", {}),
    (["cleanup-candidates"], "task_workflow_worktree_cleanup_candidates", (),
     {"older_than": "7d", "allow_manual_only": False, "include_protected": False}, "format_worktree_cleanup_candidates", {}),
    (["cleanup-candidates", "--older-than", "12h", "--policy", "after_idle", "--allow-manual-only", "--include-protected"],
     "task_workflow_worktree_cleanup_candidates", (),
     {"older_than": "12h", "cleanup_policy": "after_idle", "allow_manual_only": True, "include_protected": True},
     "format_worktree_cleanup_candidates", {}),
    (["cleanup", "--dry-run"], "worktree_cleanup_operation", (),
     {"older_than": "7d", "allow_manual_only": False, "dry_run": True, "yes": False}, "format_worktree_cleanup", {}),
    (["cleanup", "--older-than", "12h", "--policy", "after_idle", "--allow-manual-only", "--limit", "3", "--yes"],
     "worktree_cleanup_operation", (), {"older_than": "12h", "cleanup_policy": "after_idle", "allow_manual_only": True,
     "limit": 3, "dry_run": False, "yes": True}, "format_worktree_cleanup", {}),
    (["prune-stale", "--dry-run"], "worktree_prune_stale_operation", (), {"dry_run": True, "yes": False},
     "format_worktree_prune_stale", {}),
    (["recreate", "missing", "--dry-run"], "worktree_recreate_operation", (), {"name": "missing", "dry_run": True},
     "format_worktree_recreate", {}),
    (["recover-task", "LPT-1", "--remote", "named", "--dry-run"], "worktree_recover_task_operation", ("LPT-1",),
     {"remote_name": "named", "dry_run": True}, "format_worktree_recover_task", {}),
    (["recover-task", "LPT-1", "--change", "LPT-1/C-01", "--dry-run"], "worktree_recover_task_operation", ("LPT-1",),
     {"change_ref": "LPT-1/C-01", "dry_run": True}, "format_worktree_recover_task", {}),
    (["restore-owned-head", "selected", "--dry-run"], "worktree_restore_owned_head_operation", (), {"name": "selected", "dry_run": True},
     "format_worktree_restore_owned_head", {}),
    (["rebase", "selected", "--onto", "base", "--dry-run"], "worktree_rebase_operation", (),
     {"name": "selected", "onto_line": "base", "continue_rebase": False, "abort_rebase": False, "dry_run": True},
     "format_worktree_rebase", {}),
    (["rebase", "--continue"], "worktree_rebase_operation", (),
     {"continue_rebase": True, "abort_rebase": False, "dry_run": False}, "format_worktree_rebase", {}),
])
def test_worktree_routes_keep_native_defaults_guards_recovery_and_text(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object],
    formatter: str, presentation_options: dict[str, object], json_output: bool,
) -> None:
    payload = {"native": {"protection": "guarded", "recovery": "opaque", "status": "complete"}}

    def invoke(_self: NativeRuntime, name: str, /, *args: object, **kwargs: object) -> object:
        if name == operation:
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == formatter:
            assert args[0] is payload and kwargs == presentation_options
            return "native Worktree status, protection and recovery guidance"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["worktree", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native Worktree status, protection and recovery guidance\n"


@pytest.mark.parametrize("verb", ["path", "open"])
@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("shell_output", [False, True])
def test_worktree_path_uses_native_usage_touch_projection_and_shell_command(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
    verb: str, json_output: bool, shell_output: bool,
) -> None:
    payload = {"native": {"path": "physical-or-alias", "shell": "native command", "usage": "touched"}}

    def invoke(_self: NativeRuntime, name: str, /, *args: object, **kwargs: object) -> object:
        if name == "worktree_path_operation":
            assert kwargs == {"name": "selected"}
            return payload
        if name == "project_worktree_path":
            assert args[0] is payload and not kwargs
            return {"native": "frozen path fields"}
        if name == "format_worktree_path":
            assert args[0] is payload and kwargs == {"shell_output": shell_output}
            return "native shell command" if shell_output else "native alias path"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    argv = ["worktree", verb, "selected"] + (["--shell"] if shell_output else [])
    assert cli.main(argv + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == {"native": "frozen path fields"}
    else:
        assert output == ("native shell command\n" if shell_output else "native alias path\n")


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("all_worktrees,native_status", [(False, 0), (True, 2)])
def test_worktree_sync_returns_the_native_exit_after_emitting_the_result(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], json_output: bool,
    all_worktrees: bool, native_status: int,
) -> None:
    payload = {"ok": False, "native": "per-worktree failures"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "worktree_sync_operation":
            assert kwargs == {"all_worktrees": all_worktrees, "force": False, "dry_run": True}
            return payload
        if name == "worktree_sync_exit_code":
            assert args[0] is payload and kwargs == {"all_worktrees": all_worktrees}
            return native_status
        if name == "format_worktree_sync":
            assert args[0] is payload and not kwargs
            return "native grouped sync results"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    argv = ["worktree", "sync", "--dry-run"] + (["--all"] if all_worktrees else [])
    assert cli.main(argv + (["--json"] if json_output else [])) == native_status
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native grouped sync results\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,names,all_stale", [
    (["--all-stale", "--dry-run"], [], True),
    (["single", "--dry-run"], ["single"], False),
    (["first", "--dry-run", "second"], ["first", "second"], False),
])
def test_worktree_remove_text_uses_requested_count_and_native_preview_branches(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], json_output: bool,
    argv: list[str], names: list[str], all_stale: bool,
) -> None:
    payload = {"native_rows": ["first", "second", "third"]}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "worktree_remove_operation":
            assert args[1:] == (names,)
            assert kwargs == {"all_stale": all_stale, "delete_path": False, "force": False, "dry_run": True, "yes": False}
            return payload
        if name == "format_worktree_remove":
            assert args[0] is payload
            assert kwargs == {"all_stale": all_stale, "dry_run": True, "multiple": len(names) > 1}
            return "native stale/preview/single/multiple removal text"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["worktree", "remove", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native stale/preview/single/multiple removal text\n"


@pytest.mark.parametrize("argv,operation", [
    (["status", "--snapshot", "SNP-111111111111", "--line", "main"], "task_workflow_worktree_status"),
    (["restore", "--path", "src/file.py"], "worktree_restore_operation"),
    (["cleanup"], "worktree_cleanup_operation"),
    (["cleanup", "--limit", "0", "--yes"], "worktree_cleanup_operation"),
    (["prune-stale"], "worktree_prune_stale_operation"),
    (["sync", "selected", "--all"], "worktree_sync_operation"),
    (["rebase", "--continue", "--abort"], "worktree_rebase_operation"),
    (["remove", "selected", "--all-stale"], "worktree_remove_operation"),
])
def test_worktree_confirmation_and_operation_conflicts_remain_native_domain_errors(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], argv: list[str], operation: str,
) -> None:
    def invoke(_self: NativeRuntime, name: str, /, *args: object, **kwargs: object) -> object:
        if name == operation:
            raise ValueError("native confirmation, protection or option validation failed")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["worktree", *argv, "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native confirmation, protection or option validation failed\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options", [
    (["migrate", "--staging-root", r"C:\staged\generation"], "migrate", (r"C:\staged\generation",), {"apply": False}),
    (["migrate", "--staging-root", "relative-generation", "--apply"], "migrate", ("relative-generation",), {"apply": True}),
    (["recover", "--apply"], "recover", (), {"apply": True}),
    (["import", "https://example.invalid/source.git", "--all-branches-and-tags", "--dry-run"], "import",
     ("https://example.invalid/source.git",), {"all_branches_and_tags": True, "dry_run": True}),
    (["import", "local-source"], "import", ("local-source",), {"all_branches_and_tags": False, "dry_run": False}),
    (["export", "target"], "export", ("target",), {"all_lines_and_tags": False, "dry_run": False}),
    (["export", "target", "--generation", "generation", "--git-object", "commit", "--all-lines-and-tags", "--dry-run"],
     "export", ("target",), {"generation": "generation", "git_object": "commit", "all_lines_and_tags": True, "dry_run": True}),
    (["restore", "--generation", "generation", "--git-object", "commit", "--target", r"relative\new-tree"],
     "restore", ("generation", "commit", r"relative\new-tree"), {}),
    (["mirror", "endpoint", "--direction", "inbound", "--dry-run"], "mirror", ("endpoint", "inbound"), {"dry_run": True}),
    (["mirror", "endpoint", "--direction", "outbound"], "mirror", ("endpoint", "outbound"), {"dry_run": False}),
    (["mirror", "endpoint", "--direction", "bidirectional", "--dry-run"], "mirror", ("endpoint", "bidirectional"), {"dry_run": True}),
])
def test_git_routes_preserve_native_locking_selectors_and_blocked_plan_status(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object], json_output: bool,
) -> None:
    payload = {"status": "blocked", "ok": False, "native_plan": {"generation": "immutable", "direction": "validated"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "git_" + operation + "_operation":
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == "format_git_" + operation:
            assert args[0] is payload and not kwargs
            return "native Git generation, plan and recovery result"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["git", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native Git generation, plan and recovery result\n"


def test_git_explicit_empty_generation_reaches_native_domain_validation(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "git_export_operation":
            assert args[1:] == ("target",)
            assert kwargs == {"generation": "", "git_object": "", "all_lines_and_tags": False, "dry_run": False}
            raise ValueError("native Git generation or object selection is invalid")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["git", "export", "target", "--generation", "", "--git-object", "", "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native Git generation or object selection is invalid\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,verb,options", [
    (["stats"], "stats", {}),
    (["prune"], "prune", {"apply": False}),
    (["prune", "--apply"], "prune", {"apply": True}),
])
def test_gc_stats_and_prune_keep_native_preview_apply_and_payloads(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], verb: str, options: dict[str, object], json_output: bool,
) -> None:
    payload = {"needs_attention": True, "native": {"roots": "protected", "packs": "opaque"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "gc_" + verb + "_operation":
            assert args == (str(tmp_path),) and kwargs == options
            return payload
        if name == "format_gc_" + verb:
            assert args[0] is payload and not kwargs
            return "native content maintenance summary and plan"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["gc", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native content maintenance summary and plan\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("native_status", [0, 1])
def test_gc_validation_emits_its_result_before_returning_native_status(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], json_output: bool, native_status: int,
) -> None:
    payload = {"needs_attention": native_status == 1, "native_validation": "complete"}
    emitted: list[str] = []

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "gc_validate_operation":
            assert not kwargs
            return payload
        if name == "gc_validate_exit_code":
            assert args[0] is payload and not kwargs
            emitted.append(capsys.readouterr().out)
            return native_status
        if name == "format_gc_validate":
            assert args[0] is payload and not kwargs
            return "native exact content validation"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["gc", "validate"] + (["--json"] if json_output else [])) == native_status
    assert len(emitted) == 1
    if json_output:
        assert json.loads(emitted[0]) == payload
    else:
        assert emitted[0] == "native exact content validation\n"
    assert not capsys.readouterr().out


def test_gc_validation_keeps_native_post_output_schema_errors(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    payload = {"native_validation": "missing attention field"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "gc_validate_operation":
            return payload
        if name == "gc_validate_exit_code":
            assert args[0] is payload
            raise ValueError("native GC validation requires a boolean needs_attention field")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["gc", "validate", "--json"]) == 1
    output = capsys.readouterr()
    assert json.loads(output.out) == payload
    assert output.err == "Error: native GC validation requires a boolean needs_attention field\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options,formatter", [
    (["contract", *CACHE_PATH_ARGV], "current_source_native_cache_contract", CACHE_PATH_VALUES, CACHE_COMMON,
     "format_current_source_cache_contract"),
    (["contract", *CACHE_PATH_ARGV, "--rustflags=-Copt", "--worker-id", "worker", "--core-source-fingerprint", "core-fp",
      "--server-source-fingerprint", "server-fp"], "current_source_native_cache_contract", CACHE_PATH_VALUES,
     {"ext_suffix": ".so", "rustflags": "-Copt", "worker_id": "worker", "core_source_fingerprint": "core-fp",
      "server_source_fingerprint": "server-fp"}, "format_current_source_cache_contract"),
    (["mark-building", *CACHE_PATH_ARGV, "--source-mtime-ns", "18446744073709551615"],
     "current_source_cache_mark_building_operation", (*CACHE_PATH_VALUES, 2**64 - 1),
     {**CACHE_COMMON, "extra_json": "{}"}, "format_current_source_cache_mark_building"),
    (["activate", *CACHE_PATH_ARGV, "--source-mtime-ns", "1", "--register-lease", "--owner-pid", "4294967295",
      "--extra-json", '{"native":true}'], "current_source_cache_activate_operation", (*CACHE_PATH_VALUES, 1),
     {**CACHE_COMMON, "register_lease": True, "owner_pid": 2**32 - 1, "extra_json": '{"native":true}'},
     "format_current_source_cache_activate"),
    (["release-lease", "--lease-path", "cache/lease", "--namespace-root", "cache", "--remove-unleased-ready"],
     "current_source_cache_release_lease_operation", ("cache/lease", "cache"), {"remove_unleased_ready": True},
     "format_current_source_cache_release_lease"),
    (["prune", "--namespace-root", "cache"], "current_source_cache_prune_operation", ("cache",),
     {"remove_unleased_ready": False}, "format_current_source_cache_prune"),
    (["prune", "--namespace-root", "cache", "--now", "1.5", "--idle-ttl-seconds", "0", "--build-stale-seconds", "2",
      "--max-bytes", "18446744073709551615", "--remove-unleased-ready"], "current_source_cache_prune_operation", ("cache",),
     {"now": 1.5, "idle_ttl_seconds": 0, "build_stale_seconds": 2, "max_bytes": 2**64 - 1, "remove_unleased_ready": True},
     "format_current_source_cache_prune"),
    (["seed-canonical", *CACHE_PATH_ARGV, "--repo-root", "repo", "--canonical-repo-root", "canonical",
      "--core-source-mtime-ns", "9", "--core-source-fingerprint", "exact-fp"], "current_source_cache_seed_canonical_operation",
     (*CACHE_PATH_VALUES, "repo", "canonical", 9), {**CACHE_COMMON, "core_source_fingerprint": "exact-fp"},
     "format_current_source_cache_seed_canonical"),
    (["extension-fresh", "--metadata-path", "cache/metadata", "--extension-path", "cache/extension.so",
      "--source-mtime-ns", "3", "--source-fingerprint", "fp"], "current_source_cache_extension_fresh_operation",
     ("cache/metadata", "cache/extension.so", 3, "fp"), {}, "format_current_source_cache_extension_fresh"),
    (["binary-fresh", "--metadata-path", "cache/metadata", "--binary-path", "cache/binary",
      "--metadata-fingerprint-key", "fp-key", "--metadata-source-mtime-key", "source-key", "--metadata-mtime-key", "mtime-key",
      "--metadata-sha-key", "sha-key", "--source-mtime-ns", "4", "--source-fingerprint", "fp"],
     "current_source_cache_binary_fresh_operation", ("cache/metadata", "cache/binary", "fp-key", "source-key", "mtime-key", "sha-key", 4, "fp"),
     {}, "format_current_source_cache_binary_fresh"),
    (["core-fingerprint", "explicit-core"], "current_source_cache_core_fingerprint_operation", ("explicit-core",), {},
     "format_current_source_cache_core_fingerprint"),
    (["server-fingerprint", "explicit-core"], "current_source_cache_server_fingerprint_operation", ("explicit-core",), {},
     "format_current_source_cache_server_fingerprint"),
    (["core-mtime", "explicit-core"], "current_source_cache_core_mtime_operation", ("explicit-core",), {},
     "format_current_source_cache_core_mtime"),
    (["server-mtime", "explicit-core"], "current_source_cache_server_mtime_operation", ("explicit-core",), {},
     "format_current_source_cache_server_mtime"),
])
def test_current_source_cache_routes_use_explicit_paths_typed_values_and_native_results(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str],
    argv: list[str], operation: str, arguments: tuple[object, ...], options: dict[str, object],
    formatter: str, json_output: bool,
) -> None:
    payload = {"fresh": False, "native": {"cache_identity": "opaque", "lease_and_prune": "native policy"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == operation:
            assert args == arguments and kwargs == options
            return payload
        if name == formatter:
            assert args[0] is payload and not kwargs
            return "native cache title without selected fields"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["current-source-cache", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native cache title without selected fields\n"


@pytest.mark.parametrize("argv,operation", [
    (["mark-building", *CACHE_PATH_ARGV, "--source-mtime-ns", "1", "--extra-json", "not-json"],
     "current_source_cache_mark_building_operation"),
    (["seed-canonical", *CACHE_PATH_ARGV, "--repo-root", "repo", "--canonical-repo-root", "canonical", "--core-source-mtime-ns", "9"],
     "current_source_cache_seed_canonical_operation"),
])
def test_current_source_cache_json_and_required_fingerprint_validation_stay_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], argv: list[str], operation: str,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == operation:
            if operation.endswith("mark_building_operation"):
                assert kwargs["extra_json"] == "not-json"
            else:
                assert "core_source_fingerprint" not in kwargs
            raise ValueError("native cache JSON-object or canonical fingerprint validation failed")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["current-source-cache", *argv, "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native cache JSON-object or canonical fingerprint validation failed\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options", [
    (["adapter", "check", "--version", "1.2.0"], "adapter_check", ("1.2.0",), {"line_name": "main"}),
    (["adapter", "build", "--version", "1.2.0", "--line", "selected", "--target", "aarch64-apple-darwin"],
     "adapter_build", ("1.2.0",), {"line_name": "selected", "target": "aarch64-apple-darwin"}),
    (["candidate", "create", "--version", "1.2.0"], "candidate_create", ("1.2.0",), {"line_name": "main"}),
    (["candidate", "create", "--version", "1.2.0", "--line", "selected", "--profile", "family", "--channel", "rc",
      "--public-source-root", r"public\source"], "candidate_create", ("1.2.0",),
     {"line_name": "selected", "profile": "family", "channel": "rc", "public_source_root": r"public\source"}),
    (["check", "native-selected-id", "--receipts", "receipts", "--public-source-root", "public-source"],
     "check", ("native-selected-id",), {"receipts": "receipts", "public_source_root": "public-source"}),
    (["check", "legacy-id", "--tests-command", "native test command", "--skip-tests-reason", "native validation"],
     "check", ("legacy-id",), {"tests_command": "native test command", "skip_tests_reason": "native validation"}),
    (["build", "native-selected-id", "--receipts", "receipts", "--public-source-root", "public-source",
      "--native-matrix-dir", "matrix", "--native-command-dir", "commands"], "build", ("native-selected-id",),
     {"receipts": "receipts", "public_source_root": "public-source", "native_matrix_dir": "matrix", "native_command_dir": "commands"}),
    (["native-source", "native-selected-id", *RELEASE_NATIVE_ARGV], "native_source", ("native-selected-id",),
     {"target": "aarch64-apple-darwin", "source_dir": r"source\target", "runner": "runner", "runner_image": "image",
      "rust_toolchain": "stable", "rustc_path": r"C:\bin\rustc.exe"}),
    (["native-bundle", "native-selected-id", "--native-matrix-dir", "matrix"], "native_bundle", ("native-selected-id", "matrix"), {}),
    (["formula", "native-selected-id", "--name", "ait"], "formula", ("native-selected-id", "ait"), {}),
    (["formula", "native-selected-id", "--name", "ait", "--python-formula", "python@3.13"],
     "formula", ("native-selected-id", "ait"), {"python_formula": "python@3.13"}),
    (["show", "native-selected-id", "--remote", "named", "--public-source-root", "public-source"], "show", ("native-selected-id",),
     {"remote_name": "named", "public_source_root": "public-source"}),
    (["publish", "native-selected-id", "--remote", "named"], "publish", ("native-selected-id",), {"remote_name": "named"}),
    (["promote", "native-selected-id", "--channel", "stable", "--public-source-root", "public-source"],
     "promote", ("native-selected-id", "stable"), {"public_source_root": "public-source"}),
    *[(["package", "native-selected-id", "--channel", channel, "--public-source-root", "public-source"],
       "package", ("native-selected-id", channel), {"public_source_root": "public-source"})
      for channel in ("homebrew", "apt", "winget", "pypi", "npm")],
])
def test_release_cli_routes_keep_family_selection_inputs_and_output_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object], json_output: bool,
) -> None:
    payload = {"native_route": "family-or-legacy", "status": "native", "artifacts": [{"native": "immutable"}]}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "release_" + operation + "_operation":
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == "format_release":
            assert args[0] is payload and not kwargs
            return "native release family, checks, artifacts and details"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["release", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native release family, checks, artifacts and details\n"


@pytest.mark.parametrize("argv,operation", [
    (["check", "REL-FAM-selected", "--receipts", "receipts", "--tests-command", "legacy tests"], "check"),
    (["build", "legacy-id", "--receipts", "receipts"], "build"),
    (["show", "REL-FAM-selected", "--remote", "named"], "show"),
    (["publish", "REL-FAM-selected"], "publish"),
])
def test_release_family_legacy_and_publication_guards_stay_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], argv: list[str], operation: str,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "release_" + operation + "_operation":
            raise ValueError("native family/legacy selection or protected publication guard")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["release", *argv, "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native family/legacy selection or protected publication guard\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options", [
    (["status"], "status", (), {}),
    (["update"], "update", (), {"latest": False, "locked": False, "validate": False, "no_recursive": False}),
    (["update", "dependency", "--to", "SNP-111111111111", "--validate", "--no-recursive"], "update", (),
     {"name": "dependency", "snapshot": "SNP-111111111111", "latest": False, "locked": False, "validate": True, "no_recursive": True}),
    (["update", "dependency", "--latest"], "update", (),
     {"name": "dependency", "latest": True, "locked": False, "validate": False, "no_recursive": False}),
    (["update", "--locked", "--validate", "--no-recursive"], "update", (),
     {"latest": False, "locked": True, "validate": True, "no_recursive": True}),
    (["link", "dependency", r"..\other-checkout"], "link", ("dependency", r"..\other-checkout"), {}),
    (["unlink", "dependency"], "unlink", ("dependency",), {}),
])
def test_external_routes_preserve_native_selection_restore_and_text(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[str, ...], options: dict[str, object], json_output: bool,
) -> None:
    payload = {"native": {"lock": "complete graph", "linked": True, "restore_before_removal": "guarded"}}

    def invoke(_self: NativeRuntime, name: str, /, *args: object, **kwargs: object) -> object:
        if name == "external_" + operation + "_operation":
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return payload
        if name == "format_external_" + operation:
            assert args[0] is payload and not kwargs
            return "native External graph, content and link report"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["external", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native External graph, content and link report\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("fail_on_blocking", [False, True])
def test_external_doctor_emits_blocking_report_with_the_native_diagnostic_status(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], json_output: bool, fail_on_blocking: bool,
) -> None:
    payload = {"release_ready": False, "native_findings": ["blocking"]}
    native_status = 2 if fail_on_blocking else 0

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "external_doctor_operation":
            assert kwargs == {"fail_on_blocking": fail_on_blocking}
            return {"payload": payload, "exit_code": native_status}
        if name == "format_external_doctor":
            assert args[0] is payload and not kwargs
            return "native blocking External diagnostic report"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    argv = ["external", "doctor"] + (["--fail-on-blocking"] if fail_on_blocking else [])
    assert cli.main(argv + (["--json"] if json_output else [])) == native_status
    output = capsys.readouterr()
    assert not output.err
    if json_output:
        assert json.loads(output.out) == payload
    else:
        assert output.out == "native blocking External diagnostic report\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("ready", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,options,requested", [
    (["contacts"], "contacts", (), {}, None),
    (["session", "ensure", "--agent", "7"], "session_ensure", (7,), {}, 7),
    (["session", "ensure", "--agent", "4294967295", "--role", "0"], "session_ensure", (2**32 - 1,),
     {"role": 0}, 2**32 - 1),
    (["session", "acknowledge", "--activation", "raw-activation"], "session_acknowledge", ("raw-activation",), {}, None),
])
def test_team_routes_emit_native_pending_or_ready_outcomes_without_starting_frontend_work(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], operation: str, arguments: tuple[object, ...], options: dict[str, object],
    requested: int | None, json_output: bool, ready: bool,
) -> None:
    status = ("ok" if operation == "contacts" else "ready") if ready else "pending"
    payload = {"status": status, "native": "session availability and activation"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "team_" + operation + "_operation":
            assert args == (str(tmp_path), *arguments) and kwargs == options
            return {"payload": payload, "exit_code": 0 if ready else 1}
        if name == "format_team_" + operation:
            assert args == ((payload, requested) if requested is not None else (payload,)) and not kwargs
            return f'native status: "{status}"'
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["team", *argv] + (["--json"] if json_output else [])) == (0 if ready else 1)
    output = capsys.readouterr()
    assert not output.err
    if json_output:
        assert json.loads(output.out) == payload
    else:
        assert output.out == f'native status: "{status}"\n'


def test_external_restore_errors_remain_native_and_do_not_trigger_frontend_repair(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "external_unlink_operation":
            raise ValueError("native restore failed; logical link remains active")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["external", "unlink", "dependency", "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native restore failed; logical link remains active\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,operation,arguments,activate", [
    (["upgrade-u64-seconds", "--generation-root", r"new\generation", "--source-time-width", "native-selector"],
     "upgrade_u64_seconds", (".", r"new\generation", "native-selector"), False),
    (["upgrade-u64-seconds", "--repo-root", r"C:\source", "--generation-root", "generation",
      "--source-time-width", "native-selector", "--activate"],
     "upgrade_u64_seconds", (r"C:\source", "generation", "native-selector"), True),
    (["agent-pool50", "--generation-root", "generation"], "agent_pool_50", (".", "generation"), False),
    (["agent-pool50", "--repo-root", "explicit-source", "--generation-root", "generation", "--activate"],
     "agent_pool_50", ("explicit-source", "generation"), True),
])
def test_binary_db_routes_preserve_explicit_roots_native_gates_and_full_output(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], argv: list[str],
    operation: str, arguments: tuple[str, ...], activate: bool, json_output: bool,
) -> None:
    payload = {"native": "staged-retry-or-complete-generation", "value": 2**64 - 1, "optional": None}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "binary_db_" + operation + "_operation":
            assert args == arguments and kwargs == {"activate": activate}
            return payload
        if name == "format_binary_db_" + operation:
            assert args[0] is payload and not kwargs
            return "native stage, activation and full-width integer text"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.setattr(cli.Path, "cwd", lambda: pytest.fail("maintenance must use its explicit roots"))
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["binary-db", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native stage, activation and full-width integer text\n"


@pytest.mark.parametrize("argv,operation", [
    (["upgrade-u64-seconds", "--generation-root", "generation", "--source-time-width", "invalid"], "upgrade_u64_seconds"),
    (["agent-pool50", "--repo-root", "other-repository", "--generation-root", "generation"], "agent_pool_50"),
])
def test_binary_db_source_selector_and_repository_restrictions_stay_native(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], argv: list[str], operation: str,
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "binary_db_" + operation + "_operation":
            assert kwargs == {"activate": False}
            raise ValueError("native source selector or repository restriction rejected")
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main(["binary-db", *argv, "--json"]) == 1
    output = capsys.readouterr()
    assert not output.out
    assert output.err == "Error: native source selector or repository restriction rejected\n"


@pytest.mark.parametrize("json_output", [False, True])
@pytest.mark.parametrize("argv,options", [
    ([], {"jobs": 8, "apply": False}),
    (["--remote", "named", "--jobs", "1", "--apply"], {"remote_name": "named", "jobs": 1, "apply": True}),
    (["--jobs", "64"], {"jobs": 64, "apply": False}),
])
def test_remote_recovery_passes_start_path_without_normal_repository_admission(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path,
    argv: list[str], options: dict[str, object], json_output: bool,
) -> None:
    payload = {"native": {"complete_generation": "verified", "local_authority": "native recovery context"}}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "remote_recover_head_operation":
            assert args == (str(tmp_path),) and kwargs == options
            return payload
        if name == "format_remote_recover_head":
            assert args[0] is payload and not kwargs
            return "native ancestry, generation, locks and activation report"
        assert name == "encode_json" and args[0] is payload
        return native_presentation(name, args, kwargs)

    monkeypatch.chdir(tmp_path)
    use_native_calls(monkeypatch, invoke)
    assert cli.main(["remote", "recover-head", *argv] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native ancestry, generation, locks and activation report\n"


@pytest.mark.parametrize("prefix", [["snapshot", "create"], ["commit"]])
@pytest.mark.parametrize("json_output", [False, True])
def test_snapshot_capture_uses_native_scalar_reference_normalization(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], prefix: list[str], json_output: bool,
) -> None:
    payload = {"snapshot_id": "SNP-111111111111"}

    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        if name == "parse_snapshot_authoring_reference":
            assert args == (" LPT-0001/C-01 ",) and not kwargs
            return "LPT-0001/C-01"
        if name == "snapshot_capture":
            assert args[1:] == ("LPT-0001/C-01",) and kwargs == {"message": "evidence"}
            return payload
        if name == "project_snapshot_capture":
            assert args[0] is payload and kwargs == {"full": False}
            return payload
        if name == "format_snapshot_capture":
            assert args[0] is payload and not kwargs
            return "native captured Snapshot"
        return native_presentation(name, args, kwargs)

    use_native_calls(monkeypatch, invoke)
    assert cli.main([*prefix, " LPT-0001/C-01 ", "-m", "evidence"] + (["--json"] if json_output else [])) == 0
    output = capsys.readouterr().out
    if json_output:
        assert json.loads(output) == payload
    else:
        assert output == "native captured Snapshot\n"


@pytest.mark.parametrize("prefix", [["snapshot", "create"], ["commit"]])
def test_snapshot_scalar_authoring_errors_retain_parser_status_two(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], prefix: list[str],
) -> None:
    def invoke(_self: NativeRuntime, name: str, *args: object, **kwargs: object) -> object:
        assert name == "parse_snapshot_authoring_reference" and args == ("invalid",) and not kwargs
        raise ValueError("Snapshot creation requires a Task ID.")

    use_native_calls(monkeypatch, invoke)
    with pytest.raises(SystemExit) as error:
        cli.main([*prefix, "invalid", "--json"])
    assert error.value.code == 2
    output = capsys.readouterr()
    assert not output.out
    assert "Snapshot creation requires a Task ID." in output.err
