from __future__ import annotations

import json
import os
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Optional

from .repo_paths import RepoContext


class RemoteError(RuntimeError):
    pass


_DEFAULT_REQUEST_TIMEOUT_SECONDS = 30.0
_LONG_RUNNING_CI_REQUEST_TIMEOUT_SECONDS = 1800.0



def _env_first(*names: str) -> str | None:
    for name in names:
        value = os.environ.get(name)
        if value:
            return value
    return None


def _normalize_text(value: str | None) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _config_actor_identity() -> str | None:
    try:
        ctx = RepoContext.discover()
    except FileNotFoundError:
        return None
    if not ctx.config_path.exists():
        return None
    try:
        payload = json.loads(ctx.config_path.read_text(encoding="utf-8"))
    except Exception:
        return None
    if not isinstance(payload, dict):
        return None
    return _normalize_text(payload.get("user_email")) or _normalize_text(payload.get("user_name"))



def _auth_headers() -> dict[str, str]:
    headers: dict[str, str] = {}
    actor = _env_first("AIT_NATIVE_ACTOR", "AIT_ACTOR") or _config_actor_identity()
    actor_type = _env_first("AIT_NATIVE_ACTOR_TYPE", "AIT_ACTOR_TYPE")
    roles = _env_first("AIT_NATIVE_ROLES", "AIT_ROLES")
    repos = _env_first("AIT_NATIVE_REPOS", "AIT_REPOS")
    if actor:
        headers["X-AIT-Actor"] = actor
    if actor_type:
        headers["X-AIT-Actor-Type"] = actor_type
    if roles:
        headers["X-AIT-Roles"] = roles
    if repos:
        headers["X-AIT-Repos"] = repos
    return headers



def _timeout_like(exc: BaseException | None) -> bool:
    pending: list[BaseException] = [exc] if exc is not None else []
    seen: set[int] = set()
    while pending:
        current = pending.pop()
        marker = id(current)
        if marker in seen:
            continue
        seen.add(marker)
        if isinstance(current, TimeoutError):
            return True
        text = str(current or "").lower()
        if "timed out" in text or "timeout" in text:
            return True
        reason = getattr(current, "reason", None)
        if isinstance(reason, BaseException):
            pending.append(reason)
        cause = getattr(current, "__cause__", None)
        if isinstance(cause, BaseException):
            pending.append(cause)
        context = getattr(current, "__context__", None)
        if isinstance(context, BaseException):
            pending.append(context)
    return False


def _request(
    method: str,
    url: str,
    body: Optional[dict] = None,
    *,
    timeout_seconds: float = _DEFAULT_REQUEST_TIMEOUT_SECONDS,
) -> Any:
    data = None
    headers = {"Accept": "application/json"}
    headers.update(_auth_headers())
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout_seconds) as resp:
            payload = resp.read().decode("utf-8")
            return json.loads(payload) if payload else None
    except urllib.error.HTTPError as exc:
        body_text = exc.read().decode("utf-8", errors="replace")
        try:
            parsed = json.loads(body_text)
            message = parsed.get("detail") or parsed.get("error") or parsed
        except Exception:
            message = body_text or exc.reason
        raise RemoteError(f"{method} {url} failed: {exc.code} {message}") from exc
    except urllib.error.URLError as exc:
        raise RemoteError(f"{method} {url} failed: {exc.reason}") from exc
    except OSError as exc:
        if _timeout_like(exc):
            raise RemoteError(f"{method} {url} failed: timed out") from exc
        raise


def get_server_health(base_url: str) -> dict:
    return _request("GET", _join(base_url, "/healthz"))


def _join(base_url: str, path: str) -> str:
    return urllib.parse.urljoin(base_url.rstrip("/") + "/", path.lstrip("/"))



def ensure_repository(
    base_url: str,
    repo_name: str,
    default_line: str,
    policy: dict[str, Any] | None = None,
    *,
    id_namespace_prefix: str | None = None,
) -> dict:
    body: dict[str, Any] = {"repo_name": repo_name, "default_line": default_line}
    if policy is not None:
        body["policy"] = policy
    if id_namespace_prefix is not None:
        body["id_namespace_prefix"] = id_namespace_prefix
    return _request("POST", _join(base_url, "/v1/native/repositories"), body)



def get_repository(base_url: str, repo_name: str) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}"))



def auth_whoami(base_url: str, repo_name: str | None = None) -> dict:
    path = "/v1/native/auth/whoami"
    if repo_name:
        path += "?repo_name=" + urllib.parse.quote(repo_name, safe="")
    return _request("GET", _join(base_url, path))



def read_reviewer_inbox(
    base_url: str,
    repo_name: str | None = None,
    *,
    author_class: str | None = None,
    author_mode: str | None = None,
    tests: str | None = None,
    policy: str | None = None,
    freshness: str | None = None,
    review: str | None = None,
) -> dict:
    params: dict[str, str] = {}
    if repo_name is not None:
        params["repo_name"] = repo_name
    if author_class is not None:
        params["author_class"] = author_class
    if author_mode is not None:
        params["author_mode"] = author_mode
    if tests is not None:
        params["tests"] = tests
    if policy is not None:
        params["policy"] = policy
    if freshness is not None:
        params["freshness"] = freshness
    if review is not None:
        params["review"] = review
    path = "/v1/native/read/reviewer-inbox"
    if params:
        path += "?" + urllib.parse.urlencode(params)
    return _request("GET", _join(base_url, path))


