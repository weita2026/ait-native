from __future__ import annotations

from ._shared import *  # noqa: F401,F403


def test_task_tokens_local_rolls_up_usage_last_and_direct_usage(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-tokens-local"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--task-only",
            "--title",
            "Measure local task usage",
            "--intent",
            "exercise local task token reporting",
            "--risk",
            "medium",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)

    session_out = runner.invoke(
        app,
        [
            "session",
            "create",
            "--local",
            "--kind",
            "agent_run",
            "--task",
            task["task_id"],
            "--title",
            "Local token session",
            "--model",
            "gpt-test",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    usage_last_payload = {
        "text": "Measured reply one",
        "model": "gpt-test",
        "usage": {
            "last": {
                "input_tokens": 10,
                "cached_input_tokens": 2,
                "output_tokens": 4,
                "reasoning_output_tokens": 1,
                "total_tokens": 14,
            },
            "total": {
                "input_tokens": 100,
                "output_tokens": 40,
                "total_tokens": 140,
            },
        },
    }
    direct_usage_payload = {
        "text": "Measured reply two",
        "model": "gpt-alt",
        "usage": {
            "input_tokens": 3,
            "output_tokens": 2,
            "total_tokens": 5,
        },
    }
    missing_usage_payload = {"text": "Reply without usage", "model": "gpt-test"}

    for payload in (usage_last_payload, direct_usage_payload, missing_usage_payload):
        append_out = runner.invoke(
            app,
            [
                "session",
                "append",
                session["session_id"],
                "--local",
                "--type",
                "assistant.reply",
                "--payload-json",
                json.dumps(payload),
                "--json",
            ],
            catch_exceptions=False,
        )
        assert append_out.exit_code == 0, append_out.stdout

    report_out = runner.invoke(
        app,
        ["task", "tokens", task["task_id"], "--local", "--json"],
        catch_exceptions=False,
    )
    assert report_out.exit_code == 0, report_out.stdout
    payload = json.loads(report_out.stdout)

    assert payload["scope"]["mode"] == "local"
    assert payload["summary"]["session_count"] == 1
    assert payload["summary"]["sessions_with_usage_count"] == 1
    assert payload["summary"]["assistant_reply_count"] == 3
    assert payload["summary"]["metered_reply_count"] == 2
    assert payload["summary"]["usage_last_reply_count"] == 1
    assert payload["summary"]["direct_usage_reply_count"] == 1
    assert payload["summary"]["missing_usage_reply_count"] == 1
    assert payload["summary"]["prompt_tokens"] == 13
    assert payload["summary"]["completion_tokens"] == 6
    assert payload["summary"]["total_tokens"] == 19
    assert payload["summary"]["cached_input_tokens"] == 2
    assert payload["summary"]["reasoning_output_tokens"] == 1
    assert payload["summary"]["models"] == ["gpt-alt", "gpt-test"]
    assert payload["changes"][0]["change_id"] == "(task-only)"
    assert payload["worktrees"][0]["worktree_name"] == session["worktree_name"]
    assert payload["models"][0]["model_name"] == "gpt-alt"
    assert payload["models"][0]["total_tokens"] == 5
    assert payload["models"][1]["model_name"] == "gpt-test"
    assert payload["models"][1]["total_tokens"] == 14

    text_out = runner.invoke(
        app,
        ["task", "tokens", task["task_id"], "--local", "--by", "session"],
        catch_exceptions=False,
    )
    assert text_out.exit_code == 0, text_out.stdout
    output = text_out.output or text_out.stdout
    assert f"task tokens {task['task_id']}" in output
    assert "session breakdown" in output


def test_task_tokens_remote_rolls_up_remote_session_usage(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-tokens-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-task-tokens-remote") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        _set_solo_remote_advisory()

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--title",
                "Measure remote task usage",
                "--intent",
                "exercise remote task token reporting",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)

        session_out = runner.invoke(
            app,
            [
                "session",
                "create",
                "--task",
                task["task_id"],
                "--kind",
                "agent_run",
                "--title",
                "Remote token session",
                "--model",
                "gpt-remote",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)

        append_out = runner.invoke(
            app,
            [
                "session",
                "append",
                session["session_id"],
                "--type",
                "assistant.reply",
                "--payload-json",
                json.dumps(
                    {
                        "text": "Remote measured reply",
                        "model": "gpt-remote",
                        "usage": {
                            "last": {
                                "prompt_tokens": 9,
                                "completion_tokens": 3,
                                "total_tokens": 12,
                            },
                            "total": {
                                "prompt_tokens": 90,
                                "completion_tokens": 30,
                                "total_tokens": 120,
                            },
                        },
                    }
                ),
                "--json",
            ],
            catch_exceptions=False,
        )
        assert append_out.exit_code == 0, append_out.stdout

        report_out = runner.invoke(
            app,
            ["task", "tokens", task["task_id"], "--json"],
            catch_exceptions=False,
        )
        assert report_out.exit_code == 0, report_out.stdout
        payload = json.loads(report_out.stdout)

        assert payload["scope"]["mode"] == "remote"
        assert payload["scope"]["repo_name"] == "housekeeper"
        assert payload["summary"]["session_count"] >= 1
        assert payload["summary"]["metered_reply_count"] == 1
        assert payload["summary"]["usage_last_reply_count"] == 1
        assert payload["summary"]["prompt_tokens"] == 9
        assert payload["summary"]["completion_tokens"] == 3
        assert payload["summary"]["total_tokens"] == 12
        assert payload["summary"]["models"] == ["gpt-remote"]
