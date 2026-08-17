#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PATH="/Users/weita/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"
DEFAULT_REPOSITORY_CARGO_BUILD_MAX_BYTES=4294967296
DEFAULT_CANONICAL_CARGO_BUILD_MAX_BYTES=1073741824
CANONICAL_CARGO_BUILD_DIRNAME="canonical"
MANAGED_WORKTREE_CARGO_TARGET_DIRNAME="task-workspaces"
CANONICAL_CARGO_SOURCE_POLICY_HEADER="# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection."

resolve_cargo() {
  if command -v cargo >/dev/null 2>&1; then
    command -v cargo
    return 0
  fi
  if command -v rustup >/dev/null 2>&1; then
    rustup which cargo 2>/dev/null || true
    return 0
  fi
  return 1
}

resolve_rust_tool() {
  local name="$1"
  if command -v rustup >/dev/null 2>&1; then
    rustup which "$name" 2>/dev/null || true
    return 0
  fi
  command -v "$name" 2>/dev/null || true
}

repository_cargo_target_root() {
  local ait_root="${ROOT_DIR}/.ait"
  if [[ -d "${ait_root}" ]]; then
    ait_root="$(cd "${ait_root}" && pwd -P)"
  fi
  printf '%s/cargo-target\n' "${ait_root}"
}

repository_cargo_build_root() {
  local build_root="${ROOT_DIR}/.ait/cargo-build"
  mkdir -p "${build_root}"
  (cd "${build_root}" && pwd -P)
}

managed_worktree_name() {
  local marker="${ROOT_DIR}/.ait-worktree.json"
  if [[ ! -f "${marker}" || -L "${marker}" ]]; then
    return 1
  fi
  local name
  name="$(sed -n 's/.*"worktree_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    "${marker}" | head -n 1)"
  case "${name}" in
    ''|.|..|*[!a-zA-Z0-9._-]*)
      return 1
      ;;
  esac
  printf '%s\n' "${name}"
}

canonical_cargo_source_policy_is_exact() {
  local repository_root="$1"
  local config_path="${repository_root}/.cargo/config.toml"
  [[ -f "${config_path}" && ! -L "${config_path}" ]] || return 1
  awk -v expected_header="${CANONICAL_CARGO_SOURCE_POLICY_HEADER}" '
    NR == 1 { header_ok = ($0 == expected_header) }
    NR == 2 { build_header_ok = ($0 == "[build]") }
    /^[[:space:]]*target-dir[[:space:]]*=/ {
      target_count += 1
      if ($0 != "target-dir = \".ait/cargo-target\"") bad = 1
    }
    /^[[:space:]]*build-dir[[:space:]]*=/ {
      build_count += 1
      if ($0 != "build-dir = \".ait/cargo-build/canonical\"") bad = 1
    }
    /task-workspaces/ { bad = 1 }
    END {
      exit !(header_ok && build_header_ok && target_count == 1 &&
        build_count == 1 && !bad)
    }
  ' "${config_path}"
}

require_canonical_cargo_source_policy() {
  if [[ -n "$(managed_worktree_name || true)" ]]; then
    return 0
  fi
  if canonical_cargo_source_policy_is_exact "${ROOT_DIR}"; then
    return 0
  fi
  printf '%s\n' \
    'Canonical ait-core Cargo source policy is invalid.' \
    'Expected .cargo/config.toml to use .ait/cargo-target and .ait/cargo-build/canonical.' \
    'A Task-worktree projection must never be stored on canonical main.' >&2
  return 2
}

cargo_target_dir() {
  if [[ -n "${AIT_SHARED_CARGO_TARGET_DIR:-}" ]]; then
    printf '%s\n' "${AIT_SHARED_CARGO_TARGET_DIR}"
    return 0
  fi
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    printf '%s\n' "${CARGO_TARGET_DIR}"
    return 0
  fi
  local target_root
  target_root="$(repository_cargo_target_root)"
  local worktree_name
  worktree_name="$(managed_worktree_name || true)"
  if [[ -n "${worktree_name}" ]]; then
    printf '%s/%s/%s\n' \
      "${target_root}" "${MANAGED_WORKTREE_CARGO_TARGET_DIRNAME}" "${worktree_name}"
    return 0
  fi
  printf '%s\n' "${target_root}"
}

cargo_build_dir() {
  if [[ -n "${AIT_SHARED_CARGO_BUILD_DIR:-}" ]]; then
    printf '%s\n' "${AIT_SHARED_CARGO_BUILD_DIR}"
    return 0
  fi
  if [[ -n "${CARGO_BUILD_BUILD_DIR:-}" ]]; then
    printf '%s\n' "${CARGO_BUILD_BUILD_DIR}"
    return 0
  fi
  local build_root
  build_root="$(repository_cargo_build_root)"
  local worktree_name
  worktree_name="$(managed_worktree_name || true)"
  if [[ -n "${worktree_name}" ]]; then
    printf '%s/task-workspaces/%s\n' "${build_root}" "${worktree_name}"
    return 0
  fi
  printf '%s/%s\n' "${build_root}" "${CANONICAL_CARGO_BUILD_DIRNAME}"
}