def read_task_queue(base_url: str, repo_name: str | None = None, *, status: str | None = "active") -> dict:
    params: dict[str, str] = {}
    if repo_name is not None:
        params["repo_name"] = repo_name
    if status is not None:
        params["status"] = status
    path = "/v1/native/read/task-queue"
    if params:
        path += "?" + urllib.parse.urlencode(params)
    return _request("GET", _join(base_url, path))


def read_queue_summary_bundle(base_url: str, repo_name: str, *, status: str | None = "active") -> dict:
    params: dict[str, str] = {"repo_name": repo_name}
    if status is not None:
        params["status"] = status
    path = "/v1/native/read/queue-summary"
    if params:
        path += "?" + urllib.parse.urlencode(params)
    return _request("GET", _join(base_url, path))


def read_repository_ci_runs(
    base_url: str,
    repo_name: str,
    *,
    limit: int = 20,
    plane: str | None = None,
    suite_id: str | None = None,
) -> dict:
    params: dict[str, str] = {"limit": str(limit)}
    if plane is not None:
        params["plane"] = plane
    if suite_id is not None:
        params["suite_id"] = suite_id
    path = f"/v1/native/read/repositories/{urllib.parse.quote(repo_name, safe='')}/ci-runs"
    if params:
        path += "?" + urllib.parse.urlencode(params)
    return _request("GET", _join(base_url, path))


def read_patchset_ci_status(base_url: str, patchset_id: str, *, recent_limit: int = 10) -> dict:
    path = f"/v1/native/read/patchsets/{urllib.parse.quote(patchset_id, safe='')}/ci-status?{urllib.parse.urlencode({'recent_limit': recent_limit})}"
    return _request("GET", _join(base_url, path))


