from __future__ import annotations

import textwrap
from pathlib import Path

from ait_protocol.common import normalize_author_mode, policy_profile

from . import local_content_schema, local_control
from .repo_paths import APP_DIR, CONFIG_NAME, RepoContext
from .store_repo_config import load_config, load_policy, save_config, save_policy
from .task_worktree_layout import detect_init_task_worktree_defaults

_REPO_GOVERNANCE_DOCS = (
    ("docs/plan.md", "\n"),
    ("docs/milestone.md", "\n\n"),
)
_REPO_BOOTSTRAP_DIRS = ("docs/sprints",)


def _repo_agents_bootstrap(repo_name: str) -> str:
    return textwrap.dedent(
        f"""\
        # AGENTS

        Status: bootstrap instructions for agents in this `ait`-managed repository.
        Scope: workflow routing until the repository authors narrower local governance.

        ## Workspace Identity

        - This workspace is managed by `ait`.
        - Treat `ait` workflow state as the primary repository operating model.
        - Prefer `ait` workflow commands over raw Git for normal repository work.
        - Use raw Git only for exceptional interoperability or last-resort diagnostics.

        ## Session Bootstrap

        At the start of a new session in this repository:

        1. Read this file.
        2. Read [docs/plan.md](./docs/plan.md).
        3. If command routing is still unclear, run `ait workflow guide inventory` or `ait workflow guide land`.
        4. If this repository includes repo-root `ait-native.md` and you are choosing or switching `workflow_mode`, read it before picking the next workflow path.

        ## Command Routing

        - `ait init` starts this repository in a local-first `solo_local` posture by default.
        - For workflow inventory, prefer `ait queue summary` or `ait queue summary --all-changes`.
        - For one task's readiness, prefer `ait task audit <task-id>`.
        - When opening a task together with its first reviewable change, prefer `ait task start`.
        - For one change's landing path, prefer `ait workflow land <change-id>`.
        - For Markdown lineage, prefer `ait plan sync <file-or-dir>`.
        - If the repository later adds narrower local workflow docs, follow those narrower docs.

        ## Default Local-First Path

        - Start with local-only workflow unless shared durability or shared review is intentionally needed.
        - Keep sprint artifacts under `docs/sprints/`.
        - Do not add, route, or `ait plan sync` sprint entry through `docs/sprints/README.md`; keep sprint routing on the constitutional -> legal-layer -> command-layer path.
        - Common first steps:
          - `ait workflow guide inventory`
          - `ait task start --local --title "Describe the work" --intent "Explain the outcome" --base-line main`
          - `ait snapshot create --message "bootstrap"`

        ## Optional Remote Promotion

        - Stay local-first until a remote and workflow mode are intentionally configured.
        - For the common solo-remote path, use:
          - `ait remote add origin <url> --repo-name {repo_name} --default`
          - `ait config set --workflow-mode solo_remote`
          - `ait plan sync docs/plan.md --remote origin`
        """
    )


def _repo_native_bootstrap(repo_name: str, default_line: str) -> str:
    return textwrap.dedent(
        f"""\
        # ait native

        - default mode after `ait init`: `solo_local`

        ## Choose a mode

        - `solo_local`: keep work local and land with `ait workflow land-local`
        - `solo_remote`: use shared Markdown / task / change / review / land workflow

        ## Switch modes

        - `ait config set --workflow-mode solo_local`
        - `ait config set --workflow-mode solo_remote`

        Switching `workflow_mode` changes future command defaults only. It does not migrate existing plan / task / change lineage.

        ## Next steps

        - local first task:
          - `ait task start --local --title "Describe the work" --intent "Explain the outcome" --base-line {default_line}`
          - `ait snapshot create --message "bootstrap"`
        - solo_remote setup:
          - `ait remote add origin <url> --repo-name {repo_name} --default`
          - `ait config set --workflow-mode solo_remote`
          - `ait plan sync docs/plan.md --remote origin`
          - `ait task start --title "Describe the work" --intent "Explain the outcome" --base-line {default_line}`

        ## Read next

        - `AGENTS.md`
        - `docs/plan.md`
        - `ait workflow guide inventory`
        - `ait workflow guide land`
        """
    )


