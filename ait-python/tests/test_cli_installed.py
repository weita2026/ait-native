"""Exercise the installed console and actual PyO3 services, without a native CLI."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import inspect
import json
import os
from pathlib import Path
import subprocess
import sys
import threading

import pytest

from ait_python import NativeRuntime, cli


def run(root: Path, *arguments: str, status: int = 0,
        block_native: bool = False) -> subprocess.CompletedProcess[str]:
    console = Path(sys.executable).parent / ("ait.exe" if os.name == "nt" else "ait")
    environment = os.environ.copy()
    environment.pop("PYTHONPATH", None)
    environment.update(PATH=str(console.parent), PYTHONNOUSERSITE="1", PYTHONDONTWRITEBYTECODE="1")
    if block_native:
        environment["PYTHONPATH"] = str(root)
    result = subprocess.run(
        [str(console), *arguments], cwd=root, env=environment,
        capture_output=True, text=True, timeout=60,
    )
    assert result.returncode == status, (arguments, result.stdout, result.stderr)
    return result


def payload(root: Path, *arguments: str) -> object:
    return json.loads(run(root, *arguments, "--json").stdout)


def initialize(root: Path) -> None:
    root.mkdir()
    payload(root, "init")
    payload(root, "config", "set", "--user-name", "Python binding fixture",
            "--user-email", "binding@example.invalid", "--post-finish-hooks", "[]")


@pytest.mark.parametrize("arguments,target", [
    (("help",), ()),
    (("help", "task", "finish"), ("task", "finish")),
    (("task", "help", "finish"), ("task", "finish")),
    (("help", "branch", "list"), ("line", "list")),
    (("branch", "help", "list"), ("line", "list")),
    (("help", "worktree", "open"), ("worktree", "path")),
    (("worktree", "help", "open"), ("worktree", "path")),
    (("review", "task", "help", "approve"), ("review", "task", "approve")),
])
def test_installed_help_commands_need_no_repository_or_domain_runtime(
    tmp_path: Path, arguments: tuple[str, ...], target: tuple[str, ...],
) -> None:
    # An unavailable native module makes accidental domain execution observable.
    (tmp_path / "ait_py.py").write_text("raise AssertionError('help loaded PyO3')\n")
    blocked = run(tmp_path, "status", status=1, block_native=True)
    assert "could not be imported" in blocked.stderr
    result = run(tmp_path, *arguments, block_native=True)
    assert result.stdout == run(tmp_path, *target, "--help", block_native=True).stdout
    assert result.stderr == ""
    assert sorted(path.name for path in tmp_path.iterdir()) == ["ait_py.py"]


@pytest.mark.parametrize("arguments", [
    ("help", "missing"), ("task", "help", "missing"),
    ("help", "task", "finish", "extra"), ("help", "--help"),
    ("help", "task", "help", "finish"), ("branch", "help", "missing"),
])
def test_installed_invalid_help_paths_keep_parser_errors(tmp_path: Path, arguments: tuple[str, ...]) -> None:
    (tmp_path / "ait_py.py").write_text("raise AssertionError('help loaded PyO3')\n")
    result = run(tmp_path, *arguments, status=2, block_native=True)
    assert "unrecognized subcommand" in result.stderr
    assert "help loaded PyO3" not in result.stderr
    assert result.stdout == ""
    assert sorted(path.name for path in tmp_path.iterdir()) == ["ait_py.py"]


def test_every_parser_route_resolves_in_the_selected_extension() -> None:
    runtime = NativeRuntime()
    count = 0

    def visit(parser: argparse.ArgumentParser) -> None:
        nonlocal count
        binding = parser.get_default("binding")
        if binding is not None:
            count += 1
            options = {native: None for _, native in binding.options}
            options.update(binding.constant_options)
            if binding.payload:
                options["payload"] = {}
            if binding.human_progress:
                options["human_progress"] = False
            if binding.inspection_result:
                options["include_delta"] = False
            inspect.signature(runtime.resolve_callable(binding.export)).bind(
                *([None] * (int(binding.repository) + len(binding.positional))), **options)
            formatters = [binding.formatter, *(name for _, name in binding.formatter_choices)]
            for formatter in formatters:
                cli._resolve_command(runtime, binding, False, formatter)
                if formatter:
                    options = {native: None for _, native in binding.formatter_options}
                    if binding.formatter_multiple:
                        options["multiple"] = False
                    inspect.signature(runtime.resolve_callable(formatter)).bind(
                        *([None] * (1 + int(binding.requested is not None) + int(binding.formatter_request))),
                        **options)
            cli._resolve_command(runtime, binding, True, binding.formatter)
            if binding.projector:
                options = {native: None for _, native in binding.projector_options}
                if binding.projector_full:
                    options["full"] = False
                inspect.signature(runtime.resolve_callable(binding.projector)).bind(
                    *([None] * (1 + int(binding.projector_repository) + int(binding.projector_requested))),
                    **options)
        for action in parser._actions:
            if isinstance(action, argparse._SubParsersAction):
                assert "help" in action.choices
                for child in action.choices.values():
                    visit(child)

    visit(cli.build_parser())
    assert count == 170


@pytest.mark.parametrize("dirty", [True, False])
def test_installed_bound_task_capture_finish_and_native_presentations(tmp_path: Path, dirty: bool) -> None:
    root = tmp_path / "repo"
    initialize(root)
    card = root / "docs/sprints/fixture.md"
    card.parent.mkdir(parents=True, exist_ok=True)
    original = ("# Fixture [plan-ref: binding/fixture]\n\n"
                "- [ ] Finish fixture. [ref: binding/finish]\n"
                "- [ ] Preserve sibling. [ref: binding/sibling]\n")
    card.write_text(original)
    edit = tmp_path / "edit"
    started = payload(root, "task", "start", "--from", "docs/sprints/fixture.md#binding/finish",
                      "--intent", "Exercise installed Python services", "--edit-root", str(edit),
                      "--local", "--full")
    task = started["task_id"]
    assert Path(started["worktree"]["path"]).resolve() == edit.resolve()
    assert (edit / ".aitignore").is_file()
    (edit / "result.txt").write_text("native service applied\n")
    captured = payload(edit, "snapshot", "create", task, "--message", "Capture Python fixture")
    snapshot = captured["snapshot_id"]
    shown = payload(edit, "snapshot", "show", snapshot)
    assert shown["snapshot_id"] == snapshot
    assert shown["files"]
    assert run(edit, "diff").stdout == ""
    if dirty:
        (edit / "result.txt").write_text("native service applied twice\n")
    runtime = NativeRuntime()
    raw = runtime.call("task_workflow_task_show", str(edit), task, local=True)
    assert run(edit, "task", "show", task, "--local").stdout == runtime.call("format_task_show", raw) + "\n"
    missing = run(edit, "snapshot", "show", "SNP-000000000000", status=1)
    assert missing.stderr == "Error: Unknown snapshot or tag: SNP-000000000000\n"
    run(edit, "task", "show", task, "--local", "--remote", "origin", status=2)
    arguments = ["task", "finish", task, "--local", "--full"]
    if dirty:
        arguments += ["--message", "Finish dirty Python fixture"]
    finished = payload(edit, *arguments)
    assert finished["closeout_status"] == "complete"
    assert finished["plan_checklist_closeout"]["status"] == "synced"
    assert card.read_text() == original.replace("- [ ] Finish", "- [x] Finish", 1)
    assert (root / "result.txt").read_text() == "native service applied" + (" twice" if dirty else "") + "\n"
    # Recovery reads the native receipt rather than reapplying the candidate.
    repeated = payload(root, "task", "finish", task, "--local", "--full")
    assert repeated["execution_status"] == "already_landed"


@contextmanager
def remote_fixture():
    observed: list[tuple[str, str, object]] = []
    state = {"fail": False, "revision": 41}
    prefix = "/v1/native/repository-authorities/7"

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def handle_request(self):
            data = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            try:
                body = json.loads(data) if data else None
            except (UnicodeError, ValueError):
                body = data
            observed.append((self.command, self.path, body))
            status = 200
            if self.command == "GET" and self.path == prefix:
                response = {"repository": {"repository_index": 7, "repository_name": "binding-fixture",
                            "namespace": "", "policy_flags": 0, "tombstoned": False}}
            elif self.command == "POST" and self.path == "/v1/native/repository-authorities":
                assert body["user_name"] == "Python binding fixture"
                response = {"repository": {"repository_index": 7}, "actor_key_index": 0}
            elif self.command == "GET" and self.path.split("?")[0] == prefix + "/sprints":
                response = []
            elif self.command == "GET" and "/sprints/" in self.path:
                status, response = 400, {"detail": "record index out of bounds for file 'plan.bin'"}
            elif "/remote-sync/zstd-bulk/" in self.path:
                if self.command == "GET":
                    status, response = 404, {"detail": "repository pack is absent"}
                elif self.command == "PUT":
                    response = {"repo_name": "binding-fixture", "pack_id": self.path.rsplit("/", 1)[1],
                                "stored": True, "pack_bytes": len(data), "raw_binary_upload": True}
                else:
                    response = {"repo_name": "binding-fixture", "remote_line": None, "line_update": None}
                    for field, key in (("snapshots", "snapshot_id"), ("object_packs", "pack_id"), ("tree_packs", "pack_id")):
                        response["committed_" + ("snapshot_ids" if field == "snapshots" else field[:-1] + "_ids")] = [row[key] for row in body[field]]
                    response["upserted_snapshots"] = len(body["snapshots"])
            elif self.command == "POST" and (self.path == prefix + "/sprints" or self.path.endswith("/revisions")):
                if state["fail"]:
                    status, response = 503, {"detail": "injected Python Plan publication failure"}
                else:
                    state["revision"] += 1
                    response = {**body, "repo_name": "binding-fixture", "plan_id": "PR-42",
                                "plan_revision_id": f"plan-revision:{state['revision']}",
                                "head_revision_id": f"plan-revision:{state['revision']}"}
            else:
                status, response = 404, {"detail": f"unexpected {self.command} {self.path}"}
            encoded = json.dumps(response).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        do_GET = do_POST = do_PUT = handle_request

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", observed, state
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def test_installed_remote_plan_publication_error_and_recovery(tmp_path: Path) -> None:
    root = tmp_path / "repo"
    initialize(root)
    card = root / "docs/sprints/remote.md"
    card.parent.mkdir(parents=True, exist_ok=True)
    card.write_text("# Remote [plan-ref: binding/remote]\n\n- [ ] Publish. [ref: binding/publish]\n")
    with remote_fixture() as (url, observed, state):
        configuration = root / ".ait/config.json"
        config = json.loads(configuration.read_text())
        config.update(repo_name="binding-fixture", repository_index=7, default_remote="origin",
                      remotes={"origin": {"remote_id": 1, "url": url, "repo_name": "binding-fixture",
                                          "created_at": "2026-10-26T00:00:00Z"}})
        configuration.write_text(json.dumps(config))
        payload(root, "plan", "sync", str(card.relative_to(root)), "--local")
        state["fail"] = True
        failed = run(root, "plan", "sync", str(card.relative_to(root)), "--remote", "origin", "--json", status=1)
        assert "injected Python Plan publication failure" in failed.stderr + failed.stdout
        assert "[ ] Publish" in card.read_text()
        state["fail"] = False
        published = payload(root, "plan", "sync", str(card.relative_to(root)), "--remote", "origin")
        assert published["status"] == "ok"
        prefix = "/v1/native/repository-authorities/7"
        assert any(method == "POST" and path == prefix + "/sprints" and body["actor_key_index"] == 0
                   for method, path, body in observed)
        assert any(method == "PUT" and "/object-packs/" in path for method, path, _ in observed)
        assert any(method == "POST" and path.endswith("/zstd-bulk/commit") for method, path, _ in observed)
