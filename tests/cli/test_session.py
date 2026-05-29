from __future__ import annotations

from ait_server.server_control import connect
from ait_server.server_paths import ServerContext
from tests.postgres_fake import fake_postgres_dsn

from ._shared import *  # noqa: F401,F403

def test_local_session_checkpoint_resume_flow(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Resume agent work", "--intent", "persist agent continuation", "--risk", "medium", "--json"],
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
            "--title",
            "Codex local session",
            "--objective",
            "Resume without replaying full history",
            "--task",
            task["task_id"],
            "--meta",
            "topic=session-checkpoint",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)
    assert session["session_id"].startswith("S-")
    assert session["session_kind"] == "agent_run"
    assert session["status"] == "active"
    assert session["metadata"]["objective"] == "Resume without replaying full history"
    assert session["metadata"]["topic"] == "session-checkpoint"

    append_one = runner.invoke(
        app,
        ["session", "append", session["session_id"], "--local", "--type", "plan.message", "--text", "Summarize the current approach", "--json"],
        catch_exceptions=False,
    )
    assert append_one.exit_code == 0, append_one.stdout
    first_event = json.loads(append_one.stdout)
    assert first_event["sequence"] == 1

    checkpoint_out = runner.invoke(
        app,
        [
            "session",
            "checkpoint",
            session["session_id"],
            "--local",
            "--summary",
            "Persist the current plan",
            "--decision",
            "Use durable checkpoints",
            "--next-action",
            "Wire CLI and API",
            "--context",
            "phase=bootstrap",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert checkpoint_out.exit_code == 0, checkpoint_out.stdout
    checkpoint = json.loads(checkpoint_out.stdout)
    assert checkpoint["checkpoint_id"].startswith("K-")
    assert checkpoint["based_on_sequence"] == 1
    assert checkpoint["resume_payload"]["decisions"] == ["Use durable checkpoints"]
    assert checkpoint["resume_payload"]["next_actions"] == ["Wire CLI and API"]
    assert checkpoint["resume_payload"]["context"]["phase"] == "bootstrap"

    append_two = runner.invoke(
        app,
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=pytest -q", "--field", "exit_code=0", "--json"],
        catch_exceptions=False,
    )
    assert append_two.exit_code == 0, append_two.stdout
    second_event = json.loads(append_two.stdout)
    assert second_event["sequence"] == 2

    pause_out = runner.invoke(app, ["session", "close", session["session_id"], "--local", "--status", "paused", "--json"], catch_exceptions=False)
    assert pause_out.exit_code == 0, pause_out.stdout
    paused = json.loads(pause_out.stdout)
    assert paused["status"] == "paused"

    resume_out = runner.invoke(app, ["session", "resume", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert resume_out.exit_code == 0, resume_out.stdout
    resumed = json.loads(resume_out.stdout)
    assert resumed["session"]["status"] == "active"
    assert resumed["latest_checkpoint"]["checkpoint_id"] == checkpoint["checkpoint_id"]
    assert [row["sequence"] for row in resumed["pending_events"]] == [2]

    events_out = runner.invoke(app, ["session", "events", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert events_out.exit_code == 0, events_out.stdout
    events = json.loads(events_out.stdout)
    assert [row["sequence"] for row in events] == [1, 2]

    checkpoints_out = runner.invoke(app, ["session", "checkpoints", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert checkpoints_out.exit_code == 0, checkpoints_out.stdout
    checkpoints = json.loads(checkpoints_out.stdout)
    assert [row["checkpoint_id"] for row in checkpoints] == [checkpoint["checkpoint_id"]]


def test_local_session_list_json_defaults_to_summary_rows_with_full_opt_in(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-list"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        [
            "session",
            "create",
            "--local",
            "--kind",
            "agent_run",
            "--title",
            "Local session list",
            "--meta",
            "topic=session-list",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    list_out = runner.invoke(app, ["session", "list", "--local", "--json"], catch_exceptions=False)
    assert list_out.exit_code == 0, list_out.stdout
    rows = json.loads(list_out.stdout)
    row = next(row for row in rows if row["session_id"] == session["session_id"])
    assert row["title"] == "Local session list"
    assert "metadata" not in row
    assert "repo_id" not in row
    assert "session_local_id" not in row

    full_out = runner.invoke(app, ["session", "list", "--local", "--json", "--full"], catch_exceptions=False)
    assert full_out.exit_code == 0, full_out.stdout
    full_rows = json.loads(full_out.stdout)
    full_row = next(row for row in full_rows if row["session_id"] == session["session_id"])
    assert full_row["metadata"]["topic"] == "session-list"


def test_local_session_analyze_reports_ait_command_counts_and_optimization_hints(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-analysis"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze command usage", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "What remains in the workflow?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait task list --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait change list --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait task show AITT-LOCAL-1 --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait task show AITT-LOCAL-1 --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Use queue summary next time.", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(
        app,
        ["session", "analyze", session["session_id"], "--local", "--json"],
        catch_exceptions=False,
    )
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    assert analysis["ait_command_count"] == 4
    assert analysis["distinct_command_paths"] == 3
    assert [row["command_path"] for row in analysis["command_paths"]] == ["task show", "change list", "task list"]
    assert analysis["command_paths"][0]["count"] == 2
    assert len(analysis["conversation_turns"]) == 1
    assert analysis["conversation_turns"][0]["ait_command_count"] == 4
    assert analysis["repeated_command_runs"][0]["signature"] == "ait task show AITT-LOCAL-1"
    hint_codes = {row["code"] for row in analysis["optimization_hints"]}
    assert {
        "queue_summary_for_inventory",
        "avoid_duplicate_commands",
        "reuse_loaded_object_context",
        "reduce_commands_per_turn",
    }.issubset(hint_codes)
    merge = next(row for row in analysis["merge_opportunities"] if row["code"] == "queue_summary_inventory_merge")
    assert merge["suggested_command"] == "ait queue summary --all-changes"
    assert merge["observed_count"] == 4
    assert merge["minimal_count"] == 1
    assert merge["avoidable_count"] == 3


def test_local_session_analyze_rolls_up_codex_turn_analysis(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-codex-analysis"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze Codex turn churn", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    turn_one_analysis = {
        "command_count": 5,
        "distinct_command_count": 5,
        "commands": ["pwd", "ls docs", "sed -n '1,20p' docs/ait_native_quickstart.md", "head -n 5 docs/ait_native_quickstart.md", "cat src/ait_chat/codex_reply.py"],
        "top_commands": [
            {"command": "pwd", "count": 1},
            {"command": "ls docs", "count": 1},
            {"command": "sed -n '1,20p' docs/ait_native_quickstart.md", "count": 1},
        ],
        "optimization_summary": "Several read-only shell probes could have been merged into one command.",
        "optimization_hints": [
            {
                "code": "merge_inspection_commands",
                "summary": "Several read-only shell probes could have been merged into one command.",
                "detail": "These adjacent read-only checks could likely be batched into one shell call.",
                "suggested_command": "pwd && ls docs && sed -n '1,20p' docs/ait_native_quickstart.md",
                "matched_commands": ["pwd", "ls docs", "sed -n '1,20p' docs/ait_native_quickstart.md"],
            },
            {
                "code": "reuse_file_read",
                "summary": "The same file was inspected multiple times.",
                "detail": "Read `docs/ait_native_quickstart.md` once with a wider range instead of reopening it across several shell commands.",
                "matched_commands": ["sed -n '1,20p' docs/ait_native_quickstart.md", "head -n 5 docs/ait_native_quickstart.md"],
                "target": "docs/ait_native_quickstart.md",
            },
        ],
    }
    turn_two_analysis = {
        "command_count": 4,
        "distinct_command_count": 4,
        "commands": ["find src -maxdepth 2 -type f", "ls src/ait_chat", "sed -n '1,40p' src/ait_chat/codex_reply.py", "sed -n '1,40p' src/ait/cli.py"],
        "top_commands": [
            {"command": "find src -maxdepth 2 -type f", "count": 1},
            {"command": "ls src/ait_chat", "count": 1},
            {"command": "sed -n '1,40p' src/ait_chat/codex_reply.py", "count": 1},
        ],
        "optimization_summary": "The turn spent several commands on file discovery or inspection.",
        "optimization_hints": [
            {
                "code": "consolidate_file_discovery",
                "summary": "The turn spent several commands on file discovery or inspection.",
                "detail": "A broader search or one combined read can often replace multiple small probes.",
            },
            {
                "code": "merge_inspection_commands",
                "summary": "Several read-only shell probes could have been merged into one command.",
                "detail": "These adjacent read-only checks could likely be batched into one shell call.",
                "suggested_command": "find src -maxdepth 2 -type f && ls src/ait_chat && sed -n '1,40p' src/ait_chat/codex_reply.py",
                "matched_commands": ["find src -maxdepth 2 -type f", "ls src/ait_chat", "sed -n '1,40p' src/ait_chat/codex_reply.py"],
            },
        ],
    }

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "Where is the command churn coming from?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--payload-json", json.dumps({"text": "First Codex reply", "turn_analysis": turn_one_analysis}), "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "Is there a repeated shell pattern?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--payload-json", json.dumps({"text": "Second Codex reply", "turn_analysis": turn_two_analysis}), "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(app, ["session", "analyze", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    assert analysis["ait_command_count"] == 0
    assert analysis["codex_turn_count"] == 2
    assert analysis["codex_command_count"] == 9
    assert len(analysis["conversation_turns"]) == 2
    assert analysis["conversation_turns"][0]["codex_command_count"] == 5
    assert analysis["conversation_turns"][0]["codex_optimization_summary"] == "Several read-only shell probes could have been merged into one command."
    assert analysis["conversation_turns"][1]["codex_command_count"] == 4
    assert analysis["codex_turns"][0]["turn_index"] == 1
    assert analysis["codex_turns"][1]["turn_index"] == 2
    merge_hint = next(row for row in analysis["codex_optimization_hints"] if row["code"] == "merge_inspection_commands")
    assert merge_hint["turn_count"] == 2
    assert merge_hint["suggested_command"] == "pwd && ls docs && sed -n '1,20p' docs/ait_native_quickstart.md"
    assert merge_hint["matched_count"] == 6
    helper_hint = next(row for row in analysis["optimization_hints"] if row["code"] == "promote_repeated_shell_workflow")
    assert [row["turn_index"] for row in helper_hint["turns"]] == [1, 2]


def test_local_session_analyze_flags_task_start_bootstrap_merge(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-task-start"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze task bootstrap", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "Open a new task and first change.", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait task start --task-only --title 'Bootstrap native workflow' --intent 'Adopt snapshot-based review' --risk medium", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait change create --task AITT-LOCAL-1 --title 'Bootstrap native workflow' --base-line feature/bootstrap --risk medium", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Use task start next time.", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(app, ["session", "analyze", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    hint = next(row for row in analysis["optimization_hints"] if row["code"] == "prefer_task_start")
    assert hint["turns"][0]["suggested_command"] == "ait task start --base-line feature/bootstrap"
    merge = next(row for row in analysis["merge_opportunities"] if row["code"] == "task_start_bootstrap_merge")
    assert merge["turn_index"] == 1
    assert merge["suggested_command"] == "ait task start --base-line feature/bootstrap"
    assert merge["observed_count"] == 2
    assert merge["avoidable_count"] == 1


def test_local_session_analyze_flags_duplicate_inventory_reads_from_wrapped_commands(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-duplicate-inventory"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze repeated wrapped inventory reads", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "Did the queue change?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=printf ready; .venv/bin/ait queue summary --all-changes --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=./.venv/bin/ait queue summary --all-changes --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Reuse the first queue summary next time.", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(app, ["session", "analyze", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    hint = next(row for row in analysis["optimization_hints"] if row["code"] == "duplicate_inventory_reads")
    run = next(row for row in hint["runs"] if row["command_path"] == "queue summary")
    assert run["count"] == 2
    assert run["signature"] == "ait queue summary"
    cluster = next(row for row in analysis["burst_clusters"] if row["code"] == "inventory_burst")
    assert cluster["summary"] == "This turn reran the same workflow inventory command."
    assert analysis["command_paths"][0]["command_path"] == "queue summary"


def test_local_session_analyze_flags_duplicate_inventory_reads_from_env_prefixed_and_shell_control_commands(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-env-prefixed-inventory"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze env-prefixed wrapped inventory reads", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "Did the queue change after the env wrapper?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=PYTHONPATH=src:. .venv/bin/ait queue summary --all-changes --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=if [ -x .venv/bin/ait ]; then timeout 15 .venv/bin/ait queue summary --all-changes --json; fi", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Reuse the first queue summary next time.", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(app, ["session", "analyze", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    hint = next(row for row in analysis["optimization_hints"] if row["code"] == "duplicate_inventory_reads")
    run = next(row for row in hint["runs"] if row["command_path"] == "queue summary")
    assert run["count"] == 2
    assert run["signature"] == "ait queue summary"
    assert run["example"] == "PYTHONPATH=src:. .venv/bin/ait queue summary --all-changes --json"
    cluster = next(row for row in analysis["burst_clusters"] if row["code"] == "inventory_burst")
    assert cluster["summary"] == "This turn reran the same workflow inventory command."


def test_local_session_analyze_flags_task_audit_merge_without_queue_summary(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-task-audit"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze task readiness", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "Did this task already land?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait task show AITT-LOCAL-1 --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait change list --task AITT-LOCAL-1 --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Use task audit next time.", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(app, ["session", "analyze", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    hint = next(row for row in analysis["optimization_hints"] if row["code"] == "prefer_task_audit")
    assert hint["turns"][0]["task_id"] == "AITT-LOCAL-1"
    assert hint["turns"][0]["suggested_command"] == "ait task audit AITT-LOCAL-1"
    assert all(row["code"] != "queue_summary_for_inventory" for row in analysis["optimization_hints"])
    merge = next(row for row in analysis["merge_opportunities"] if row["code"] == "task_audit_read_merge")
    assert merge["turn_index"] == 1
    assert merge["suggested_command"] == "ait task audit AITT-LOCAL-1"
    assert merge["observed_count"] == 2
    assert merge["avoidable_count"] == 1


def test_local_session_analyze_suggests_workflow_guide_for_help_burst(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-workflow-guide"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze land help burst", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "How do I land this change?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=./.venv/bin/ait snapshot create --help", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=printf ready; .venv/bin/ait patchset publish --help", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=.venv/bin/ait land submit --help", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Start with the workflow guide next time.", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(app, ["session", "analyze", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    hint = next(row for row in analysis["optimization_hints"] if row["code"] == "prefer_workflow_guide")
    assert hint["turns"][0]["suggested_command"] == "ait workflow guide land"
    cluster = next(row for row in analysis["burst_clusters"] if row["code"] == "help_burst")
    assert cluster["suggested_command"] == "ait workflow guide land"
    assert hint["matched_commands"] == [
        "./.venv/bin/ait snapshot create --help",
        "printf ready; .venv/bin/ait patchset publish --help",
        ".venv/bin/ait land submit --help",
    ]


def test_local_session_analyze_suggests_workflow_land_for_land_burst(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-workflow-land"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze land workflow burst", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "Can you finish landing this change?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait patchset publish --change AITC-LOCAL-1 --summary 'review summary'", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait attest put AITP-LOCAL-1 --tests pass", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait review task approve AITC-LOCAL-1 --patchset AITP-LOCAL-1", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait policy eval AITP-LOCAL-1", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait land submit AITC-LOCAL-1 --patchset AITP-LOCAL-1 --target main --mode direct", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Start with workflow land next time.", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(app, ["session", "analyze", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    hint = next(row for row in analysis["optimization_hints"] if row["code"] == "prefer_workflow_land")
    assert hint["turns"][0]["suggested_command"] == "ait workflow land AITC-LOCAL-1"
    cluster = next(row for row in analysis["burst_clusters"] if row["code"] == "land_workflow_burst")
    assert cluster["suggested_command"] == "ait workflow land AITC-LOCAL-1"
    assert cluster["turn_count"] == 1


def test_local_session_analyze_keeps_workflow_land_on_change_placeholder_when_only_patchset_ids_are_visible(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-patchset-land-burst"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze patchset-only land workflow burst", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "What is left before land?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait patchset show AITP-LOCAL-1", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait attest show AITP-LOCAL-1", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait policy show AITP-LOCAL-1", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=ait patchset rerun-ci AITP-LOCAL-1", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Start with workflow land next time.", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(app, ["session", "analyze", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    hint = next(row for row in analysis["optimization_hints"] if row["code"] == "prefer_workflow_land")
    assert hint["turns"][0]["suggested_command"].startswith("ait workflow land ")
    assert "--patchset" not in hint["turns"][0]["suggested_command"]
    cluster = next(row for row in analysis["burst_clusters"] if row["code"] == "land_workflow_burst")
    assert cluster["suggested_command"].startswith("ait workflow land ")
    assert "--patchset" not in cluster["suggested_command"]
    assert cluster["turn_count"] == 1


def test_local_session_analyze_flags_duplicate_inventory_reads_from_env_prefixed_python_module_invocations(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-python-module-inventory"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Analyze python module wrapped inventory reads", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    for argv in (
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "Did the python module queue read change?", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=PYTHONPATH=src:. .venv/bin/python -m ait.cli queue summary --all-changes --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "tool.result", "--field", "command=env PYTHONPATH=src:. timeout 15 .venv/bin/python -m ait.cli queue summary --all-changes --json", "--json"],
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Reuse the first queue summary next time.", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    analyze_out = runner.invoke(app, ["session", "analyze", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    hint = next(row for row in analysis["optimization_hints"] if row["code"] == "duplicate_inventory_reads")
    run = next(row for row in hint["runs"] if row["command_path"] == "queue summary")
    assert run["count"] == 2
    assert run["signature"] == "ait queue summary"
    assert run["example"] == "PYTHONPATH=src:. .venv/bin/python -m ait.cli queue summary --all-changes --json"


def test_local_session_autolog_tracks_commands_per_conversation_turn(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-autolog"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Draft local inventory", "--intent", "exercise autolog counting", "--risk", "low", "--json"],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    _bind_task_worktree(task["task_id"], monkeypatch, name="local-session-autolog")

    change_out = runner.invoke(
        app,
        ["change", "create", "--local", "--task", task["task_id"], "--title", "Draft local queue polish", "--base-line", "main", "--risk", "low", "--json"],
        catch_exceptions=False,
    )
    assert change_out.exit_code == 0, change_out.stdout
    json.loads(change_out.stdout)

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Autolog local command usage", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    prompt_out = runner.invoke(
        app,
        ["session", "append", session["session_id"], "--local", "--type", "session.message", "--text", "What remains in the workflow?", "--json"],
        catch_exceptions=False,
    )
    assert prompt_out.exit_code == 0, prompt_out.stdout

    monkeypatch.setenv("AIT_SESSION_ID", session["session_id"])
    monkeypatch.setenv("AIT_SESSION_LOCAL", "1")
    monkeypatch.delenv("AIT_SESSION_REMOTE", raising=False)
    monkeypatch.delenv("AIT_SESSION_AUTOLOG", raising=False)

    for argv in (
        ["task", "list", "--local", "--json"],
        ["change", "list", "--local", "--json"],
        ["task", "show", task["task_id"], "--local", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")

    reply_out = runner.invoke(
        app,
        ["session", "append", session["session_id"], "--local", "--type", "assistant.reply", "--text", "Use queue summary next time.", "--json"],
        catch_exceptions=False,
    )
    assert reply_out.exit_code == 0, reply_out.stdout

    events_out = runner.invoke(app, ["session", "events", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert events_out.exit_code == 0, events_out.stdout
    events = json.loads(events_out.stdout)
    started = [row for row in events if row["payload"].get("command_phase") == "started"]
    finished = [row for row in events if row["payload"].get("command_phase") == "finished"]
    assert len(started) == 3
    assert len(finished) == 3
    assert all(row["payload"]["capture_mode"] == "auto" for row in started)

    analyze_out = runner.invoke(
        app,
        ["session", "analyze", session["session_id"], "--local", "--json"],
        catch_exceptions=False,
    )
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)

    assert analysis["ait_command_count"] == 3
    assert any(row == {"capture_mode": "auto", "count": 3} for row in analysis["capture_modes"])
    assert len(analysis["conversation_turns"]) == 1
    assert analysis["conversation_turns"][0]["ait_command_count"] == 3
    merge = next(row for row in analysis["merge_opportunities"] if row["code"] == "queue_summary_inventory_merge")
    assert merge["turn_index"] == 1
    assert merge["suggested_command"] == "ait queue summary --all-changes"
    assert merge["observed_count"] == 3
    assert merge["minimal_count"] == 1
    assert merge["avoidable_count"] == 2


def test_local_session_checkpoint_rejects_unknown_snapshot(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-session-invalid-snapshot"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Local invalid snapshot test", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    checkpoint_out = runner.invoke(
        app,
        [
            "session",
            "checkpoint",
            session["session_id"],
            "--local",
            "--summary",
            "Should reject invalid snapshot",
            "--snapshot",
            "SNP-NOT-REAL",
        ],
        catch_exceptions=False,
    )
    assert checkpoint_out.exit_code == 2
    assert "Unknown snapshot: SNP-NOT-REAL" in checkpoint_out.output


def test_remote_session_checkpoint_resume_and_attestation_provenance(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-session"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-session") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Track session provenance", "--intent", "bind attestation to resumable sessions", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="remote-session-provenance")

        assert runner.invoke(app, ["line", "create", "feature/session-provenance"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/session-provenance"], catch_exceptions=False).exit_code == 0
        (workspace / "session_provenance.txt").write_text("session provenance\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "session feature", "--json"], catch_exceptions=False)
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
        feature_snapshot = json.loads(feature_snap_out.stdout)

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Implement session provenance", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "session provenance patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        session_out = runner.invoke(
            app,
            [
                "session",
                "create",
                "--kind",
                "agent_run",
                "--title",
                "Remote Codex session",
                "--objective",
                "Keep enough state for remote resume",
                "--change",
                change["change_id"],
                "--json",
            ],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)
        assert session["session_id"].startswith("S-")
        assert session["change_id"] == change["change_id"]
        assert session["metadata"]["objective"] == "Keep enough state for remote resume"

        append_one = runner.invoke(
            app,
            ["session", "append", session["session_id"], "--type", "plan.message", "--text", "Capture the next review step", "--json"],
            catch_exceptions=False,
        )
        assert append_one.exit_code == 0, append_one.stdout
        assert json.loads(append_one.stdout)["sequence"] == 1

        checkpoint_out = runner.invoke(
            app,
            [
                "session",
                "checkpoint",
                session["session_id"],
                "--summary",
                "Remote checkpoint",
                "--snapshot",
                feature_snapshot["snapshot_id"],
                "--decision",
                "Attach provenance evidence",
                "--next-action",
                "Publish attestation",
                "--context",
                "phase=review",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert checkpoint_out.exit_code == 0, checkpoint_out.stdout
        checkpoint = json.loads(checkpoint_out.stdout)
        assert checkpoint["snapshot_id"] == feature_snapshot["snapshot_id"]
        assert checkpoint["resume_payload"]["context"]["phase"] == "review"

        append_two = runner.invoke(
            app,
            ["session", "append", session["session_id"], "--type", "tool.result", "--field", "command=policy eval", "--field", "result=pass", "--json"],
            catch_exceptions=False,
        )
        assert append_two.exit_code == 0, append_two.stdout
        assert json.loads(append_two.stdout)["sequence"] == 2

        pause_out = runner.invoke(app, ["session", "close", session["session_id"], "--status", "paused", "--json"], catch_exceptions=False)
        assert pause_out.exit_code == 0, pause_out.stdout
        assert json.loads(pause_out.stdout)["status"] == "paused"

        resume_out = runner.invoke(app, ["session", "resume", session["session_id"], "--json"], catch_exceptions=False)
        assert resume_out.exit_code == 0, resume_out.stdout
        resumed = json.loads(resume_out.stdout)
        assert resumed["session"]["status"] == "active"
        assert resumed["latest_checkpoint"]["checkpoint_id"] == checkpoint["checkpoint_id"]
        assert [row["sequence"] for row in resumed["pending_events"]] == [2]

        attest_out = runner.invoke(
            app,
            [
                "attest",
                "put",
                patchset["patchset_id"],
                "--tests",
                "pass",
                "--model",
                "gpt-5.4-codex",
                "--session",
                session["session_id"],
                "--checkpoint",
                checkpoint["checkpoint_id"],
                "--json",
            ],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout
        attestation = json.loads(attest_out.stdout)
        assert attestation["provenance_summary"]["session_id"] == session["session_id"]
        assert attestation["provenance_summary"]["checkpoint_id"] == checkpoint["checkpoint_id"]
        assert attestation["provenance_summary"]["evidence_readiness"] == "complete"


def test_session_turn_requires_explicit_session_even_when_tracking_remote_task(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-session-turn-tracked-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-session-turn-tracked-remote") as base_url:
        monkeypatch.chdir(repo)
        monkeypatch.setenv("AIT_CHAT_APPEND_TURN_ANALYSIS", "true")
        monkeypatch.setenv("TERM_PROGRAM", "vscode")
        captured: dict[str, object] = {}

        def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
            captured["session_id"] = session["session_id"]
            captured["events"] = events
            captured["chat_id"] = chat_id
            captured["chat_title"] = chat_title
            captured["surface"] = surface
            return AiReplyResult(
                text="Tracked session reply.",
                model="gpt-5.4-codex",
                response_id="turn_cli_session_123",
                source="codex",
                turn_analysis={
                    "command_count": 2,
                    "distinct_command_count": 2,
                    "commands": ["ait queue summary --all-changes", "ait task audit AITT-0001"],
                    "top_commands": [
                        {"command": "ait queue summary --all-changes", "count": 1},
                        {"command": "ait task audit AITT-0001", "count": 1},
                    ],
                    "optimization_hints": [
                        {
                            "code": "reuse_queue_summary",
                            "summary": "Reuse the first queue summary result.",
                            "detail": "The earlier queue summary already covered this inventory question.",
                        }
                    ],
                    "optimization_summary": "Reuse the first queue summary result.",
                },
            )

        monkeypatch.setattr(server_app_module, "generate_session_reply", fake_generate)

        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-tracking", "on", "--json"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Tracked remote task", "--intent", "Exercise session turn", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        session_id = task["tracking"]["session_id"]

        missing_session_out = runner.invoke(
            app,
            ["session", "turn", "--text", "What should I do next?", "--json"],
            catch_exceptions=False,
        )
        assert missing_session_out.exit_code == 2
        missing_session_text = " ".join((missing_session_out.output or missing_session_out.stdout).split())
        assert "default live-turn session" in missing_session_text

        turn_out = runner.invoke(
            app,
            ["session", "turn", session_id, "--text", "What should I do next?", "--json"],
            catch_exceptions=False,
        )
        assert turn_out.exit_code == 0, turn_out.stdout
        payload = json.loads(turn_out.stdout)

        assert payload["ok"] is True
        assert payload["session_id"] == session_id
        assert payload["surface"] == "vscode"
        assert payload["reply_text"] == "Tracked session reply.\n\n[turn analysis] ran 2 commands · Reuse the first queue summary result."
        assert captured["session_id"] == session_id
        assert captured["chat_id"] == session_id
        assert captured["chat_title"] == "VSCode Codex"
        assert captured["surface"] == "vscode"

        monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")
        events_out = runner.invoke(app, ["session", "events", session_id, "--json"], catch_exceptions=False)
        assert events_out.exit_code == 0, events_out.stdout
        events = json.loads(events_out.stdout)
        assert [row["event_type"] for row in events] == ["session.message", "assistant.reply"]
        assert events[0]["payload"]["source"] == "vscode"
        assert events[0]["payload"]["surface_title"] == "VSCode Codex"
        assert events[1]["payload"]["delivered_via"] == "session_live"
        assert events[1]["payload"]["turn_analysis"]["command_count"] == 2


def test_session_turn_rejects_local_tracked_session(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-session-turn-local-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)
    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--task-tracking", "on", "--json"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Tracked local task", "--intent", "Exercise local session turn guard", "--risk", "low", "--json"],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)

    turn_out = runner.invoke(
        app,
        ["session", "turn", task["tracking"]["session_id"], "--text", "Can you reply through the server?"],
        catch_exceptions=False,
    )
    assert turn_out.exit_code == 2
    output = turn_out.output or turn_out.stdout
    assert "tracked session" in output
    assert "requires a remote session" in output


def test_session_turn_rejects_compact_dag_batch_session_scaffold(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-session-turn-compact-dag-batch"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-session-turn-compact-dag-batch") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        metadata = {
            "batch_id": "batch-1",
            "plan_id": "PL-TEST123",
            "task_graph_json": "docs/sprints/example.task_graph.json",
            "session_policy": "task_dag_compact_packet_worker",
            "compact_packet_surface": {
                "surface_id": "worker_only_compact_ait_dag_packet",
                "packet_generation_required": True,
            },
        }
        session_out = runner.invoke(
            app,
            [
                "session",
                "create",
                "--remote",
                "origin",
                "--kind",
                "agent_run",
                "--title",
                "Compact DAG batch scaffold",
                "--metadata-json",
                json.dumps(metadata),
                "--json",
            ],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)

        turn_out = runner.invoke(
            app,
            ["session", "turn", session["session_id"], "--remote", "origin", "--text", "Start the batch worker now."],
            catch_exceptions=False,
        )
        assert turn_out.exit_code == 2
        output = " ".join((turn_out.output or turn_out.stdout).split()).lower()
        assert "packet generation still pending" in output
        assert "ait plan execute pl-test123 --from-json" in output
        assert "docs/sprints/example.task_graph.json --auto-compact-worker --remote" in output
        assert "--remote origin" in output
        assert "--yes" in output

        events_out = runner.invoke(app, ["session", "events", session["session_id"], "--remote", "origin", "--json"], catch_exceptions=False)
        assert events_out.exit_code == 0, events_out.stdout
        assert json.loads(events_out.stdout) == []


def test_session_turn_rejects_started_compact_dag_worker_session(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-session-turn-compact-dag-worker"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-session-turn-compact-dag-worker") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        metadata = {
            "plan_id": "PL-TEST123",
            "task_graph_json": "docs/sprints/example.task_graph.json",
            "session_policy": "task_dag_compact_packet_worker",
            "packet_available": True,
            "compact_packet_surface": {
                "surface_id": "worker_only_compact_ait_dag_packet",
                "packet_generation_required": True,
            },
        }
        session_out = runner.invoke(
            app,
            [
                "session",
                "create",
                "--remote",
                "origin",
                "--kind",
                "agent_run",
                "--title",
                "Compact DAG worker",
                "--metadata-json",
                json.dumps(metadata),
                "--json",
            ],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)

        turn_out = runner.invoke(
            app,
            ["session", "turn", session["session_id"], "--remote", "origin", "--text", "Continue the worker remotely."],
            catch_exceptions=False,
        )
        assert turn_out.exit_code == 2
        output = " ".join((turn_out.output or turn_out.stdout).split()).lower()
        assert "generated locally" in output
        assert "durable lineage/events" in output
        assert "ait session turn" in output
        assert "auto-compact-worker" in output

        events_out = runner.invoke(app, ["session", "events", session["session_id"], "--remote", "origin", "--json"], catch_exceptions=False)
        assert events_out.exit_code == 0, events_out.stdout
        assert json.loads(events_out.stdout) == []


def test_remote_task_tracking_reuses_server_created_task_run_session(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-tracking-reuse"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-tracking-reuse") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-tracking", "on", "--json"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Tracked remote task", "--intent", "Reuse server-guaranteed session", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)

        config_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
        assert config_out.exit_code == 0, config_out.stdout
        config_data = json.loads(config_out.stdout)
        assert config_data["tracked_session"]["task_id"] == task["task_id"]
        assert config_data["tracked_session"]["session_id"] == task["tracking"]["session_id"]

        sessions = [
            row
            for row in remote_client_module.list_sessions(base_url, "housekeeper")
            if row.get("task_id") == task["task_id"] and row.get("session_kind") == "task_run"
        ]
        assert len(sessions) == 1
        assert sessions[0]["session_id"] == task["tracking"]["session_id"]
        assert sessions[0]["metadata"]["objective"] == "Reuse server-guaranteed session"


def test_remote_session_list_json_defaults_to_summary_rows_with_full_opt_in(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-session-list"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-session-list") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        session = remote_client_module.create_session(
            base_url,
            "housekeeper",
            "telegram_chat",
            title="Telegram chat · Wei",
            metadata={
                "source": "telegram",
                "telegram_chat_id": "123",
                "telegram_chat_title": "Wei",
                "topic": "session-list-remote",
            },
        )

        list_out = runner.invoke(app, ["session", "list", "--json"], catch_exceptions=False)
        assert list_out.exit_code == 0, list_out.stdout
        rows = json.loads(list_out.stdout)
        row = next(row for row in rows if row["session_id"] == session["session_id"])
        assert row["title"] == "Telegram chat · Wei"
        assert "metadata" not in row
        assert "telegram_context_runtime" not in row
        assert "repo_id" not in row
        assert "session_local_id" not in row

        full_out = runner.invoke(app, ["session", "list", "--json", "--full"], catch_exceptions=False)
        assert full_out.exit_code == 0, full_out.stdout
        full_rows = json.loads(full_out.stdout)
        full_row = next(row for row in full_rows if row["session_id"] == session["session_id"])
        assert full_row["metadata"]["topic"] == "session-list-remote"
        assert full_row["telegram_context_runtime"]["reply_context_mode"] == "recent_tail"

        default_remote_rows = remote_client_module.list_sessions(base_url, "housekeeper")
        default_remote_row = next(row for row in default_remote_rows if row["session_id"] == session["session_id"])
        assert default_remote_row["metadata"]["topic"] == "session-list-remote"
        assert default_remote_row["telegram_context_runtime"]["reply_context_mode"] == "recent_tail"

        summary_remote_rows = remote_client_module.list_sessions(base_url, "housekeeper", full=False)
        summary_remote_row = next(row for row in summary_remote_rows if row["session_id"] == session["session_id"])
        assert "metadata" not in summary_remote_row
        assert "telegram_context_runtime" not in summary_remote_row


def test_task_backfill_sessions_command_recovers_missing_remote_task_run_session(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-backfill-sessions"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-task-backfill-sessions") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task = remote_client_module.create_task(
            base_url,
            "housekeeper",
            "Legacy remote task",
            "Recover missing task_run session",
            "medium",
        )
        data_dir = tmp_path / "server-data-task-backfill-sessions"
        server_ctx = ServerContext.create(data_dir, backend="postgres", postgres_dsn=fake_postgres_dsn(data_dir))
        with connect(server_ctx) as conn:
            conn.execute("delete from sessions where task_id = ? and session_kind = 'task_run'", (task["task_id"],))
            conn.commit()

        backfill_out = runner.invoke(
            app,
            ["task", "backfill-sessions", "--task", task["task_id"], "--json"],
            catch_exceptions=False,
        )
        assert backfill_out.exit_code == 0, backfill_out.stdout
        payload = json.loads(backfill_out.stdout)
        assert payload["missing_task_count"] == 1
        assert payload["created_session_count"] == 1
        sessions = [
            row
            for row in remote_client_module.list_sessions(base_url, "housekeeper")
            if row.get("task_id") == task["task_id"] and row.get("session_kind") == "task_run"
        ]
        assert len(sessions) == 1


def test_server_store_create_session_uses_globally_unique_ids_across_repositories(tmp_path: Path):
    server_data = tmp_path / "server-data-direct-session-store"
    ctx = fake_postgres_context(server_data)
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main")
    server_store_module.ensure_repository(ctx, "repo-b", "main")

    session_a = server_store_module.create_session(ctx, "repo-a", "agent_run")
    session_b = server_store_module.create_session(ctx, "repo-b", "agent_run")

    assert session_a["session_id"] != session_b["session_id"]