def read_task_dag_readiness(
    base_url: str,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {"graph": graph}
    if current_plan_revision_id is not None:
        body["current_plan_revision_id"] = current_plan_revision_id
    return _request("POST", _join(base_url, "/v1/native/read/task-dag-readiness"), body)


def read_task_dag_graph(
    base_url: str,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {"graph": graph}
    if current_plan_revision_id is not None:
        body["current_plan_revision_id"] = current_plan_revision_id
    return _request("POST", _join(base_url, "/v1/native/read/task-dag-graph"), body)


def read_task_dag_schedule(
    base_url: str,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {"graph": graph}
    if current_plan_revision_id is not None:
        body["current_plan_revision_id"] = current_plan_revision_id
    return _request("POST", _join(base_url, "/v1/native/read/task-dag-schedule"), body)


def read_task_dag_progress(
    base_url: str,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {"graph": graph}
    if current_plan_revision_id is not None:
        body["current_plan_revision_id"] = current_plan_revision_id
    return _request("POST", _join(base_url, "/v1/native/read/task-dag-progress"), body)


def advance_task_dag_run(
    base_url: str,
    session_id: str,
    graph: dict[str, Any],
    *,
    current_plan_revision_id: str | None = None,
    repo_name: str | None = None,
) -> dict:
    body: dict[str, Any] = {"graph": graph}
    if current_plan_revision_id is not None:
        body["current_plan_revision_id"] = current_plan_revision_id
    path = f"/v1/native/task-dag-runs/{session_id}:advance"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/task-dag-runs/{session_id}:advance"
    return _request("POST", _join(base_url, path), body)


def grant_roles(base_url: str, repo_name: str, actor_identity: str, roles: list[str]) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/admin/repositories/{repo_name}/bindings"),
        {"actor_identity": actor_identity, "roles": roles},
    )



def list_role_bindings(base_url: str, repo_name: str) -> list[dict]:
    return _request("GET", _join(base_url, f"/v1/native/admin/repositories/{repo_name}/bindings"))



def list_remote_lines(base_url: str, repo_name: str) -> list[dict]:
    return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/lines"))



def get_remote_line(base_url: str, repo_name: str, line_name: str) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/lines/{urllib.parse.quote(line_name, safe='')}"))



def update_remote_line(
    base_url: str,
    repo_name: str,
    line_name: str,
    head_snapshot_id: Optional[str],
    *,
    expected_head_snapshot_id: Optional[str] = None,
) -> dict:
    body: dict[str, Any] = {"head_snapshot_id": head_snapshot_id}
    if expected_head_snapshot_id is not None:
        body["expected_head_snapshot_id"] = expected_head_snapshot_id
    return _request(
        "PUT",
        _join(base_url, f"/v1/native/repositories/{repo_name}/lines/{urllib.parse.quote(line_name, safe='')}"),
        body,
    )



def close_remote_line(base_url: str, repo_name: str, line_name: str, status: str = "archived") -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/repositories/{repo_name}/lines/{urllib.parse.quote(line_name, safe='')}:close"),
        {"status": status},
    )


def put_remote_snapshot(
    base_url: str,
    repo_name: str,
    snapshot_id: str,
    bundle: dict,
    *,
    storage_ingest_mode: str | None = None,
) -> dict:
    body = dict(bundle)
    if storage_ingest_mode:
        body["storage_ingest_mode"] = storage_ingest_mode
    return _request("PUT", _join(base_url, f"/v1/native/repositories/{repo_name}/snapshots/{snapshot_id}"), body)



def get_remote_snapshot(
    base_url: str,
    repo_name: str,
    snapshot_id: str,
    *,
    include_content: bool = True,
    path: str | None = None,
) -> dict:
    snapshot_path = f"/v1/native/repositories/{repo_name}/snapshots/{snapshot_id}"
    query: dict[str, str] = {}
    if not include_content:
        query["include_content"] = "false"
    if path is not None:
        query["path"] = path
    if query:
        snapshot_path += "?" + urllib.parse.urlencode(query)
    return _request("GET", _join(base_url, snapshot_path))


def get_remote_snapshots_existence(base_url: str, repo_name: str, snapshot_ids: list[str]) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/repositories/{repo_name}/snapshots:exists"),
        {"snapshot_ids": snapshot_ids},
    )



def create_plan(
    base_url: str,
    repo_name: str,
    title: str,
    artifact_path: str,
    artifact_selector: str | None,
    artifact_heading: str,
    items: list[dict[str, Any]],
    *,
    summary: str | None = None,
    status: str = "draft",
    plan_id: str | None = None,
    source_kind: str = "manual_edit",
    source_session_id: str | None = None,
    artifact_body: str | None = None,
) -> dict:
    body: dict[str, Any] = {
        "title": title,
        "artifact_path": artifact_path,
        "artifact_selector": artifact_selector,
        "artifact_heading": artifact_heading,
        "items": items,
        "status": status,
        "source_kind": source_kind,
    }
    if summary is not None:
        body["summary"] = summary
    if plan_id is not None:
        body["plan_id"] = plan_id
    if source_session_id is not None:
        body["source_session_id"] = source_session_id
    if artifact_body is not None:
        body["artifact_body"] = artifact_body
    return _request("POST", _join(base_url, f"/v1/native/repositories/{repo_name}/sprints"), body)



def list_plans(
    base_url: str,
    repo_name: str,
    *,
    artifact_path: str | None = None,
) -> list[dict]:
    path = f"/v1/native/repositories/{repo_name}/sprints"
    query: dict[str, str] = {}
    normalized_artifact_path = _normalize_text(artifact_path)
    if normalized_artifact_path is not None:
        query["artifact_path"] = normalized_artifact_path
    if query:
        path += "?" + urllib.parse.urlencode(query)
    return _request("GET", _join(base_url, path))



def get_plan(base_url: str, plan_id: str) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/sprints/{plan_id}"))



def list_plan_revisions(base_url: str, plan_id: str) -> list[dict]:
    return _request("GET", _join(base_url, f"/v1/native/sprints/{plan_id}/revisions"))



def get_plan_revision(base_url: str, plan_id: str, plan_revision_id: str) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/sprints/{plan_id}/revisions/{plan_revision_id}"))


def put_plan_revision_artifacts(
    base_url: str,
    plan_id: str,
    plan_revision_id: str,
    artifacts: list[dict[str, Any]],
) -> dict:
    return _request(
        "PUT",
        _join(base_url, f"/v1/native/sprints/{plan_id}/revisions/{plan_revision_id}/artifacts"),
        {"artifacts": artifacts},
    )



def revise_plan(
    base_url: str,
    plan_id: str,
    artifact_path: str,
    artifact_selector: str | None,
    artifact_heading: str,
    items: list[dict[str, Any]],
    *,
    title: str | None = None,
    summary: str | None = None,
    source_kind: str = "manual_edit",
    source_session_id: str | None = None,
    artifact_body: str | None = None,
    expected_head_revision_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {
        "artifact_path": artifact_path,
        "artifact_selector": artifact_selector,
        "artifact_heading": artifact_heading,
        "items": items,
        "source_kind": source_kind,
    }
    if title is not None:
        body["title"] = title
    if summary is not None:
        body["summary"] = summary
    if source_session_id is not None:
        body["source_session_id"] = source_session_id
    if artifact_body is not None:
        body["artifact_body"] = artifact_body
    if expected_head_revision_id is not None:
        body["expected_head_revision_id"] = expected_head_revision_id
    return _request("POST", _join(base_url, f"/v1/native/sprints/{plan_id}/revisions"), body)


def update_plan_status(base_url: str, plan_id: str, status: str) -> dict:
    return _request("PATCH", _join(base_url, f"/v1/native/sprints/{plan_id}"), {"status": status})


def create_planning_session(
    base_url: str,
    plan_id: str,
    *,
    title: str | None = None,
    mode: str = "connected_local",
    preferred_agent: str | None = None,
    resume_if_active: bool = True,
    planning_session_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {
        "mode": mode,
        "resume_if_active": bool(resume_if_active),
    }
    if title is not None:
        body["title"] = title
    if preferred_agent is not None:
        body["preferred_agent"] = preferred_agent
    if planning_session_id is not None:
        body["planning_session_id"] = planning_session_id
    return _request("POST", _join(base_url, f"/v1/native/sprints/{plan_id}/planning-sessions"), body)


def list_planning_sessions(base_url: str, plan_id: str, *, status: str | None = None) -> list[dict]:
    path = f"/v1/native/sprints/{plan_id}/planning-sessions"
    if status is not None:
        path += "?" + urllib.parse.urlencode({"status": status})
    return _request("GET", _join(base_url, path))


def get_planning_session(base_url: str, planning_session_id: str) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/planning-sessions/{planning_session_id}"))


def append_planning_session_event(
    base_url: str,
    planning_session_id: str,
    event_type: str,
    payload: dict[str, Any] | None = None,
) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/planning-sessions/{planning_session_id}/events"),
        {"event_type": event_type, "payload": payload or {}},
    )


def list_planning_session_events(
    base_url: str,
    planning_session_id: str,
    *,
    after_sequence: int = 0,
    limit: int = 200,
) -> list[dict]:
    path = f"/v1/native/planning-sessions/{planning_session_id}/events?" + urllib.parse.urlencode(
        {"after_sequence": after_sequence, "limit": limit}
    )
    return _request("GET", _join(base_url, path))


def join_planning_session(
    base_url: str,
    planning_session_id: str,
    *,
    surface: str = "cli",
    title: str | None = None,
    model_name: str | None = None,
    resume_if_active: bool = True,
    session_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {
        "surface": surface,
        "resume_if_active": bool(resume_if_active),
    }
    if title is not None:
        body["title"] = title
    if model_name is not None:
        body["model_name"] = model_name
    if session_id is not None:
        body["session_id"] = session_id
    return _request("POST", _join(base_url, f"/v1/native/planning-sessions/{planning_session_id}:join"), body)


def promote_planning_session(
    base_url: str,
    planning_session_id: str,
    artifact_path: str,
    artifact_selector: str,
    artifact_heading: str,
    items: list[dict[str, Any]],
    *,
    title: str | None = None,
    summary: str | None = None,
    artifact_body: str | None = None,
) -> dict:
    body: dict[str, Any] = {
        "artifact_path": artifact_path,
        "artifact_selector": artifact_selector,
        "artifact_heading": artifact_heading,
        "items": items,
    }
    if title is not None:
        body["title"] = title
    if summary is not None:
        body["summary"] = summary
    if artifact_body is not None:
        body["artifact_body"] = artifact_body
    return _request("POST", _join(base_url, f"/v1/native/planning-sessions/{planning_session_id}:promote"), body)


def close_planning_session(base_url: str, planning_session_id: str, *, status: str = "closed") -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/planning-sessions/{planning_session_id}:close"),
        {"status": status},
    )


def create_task(
    base_url: str,
    repo_name: str,
    title: str,
    intent: str,
    risk_tier: str,
    task_id: str | None = None,
    *,
    plan_id: str | None = None,
    origin_plan_revision_id: str | None = None,
    plan_item_ref: str | None = None,
    tracking_session: dict[str, Any] | None = None,
) -> dict:
    body = {"title": title, "intent": intent, "risk_tier": risk_tier}
    if task_id is not None:
        body["task_id"] = task_id
    if plan_id is not None:
        body["plan_id"] = plan_id
    if origin_plan_revision_id is not None:
        body["origin_plan_revision_id"] = origin_plan_revision_id
    if plan_item_ref is not None:
        body["plan_item_ref"] = plan_item_ref
    if tracking_session is not None:
        body["tracking_session"] = tracking_session
    return _request(
        "POST",
        _join(base_url, f"/v1/native/repositories/{repo_name}/tasks"),
        body,
    )



def list_tasks(base_url: str, repo_name: str) -> list[dict]:
    return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/tasks"))



def get_task(base_url: str, task_id: str, *, repo_name: str | None = None) -> dict:
    if repo_name:
        return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/tasks/{task_id}"))
    return _request("GET", _join(base_url, f"/v1/native/tasks/{task_id}"))


def read_task_audit(
    base_url: str,
    task_id: str,
    *,
    repo_name: str | None = None,
    target_line: str = "main",
) -> dict:
    if repo_name is not None:
        path = f"/v1/native/repositories/{repo_name}/read/tasks/{task_id}/audit?"
    else:
        path = f"/v1/native/read/tasks/{task_id}/audit?"
    path += urllib.parse.urlencode({"target_line": target_line})
    return _request("GET", _join(base_url, path))


def close_task(base_url: str, task_id: str, status: str = "completed", *, repo_name: str | None = None) -> dict:
    if repo_name is not None:
        task_id = str(get_task(base_url, task_id, repo_name=repo_name).get("task_id") or task_id)
    return _request("POST", _join(base_url, f"/v1/native/tasks/{task_id}:close"), {"status": status})


def restart_task(base_url: str, task_id: str, *, repo_name: str | None = None) -> dict:
    if repo_name is not None:
        task_id = str(get_task(base_url, task_id, repo_name=repo_name).get("task_id") or task_id)
    return _request("POST", _join(base_url, f"/v1/native/tasks/{task_id}:restart"))


def backfill_task_tracking_sessions(base_url: str, repo_name: str, *, task_id: str | None = None) -> dict:
    body: dict[str, Any] = {}
    if task_id is not None:
        body["task_id"] = task_id
    return _request("POST", _join(base_url, f"/v1/native/repositories/{repo_name}/tasks:backfill-sessions"), body)


def ensure_task_tracking_session(
    base_url: str,
    repo_name: str,
    task_id: str,
    *,
    tracking_session: dict[str, Any] | None = None,
) -> dict:
    body: dict[str, Any] = {}
    if tracking_session is not None:
        body["tracking_session"] = tracking_session
    return _request("POST", _join(base_url, f"/v1/native/repositories/{repo_name}/tasks/{task_id}:ensure-session"), body)


def create_session(
    base_url: str,
    repo_name: str,
    session_kind: str,
    *,
    task_id: str | None = None,
    change_id: str | None = None,
    title: str | None = None,
    line_name: str | None = None,
    worktree_name: str | None = None,
    model_name: str | None = None,
    metadata: dict[str, Any] | None = None,
    session_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {
        "session_kind": session_kind,
        "task_id": task_id,
        "change_id": change_id,
        "title": title,
        "line_name": line_name,
        "worktree_name": worktree_name,
        "model_name": model_name,
        "metadata": metadata or {},
    }
    if session_id is not None:
        body["session_id"] = session_id
    return _request("POST", _join(base_url, f"/v1/native/repositories/{repo_name}/sessions"), body)


def list_sessions(
    base_url: str,
    repo_name: str,
    *,
    status: str | None = None,
    full: bool = True,
) -> list[dict]:
    path = f"/v1/native/repositories/{repo_name}/sessions"
    query: dict[str, str] = {}
    if status:
        query["status"] = status
    if full:
        query["full"] = "1"
    if query:
        path += "?" + urllib.parse.urlencode(query)
    return _request("GET", _join(base_url, path))


def get_session(base_url: str, session_id: str, *, repo_name: str | None = None) -> dict:
    path = f"/v1/native/sessions/{session_id}"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/sessions/{session_id}"
    return _request("GET", _join(base_url, path))


def append_session_event(
    base_url: str,
    session_id: str,
    event_type: str,
    payload: dict[str, Any] | None = None,
    *,
    repo_name: str | None = None,
) -> dict:
    path = f"/v1/native/sessions/{session_id}/events"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/sessions/{session_id}/events"
    return _request(
        "POST",
        _join(base_url, path),
        {"event_type": event_type, "payload": payload or {}},
    )


def create_session_turn(
    base_url: str,
    session_id: str,
    *,
    text: str,
    surface: str | None = None,
    title: str | None = None,
    repo_name: str | None = None,
) -> dict:
    body: dict[str, Any] = {"text": text}
    if surface is not None:
        body["surface"] = surface
    if title is not None:
        body["title"] = title
    path = f"/v1/native/sessions/{session_id}:turn"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/sessions/{session_id}:turn"
    return _request("POST", _join(base_url, path), body)


def list_session_events(
    base_url: str,
    session_id: str,
    *,
    after_sequence: int = 0,
    limit: int = 200,
    repo_name: str | None = None,
) -> list[dict]:
    path = f"/v1/native/sessions/{session_id}/events?" + urllib.parse.urlencode({"after_sequence": after_sequence, "limit": limit})
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/sessions/{session_id}/events?" + urllib.parse.urlencode(
            {"after_sequence": after_sequence, "limit": limit}
        )
    return _request("GET", _join(base_url, path))


def create_session_checkpoint(
    base_url: str,
    session_id: str,
    summary: str,
    *,
    snapshot_id: str | None = None,
    resume_payload: dict[str, Any] | None = None,
    based_on_sequence: int | None = None,
    checkpoint_id: str | None = None,
    repo_name: str | None = None,
) -> dict:
    body: dict[str, Any] = {
        "summary": summary,
        "snapshot_id": snapshot_id,
        "resume_payload": resume_payload or {},
        "based_on_sequence": based_on_sequence,
    }
    if checkpoint_id is not None:
        body["checkpoint_id"] = checkpoint_id
    path = f"/v1/native/sessions/{session_id}/checkpoints"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/sessions/{session_id}/checkpoints"
    return _request("POST", _join(base_url, path), body)


def list_session_checkpoints(base_url: str, session_id: str, *, repo_name: str | None = None) -> list[dict]:
    path = f"/v1/native/sessions/{session_id}/checkpoints"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/sessions/{session_id}/checkpoints"
    return _request("GET", _join(base_url, path))


def get_session_checkpoint(base_url: str, checkpoint_id: str) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/checkpoints/{checkpoint_id}"))


def resume_session(
    base_url: str,
    session_id: str,
    *,
    after_sequence: int | None = None,
    limit: int = 200,
    repo_name: str | None = None,
) -> dict:
    path = f"/v1/native/sessions/{session_id}:resume"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/sessions/{session_id}:resume"
    return _request(
        "POST",
        _join(base_url, path),
        {"after_sequence": after_sequence, "limit": limit},
    )


def close_session(base_url: str, session_id: str, status: str = "paused", *, repo_name: str | None = None) -> dict:
    path = f"/v1/native/sessions/{session_id}:close"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/sessions/{session_id}:close"
    return _request("POST", _join(base_url, path), {"status": status})



def create_change(
    base_url: str,
    repo_name: str,
    task_id: str,
    title: str,
    base_line: str,
    risk_tier: str,
    change_id: str | None = None,
    *,
    fork_snapshot_id: str | None = None,
    forked_from_line: str | None = None,
) -> dict:
    body = {"task_id": task_id, "title": title, "base_line": base_line, "risk_tier": risk_tier}
    if change_id is not None:
        body["change_id"] = change_id
    if fork_snapshot_id is not None:
        body["fork_snapshot_id"] = fork_snapshot_id
    if forked_from_line is not None:
        body["forked_from_line"] = forked_from_line
    return _request(
        "POST",
        _join(base_url, f"/v1/native/repositories/{repo_name}/changes"),
        body,
    )



def list_changes(base_url: str, repo_name: str) -> list[dict]:
    return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/changes"))



def get_change(base_url: str, change_id: str, *, repo_name: str | None = None) -> dict:
    if repo_name:
        return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/changes/{change_id}"))
    return _request("GET", _join(base_url, f"/v1/native/changes/{change_id}"))


def get_change_detail(base_url: str, change_id: str, *, repo_name: str | None = None) -> dict:
    if repo_name:
        return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/read/changes/{change_id}"))
    return _request("GET", _join(base_url, f"/v1/native/read/changes/{change_id}"))



def close_change(base_url: str, change_id: str, status: str = "archived", *, repo_name: str | None = None) -> dict:
    if repo_name is not None:
        change_id = str(get_change(base_url, change_id, repo_name=repo_name).get("change_id") or change_id)
    return _request("POST", _join(base_url, f"/v1/native/changes/{change_id}:close"), {"status": status})



def create_stack(base_url: str, repo_name: str, title: str, change_ids: list[str] | None = None, landing_policy: str = "ordered") -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/repositories/{repo_name}/stacks"),
        {"title": title, "change_ids": change_ids or [], "landing_policy": landing_policy},
    )



def list_stacks(base_url: str, repo_name: str) -> list[dict]:
    return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/stacks"))



def get_stack(base_url: str, stack_id: str) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/stacks/{stack_id}"))



def update_stack(base_url: str, stack_id: str, title: str | None = None, landing_policy: str | None = None, status: str | None = None) -> dict:
    return _request(
        "PATCH",
        _join(base_url, f"/v1/native/stacks/{stack_id}"),
        {"title": title, "landing_policy": landing_policy, "status": status},
    )



def stack_add_change(base_url: str, stack_id: str, change_id: str, position: int | None = None) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/stacks/{stack_id}:addChange"),
        {"change_id": change_id, "position": position},
    )