def _ensure_repo_governance_bootstrap(root: Path, repo_name: str, default_line: str) -> None:
    for relative_path in _REPO_BOOTSTRAP_DIRS:
        path = root / relative_path
        path.mkdir(parents=True, exist_ok=True)
    bootstrap_files = (
        ("AGENTS.md", _repo_agents_bootstrap(repo_name)),
        ("ait-native.md", _repo_native_bootstrap(repo_name, default_line)),
        *_REPO_GOVERNANCE_DOCS,
    )
    for relative_path, content in bootstrap_files:
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.exists():
            continue
        path.write_text(content, encoding="utf-8")


def _ensure_dirs(ait_dir: Path) -> None:
    (ait_dir / "objects" / "manifests").mkdir(parents=True, exist_ok=True)
    (ait_dir / "objects" / "packs").mkdir(parents=True, exist_ok=True)
    (ait_dir / "objects" / "tree-packs").mkdir(parents=True, exist_ok=True)
    (ait_dir / "refs" / "lines").mkdir(parents=True, exist_ok=True)
    (ait_dir / "workspace").mkdir(parents=True, exist_ok=True)
    (ait_dir / "worktrees").mkdir(parents=True, exist_ok=True)


def init_repo(
    root: Path,
    repo_name: str | None,
    default_line: str,
    policy_profile_name: str = "prototype",
    default_author_mode: str = "ai_with_human_review",
    default_model: str | None = None,
) -> RepoContext:
    root = root.resolve()
    ait_dir = root / APP_DIR
    if ait_dir.exists():
        raise FileExistsError(f"{ait_dir} already exists")
    ait_dir.mkdir(exist_ok=True)
    _ensure_dirs(ait_dir)

    ctx = RepoContext(
        root=root,
        ait_dir=ait_dir,
        content_db_path=ait_dir / "content.db",
        control_db_path=ait_dir / "control.db",
        config_path=ait_dir / CONFIG_NAME,
    )
    existing_config = load_config(ctx) if ctx.config_path.exists() else {}
    resolved_repo_name = str(existing_config.get("repo_name") or repo_name or root.name)
    resolved_default_line = str(existing_config.get("default_line") or default_line)
    local_control.initialize(ctx, resolved_repo_name, resolved_default_line)
    local_content_schema.initialize(ctx, resolved_default_line)

    if not ctx.policy_path.exists():
        save_policy(ctx, policy_profile(policy_profile_name))

    model_name = default_model.strip() if isinstance(default_model, str) else None
    if model_name == "":
        model_name = None
    config = dict(existing_config)
    config["repo_name"] = resolved_repo_name
    config["default_line"] = resolved_default_line
    config["current_line"] = str(config.get("current_line") or resolved_default_line)
    config.setdefault("default_remote", None)
    config.setdefault("id_namespace_prefix", "")
    config.setdefault("policy_profile", load_policy(ctx)["policy_id"])
    config["default_author_mode"] = normalize_author_mode(
        str(config.get("default_author_mode") or default_author_mode)
    )
    if "task_worktree" not in config:
        detected_task_worktree_defaults = detect_init_task_worktree_defaults(ctx)
        if detected_task_worktree_defaults:
            config["task_worktree"] = detected_task_worktree_defaults
    if model_name and "default_model" not in config:
        config["default_model"] = model_name
    save_config(ctx, config)

    local_control.record_event(
        ctx,
        "repository.initialized",
        "repository",
        resolved_repo_name,
        {
            "repo_name": resolved_repo_name,
            "default_line": resolved_default_line,
            "content_db": str(ctx.content_db_path),
            "control_db": str(ctx.control_db_path),
        },
    )
    _ensure_repo_governance_bootstrap(root, resolved_repo_name, resolved_default_line)
    return ctx
