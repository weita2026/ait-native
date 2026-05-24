from __future__ import annotations

from ._shared import *  # noqa: F401,F403


def test_command_profiling_writes_status_summary_with_phase_timings(tmp_path: Path, monkeypatch) -> None:
    repo = tmp_path / "housekeeper-command-profiling"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--command-profiling", "on"], catch_exceptions=False).exit_code == 0

    status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout

    log_path = repo / ".ait" / "generated" / "profiling" / "commands.jsonl"
    rows = [json.loads(line) for line in log_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    assert len(rows) == 1
    row = rows[0]
    assert row["argv"] == ["status", "--json"]
    assert row["command"] == "ait status --json"
    assert row["returncode"] == 0
    assert row["success"] is True
    assert row["duration_ms"] >= 0
    assert row["phase_timings_ms"]["local_content.repo_status"]["total"] >= 0
    assert row["phase_timings_ms"]["local_content.repo_status"]["workspace_delta"]["total"] >= 0
    assert row["phase_timings_ms"]["worktree_doctor"] >= 0
    assert row["phase_timings_ms"]["list_line_cleanup_candidates"] >= 0


def test_command_profiling_is_inert_when_disabled_and_coexists_with_session_autolog(
    tmp_path: Path,
    monkeypatch,
) -> None:
    repo = tmp_path / "housekeeper-command-profiling-session"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    disabled_status = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert disabled_status.exit_code == 0, disabled_status.stdout
    assert not (repo / ".ait" / "generated" / "profiling" / "commands.jsonl").exists()

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Profiling session", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    assert runner.invoke(app, ["config", "set", "--command-profiling", "on"], catch_exceptions=False).exit_code == 0
    monkeypatch.setenv("AIT_SESSION_ID", session["session_id"])
    monkeypatch.setenv("AIT_SESSION_LOCAL", "1")
    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "1")

    profiled_status = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert profiled_status.exit_code == 0, profiled_status.stdout

    log_path = repo / ".ait" / "generated" / "profiling" / "commands.jsonl"
    rows = [json.loads(line) for line in log_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    assert [row["argv"] for row in rows] == [["status", "--json"]]

    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")

    events_out = runner.invoke(app, ["session", "events", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert events_out.exit_code == 0, events_out.stdout
    events = json.loads(events_out.stdout)
    assert any(row["event_type"] == "tool.command" for row in events)
    assert any(row["event_type"] == "tool.result" for row in events)
