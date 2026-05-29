#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
AIT_DIR="$(cd "${WORKSPACE_ROOT}/.ait" && pwd -P)"
REPO_ROOT_DEFAULT="$(cd "${AIT_DIR}/.." && pwd -P)"
REPO_ROOT="${AIT_GITHUB_PUBLISH_WORK_TREE:-${REPO_ROOT_DEFAULT}}"
GIT_DIR_PATH="${AIT_GITHUB_PUBLISH_GIT_DIR:-${AIT_DIR}/publisher-git}"
DEFAULT_REMOTE_NAME="${AIT_GITHUB_PUBLISH_REMOTE_NAME:-origin}"
DEFAULT_BRANCH="${AIT_GITHUB_PUBLISH_BRANCH:-main}"
DEFAULT_REMOTE_URL="${AIT_GITHUB_PUBLISH_REMOTE_URL:-git@github.com:weita2026/ait.git}"
GITHUB_PUBLISH_IGNORE_FILE="${AIT_GITHUB_PUBLISH_IGNORE_FILE:-.ait-github-publish-ignore}"
usage() {
  cat <<EOF
Usage:
  $(basename "$0") bootstrap [--remote <name>] [--remote-url <url>] [--branch <branch>]
  $(basename "$0") python-release
  $(basename "$0") status
  $(basename "$0") fetch [remote]
  $(basename "$0") rebase [upstream]
  $(basename "$0") add [path ...]
  $(basename "$0") add-python-release [extra-path ...]
  $(basename "$0") commit [git-commit-args ...]
  $(basename "$0") push-origin [git-push-args ...] [refspec]
  $(basename "$0") git <git-args ...>

Notes:
  - This helper keeps publish-time Git metadata under .ait/publisher-git instead of .git.
  - By default it publishes from the canonical repo root resolved from the real .ait path.
  - The default remote URL is ${DEFAULT_REMOTE_URL}
  - push-origin defaults to refspec HEAD:${DEFAULT_BRANCH} when none is supplied.
  - Override the remote URL with --remote-url or AIT_GITHUB_PUBLISH_REMOTE_URL.
  - "add" without explicit paths excludes matches from .aitignore and ${GITHUB_PUBLISH_IGNORE_FILE} when present.
EOF
}

git_cmd() {
  git --git-dir="${GIT_DIR_PATH}" --work-tree="${REPO_ROOT}" "$@"
}

python_cmd() {
  "${AIT_GITHUB_PUBLISH_PYTHON:-python3}" "$@"
}

emit_python_release_metadata() {
  python_cmd - "${REPO_ROOT}" <<'PY'
from __future__ import annotations

from pathlib import Path
import sys
import tomllib

root = Path(sys.argv[1])
pyproject_path = root / "pyproject.toml"
if not pyproject_path.is_file():
    raise SystemExit(f"pyproject.toml is missing from {root}")

data = tomllib.loads(pyproject_path.read_text(encoding="utf-8"))
project = data.get("project")
if not isinstance(project, dict):
    raise SystemExit("pyproject.toml is missing a [project] table.")

name = str(project.get("name") or "").strip()
version = str(project.get("version") or "").strip()
requires_python = str(project.get("requires-python") or "").strip()
readme = str(project.get("readme") or "").strip()
if not name or not version:
    raise SystemExit("pyproject.toml must define project.name and project.version.")

print(f"package_name={name}")
print(f"package_version={version}")
if requires_python:
    print(f"requires_python={requires_python}")
if readme:
    print(f"readme={readme}")
PY
}

python_release_surface_paths_nul() {
  python_cmd - "${REPO_ROOT}" <<'PY'
from __future__ import annotations

from pathlib import Path
import sys
import tomllib

root = Path(sys.argv[1])
pyproject_path = root / "pyproject.toml"
if not pyproject_path.is_file():
    raise SystemExit(f"pyproject.toml is missing from {root}")

data = tomllib.loads(pyproject_path.read_text(encoding="utf-8"))
project = data.get("project")
if not isinstance(project, dict):
    raise SystemExit("pyproject.toml is missing a [project] table.")

paths: list[str] = ["pyproject.toml"]
readme = str(project.get("readme") or "").strip()
if readme:
    candidate = root / readme
    if candidate.exists():
        paths.append(readme)

for candidate in ("src", "LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md"):
    if (root / candidate).exists():
        paths.append(candidate)

seen: set[str] = set()
for path in paths:
    normalized = str(Path(path).as_posix())
    if normalized in seen:
        continue
    seen.add(normalized)
    sys.stdout.buffer.write(normalized.encode("utf-8"))
    sys.stdout.buffer.write(b"\0")
PY
}

