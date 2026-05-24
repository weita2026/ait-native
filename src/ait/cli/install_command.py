from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any, Optional

import typer

from .bootstrap_views import _emit
from .workflow_mode_config import WORKFLOW_MODE_PRESETS, _effective_workflow_mode
from ..local_content import workspace_runtime_root_hygiene
from ..server_runtime_preflight import postgres_preflight_report
from ..store import RepoContext, add_remote, init_repo, list_remotes, load_config, save_config

_MODE_ALIASES = {
    "local": "solo_local",
    "solo_local": "solo_local",
    "remote": "solo_remote",
    "solo_remote": "solo_remote",
}
_ATTACH_CHOICES = frozenset({"none", "telegram", "discord", "both"})
_SERVER_SETUP_ALIASES = {
    "skip": "skip",
    "none": "skip",
    "no": "skip",
    "connect": "connect",
    "existing": "connect",
    "deploy": "deploy",
    "prepare": "deploy",
    "self-hosted": "deploy",
    "self_hosted": "deploy",
}
_SERVER_SETUP_CHOICES = frozenset({"skip", "connect", "deploy"})


def _normalize_mode(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = str(value).strip().lower()
    return _MODE_ALIASES.get(normalized)


def _normalize_attach(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = str(value).strip().lower()
    if normalized not in _ATTACH_CHOICES:
        return None
    return normalized


def _normalize_server_setup(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = str(value).strip().lower()
    return _SERVER_SETUP_ALIASES.get(normalized)


def _prompt_choice(prompt: str, *, default: str, choices: set[str]) -> str:
    choice = str(typer.prompt(prompt, default=default)).strip().lower()
    if choice in choices:
        return choice
    allowed = ", ".join(sorted(choices))
    raise typer.BadParameter(f"{prompt} must be one of: {allowed}.")


def _resolve_mode(
    mode: str | None,
    *,
    json_output: bool,
) -> str:
    normalized = _normalize_mode(mode)
    if normalized is not None:
        return normalized
    if mode is not None:
        raise typer.BadParameter("`--mode` must be `local`, `remote`, `solo_local`, or `solo_remote`.")
    if json_output:
        return "solo_local"
    prompt_value = _prompt_choice(
        "Choose workflow mode: local or remote",
        default="local",
        choices={"local", "remote"},
    )
    return _MODE_ALIASES[prompt_value]


def _resolve_attach(
    attach: str | None,
    *,
    json_output: bool,
) -> str:
    normalized = _normalize_attach(attach)
    if normalized is not None:
        return normalized
    if attach is not None:
        raise typer.BadParameter("`--attach` must be `none`, `telegram`, `discord`, or `both`.")
    if json_output:
        return "none"
    return _prompt_choice(
        "Choose transport attach: none, telegram, discord, or both",
        default="none",
        choices={"none", "telegram", "discord", "both"},
    )


def _resolve_server_setup(
    server_setup: str | None,
    *,
    mode: str,
    json_output: bool,
) -> str:
    normalized = _normalize_server_setup(server_setup)
    if normalized is not None:
        if mode != "solo_remote" and normalized != "skip":
            raise typer.BadParameter("`--server-setup` can only be `connect` or `deploy` when `--mode` is remote.")
        return normalized
    if server_setup is not None:
        raise typer.BadParameter("`--server-setup` must be `skip`, `connect`, or `deploy`.")
    if mode != "solo_remote" or json_output:
        return "skip"
    return _prompt_choice(
        "Choose ait-server setup: skip, connect, or deploy",
        default="skip",
        choices=set(_SERVER_SETUP_CHOICES),
    )


def _discover_repo_context() -> RepoContext | None:
    try:
        return RepoContext.discover(Path.cwd())
    except FileNotFoundError:
        return None


def _ensure_repo_context(
    *,
    repo_name: str | None,
    initialize: bool | None,
    json_output: bool,
    dry_run: bool,
) -> tuple[RepoContext | None, dict[str, Any]]:
    existing = _discover_repo_context()
    cwd = Path.cwd().resolve()
    if existing is not None:
        return existing, {
            "state": "existing_repo",
            "repo_root": str(existing.root),
            "repo_initialized": False,
            "action": "unchanged",
        }
    if initialize is False:
        raise typer.BadParameter("No `ait` repository found in the current directory or its parents.")
    should_init = True
    if initialize is None and not json_output:
        should_init = typer.confirm("No `ait` repository found. Initialize the current directory now?", default=True)
    if not should_init:
        raise typer.Abort()
    if dry_run:
        return None, {
            "state": "missing_repo",
            "repo_root": str(cwd),
            "repo_initialized": False,
            "action": "would_create",
            "repo_name": repo_name or cwd.name,
        }
    ctx = init_repo(cwd, repo_name, "main")
    return ctx, {
        "state": "initialized_repo",
        "repo_root": str(ctx.root),
        "repo_initialized": True,
        "action": "created",
        "repo_name": repo_name or ctx.root.name,
    }


def _agent_config_path(repo_root: Path) -> Path:
    return repo_root / ".ait" / "agent-workers.json"


def _load_agent_workers(repo_root: Path) -> dict[str, dict[str, Any]]:
    path = _agent_config_path(repo_root)
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, OSError, json.JSONDecodeError):
        return {}
    workers = payload.get("workers")
    if not isinstance(workers, dict):
        return {}
    normalized: dict[str, dict[str, Any]] = {}
    for key, value in workers.items():
        if isinstance(key, str) and isinstance(value, dict):
            normalized[key] = dict(value)
    return normalized


def _worker_action(before: dict[str, Any] | None, desired: dict[str, Any], *, fields: tuple[str, ...]) -> str:
    if before is None:
        return "created"
    for field in fields:
        if str(before.get(field) or "") != str(desired.get(field) or ""):
            return "updated"
    return "unchanged"


def _preview_action(action: str) -> str:
    if action == "created":
        return "would_create"
    if action == "updated":
        return "would_update"
    return f"would_{action}"


def _require_secret(
    value: str | None,
    *,
    prompt: str,
    json_output: bool,
    hide_input: bool = True,
) -> str:
    if value is not None and str(value).strip():
        return str(value).strip()
    if json_output:
        raise typer.BadParameter(f"Missing required value for {prompt}.")
    return str(typer.prompt(prompt, hide_input=hide_input)).strip()


def _optional_prompt(
    value: str | None,
    *,
    prompt: str,
    json_output: bool,
) -> str | None:
    if value is not None:
        text = str(value).strip()
        return text or None
    if json_output:
        return None
    text = str(typer.prompt(prompt, default="")).strip()
    return text or None


def _run_agent_cli(repo_root: Path, args: list[str]) -> dict[str, Any]:
    env = dict(os.environ)
    env["AIT_REPO_ROOT"] = str(repo_root)
    env.pop("AIT_AGENT_CONFIG_PATH", None)
    src_candidates = [str(Path(__file__).resolve().parents[2])]
    repo_src_path = repo_root / "src"
    if repo_src_path.exists():
        repo_src = str(repo_src_path)
        if repo_src not in src_candidates:
            src_candidates.append(repo_src)
    existing_pythonpath = env.get("PYTHONPATH")
    combined_pythonpath = ":".join(src_candidates)
    env["PYTHONPATH"] = combined_pythonpath if not existing_pythonpath else f"{combined_pythonpath}:{existing_pythonpath}"
    command = [
        sys.executable,
        "-c",
        "from ait_agent.cli import app; app()",
        *args,
        "--json",
    ]
    completed = subprocess.run(
        command,
        cwd=repo_root,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or f"exit code {completed.returncode}"
        raise typer.BadParameter(f"`{' '.join(command)}` failed: {detail}")
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise typer.BadParameter(f"`{' '.join(command)}` returned invalid JSON.") from exc


def _classify_runtime_root(repo_root: Path) -> dict[str, Any]:
    report = workspace_runtime_root_hygiene(repo_root)
    if str(report.get("runtime_root_source") or "") == "unconfigured":
        classification = "installed_but_not_configured"
    else:
        classification = "healthy" if str(report.get("state") or "") == "pass" else "configured_but_unhealthy"
    return {"classification": classification, "report": report}


def _apply_workflow_mode_preset(config: dict[str, Any], mode: str) -> dict[str, Any]:
    preset = WORKFLOW_MODE_PRESETS[mode]
    updated = dict(config)
    updated["workflow_mode"] = mode
    updated["workflow_default_scope"] = preset["workflow_default_scope"]
    updated["task_default_scope"] = preset["task_default_scope"]
    updated["change_default_scope"] = preset["change_default_scope"]
    binding_cfg = dict(updated.get("plan_task_binding") or {})
    binding_cfg["mode"] = preset["plan_task_binding_mode"]
    updated["plan_task_binding"] = binding_cfg
    return updated


def _classify_postgres() -> dict[str, Any]:
    try:
        report = postgres_preflight_report(
            server_data=None,
            backend="postgres",
            dsn=None,
            content_schema=None,
            control_schema=None,
            connect=False,
        )
    except RuntimeError as exc:
        return {
            "classification": "installed_but_not_configured",
            "report": {
                "ready": False,
                "issues": [str(exc)],
                "next_actions": [
                    "Set AIT_NATIVE_SERVER_DATA before running local ait-server or PostgreSQL preflight checks.",
                ],
            },
        }
    if not bool(report.get("psycopg_installed")):
        classification = "missing"
    elif not bool(report.get("postgres_dsn_configured")):
        classification = "installed_but_not_configured"
    elif bool(report.get("ready")):
        classification = "healthy"
    else:
        classification = "configured_but_unhealthy"
    return {"classification": classification, "report": report}


def _require_text(
    value: str | None,
    *,
    prompt: str,
    json_output: bool,
) -> str:
    if value is not None and str(value).strip():
        return str(value).strip()
    if json_output:
        raise typer.BadParameter(f"Missing required value for {prompt}.")
    return str(typer.prompt(prompt)).strip()


def _existing_remote_matches(row: dict[str, Any], *, url: str, repo_name: str | None) -> bool:
    current_repo_name = row.get("repo_name")
    if str(row.get("url") or "") != url:
        return False
    if repo_name is None:
        return True
    return str(current_repo_name or "") == repo_name


def _configure_server_setup(
    ctx: RepoContext | None,
    repo_info: dict[str, Any],
    *,
    mode: str,
    server_setup: str,
    server_url: str | None,
    remote_name: str,
    remote_repo_name: str | None,
    json_output: bool,
    dry_run: bool,
) -> dict[str, Any]:
    if mode != "solo_remote":
        return {
            "choice": "skip",
            "action": "not_applicable",
            "classification": "not_applicable",
            "remote_name": None,
            "server_url": None,
            "next_steps": [],
        }
    if server_setup == "skip":
        return {
            "choice": "skip",
            "action": "skipped",
            "classification": "installed_but_not_configured",
            "remote_name": None,
            "server_url": None,
            "next_steps": [
                "Connect an existing ait-server later with `ait remote add origin <url> --repo-name <repo-name> --default`.",
                "Or rerun `ait install --mode remote --server-setup connect --server-url <url>`.",
            ],
        }
    if server_setup == "deploy":
        return {
            "choice": "deploy",
            "action": "deferred",
            "classification": "installed_but_not_configured",
            "remote_name": remote_name,
            "server_url": None,
            "next_steps": [
                "Run `ait doctor runtime-root --json` before placing local ait-server data.",
                "Run `ait doctor postgres --json` and configure PostgreSQL before starting shared workflow services.",
                "Deploy or start ait-server through the dedicated self-hosted/operator path, then rerun `ait install --mode remote --server-setup connect --server-url <url>`.",
            ],
        }

    resolved_url = _require_text(server_url, prompt="ait-server URL", json_output=json_output).rstrip("/")
    desired_repo_name = remote_repo_name or str(repo_info.get("repo_name") or "").strip() or None
    if desired_repo_name is None and ctx is not None:
        desired_repo_name = ctx.root.name

    if ctx is None:
        return {
            "choice": "connect",
            "action": "would_configure_after_init" if dry_run else "blocked_missing_repo",
            "classification": "installed_but_not_configured",
            "remote_name": remote_name,
            "server_url": resolved_url,
            "repo_name": desired_repo_name,
            "next_steps": [
                "Initialize this directory as an ait repository before writing remote server config.",
            ],
        }

    existing = {str(row.get("name") or ""): row for row in list_remotes(ctx)}
    existing_row = existing.get(remote_name)
    default_remote_before = str((load_config(ctx) or {}).get("default_remote") or "") or None
    if existing_row is not None and not _existing_remote_matches(existing_row, url=resolved_url, repo_name=desired_repo_name):
        return {
            "choice": "connect",
            "action": "blocked_existing_remote_mismatch",
            "classification": "configured_but_unhealthy",
            "remote_name": remote_name,
            "server_url": resolved_url,
            "repo_name": desired_repo_name,
            "existing_remote": {
                "url": existing_row.get("url"),
                "repo_name": existing_row.get("repo_name"),
            },
            "next_steps": [
                f"Remote `{remote_name}` already points somewhere else; choose a different --remote-name or reconcile the existing remote manually.",
            ],
        }

    if existing_row is None:
        action = "would_create" if dry_run else "created"
        if not dry_run:
            add_remote(ctx, remote_name, resolved_url, desired_repo_name, make_default=True)
    else:
        action = "unchanged" if default_remote_before == remote_name else "default_updated"
        if dry_run and action == "default_updated":
            action = "would_update_default"
        elif not dry_run and default_remote_before != remote_name:
            cfg = load_config(ctx)
            cfg["default_remote"] = remote_name
            save_config(ctx, cfg)

    return {
        "choice": "connect",
        "action": action,
        "classification": "healthy",
        "remote_name": remote_name,
        "server_url": resolved_url,
        "repo_name": desired_repo_name,
        "next_steps": [
            f"Verify the ait-server with `ait queue summary --remote {remote_name}`.",
            f"Publish Markdown lineage with `ait plan sync <file-or-dir> --remote {remote_name}` when the plan should become shared.",
        ],
    }


def _mode_next_steps(mode: str, *, transport_actions: list[dict[str, Any]], server_setup: dict[str, Any]) -> list[str]:
    if mode == "solo_local":
        steps = [
            "Shape the work in Markdown first.",
            "Run `ait plan sync <file-or-dir>` for the relevant Markdown artifact.",
            'Run `ait task start --title "<title>" --intent "<intent>"` for the first local slice.',
        ]
    else:
        remote_name = str(server_setup.get("remote_name") or "origin")
        steps = list(server_setup.get("next_steps") or [])
        steps.extend(
            [
                f"Publish the relevant Markdown lineage with `ait plan sync <file-or-dir> --remote {remote_name}` after a remote is connected.",
                f'Start the remote-backed slice with `ait task start --title "<title>" --intent "<intent>" --remote {remote_name}` after shared lineage is ready.',
            ]
        )
    for item in transport_actions:
        if item.get("kind") in {"telegram", "discord"}:
            steps.append(
                f"{item['kind'].title()} worker `{item['name']}` was configured but not started automatically; use `ait-agent {item['kind']} start {item['name']}` when you are ready."
            )
    if not transport_actions:
        steps.append("You can add Telegram or Discord later by rerunning `ait install --attach ...`.")
    return steps


def _render_install_summary(payload: dict[str, Any]) -> None:
    lines = ["ait install summary", ""]
    repo = payload.get("repository") if isinstance(payload.get("repository"), dict) else {}
    mode = payload.get("mode") if isinstance(payload.get("mode"), dict) else {}
    lines.append(f"- repo root: {repo.get('repo_root')}")
    lines.append(f"- repo action: {repo.get('action')}")
    lines.append(f"- workflow mode: {mode.get('effective_mode')}")
    if repo.get("repo_initialized"):
        lines.append("- repository was initialized during this run")
    runtime_root = payload.get("runtime_root") if isinstance(payload.get("runtime_root"), dict) else {}
    lines.append(f"- runtime-root classification: {runtime_root.get('classification')}")
    postgres = payload.get("postgres") if isinstance(payload.get("postgres"), dict) else {}
    if postgres:
        lines.append(f"- postgres classification: {postgres.get('classification')}")
    server = payload.get("server") if isinstance(payload.get("server"), dict) else {}
    if server:
        lines.append(f"- ait-server setup: {server.get('choice')} ({server.get('action')})")
    transport_actions = payload.get("transport_actions") if isinstance(payload.get("transport_actions"), list) else []
    if transport_actions:
        lines.append("")
        lines.append("Transport actions")
        for item in transport_actions:
            lines.append(f"- {item.get('kind')}/{item.get('name')}: {item.get('action')}")
    lines.append("")
    lines.append("Next steps")
    for step in payload.get("next_steps") or []:
        lines.append(f"- {step}")
    typer.echo("\n".join(lines))


def register_install_command(app: typer.Typer) -> None:
    @app.command("install")
    def install_cmd(
        mode: Optional[str] = typer.Option(
            None,
            "--mode",
            help="Workflow mode choice: local, remote, solo_local, or solo_remote.",
        ),
        attach: Optional[str] = typer.Option(
            None,
            "--attach",
            help="Optional transport attach: none, telegram, discord, or both.",
        ),
        server_setup: Optional[str] = typer.Option(
            None,
            "--server-setup",
            help="Remote-backed ait-server setup: skip, connect, or deploy.",
        ),
        server_url: Optional[str] = typer.Option(None, "--server-url", help="ait-server URL when --server-setup connect is selected."),
        remote_name: str = typer.Option("origin", "--remote-name", help="Remote name to use when connecting an ait-server."),
        remote_repo_name: Optional[str] = typer.Option(None, "--remote-repo-name", help="Remote repository name to associate with the ait-server."),
        repo_name: Optional[str] = typer.Option(
            None,
            "--name",
            help="Repository name to use if this command initializes the current directory.",
        ),
        initialize: Optional[bool] = typer.Option(
            None,
            "--init/--no-init",
            help="Initialize the current directory when no `ait` repository exists yet.",
        ),
        worker_name: str = typer.Option("main", "--worker-name", help="Worker name to use for attached transports."),
        telegram_token: Optional[str] = typer.Option(None, "--telegram-token", help="Telegram bot token."),
        telegram_username: Optional[str] = typer.Option(None, "--telegram-username", help="Telegram bot username."),
        discord_application_id: Optional[str] = typer.Option(None, "--discord-application-id", help="Discord application id."),
        discord_bot_token: Optional[str] = typer.Option(None, "--discord-bot-token", help="Discord bot token."),
        dry_run: bool = typer.Option(False, "--dry-run", help="Preview the detected state and planned changes without writing config."),
        json_output: bool = typer.Option(False, "--json"),
    ) -> None:
        """Bootstrap local trust-layer mode selection and optional agent transport config."""

        resolved_mode = _resolve_mode(mode, json_output=json_output)
        resolved_attach = _resolve_attach(attach, json_output=json_output)
        resolved_server_setup = _resolve_server_setup(
            server_setup,
            mode=resolved_mode,
            json_output=json_output,
        )
        ctx, repo_info = _ensure_repo_context(
            repo_name=repo_name,
            initialize=initialize,
            json_output=json_output,
            dry_run=dry_run,
        )
        repo_root = Path(str(repo_info["repo_root"])).resolve()

        before_config = load_config(ctx) if ctx is not None else {}
        desired_config = _apply_workflow_mode_preset(before_config, resolved_mode)
        mode_action = "unchanged" if desired_config == before_config else "updated"
        if ctx is not None and not dry_run:
            save_config(ctx, desired_config)

        effective_mode = resolved_mode
        if ctx is not None and not dry_run:
            effective_mode = str(_effective_workflow_mode(ctx).get("value") or resolved_mode)

        before_workers = _load_agent_workers(repo_root)
        transport_actions: list[dict[str, Any]] = []
        server_action = _configure_server_setup(
            ctx,
            repo_info,
            mode=resolved_mode,
            server_setup=resolved_server_setup,
            server_url=server_url,
            remote_name=remote_name,
            remote_repo_name=remote_repo_name,
            json_output=json_output,
            dry_run=dry_run,
        )

        if resolved_attach in {"telegram", "both"}:
            token = _require_secret(
                telegram_token,
                prompt="Telegram bot token",
                json_output=json_output,
            )
            username = _optional_prompt(
                telegram_username,
                prompt="Telegram bot username (optional)",
                json_output=json_output,
            )
            desired = {"token": token, "username": username}
            before = before_workers.get(f"telegram/{worker_name}")
            action = _worker_action(before, desired, fields=("token", "username"))
            if not dry_run:
                _run_agent_cli(
                    repo_root,
                    [
                        "telegram",
                        "add",
                        worker_name,
                        "--token",
                        token,
                        *(["--username", username] if username else []),
                    ],
                )
            transport_actions.append(
                {
                    "kind": "telegram",
                    "name": worker_name,
                    "action": action if not dry_run else _preview_action(action),
                    "configured": True,
                }
            )

        if resolved_attach in {"discord", "both"}:
            application_id = _require_secret(
                discord_application_id,
                prompt="Discord application id",
                json_output=json_output,
                hide_input=False,
            )
            bot_token = _require_secret(
                discord_bot_token,
                prompt="Discord bot token",
                json_output=json_output,
            )
            desired = {"application_id": application_id, "bot_token": bot_token}
            before = before_workers.get(f"discord/{worker_name}")
            action = _worker_action(before, desired, fields=("application_id", "bot_token"))
            if not dry_run:
                _run_agent_cli(
                    repo_root,
                    [
                        "discord",
                        "add",
                        worker_name,
                        "--application-id",
                        application_id,
                        "--bot-token",
                        bot_token,
                    ],
                )
            transport_actions.append(
                {
                    "kind": "discord",
                    "name": worker_name,
                    "action": action if not dry_run else _preview_action(action),
                    "configured": True,
                }
            )

        payload = {
            "repository": repo_info,
            "mode": {
                "requested_mode": resolved_mode,
                "effective_mode": effective_mode,
                "action": mode_action if not dry_run else _preview_action(mode_action),
            },
            "attach_choice": resolved_attach,
            "server": server_action,
            "runtime_root": _classify_runtime_root(repo_root),
            "transport_actions": transport_actions,
            "next_steps": _mode_next_steps(
                resolved_mode,
                transport_actions=transport_actions,
                server_setup=server_action,
            ),
            "dry_run": dry_run,
        }
        if resolved_mode == "solo_remote":
            payload["postgres"] = _classify_postgres()
        if json_output:
            _emit(payload, True)
            return
        _render_install_summary(payload)


__all__ = ["register_install_command"]