resolved_cargo_dir() {
  local path="$1"
  if [[ "$(basename "${path}")" == "{workspace-path-hash}" ]]; then
    local parent
    parent="$(dirname "${path}")"
    mkdir -p "${parent}"
    printf '%s/{workspace-path-hash}\n' "$(cd "${parent}" && pwd -P)"
    return 0
  fi
  mkdir -p "${path}"
  (cd "${path}" && pwd -P)
}

ensure_distinct_cargo_dirs() {
  local target_dir="$1"
  local build_dir="$2"
  local resolved_target
  local resolved_build
  resolved_target="$(resolved_cargo_dir "${target_dir}")"
  resolved_build="$(resolved_cargo_dir "${build_dir}")"
  if [[ "${resolved_target}" == "${resolved_build}" ]]; then
    printf 'Cargo target-dir and build-dir must be distinct: %s\n' "${resolved_target}" >&2
    return 2
  fi
}

file_mtime_epoch() {
  local path="$1"
  stat -f '%m' "${path}" 2>/dev/null || stat -c '%Y' "${path}" 2>/dev/null
}

cargo_cache_in_use() {
  local cache_dir="$1"
  if ! command -v lsof >/dev/null 2>&1; then
    return 2
  fi
  local owner_pid
  owner_pid="$(lsof -t +D "${cache_dir}" 2>/dev/null | head -n 1 || true)"
  [[ -n "${owner_pid}" ]]
}

repository_workspace_cargo_build_leaves() {
  local build_root="$1"
  local shard
  local leaf
  local shard_name
  local leaf_name
  for shard in "${build_root}"/workspaces/*; do
    [[ -d "${shard}" && ! -L "${shard}" ]] || continue
    shard_name="$(basename "${shard}")"
    if [[ ${#shard_name} -ne 2 || ! "${shard_name}" =~ ^[0-9a-f]+$ ]]; then
      continue
    fi
    for leaf in "${shard}"/*; do
      [[ -d "${leaf}" && ! -L "${leaf}" ]] || continue
      leaf_name="$(basename "${leaf}")"
      if [[ ${#leaf_name} -ne 14 || ! "${leaf_name}" =~ ^[0-9a-f]+$ ]]; then
        continue
      fi
      (cd "${leaf}" && pwd -P)
    done
  done
}

repository_task_cargo_build_leaves() {
  local build_root="$1"
  local leaf
  for leaf in "${build_root}"/task-workspaces/*; do
    [[ -d "${leaf}" && ! -L "${leaf}" ]] || continue
    (cd "${leaf}" && pwd -P)
  done
}

repository_task_cargo_target_leaves() {
  local target_root="$1"
  local leaf
  for leaf in "${target_root}/${MANAGED_WORKTREE_CARGO_TARGET_DIRNAME}"/*; do
    [[ -d "${leaf}" && ! -L "${leaf}" ]] || continue
    (cd "${leaf}" && pwd -P)
  done
}

repository_managed_cargo_build_leaves() {
  local build_root="$1"
  local canonical_dir="${build_root}/${CANONICAL_CARGO_BUILD_DIRNAME}"
  if [[ -d "${canonical_dir}" && ! -L "${canonical_dir}" ]]; then
    (cd "${canonical_dir}" && pwd -P)
  fi
  repository_workspace_cargo_build_leaves "${build_root}"
  repository_task_cargo_build_leaves "${build_root}"
}

cargo_build_leaf_activity_mtime() {
  local leaf="$1"
  local entry
  local entry_mtime
  local newest_mtime=0
  local entries=()
  shopt -s dotglob nullglob
  entries=("${leaf}"/* "${leaf}"/*/*)
  shopt -u dotglob nullglob
  for entry in "${entries[@]}"; do
    case "${entry}" in
      "${leaf}/.ait-gc-lock"|"${leaf}/.ait-gc-marker")
        continue
        ;;
    esac
    entry_mtime="$(file_mtime_epoch "${entry}" || true)"
    [[ -n "${entry_mtime}" ]] || continue
    if (( entry_mtime > newest_mtime )); then
      newest_mtime="${entry_mtime}"
    fi
  done
  printf '%s\n' "${newest_mtime}"
}