def stack_remove_change(base_url: str, stack_id: str, change_id: str) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/stacks/{stack_id}:removeChange"),
        {"change_id": change_id},
    )



def stack_reorder_change(base_url: str, stack_id: str, change_id: str, position: int) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/stacks/{stack_id}:reorderChange"),
        {"change_id": change_id, "position": position},
    )



def get_stack_graph(base_url: str, stack_id: str) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/stacks/{stack_id}/graph"))



def publish_release(
    base_url: str,
    repo_name: str,
    release_id: str,
    version: str,
    line: str,
    snapshot_id: str,
    manifest_hash: str,
    profile: str,
    *,
    package: dict[str, Any] | None = None,
    checks: list[dict[str, Any]] | None = None,
    artifacts: list[dict[str, Any]] | None = None,
    formula: dict[str, Any] | None = None,
    metadata: dict[str, Any] | None = None,
) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/repositories/{repo_name}/releases"),
        {
            "release_id": release_id,
            "version": version,
            "line": line,
            "snapshot_id": snapshot_id,
            "manifest_hash": manifest_hash,
            "profile": profile,
            "package": package or {},
            "checks": checks or [],
            "artifacts": artifacts or [],
            "formula": formula or {},
            "metadata": metadata or {},
        },
    )


def get_release(base_url: str, release_id: str, *, repo_name: str | None = None) -> dict:
    if repo_name:
        return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/releases/{release_id}"))
    return _request("GET", _join(base_url, f"/v1/native/releases/{release_id}"))