rule_file_excluded_paths_nul() {
  local rule_file="$1"
  if [[ ! -f "${rule_file}" ]]; then
    return 0
  fi

  python_cmd - "${REPO_ROOT}" "${rule_file}" <<'PY'
from __future__ import annotations

import fnmatch
import sys
from dataclasses import dataclass
from pathlib import Path

root = Path(sys.argv[1])
ignore_path = Path(sys.argv[2])
text = ignore_path.read_text(encoding="utf-8", errors="replace")


@dataclass(frozen=True)
class WorkspaceIgnoreRule:
    pattern: str
    negated: bool
    directory_only: bool
    anchored: bool
    basename_only: bool


def parse_rule(line: str) -> WorkspaceIgnoreRule | None:
    text = line.strip()
    if not text or text.startswith("#"):
        return None
    escaped = False
    if text.startswith("\\#") or text.startswith("\\!"):
        text = text[1:]
        escaped = True
    negated = text.startswith("!") and not escaped
    if negated:
        text = text[1:]
    while text.startswith("./"):
        text = text[2:]
    anchored = text.startswith("/")
    if anchored:
        text = text[1:]
    directory_only = text.endswith("/")
    text = text.rstrip("/")
    if not text:
        return None
    return WorkspaceIgnoreRule(
        pattern=text,
        negated=negated,
        directory_only=directory_only,
        anchored=anchored,
        basename_only="/" not in text,
    )


def rule_matches(rel_path: Path, rule: WorkspaceIgnoreRule) -> bool:
    parts = rel_path.parts
    if not parts:
        return False
    max_parts = len(parts) - 1 if rule.directory_only else len(parts)
    if max_parts <= 0:
        return False
    if rule.basename_only:
        return any(fnmatch.fnmatchcase(part, rule.pattern) for part in parts[:max_parts])
    starts = (0,) if rule.anchored else range(max_parts)
    for start in starts:
        for end in range(start + 1, max_parts + 1):
            candidate = "/".join(parts[start:end])
            if fnmatch.fnmatchcase(candidate, rule.pattern):
                return True
    return False


def path_is_ignored(rel_path: Path, rules: tuple[WorkspaceIgnoreRule, ...]) -> bool:
    ignored = False
    for rule in rules:
        if rule_matches(rel_path, rule):
            ignored = not rule.negated
    return ignored


rules = tuple(rule for line in text.splitlines() if (rule := parse_rule(line)) is not None)
if not rules:
    raise SystemExit(0)

for path in sorted(root.rglob("*")):
    if path.is_dir():
        continue
    rel = path.relative_to(root)
    if path_is_ignored(rel, rules):
        sys.stdout.buffer.write(rel.as_posix().encode("utf-8"))
        sys.stdout.buffer.write(b"\0")
PY
}