clear_cargo_build_contents() {
  local build_dir="$1"
  local candidate
  local candidates=()
  local failed=0
  shopt -s dotglob nullglob
  candidates=("${build_dir}"/*)
  shopt -u dotglob nullglob
  for candidate in "${candidates[@]}"; do
    case "$(basename "${candidate}")" in
      .ait-gc-lock|.ait-gc-marker)
        continue
        ;;
    esac
    if ! rm -rf -- "${candidate}"; then
      failed=1
    fi
  done
  return "${failed}"
}

release_cargo_gc_lock() {
  local lock_dir="$1"
  rm -f -- "${lock_dir}/pid" 2>/dev/null || true
  rmdir "${lock_dir}" 2>/dev/null || true
}

acquire_cargo_gc_lock() {
  local lock_dir="$1"
  local now="$2"
  if mkdir "${lock_dir}" 2>/dev/null; then
    printf '%s\n' "$$" > "${lock_dir}/pid"
    return 0
  fi

  local lock_pid=""
  local lock_mtime=""
  lock_pid="$(cat "${lock_dir}/pid" 2>/dev/null || true)"
  case "${lock_pid}" in
    ''|*[!0-9]*)
      lock_mtime="$(file_mtime_epoch "${lock_dir}" || true)"
      if [[ -z "${lock_mtime}" ]] || (( now - lock_mtime < 60 )); then
        return 1
      fi
      ;;
    *)
      if kill -0 "${lock_pid}" 2>/dev/null; then
        return 1
      fi
      ;;
  esac
  release_cargo_gc_lock "${lock_dir}"
  if ! mkdir "${lock_dir}" 2>/dev/null; then
    return 1
  fi
  printf '%s\n' "$$" > "${lock_dir}/pid"
}

auto_reclaim_single_cargo_build_dir() {
  local build_dir="$1"
  local max_bytes="$2"
  local interval_seconds="$3"
  if [[ "$(basename "${build_dir}")" == "{workspace-path-hash}" ]]; then
    printf 'Skipping Cargo build-dir GC for a workspace template; use explicit cache maintenance after Cargo expands it: %s\n' \
      "${build_dir}" >&2
    return 0
  fi

  mkdir -p "${build_dir}"
  local marker="${build_dir}/.ait-gc-marker"
  local lock_dir="${build_dir}/.ait-gc-lock"
  local now
  local marker_mtime
  now="$(date +%s)"
  # Full-tree sizing is intentionally periodic: it is much slower than a warm
  # Cargo no-op on large external caches. CI RAM has its own admission reclaimer.
  if [[ -f "${marker}" && "${interval_seconds}" != "0" ]]; then
    marker_mtime="$(file_mtime_epoch "${marker}" || true)"
    if [[ -n "${marker_mtime}" ]] && (( now - marker_mtime < interval_seconds )); then
      return 0
    fi
  fi
  if ! acquire_cargo_gc_lock "${lock_dir}" "${now}"; then
    return 0
  fi

  local size_kib
  local max_kib
  size_kib="$(du -sk "${build_dir}" 2>/dev/null | awk '{print $1}')"
  max_kib=$((max_bytes / 1024))
  if [[ -z "${size_kib}" ]] || (( size_kib <= max_kib )); then
    touch "${marker}"
    release_cargo_gc_lock "${lock_dir}"
    return 0
  fi
  if ! command -v lsof >/dev/null 2>&1; then
    printf 'Skipping Cargo build-dir GC because active use cannot be verified without lsof: %s\n' \
      "${build_dir}" >&2
    release_cargo_gc_lock "${lock_dir}"
    return 0
  fi
  if cargo_cache_in_use "${build_dir}"; then
    printf 'Skipping active Cargo build-dir GC: %s\n' "${build_dir}" >&2
    release_cargo_gc_lock "${lock_dir}"
    return 0
  fi

  printf 'Cargo build-dir exceeds %s bytes; reclaiming idle intermediates: %s\n' \
    "${max_bytes}" "${build_dir}"
  if ! clear_cargo_build_contents "${build_dir}"; then
    printf 'Cargo build-dir GC could not remove every intermediate: %s\n' "${build_dir}" >&2
    release_cargo_gc_lock "${lock_dir}"
    return 0
  fi
  touch "${marker}"
  release_cargo_gc_lock "${lock_dir}"
}

auto_reclaim_repository_cargo_build_pool() {
  local build_dir="$1"
  local max_bytes="$2"
  local interval_seconds="$3"
  local build_root
  build_root="$(repository_cargo_build_root)"
  local marker="${build_root}/.ait-gc-marker"
  local lock_dir="${build_root}/.ait-gc-lock"
  local now
  local marker_mtime
  now="$(date +%s)"
  if [[ -f "${marker}" && "${interval_seconds}" != "0" ]]; then
    marker_mtime="$(file_mtime_epoch "${marker}" || true)"
    if [[ -n "${marker_mtime}" ]] && (( now - marker_mtime < interval_seconds )); then
      return 0
    fi
  fi
  if ! acquire_cargo_gc_lock "${lock_dir}" "${now}"; then
    return 0
  fi

  local size_kib
  local max_kib
  size_kib="$(du -sk "${build_root}" 2>/dev/null | awk '{print $1}')"
  max_kib=$((max_bytes / 1024))
  if [[ -z "${size_kib}" ]] || (( size_kib <= max_kib )); then
    touch "${marker}"
    release_cargo_gc_lock "${lock_dir}"
    return 0
  fi
  if ! command -v lsof >/dev/null 2>&1; then
    printf 'Skipping repository Cargo build-dir GC because active use cannot be verified without lsof: %s\n' \
      "${build_root}" >&2
    release_cargo_gc_lock "${lock_dir}"
    return 0
  fi

  local protected_dir=""
  local candidate
  local candidate_mtime
  if [[ "$(basename "${build_dir}")" == "{workspace-path-hash}" ]]; then
    local newest_mtime=-1
    while IFS= read -r candidate; do
      candidate_mtime="$(cargo_build_leaf_activity_mtime "${candidate}")"
      [[ -n "${candidate_mtime}" ]] || continue
      if (( candidate_mtime > newest_mtime )); then
        newest_mtime="${candidate_mtime}"
        protected_dir="${candidate}"
      fi
    done < <(repository_workspace_cargo_build_leaves "${build_root}")
  elif [[ -d "${build_dir}" ]]; then
    protected_dir="$(cd "${build_dir}" && pwd -P)"
  fi
  if [[ -n "${protected_dir}" ]]; then
    touch "${protected_dir}/.ait-last-used"
  fi

  printf 'Repository Cargo build cache exceeds %s bytes; reclaiming oldest idle leaves: %s\n' \
    "${max_bytes}" "${build_root}"
  local resolved_candidate
  while IFS=$'\t' read -r _ candidate; do
    [[ -n "${candidate:-}" ]] || continue
    if [[ "${candidate}" == "${protected_dir}" ]]; then
      continue
    fi
    size_kib="$(du -sk "${build_root}" 2>/dev/null | awk '{print $1}')"
    if [[ -z "${size_kib}" ]] || (( size_kib <= max_kib )); then
      break
    fi
    if cargo_cache_in_use "${candidate}"; then
      printf 'Skipping active Cargo build-dir GC: %s\n' "${candidate}" >&2
      continue
    fi
    resolved_candidate="$(cd "${candidate}" && pwd -P)"
    if [[ "${resolved_candidate}" != "${candidate}" ]]; then
      printf 'Skipping Cargo build-dir GC after path resolution changed: %s\n' \
        "${candidate}" >&2
      continue
    fi
    printf 'Reclaiming idle Cargo build-dir leaf: %s\n' "${candidate}"
    if ! clear_cargo_build_contents "${candidate}"; then
      printf 'Cargo build-dir GC could not remove every intermediate: %s\n' \
        "${candidate}" >&2
      continue
    fi
    touch "${candidate}/.ait-gc-marker"
  done < <(
    while IFS= read -r candidate; do
      candidate_mtime="$(cargo_build_leaf_activity_mtime "${candidate}")"
      printf '%020d\t%s\n' "${candidate_mtime}" "${candidate}"
    done < <(repository_managed_cargo_build_leaves "${build_root}") | sort -n
  )

  size_kib="$(du -sk "${build_root}" 2>/dev/null | awk '{print $1}')"
  if [[ -n "${size_kib}" ]] && (( size_kib > max_kib )); then
    printf 'Repository Cargo build cache remains above %s bytes because the newest or active leaves were retained: %s\n' \
      "${max_bytes}" "${build_root}" >&2
  fi
  touch "${marker}"
  release_cargo_gc_lock "${lock_dir}"
}

cargo_build_dir_is_repository_owned() {
  local build_dir="$1"
  local build_root
  local resolved_build_dir
  build_root="$(repository_cargo_build_root)"
  resolved_build_dir="$(resolved_cargo_dir "${build_dir}")"
  [[ "${resolved_build_dir}" == "${build_root}" || \
    "${resolved_build_dir}" == "${build_root}/"* ]]
}

cargo_build_dir_is_canonical() {
  local build_dir="$1"
  local build_root
  local resolved_build_dir
  build_root="$(repository_cargo_build_root)"
  resolved_build_dir="$(resolved_cargo_dir "${build_dir}")"
  [[ "${resolved_build_dir}" == "${build_root}/${CANONICAL_CARGO_BUILD_DIRNAME}" ]]
}

auto_reclaim_cargo_build_dir() {
  local build_dir="$1"
  local interval_seconds="${AIT_CARGO_BUILD_GC_INTERVAL_SECONDS:-3600}"
  local repository_owned=0
  if cargo_build_dir_is_repository_owned "${build_dir}"; then
    repository_owned=1
  fi
  local max_bytes
  if [[ "${AIT_CARGO_BUILD_MAX_BYTES+configured}" == "configured" ]]; then
    max_bytes="${AIT_CARGO_BUILD_MAX_BYTES}"
  elif [[ "${repository_owned}" == "1" ]]; then
    max_bytes="${DEFAULT_REPOSITORY_CARGO_BUILD_MAX_BYTES}"
  else
    max_bytes=0
  fi
  case "${max_bytes}:${interval_seconds}" in
    *[!0-9:]*|:*)
      printf 'Skipping Cargo build-dir GC because its numeric configuration is invalid.\n' >&2
      return 0
      ;;
  esac
  if [[ "${max_bytes}" == "0" ]]; then
    return 0
  fi
  if [[ "${repository_owned}" == "1" ]]; then
    auto_reclaim_repository_cargo_build_pool \
      "${build_dir}" "${max_bytes}" "${interval_seconds}"
    if cargo_build_dir_is_canonical "${build_dir}"; then
      local canonical_max_bytes
      if [[ "${AIT_CARGO_BUILD_MAX_BYTES+configured}" == "configured" ]]; then
        canonical_max_bytes="${AIT_CARGO_BUILD_MAX_BYTES}"
      else
        canonical_max_bytes="${DEFAULT_CANONICAL_CARGO_BUILD_MAX_BYTES}"
      fi
      auto_reclaim_single_cargo_build_dir \
        "${build_dir}" "${canonical_max_bytes}" 0
    fi
    return 0
  fi
  auto_reclaim_single_cargo_build_dir "${build_dir}" "${max_bytes}" "${interval_seconds}"
}

core_build_profile() {
  local profile
  profile="${AIT_CORE_BUILD_PROFILE:-release}"
  case "${profile}" in
    release)
      printf '%s\n' "${profile}"
      ;;
    *)
      printf 'Unsupported AIT_CORE_BUILD_PROFILE: %s\n' "${profile}" >&2
      printf 'Only release is supported; debug/dev profile is forbidden.\n' >&2
      return 2
      ;;
  esac
}

cargo_profile_dir() {
  core_build_profile
}

run_cargo() {
  local cargo_bin
  cargo_bin="$(resolve_cargo)"
  local rustc_bin
  rustc_bin="$(resolve_rust_tool rustc)"
  local rustdoc_bin
  rustdoc_bin="$(resolve_rust_tool rustdoc)"
  if [[ -n "${rustc_bin}" ]]; then
    export RUSTC="${rustc_bin}"
  fi
  if [[ -n "${rustdoc_bin}" ]]; then
    export RUSTDOC="${rustdoc_bin}"
  fi
  local existing_rustflags
  existing_rustflags="${RUSTFLAGS:-}"
  local pyo3_link_flags="-C link-arg=-undefined -C link-arg=dynamic_lookup"
  if [[ -n "${existing_rustflags}" ]]; then
    export RUSTFLAGS="${existing_rustflags} ${pyo3_link_flags}"
  else
    export RUSTFLAGS="${pyo3_link_flags}"
  fi
  local target_dir
  local build_dir
  target_dir="$(cargo_target_dir)"
  build_dir="$(cargo_build_dir)"
  ensure_distinct_cargo_dirs "${target_dir}" "${build_dir}"
  export CARGO_TARGET_DIR="${target_dir}"
  export CARGO_BUILD_BUILD_DIR="${build_dir}"
  local cargo_status=0
  "${cargo_bin}" "$@" || cargo_status=$?
  auto_reclaim_cargo_build_dir "${build_dir}"
  return "${cargo_status}"
}

refresh_build_artifact_mtimes() {
  local target_dir
  target_dir="$(cargo_target_dir)"
  local profile_dir
  profile_dir="$(cargo_profile_dir)"
  local artifact
  for artifact in \
    "${target_dir}/${profile_dir}/ait-cli" \
    "${target_dir}/${profile_dir}/ait-cli.exe" \
    "${target_dir}/${profile_dir}/ait-agent" \
    "${target_dir}/${profile_dir}/ait-agent.exe" \
    "${target_dir}/${profile_dir}/ait-agent-worker" \
    "${target_dir}/${profile_dir}/ait-agent-worker.exe" \
    "${target_dir}/${profile_dir}/libait_py.dylib" \
    "${target_dir}/${profile_dir}/libait_py.so" \
    "${target_dir}/${profile_dir}/ait_py.dll"; do
    if [[ -e "${artifact}" ]]; then
      touch "${artifact}"
    fi
  done
}

resolve_native_artifact() {
  local artifact_dir="$1"
  local name="$2"
  if [[ -f "${artifact_dir}/${name}" ]]; then
    printf '%s\n' "${artifact_dir}/${name}"
    return 0
  fi
  if [[ -f "${artifact_dir}/${name}.exe" ]]; then
    printf '%s\n' "${artifact_dir}/${name}.exe"
    return 0
  fi
  printf 'Missing native release artifact: %s/%s[.exe]\n' \
    "${artifact_dir}" "${name}" >&2
  return 2
}

install_native_core_commands() {
  local bin_dir="${AIT_NATIVE_BIN_DIR:-}"
  local skip_build=0
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --bin-dir)
        if [[ $# -lt 2 || -z "$2" ]]; then
          printf 'The --bin-dir option requires a non-empty path.\n' >&2
          return 2
        fi
        bin_dir="$2"
        shift
        ;;
      --skip-build)
        skip_build=1
        ;;
      --help|-h)
        cat <<'EOF'
Usage:
  ./ait.sh core install [--bin-dir <path>] [--skip-build]

Build and install the ait-core-owned native commands:

  ait-cli            -> ait
  ait-agent          -> ait-agent
  ait-agent-worker   -> ait-agent-worker

The default destination is AIT_NATIVE_BIN_DIR when set, otherwise
${XDG_BIN_HOME}/ when set, otherwise ${HOME}/.local/bin. The destination must
already be on PATH if the commands should be available by name. --skip-build
uses the existing release artifacts and fails when any artifact is missing.
EOF
        return 0
        ;;
      *)
        printf 'Unknown install option: %s\n' "$1" >&2
        return 2
        ;;
    esac
    shift
  done

  if [[ -z "${bin_dir}" ]]; then
    if [[ -n "${XDG_BIN_HOME:-}" ]]; then
      bin_dir="${XDG_BIN_HOME}"
    elif [[ -n "${HOME:-}" ]]; then
      bin_dir="${HOME}/.local/bin"
    else
      printf 'Cannot choose an install destination: set --bin-dir or AIT_NATIVE_BIN_DIR.\n' >&2
      return 2
    fi
  fi

  if [[ "${skip_build}" != "1" ]]; then
    require_canonical_cargo_source_policy
    local profile
    profile="$(core_build_profile)"
    run_cargo build --profile "${profile}" --manifest-path "${ROOT_DIR}/rust/Cargo.toml" --workspace
    refresh_build_artifact_mtimes
  fi

  local target_dir
  local profile_dir
  local artifact_dir
  target_dir="$(cargo_target_dir)"
  profile_dir="$(cargo_profile_dir)"
  artifact_dir="${target_dir}/${profile_dir}"

  local cli_artifact
  local agent_artifact
  local worker_artifact
  cli_artifact="$(resolve_native_artifact "${artifact_dir}" ait-cli)"
  agent_artifact="$(resolve_native_artifact "${artifact_dir}" ait-agent)"
  worker_artifact="$(resolve_native_artifact "${artifact_dir}" ait-agent-worker)"

  local command_suffix=""
  local agent_suffix=""
  local worker_suffix=""
  if [[ "${cli_artifact}" == *.exe ]]; then
    command_suffix=".exe"
  fi
  if [[ "${agent_artifact}" == *.exe ]]; then
    agent_suffix=".exe"
  fi
  if [[ "${worker_artifact}" == *.exe ]]; then
    worker_suffix=".exe"
  fi
  if [[ "${agent_suffix}" != "${command_suffix}" ||
    "${worker_suffix}" != "${command_suffix}" ]]; then
    printf 'Native release artifacts use inconsistent executable suffixes.\n' >&2
    return 2
  fi

  mkdir -p "${bin_dir}"
  bin_dir="$(cd "${bin_dir}" && pwd -P)"
  local staging_dir
  staging_dir="$(mktemp -d "${bin_dir}/.ait-native-install.XXXXXX")"

  if ! install -m 0755 "${cli_artifact}" "${staging_dir}/ait${command_suffix}" ||
    ! install -m 0755 "${agent_artifact}" "${staging_dir}/ait-agent${command_suffix}" ||
    ! install -m 0755 "${worker_artifact}" "${staging_dir}/ait-agent-worker${command_suffix}"; then
    rm -rf -- "${staging_dir}"
    return 2
  fi

  local command_name
  for command_name in ait ait-agent ait-agent-worker; do
    command_name="${command_name}${command_suffix}"
    if ! mv -f -- "${staging_dir}/${command_name}" "${bin_dir}/${command_name}"; then
      rm -rf -- "${staging_dir}"
      return 2
    fi
    printf 'Installed %s\n' "${bin_dir}/${command_name}"
  done
  rmdir "${staging_dir}"
}

cargo_target_size() {
  local target_dir="$1"
  if [[ -e "${target_dir}" ]]; then
    du -sh "${target_dir}" 2>/dev/null | awk '{print $1}'
  else
    printf '0B\n'
  fi
}

compact_one_cargo_target() {
  local target_dir="$1"
  local dry_run="$2"
  local force="$3"

  if [[ ! -d "${target_dir}" ]]; then
    printf 'Skipping missing Cargo target: %s\n' "${target_dir}"
    return 0
  fi
  if [[ "${dry_run}" != "1" && "${force}" != "1" ]] && \
    ! command -v lsof >/dev/null 2>&1; then
    printf 'Refusing to compact Cargo target without lsof active-use verification: %s\n' \
      "${target_dir}" >&2
    return 2
  fi
  if [[ "${dry_run}" != "1" && "${force}" != "1" ]] && \
    cargo_cache_in_use "${target_dir}"; then
    printf 'Refusing to compact active Cargo target: %s\n' "${target_dir}" >&2
    printf 'Stop processes using it, or pass --force if you have verified it is safe.\n' >&2
    return 2
  fi

  local before
  before="$(cargo_target_size "${target_dir}")"
  printf 'Compacting Cargo target: %s (before: %s)\n' "${target_dir}" "${before}"

  local candidate
  local candidates=(
    "${target_dir}/debug"
    "${target_dir}/release/incremental"
    "${target_dir}/release/deps"
    "${target_dir}/release/build"
    "${target_dir}/release/.fingerprint"
    "${target_dir}/ait-ci/incremental"
    "${target_dir}/ait-ci/deps"
    "${target_dir}/ait-ci/build"
    "${target_dir}/ait-ci/.fingerprint"
    "${target_dir}/tmp"
  )
  for candidate in "${candidates[@]}"; do
    if [[ ! -e "${candidate}" ]]; then
      continue
    fi
    if [[ "${dry_run}" == "1" ]]; then
      printf 'Would remove %s\n' "${candidate}"
    else
      rm -rf -- "${candidate}"
      printf 'Removed %s\n' "${candidate}"
    fi
  done

  if [[ "${dry_run}" == "1" ]]; then
    printf 'Dry run complete for %s\n' "${target_dir}"
  else
    local after
    after="$(cargo_target_size "${target_dir}")"
    printf 'Cargo target compacted: %s (after: %s)\n' "${target_dir}" "${after}"
  fi
}

compact_one_cargo_build_dir() {
  local build_dir="$1"
  local dry_run="$2"
  local force="$3"

  if [[ ! -d "${build_dir}" ]]; then
    printf 'Skipping missing Cargo build dir: %s\n' "${build_dir}"
    return 0
  fi
  if [[ "${dry_run}" != "1" && "${force}" != "1" ]] && \
    ! command -v lsof >/dev/null 2>&1; then
    printf 'Refusing to compact Cargo build dir without lsof active-use verification: %s\n' \
      "${build_dir}" >&2
    return 2
  fi
  if [[ "${dry_run}" != "1" && "${force}" != "1" ]] && \
    cargo_cache_in_use "${build_dir}"; then
    printf 'Refusing to compact active Cargo build dir: %s\n' "${build_dir}" >&2
    printf 'Stop processes using it, or pass --force if you have verified it is safe.\n' >&2
    return 2
  fi

  local before
  before="$(cargo_target_size "${build_dir}")"
  if [[ "${dry_run}" == "1" ]]; then
    printf 'Would remove idle Cargo intermediates from %s (current size: %s)\n' \
      "${build_dir}" "${before}"
    return 0
  fi
  if ! clear_cargo_build_contents "${build_dir}"; then
    printf 'Failed to compact every Cargo build-dir intermediate: %s\n' "${build_dir}" >&2
    return 2
  fi
  touch "${build_dir}/.ait-gc-marker"
  printf 'Cargo build dir compacted: %s (before: %s)\n' "${build_dir}" "${before}"
}

compact_cargo_targets() {
  local dry_run=0
  local force=0
  local include_worktrees=0
  local include_legacy=0
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --dry-run)
        dry_run=1
        ;;
      --force)
        force=1
        ;;
      --include-worktrees)
        include_worktrees=1
        ;;
      --include-legacy)
        include_legacy=1
        ;;
      --help|-h)
        cat <<'EOF'
Usage:
  ./ait.sh core compact [--dry-run] [--force] [--include-worktrees] [--include-legacy]

Remove Cargo intermediates from the configured build dir and legacy
intermediate paths from the target dir while leaving final release binaries in
place. With --include-worktrees it also scans managed Task cache leaves and
worktree-local targets. With --include-legacy it scans only the known old
ait-core Cargo target/build locations, including former two-level workspace
hash leaves.

Target selection follows the build/test path:
  1. AIT_SHARED_CARGO_TARGET_DIR, when set, explicitly opts into a shared target.
  2. CARGO_TARGET_DIR, when set, is honored for caller-managed targets.
  3. Otherwise a managed Task worktree uses its task-workspaces leaf beneath
     .ait/cargo-target, and the canonical checkout uses .ait/cargo-target.

Build-dir selection follows:
  1. AIT_SHARED_CARGO_BUILD_DIR, when set.
  2. CARGO_BUILD_BUILD_DIR, when set.
  3. Otherwise a managed Task leaf or the fixed canonical directory beneath
     .ait/cargo-build is used.
EOF
        return 0
        ;;
      *)
        printf 'Unknown compact option: %s\n' "$1" >&2
        return 2
        ;;
    esac
    shift
  done

  local targets=()
  targets+=("$(cargo_target_dir)")
  local build_dirs=()
  local configured_build_dir
  configured_build_dir="$(cargo_build_dir)"
  build_dirs+=("${configured_build_dir}")
  if [[ "${include_worktrees}" == "1" ]]; then
    local repository_build_root
    repository_build_root="$(repository_cargo_build_root)"
    local task_build_dir
    while IFS= read -r task_build_dir; do
      build_dirs+=("${task_build_dir}")
    done < <(repository_task_cargo_build_leaves "${repository_build_root}")
    local repository_target_root
    repository_target_root="$(repository_cargo_target_root)"
    local task_target_dir
    while IFS= read -r task_target_dir; do
      targets+=("${task_target_dir}")
    done < <(repository_task_cargo_target_leaves "${repository_target_root}")
    local worktree_target
    for worktree_target in \
      "${ROOT_DIR}"/.ait-worktree-links/*/rust/target; do
      [[ -d "${worktree_target}" && ! -L "${worktree_target}" ]] || continue
      targets+=("${worktree_target}")
    done
    local worktree_build
    for worktree_build in \
      "${ROOT_DIR}"/.ait-worktree-links/*/.ait/cargo-build/workspaces/*/* \
      "${ROOT_DIR}"/.ait-worktree-links/*/.ait/cargo-build/task-workspaces/*; do
      [[ -d "${worktree_build}" && ! -L "${worktree_build}" ]] || continue
      build_dirs+=("${worktree_build}")
    done
  fi
  if [[ "${include_legacy}" == "1" ]]; then
    local repository_build_root
    repository_build_root="$(repository_cargo_build_root)"
    local workspace_hash_build_dir
    while IFS= read -r workspace_hash_build_dir; do
      build_dirs+=("${workspace_hash_build_dir}")
    done < <(repository_workspace_cargo_build_leaves "${repository_build_root}")
    local legacy_target
    for legacy_target in \
      "${ROOT_DIR}"/rust/target \
      "${ROOT_DIR}"/target \
      "${ROOT_DIR}"/.ait-runtime/*-cargo-target \
      "${ROOT_DIR}"/.ait/generated/runner/cargo-target; do
      [[ -d "${legacy_target}" && ! -L "${legacy_target}" ]] || continue
      targets+=("${legacy_target}")
    done
    local legacy_build_dir
    for legacy_build_dir in \
      "${ROOT_DIR}"/.ait/cargo-build-rct-* \
      "${ROOT_DIR}"/.ait-runtime/*-cargo-build \
      "${ROOT_DIR}"/.ait/generated/runner/cargo-build; do
      [[ -d "${legacy_build_dir}" && ! -L "${legacy_build_dir}" ]] || continue
      build_dirs+=("${legacy_build_dir}")
    done
  fi

  local seen="|"
  local target_dir
  local resolved
  local failed=0
  for target_dir in "${targets[@]}"; do
    if [[ -d "${target_dir}" ]]; then
      resolved="$(cd "${target_dir}" && pwd -P)"
    else
      resolved="${target_dir}"
    fi
    case "${seen}" in
      *"|${resolved}|"*)
        continue
        ;;
    esac
    seen="${seen}${resolved}|"
    if ! compact_one_cargo_target "${resolved}" "${dry_run}" "${force}"; then
      failed=1
    fi
  done
  local build_dir
  for build_dir in "${build_dirs[@]}"; do
    if [[ -d "${build_dir}" ]]; then
      resolved="$(cd "${build_dir}" && pwd -P)"
    else
      resolved="${build_dir}"
    fi
    case "${seen}" in
      *"|${resolved}|"*)
        continue
        ;;
    esac
    seen="${seen}${resolved}|"
    if ! compact_one_cargo_build_dir "${resolved}" "${dry_run}" "${force}"; then
      failed=1
    fi
  done
  return "${failed}"
}

usage() {
  cat <<'EOF'
Usage:
  ./ait.sh core build    # release profile
  ./ait.sh core install [--bin-dir <path>] [--skip-build]
  ./ait.sh core compact [--dry-run] [--force] [--include-worktrees] [--include-legacy]
  ./ait.sh core test     # lean ait-ci profile

`ait-core` is Rust-only and owns its native build. Python packaging and
compatibility glue belong to `../ait-python`; `../ait` is transitional.
AIT_CORE_BUILD_PROFILE may be set to release; debug/dev profiles are forbidden.
AIT-owned tests use the non-debug ait-ci profile.
Set AIT_SHARED_CARGO_TARGET_DIR to opt into a shared Cargo target. Otherwise
CARGO_TARGET_DIR is honored when set, managed Task worktrees use their
task-workspaces leaf beneath .ait/cargo-target, and the canonical checkout uses
.ait/cargo-target.
Set AIT_SHARED_CARGO_BUILD_DIR to opt into shared intermediates. Otherwise
CARGO_BUILD_BUILD_DIR is honored, managed Task worktrees use their dedicated
task-workspaces leaf, and the canonical checkout reuses the fixed
.ait/cargo-build/canonical directory. The repository pool is bounded to 4 GiB
and the canonical cache to 1 GiB by default. Set AIT_CARGO_BUILD_MAX_BYTES=0 to
opt out or set one explicit byte limit. Caller-managed external build dirs
remain unbounded unless that variable is set. Repository-pool reclamation is
rate-limited by AIT_CARGO_BUILD_GC_INTERVAL_SECONDS (default 3600 seconds);
the canonical bound is checked after every launcher-owned Cargo invocation.
EOF
}

main() {
  if [[ "${1:-}" != "core" ]]; then
    usage >&2
    return 1
  fi

  case "${2:-}" in
    build)
      require_canonical_cargo_source_policy
      local profile
      profile="$(core_build_profile)"
      run_cargo build --profile "${profile}" --manifest-path "${ROOT_DIR}/rust/Cargo.toml" --workspace
      refresh_build_artifact_mtimes
      ;;
    install)
      shift 2
      install_native_core_commands "$@"
      ;;
    compact)
      shift 2
      compact_cargo_targets "$@"
      ;;
    test)
      require_canonical_cargo_source_policy
      run_cargo test --manifest-path "${ROOT_DIR}/rust/Cargo.toml" --workspace --profile ait-ci
      "${ROOT_DIR}/tests/test_ait_sh_core_compact.sh"
      "${ROOT_DIR}/tests/test_ait_sh_core_cargo_policy.sh"
      ;;
    *)
      usage >&2
      return 1
      ;;
  esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