def publish_patchset(
    base_url: str,
    change_id: str,
    base_snapshot_id: str,
    revision_snapshot_id: str,
    summary: str,
    author_mode: str,
    *,
    repo_name: str | None = None,
    exact_id: bool = False,
) -> dict:
    if repo_name is not None and not exact_id:
        change_id = str(get_change(base_url, change_id, repo_name=repo_name).get("change_id") or change_id)
    return _request(
        "POST",
        _join(base_url, f"/v1/native/changes/{change_id}/patchsets"),
        {
            "base_snapshot_id": base_snapshot_id,
            "revision_snapshot_id": revision_snapshot_id,
            "summary": summary,
            "author_mode": author_mode,
        },
    )


def list_patchsets(base_url: str, change_id: str, *, repo_name: str | None = None) -> list[dict]:
    if repo_name:
        return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/changes/{change_id}/patchsets"))
    return _request("GET", _join(base_url, f"/v1/native/changes/{change_id}/patchsets"))



def get_patchset(base_url: str, patchset_id: str, *, repo_name: str | None = None, change_ref: str | None = None) -> dict:
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/patchsets/{patchset_id}"
        if change_ref is not None:
            path += "?" + urllib.parse.urlencode({"change_ref": change_ref})
        return _request("GET", _join(base_url, path))
    return _request("GET", _join(base_url, f"/v1/native/patchsets/{patchset_id}"))