add_with_aitignore_excludes() {
  local excluded_paths=()
  local rel_path
  while IFS= read -r -d '' rel_path; do
    excluded_paths+=("${rel_path}")
  done < <(rule_file_excluded_paths_nul "${REPO_ROOT}/.aitignore")
  while IFS= read -r -d '' rel_path; do
    excluded_paths+=("${rel_path}")
  done < <(rule_file_excluded_paths_nul "${REPO_ROOT}/${GITHUB_PUBLISH_IGNORE_FILE}")

  if [[ ${#excluded_paths[@]} -eq 0 ]]; then
    git_cmd add -A
    return
  fi

  git_cmd add -A
  git_cmd reset -q -- "${excluded_paths[@]}"
}

add_python_release_surface() {
  local release_paths=()
  while IFS= read -r -d '' rel_path; do
    release_paths+=("${rel_path}")
  done < <(python_release_surface_paths_nul)

  if [[ ${#release_paths[@]} -eq 0 ]]; then
    echo "No Python release surface paths were discovered under ${REPO_ROOT}" >&2
    exit 1
  fi

  if [[ $# -gt 0 ]]; then
    release_paths+=("$@")
  fi

  git_cmd add -- "${release_paths[@]}"
  emit_python_release_metadata
}

init_git_dir() {
  mkdir -p "${AIT_DIR}"
  if [[ ! -f "${GIT_DIR_PATH}/HEAD" ]]; then
    git init --bare "${GIT_DIR_PATH}" >/dev/null
  fi
  git config --file "${GIT_DIR_PATH}/config" core.bare false
  git config --file "${GIT_DIR_PATH}/config" core.worktree "${REPO_ROOT}"
  git config --file "${GIT_DIR_PATH}/config" advice.detachedHead false
}

ensure_remote() {
  local remote_name="$1"
  local remote_url="$2"
  if git_cmd remote get-url "${remote_name}" >/dev/null 2>&1; then
    git_cmd remote set-url "${remote_name}" "${remote_url}"
  else
    git_cmd remote add "${remote_name}" "${remote_url}"
  fi
}

bootstrap_hidden_git() {
  local remote_name="${1:-${DEFAULT_REMOTE_NAME}}"
  local remote_url="${2:-${DEFAULT_REMOTE_URL}}"
  local branch_name="${3:-${DEFAULT_BRANCH}}"
  local allow_missing_fetch="${4:-0}"
  local local_branch="publish-${branch_name}"

  init_git_dir
  ensure_remote "${remote_name}" "${remote_url}"
  if ! git_cmd fetch --prune "${remote_name}"; then
    if [[ "${allow_missing_fetch}" != "1" ]]; then
      return 1
    fi
  fi

  if git_cmd show-ref --verify --quiet "refs/remotes/${remote_name}/${branch_name}"; then
    git_cmd update-ref "refs/heads/${local_branch}" "refs/remotes/${remote_name}/${branch_name}"
    git_cmd symbolic-ref HEAD "refs/heads/${local_branch}"
    git_cmd reset --mixed "${local_branch}" >/dev/null
    git_cmd branch --set-upstream-to "${remote_name}/${branch_name}" "${local_branch}" >/dev/null 2>&1 || true
  fi

  printf 'repo_root=%s\n' "${REPO_ROOT}"
  printf 'workspace_root=%s\n' "${WORKSPACE_ROOT}"
  printf 'git_dir=%s\n' "${GIT_DIR_PATH}"
  printf 'remote=%s\n' "${remote_name}"
  printf 'branch=%s\n' "${branch_name}"
}

ensure_bootstrapped() {
  if [[ ! -f "${GIT_DIR_PATH}/HEAD" ]]; then
    local allow_missing_fetch=0
    if [[ -z "${AIT_GITHUB_PUBLISH_REMOTE_URL:-}" ]]; then
      allow_missing_fetch=1
    fi
    bootstrap_hidden_git "${DEFAULT_REMOTE_NAME}" "${DEFAULT_REMOTE_URL}" "${DEFAULT_BRANCH}" "${allow_missing_fetch}" >/dev/null
  else
    init_git_dir
  fi
}

require_clean_rebase_work_tree() {
  local tracked_status
  tracked_status="$(git_cmd status --porcelain --untracked-files=no)"
  if [[ -n "${tracked_status}" ]]; then
    echo "Publish rebase requires a clean tracked publish work tree: ${REPO_ROOT}" >&2
    echo "Commit or stash tracked changes before rebasing, or set AIT_GITHUB_PUBLISH_WORK_TREE to a clean release mirror." >&2
    exit 1
  fi
}

cmd="${1:-}"
if [[ -z "${cmd}" ]]; then
  usage
  exit 0
fi
shift

case "${cmd}" in
  bootstrap)
    remote_name="${DEFAULT_REMOTE_NAME}"
    remote_url="${DEFAULT_REMOTE_URL}"
    branch_name="${DEFAULT_BRANCH}"
    allow_missing_fetch=0
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --remote)
          remote_name="$2"
          shift 2
          ;;
        --remote-url)
          remote_url="$2"
          allow_missing_fetch=0
          shift 2
          ;;
        --branch)
          branch_name="$2"
          shift 2
          ;;
        *)
          echo "Unknown bootstrap argument: $1" >&2
          exit 2
          ;;
      esac
    done
    if [[ "${remote_url}" == "${DEFAULT_REMOTE_URL}" && -z "${AIT_GITHUB_PUBLISH_REMOTE_URL:-}" ]]; then
      allow_missing_fetch=1
    fi
    bootstrap_hidden_git "${remote_name}" "${remote_url}" "${branch_name}" "${allow_missing_fetch}"
    ;;
  python-release)
    emit_python_release_metadata
    while IFS= read -r -d '' rel_path; do
      printf 'release_path=%s\n' "${rel_path}"
    done < <(python_release_surface_paths_nul)
    ;;
  status)
    ensure_bootstrapped
    git_cmd status --short
    ;;
  fetch)
    ensure_bootstrapped
    remote_name="${1:-${DEFAULT_REMOTE_NAME}}"
    if [[ $# -gt 0 ]]; then
      shift
    fi
    git_cmd fetch --prune "${remote_name}" "$@"
    ;;
  rebase)
    ensure_bootstrapped
    upstream="${1:-${DEFAULT_REMOTE_NAME}/${DEFAULT_BRANCH}}"
    if [[ $# -gt 0 ]]; then
      shift
    fi
    require_clean_rebase_work_tree
    git_cmd rebase "${upstream}" "$@"
    ;;
  add)
    ensure_bootstrapped
    if [[ $# -eq 0 ]]; then
      add_with_aitignore_excludes
    else
      git_cmd add "$@"
    fi
    ;;
  add-python-release)
    ensure_bootstrapped
    add_python_release_surface "$@"
    ;;
  commit)
    ensure_bootstrapped
    git_cmd commit "$@"
    ;;
  push-origin)
    ensure_bootstrapped
    if [[ $# -eq 0 ]]; then
      git_cmd push "${DEFAULT_REMOTE_NAME}" "HEAD:${DEFAULT_BRANCH}"
      exit 0
    fi

    has_refspec=0
    for arg in "$@"; do
      case "${arg}" in
        *:*|HEAD|"${DEFAULT_BRANCH}"|refs/*)
          has_refspec=1
          break
          ;;
      esac
    done

    if [[ ${has_refspec} -eq 0 ]]; then
      git_cmd push "${DEFAULT_REMOTE_NAME}" "$@" "HEAD:${DEFAULT_BRANCH}"
    else
      git_cmd push "${DEFAULT_REMOTE_NAME}" "$@"
    fi
    ;;
  git)
    ensure_bootstrapped
    git_cmd "$@"
    ;;
  *)
    echo "Unknown command: ${cmd}" >&2
    usage >&2
    exit 2
    ;;
esac
