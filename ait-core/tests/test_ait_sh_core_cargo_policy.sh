#!/usr/bin/env bash

set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
launcher=${repo_root}/ait.sh
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-core-cargo-policy.XXXXXX")

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-core-cargo-policy.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected Cargo policy fixture: %s\n' \
        "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

write_source_policy() {
  local root=$1
  mkdir -p "${root}/.cargo" "${root}/.ait"
  printf '%s\n' \
    '# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.' \
    '[build]' \
    'target-dir = ".ait/cargo-target"' \
    'build-dir = ".ait/cargo-build/canonical"' \
    >"${root}/.cargo/config.toml"
}

write_task_projection() {
  local root=$1
  local task_name=$2
  mkdir -p "${root}/.cargo" "${root}/.ait"
  printf '%s\n' \
    '# Managed by ait: workspace-isolated final artifacts and intermediates.' \
    '[build]' \
    "target-dir = \"${root}/.ait/cargo-target/task-workspaces/${task_name}\"" \
    "build-dir = \"${root}/.ait/cargo-build/task-workspaces/${task_name}\"" \
    >"${root}/.cargo/config.toml"
}

canonical_root=${temporary_root}/canonical
write_source_policy "${canonical_root}"
canonical_physical=$(CDPATH='' cd -- "${canonical_root}" && pwd -P)
(
  # shellcheck source=../ait.sh
  source "${launcher}"
  ROOT_DIR=${canonical_root}
  require_canonical_cargo_source_policy
  test "$(cargo_target_dir)" = "${canonical_physical}/.ait/cargo-target"
  test "$(cargo_build_dir)" = "${canonical_physical}/.ait/cargo-build/canonical"
)

projected_root=${temporary_root}/projected
write_task_projection "${projected_root}" stale-task
if (
  # shellcheck source=../ait.sh
  source "${launcher}"
  ROOT_DIR=${projected_root}
  require_canonical_cargo_source_policy
) >"${temporary_root}/projected.stdout" 2>"${temporary_root}/projected.stderr"; then
  printf 'canonical Cargo policy accepted a Task-worktree projection\n' >&2
  exit 65
fi
grep -F 'Task-worktree projection must never be stored on canonical main.' \
  "${temporary_root}/projected.stderr" >/dev/null

worktree_root=${temporary_root}/worktree
write_task_projection "${worktree_root}" task-current
printf '%s\n' '{"worktree_name":"task-current"}' \
  >"${worktree_root}/.ait-worktree.json"
worktree_physical=$(CDPATH='' cd -- "${worktree_root}" && pwd -P)
(
  # shellcheck source=../ait.sh
  source "${launcher}"
  ROOT_DIR=${worktree_root}
  require_canonical_cargo_source_policy
  target_dir=$(cargo_target_dir)
  build_dir=$(cargo_build_dir)
  test "${target_dir}" = \
    "${worktree_physical}/.ait/cargo-target/task-workspaces/task-current"
  test "${build_dir}" = \
    "${worktree_physical}/.ait/cargo-build/task-workspaces/task-current"
  test "${target_dir}" != "${build_dir}"
)

for required in \
  'require_canonical_cargo_source_policy' \
  'A Task-worktree projection must never be stored on canonical main.' \
  'tests/test_ait_sh_core_cargo_policy.sh'; do
  grep -F -- "${required}" "${launcher}" >/dev/null
done

printf 'ait.sh canonical Cargo policy contract: pass\n'