def select_patchset(
    base_url: str,
    change_id: str,
    patchset_id: str,
    *,
    repo_name: str | None = None,
    exact_id: bool = False,
) -> dict:
    if repo_name is not None and not exact_id:
        change_id = str(get_change(base_url, change_id, repo_name=repo_name).get("change_id") or change_id)
    return _request("POST", _join(base_url, f"/v1/native/changes/{change_id}:selectPatchset"), {"patchset_id": patchset_id})


def run_patchset_ci(
    base_url: str,
    patchset_id: str,
    *,
    trigger: str = "manual_rerun",
    repo_name: str | None = None,
    exact_id: bool = False,
) -> dict:
    if repo_name is not None and not exact_id:
        patchset_id = str(get_patchset(base_url, patchset_id, repo_name=repo_name).get("patchset_id") or patchset_id)
    return _request(
        "POST",
        _join(base_url, f"/v1/native/patchsets/{patchset_id}:runCi"),
        {"trigger": trigger},
        timeout_seconds=_LONG_RUNNING_CI_REQUEST_TIMEOUT_SECONDS,
    )


def run_repo_ci(
    base_url: str,
    repo_name: str,
    *,
    suite_ids: list[str] | None = None,
    plane: str | None = None,
    target_line: str = "main",
    trigger: str = "manual_rerun",
    selector: str | None = None,
    task_ids: list[str] | None = None,
    curated_corpus: str | None = None,
    count: int | None = None,
    window_days: int | None = None,
    dependency_evidence: list[str] | None = None,
    compliance_evidence: list[str] | None = None,
) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/admin/repositories/{urllib.parse.quote(repo_name, safe='')}:runCi"),
        {
            "suite_ids": list(suite_ids or []),
            "plane": plane,
            "target_line": target_line,
            "trigger": trigger,
            "selector": selector,
            "task_ids": list(task_ids or []),
            "curated_corpus": curated_corpus,
            "count": count,
            "window_days": window_days,
            "dependency_evidence": list(dependency_evidence or []),
            "compliance_evidence": list(compliance_evidence or []),
        },
        timeout_seconds=_LONG_RUNNING_CI_REQUEST_TIMEOUT_SECONDS,
    )



