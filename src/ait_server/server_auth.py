from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Iterable, Optional

from fastapi import HTTPException, Request

from .server_control import connect, resolve_bound_roles
from .server_content import get_repository
from .server_paths import ServerContext

REPO_READER = "repo_reader"
REPO_CONTRIBUTOR = "repo_contributor"
REPO_REVIEWER = "repo_reviewer"
REPO_OWNER = "repo_owner"
RELEASE_MANAGER = "release_manager"
POLICY_ADMIN = "policy_admin"
SECURITY_REVIEWER = "security_reviewer"
OPERATOR = "operator"

ALL_ROLES = {
    REPO_READER,
    REPO_CONTRIBUTOR,
    REPO_REVIEWER,
    REPO_OWNER,
    RELEASE_MANAGER,
    POLICY_ADMIN,
    SECURITY_REVIEWER,
    OPERATOR,
}

ROLE_SETS = {
    "read": {REPO_READER, REPO_CONTRIBUTOR, REPO_REVIEWER, REPO_OWNER, RELEASE_MANAGER, POLICY_ADMIN, SECURITY_REVIEWER, OPERATOR},
    "contribute": {REPO_CONTRIBUTOR, REPO_REVIEWER, REPO_OWNER, RELEASE_MANAGER, OPERATOR},
    "review": {REPO_REVIEWER, REPO_OWNER, RELEASE_MANAGER, SECURITY_REVIEWER, OPERATOR},
    "approve_assisted": {REPO_REVIEWER, REPO_OWNER, RELEASE_MANAGER, SECURITY_REVIEWER, OPERATOR},
    "approve_critical": {REPO_OWNER, RELEASE_MANAGER, SECURITY_REVIEWER, OPERATOR},
    "waive": {POLICY_ADMIN, SECURITY_REVIEWER, REPO_OWNER, OPERATOR},
    "land": {RELEASE_MANAGER, REPO_OWNER, OPERATOR},
    "admin": {REPO_OWNER, OPERATOR},
}


@dataclass(frozen=True)
class ActorContext:
    identity: str
    actor_type: str
    claimed_roles: frozenset[str]
    claimed_repos: frozenset[str]
    mode: str


class AuthzError(HTTPException):
    pass



def _parse_csv(raw: Optional[str]) -> set[str]:
    if not raw:
        return set()
    return {item.strip() for item in raw.split(",") if item.strip()}



def auth_mode() -> str:
    return os.environ.get("AIT_NATIVE_AUTH_MODE", "open").strip().lower() or "open"



def actor_from_request(request: Request) -> ActorContext:
    mode = auth_mode()
    identity = request.headers.get("X-AIT-Actor", "").strip()
    actor_type = request.headers.get("X-AIT-Actor-Type", "human").strip() or "human"
    claimed_roles = frozenset(role for role in _parse_csv(request.headers.get("X-AIT-Roles")) if role in ALL_ROLES)
    claimed_repos = frozenset(_parse_csv(request.headers.get("X-AIT-Repos")))

    if mode == "open":
        if not identity:
            identity = "anonymous"
        return ActorContext(
            identity=identity,
            actor_type=actor_type,
            claimed_roles=frozenset(ALL_ROLES),
            claimed_repos=frozenset({"*"}),
            mode=mode,
        )

    if not identity:
        raise AuthzError(status_code=401, detail="Missing X-AIT-Actor in strict auth mode")
    return ActorContext(identity=identity, actor_type=actor_type, claimed_roles=claimed_roles, claimed_repos=claimed_repos, mode=mode)



def effective_roles(ctx: ServerContext, actor: ActorContext, repo_name: str) -> set[str]:
    if actor.mode == "open":
        return set(ALL_ROLES)

    repo_id = None
    try:
        repository = get_repository(ctx, repo_name)
    except KeyError:
        repository = None
    if repository:
        repo_id = str(repository.get("repo_id") or "").strip() or None

    roles = set()
    conn = connect(ctx)
    try:
        roles |= resolve_bound_roles(conn, repo_name, actor.identity, repo_id=repo_id)
    finally:
        conn.close()

    if OPERATOR in actor.claimed_roles:
        roles.add(OPERATOR)
    if "*" in actor.claimed_repos or repo_name in actor.claimed_repos:
        roles |= set(actor.claimed_roles)
    return roles



def ensure_repo_action(ctx: ServerContext, actor: ActorContext, repo_name: str, action: str, *, detail: Optional[str] = None) -> set[str]:
    allowed = ROLE_SETS[action]
    roles = effective_roles(ctx, actor, repo_name)
    if not (roles & allowed):
        message = detail or f"Actor {actor.identity} lacks permission for {action} on repository {repo_name}"
        raise AuthzError(status_code=403, detail=message)
    return roles



def ensure_line_update(ctx: ServerContext, actor: ActorContext, repo_name: str, line_name: str, default_line: str) -> set[str]:
    if line_name == default_line:
        return ensure_repo_action(ctx, actor, repo_name, "land", detail=f"Updating default line {line_name} requires release or owner authority")
    return ensure_repo_action(ctx, actor, repo_name, "contribute")



def ensure_review_action(ctx: ServerContext, actor: ActorContext, repo_name: str, action: str, lane: Optional[str]) -> set[str]:
    if action in {"approve", "task_approve"}:
        if lane == "critical":
            return ensure_repo_action(ctx, actor, repo_name, "approve_critical", detail="Critical approvals require owner, release, security, or operator authority")
        return ensure_repo_action(ctx, actor, repo_name, "approve_assisted")
    return ensure_repo_action(ctx, actor, repo_name, "review")



def ensure_admin_action(ctx: ServerContext, actor: ActorContext, repo_name: str) -> set[str]:
    return ensure_repo_action(ctx, actor, repo_name, "admin", detail=f"Managing role bindings for {repo_name} requires repo_owner or operator")



def whoami_payload(ctx: ServerContext, actor: ActorContext, repo_name: Optional[str] = None) -> dict:
    payload = {
        "identity": actor.identity,
        "actor_type": actor.actor_type,
        "mode": actor.mode,
        "claimed_roles": sorted(actor.claimed_roles),
        "claimed_repos": sorted(actor.claimed_repos),
    }
    if repo_name:
        payload["repo_name"] = repo_name
        payload["effective_roles"] = sorted(effective_roles(ctx, actor, repo_name))
    return payload