def request_review(
    base_url: str,
    change_id: str,
    patchset_id: str,
    reviewer_groups: list[str],
    note: str | None = None,
    *,
    repo_name: str | None = None,
    exact_id: bool = False,
) -> dict:
    if repo_name is not None and not exact_id:
        change_id = str(get_change(base_url, change_id, repo_name=repo_name).get("change_id") or change_id)
    return _request(
        "POST",
        _join(base_url, f"/v1/native/changes/{change_id}:requestReview"),
        {"patchset_id": patchset_id, "reviewer_groups": reviewer_groups, "note": note},
    )



def list_reviews(base_url: str, change_id: str, *, repo_name: str | None = None, exact_id: bool = False) -> dict:
    if repo_name is not None and not exact_id:
        change_id = str(get_change(base_url, change_id, repo_name=repo_name).get("change_id") or change_id)
    return _request("GET", _join(base_url, f"/v1/native/changes/{change_id}/reviews"))



def record_review(
    base_url: str,
    change_id: str,
    patchset_id: str,
    reviewer: str,
    action: str,
    comment: str | None = None,
    blocking: bool = False,
    *,
    repo_name: str | None = None,
    exact_id: bool = False,
) -> dict:
    if repo_name is not None and not exact_id:
        change_id = str(get_change(base_url, change_id, repo_name=repo_name).get("change_id") or change_id)
    return _request(
        "POST",
        _join(base_url, f"/v1/native/changes/{change_id}/reviews"),
        {"patchset_id": patchset_id, "reviewer": reviewer, "action": action, "comment": comment, "blocking": blocking},
    )



def put_attestation(
    base_url: str,
    patchset_id: str,
    author_mode: str,
    evaluation_summary: dict[str, Any],
    provenance_summary: dict[str, Any] | None = None,
    detail: dict[str, Any] | None = None,
    *,
    repo_name: str | None = None,
    exact_id: bool = False,
) -> dict:
    if repo_name is not None and not exact_id:
        patchset_id = str(get_patchset(base_url, patchset_id, repo_name=repo_name).get("patchset_id") or patchset_id)
    return _request(
        "PUT",
        _join(base_url, f"/v1/native/patchsets/{patchset_id}/attestation"),
        {
            "author_mode": author_mode,
            "evaluation_summary": evaluation_summary,
            "provenance_summary": provenance_summary or {},
            "detail": detail or {},
        },
    )



def get_attestation(base_url: str, patchset_id: str, *, repo_name: str | None = None, exact_id: bool = False) -> dict:
    if repo_name is not None and not exact_id:
        patchset_id = str(get_patchset(base_url, patchset_id, repo_name=repo_name).get("patchset_id") or patchset_id)
    return _request("GET", _join(base_url, f"/v1/native/patchsets/{patchset_id}/attestation"))



def evaluate_policy(base_url: str, patchset_id: str, *, repo_name: str | None = None, exact_id: bool = False) -> dict:
    if repo_name is not None and not exact_id:
        patchset_id = str(get_patchset(base_url, patchset_id, repo_name=repo_name).get("patchset_id") or patchset_id)
    return _request("POST", _join(base_url, f"/v1/native/patchsets/{patchset_id}:evaluatePolicy"), {})



def get_policy(base_url: str, patchset_id: str, *, repo_name: str | None = None, exact_id: bool = False) -> dict:
    if repo_name is not None and not exact_id:
        patchset_id = str(get_patchset(base_url, patchset_id, repo_name=repo_name).get("patchset_id") or patchset_id)
    return _request("GET", _join(base_url, f"/v1/native/patchsets/{patchset_id}/policy"))



def create_waiver(
    base_url: str,
    patchset_id: str,
    rule_name: str,
    reason: str,
    expires_at: str | None = None,
    *,
    repo_name: str | None = None,
    exact_id: bool = False,
) -> dict:
    if repo_name is not None and not exact_id:
        patchset_id = str(get_patchset(base_url, patchset_id, repo_name=repo_name).get("patchset_id") or patchset_id)
    return _request(
        "POST",
        _join(base_url, f"/v1/native/patchsets/{patchset_id}/waivers"),
        {"rule_name": rule_name, "reason": reason, "expires_at": expires_at},
    )



def submit_land(base_url: str, change_id: str, patchset_id: str | None, target_line: str, mode: str, *, repo_name: str | None = None) -> dict:
    path = f"/v1/native/changes/{change_id}:submit"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/changes/{change_id}:submit"
    return _request(
        "POST",
        _join(base_url, path),
        {"patchset_id": patchset_id, "target_line": target_line, "mode": mode},
    )



def get_land(base_url: str, submission_id: str, *, repo_name: str | None = None) -> dict:
    if repo_name:
        return _request("GET", _join(base_url, f"/v1/native/repositories/{repo_name}/lands/{submission_id}"))
    return _request("GET", _join(base_url, f"/v1/native/lands/{submission_id}"))



def retry_land(base_url: str, submission_id: str, reason: str | None = None, *, repo_name: str | None = None) -> dict:
    path = f"/v1/native/lands/{submission_id}:retry"
    if repo_name:
        path = f"/v1/native/repositories/{repo_name}/lands/{submission_id}:retry"
    return _request("POST", _join(base_url, path), {"reason": reason})


def list_jobs(
    base_url: str,
    repo_name: str,
    state: str | None = None,
    limit: int = 50,
    *,
    diagnostics: bool = False,
    stale_after_seconds: int = 300,
) -> list[dict] | dict:
    path = f"/v1/native/admin/repositories/{urllib.parse.quote(repo_name, safe='')}/jobs?limit={int(limit)}"
    if state:
        path += "&state=" + urllib.parse.quote(state, safe="")
    if diagnostics:
        path += "&diagnostics=true&stale_after_seconds=" + urllib.parse.quote(str(int(stale_after_seconds)), safe="")
    return _request("GET", _join(base_url, path))


def get_job(base_url: str, job_id: int) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/admin/jobs/{int(job_id)}"))


def get_server_metrics(base_url: str, *, recent_jobs_limit: int = 50, stale_after_seconds: int = 300) -> dict:
    path = (
        "/v1/native/admin/metrics"
        f"?recent_jobs_limit={int(recent_jobs_limit)}"
        "&stale_after_seconds=" + urllib.parse.quote(str(int(stale_after_seconds)), safe="")
    )
    return _request("GET", _join(base_url, path))


def get_server_readiness(base_url: str, *, recent_jobs_limit: int = 50, stale_after_seconds: int = 300) -> dict:
    path = (
        "/v1/native/admin/readiness"
        f"?recent_jobs_limit={int(recent_jobs_limit)}"
        "&stale_after_seconds=" + urllib.parse.quote(str(int(stale_after_seconds)), safe="")
    )
    return _request("GET", _join(base_url, path))


def reconcile_repo(base_url: str, repo_name: str, repair: bool = False) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/admin/repositories/{urllib.parse.quote(repo_name, safe='')}:reconcile"),
        {"repair": bool(repair)},
    )


def run_repo_migrations(base_url: str, repo_name: str) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/admin/repositories/{urllib.parse.quote(repo_name, safe='')}:migrations"),
    )


def get_repository_storage(base_url: str, repo_name: str) -> dict:
    return _request("GET", _join(base_url, f"/v1/native/admin/repositories/{urllib.parse.quote(repo_name, safe='')}/storage"))



def pack_repo(base_url: str, repo_name: str, *, repack: bool = False, max_members: int | None = None) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/admin/repositories/{urllib.parse.quote(repo_name, safe='')}:pack"),
        {"repack": bool(repack), "max_members": max_members},
    )



def optimize_repo(base_url: str, repo_name: str, *, repair: bool = True) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/admin/repositories/{urllib.parse.quote(repo_name, safe='')}:optimize"),
        {"repair": bool(repair)},
    )


def gc_repo(
    base_url: str,
    repo_name: str,
    *,
    prune_unreferenced: bool = True,
    prune_orphan_packs: bool = True,
) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/admin/repositories/{urllib.parse.quote(repo_name, safe='')}:gc"),
        {
            "prune_unreferenced": bool(prune_unreferenced),
            "prune_orphan_packs": bool(prune_orphan_packs),
        },
    )


def retire_repo(
    base_url: str,
    repo_name: str,
    *,
    expected_repo_id: str,
    require_verified_export: bool = True,
) -> dict:
    return _request(
        "POST",
        _join(base_url, f"/v1/native/admin/repositories/{urllib.parse.quote(repo_name, safe='')}:retire"),
        {
            "expected_repo_id": expected_repo_id,
            "require_verified_export": bool(require_verified_export),
        },
    )
