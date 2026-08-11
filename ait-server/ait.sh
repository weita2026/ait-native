#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PATH="/Users/weita/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"

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

cargo_target_dir() {
  if [[ -n "${AIT_SHARED_CARGO_TARGET_DIR:-}" ]]; then
    printf '%s\n' "${AIT_SHARED_CARGO_TARGET_DIR}"
    return 0
  fi
  printf '%s\n' "${CARGO_TARGET_DIR:-${ROOT_DIR}/.ait/cargo-target}"
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
  printf '%s\n' "${ROOT_DIR}/.ait/cargo-build/workspaces/{workspace-path-hash}"
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

auto_reclaim_cargo_build_dir() {
  local build_dir="$1"
  local max_bytes="${AIT_CARGO_BUILD_MAX_BYTES:-0}"
  local interval_seconds="${AIT_CARGO_BUILD_GC_INTERVAL_SECONDS:-3600}"
  case "${max_bytes}:${interval_seconds}" in
    *[!0-9:]*|:*)
      printf 'Skipping Cargo build-dir GC because its numeric configuration is invalid.\n' >&2
      return 0
      ;;
  esac
  if [[ "${max_bytes}" == "0" ]]; then
    return 0
  fi
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
  local target_dir
  local build_dir
  target_dir="$(cargo_target_dir)"
  build_dir="$(cargo_build_dir)"
  ensure_distinct_cargo_dirs "${target_dir}" "${build_dir}"
  export CARGO_TARGET_DIR="${target_dir}"
  export CARGO_BUILD_BUILD_DIR="${build_dir}"
  export CARGO_INCREMENTAL=0
  local cargo_status=0
  "${cargo_bin}" "$@" || cargo_status=$?
  auto_reclaim_cargo_build_dir "${build_dir}"
  return "${cargo_status}"
}

server_usage() {
  cat <<'EOF'
Usage:
  ./ait.sh server init
  ./ait.sh server probe
  ./ait.sh server run
  ./ait.sh server start
  ./ait.sh server status

Required:
  AIT_NATIVE_SERVER_DATA=/durable/path/to/server-data

Optional:
  AITSERVER_LISTEN=127.0.0.1:8088
  AIT_NATIVE_SERVER_CI_TMP_ROOT=/fast/tmp/root
  AIT_NATIVE_SERVER_RAM_SHARD_ROOT=/fast/shard/root
  AIT_SERVER_FULL_TEST_JOB_CPU_TOKENS=8
  AIT_SERVER_LAUNCHER=foreground|screen
  AIT_SERVER_BIN=/path/to/release/ait-server
  AIT_SERVER_BUILD=0

`server init` safely creates a fresh Binary authority only when the configured
runtime root is missing or empty. It is idempotent for an existing activation.

`server probe` validates the current process context against
AIT_NATIVE_SERVER_DATA before the server binds a socket. It checks read,
write, readback, cleanup, and existing object-pack content access.

`server start` defaults to foreground. Set AIT_SERVER_LAUNCHER=screen for a
local user-session background process. Production deployments should normally
use their own service manager or container runtime with the same environment.
EOF
}

require_server_runtime_root() {
  if [[ -z "${AIT_NATIVE_SERVER_DATA:-}" ]]; then
    cat >&2 <<'EOF'
AIT_NATIVE_SERVER_DATA is required.

Set it to a durable server-data directory that the selected process context can
read and write. Example:

  export AIT_NATIVE_SERVER_DATA=/path/to/durable/server-data

Do not assume an interactive shell and a daemon/service manager have the same
permissions; run `./ait.sh server probe` under the same launcher context before
serving requests.
EOF
    return 2
  fi
}

prepare_server_runtime_env() {
  require_server_runtime_root
  export AIT_LOG_DIR="${AIT_LOG_DIR:-${AIT_NATIVE_SERVER_DATA}/logs}"
  export AIT_NATIVE_SERVER_CI_TMP_ROOT="${AIT_NATIVE_SERVER_CI_TMP_ROOT:-${AIT_NATIVE_SERVER_DATA}/tmp}"
  export AIT_PATCHSET_CI_TMPDIR="${AIT_PATCHSET_CI_TMPDIR:-${AIT_NATIVE_SERVER_CI_TMP_ROOT}/patchset-ci}"
  export AIT_REPO_CI_TMPDIR="${AIT_REPO_CI_TMPDIR:-${AIT_NATIVE_SERVER_CI_TMP_ROOT}/repo-ci}"
  export AIT_LAND_MAIN_SEED_TMPDIR="${AIT_LAND_MAIN_SEED_TMPDIR:-${AIT_NATIVE_SERVER_CI_TMP_ROOT}/land-main-seed}"
}

server_binary_path() {
  if [[ -n "${AIT_SERVER_BIN:-}" ]]; then
    printf '%s\n' "${AIT_SERVER_BIN}"
    return 0
  fi
  printf '%s\n' "$(cargo_target_dir)/release/ait-server"
}

reject_debug_server_binary() {
  local server_bin="$1"
  case "${server_bin}" in
    */[Dd][Ee][Bb][Uu][Gg]/*)
      printf 'ait-server debug-profile binaries are forbidden: %s\n' "${server_bin}" >&2
      printf 'Build or select a release binary instead.\n' >&2
      return 2
      ;;
  esac
}

ensure_server_binary() {
  if [[ "${AIT_SERVER_BUILD:-1}" != "0" ]]; then
    run_cargo build --manifest-path "${ROOT_DIR}/rust/Cargo.toml" -p ait-server --release
  fi
  local server_bin
  server_bin="$(server_binary_path)"
  reject_debug_server_binary "${server_bin}"
  if [[ ! -x "${server_bin}" ]]; then
    printf 'ait-server binary is not executable: %s\n' "${server_bin}" >&2
    printf 'Run ./ait.sh core build or set AIT_SERVER_BIN to a release binary.\n' >&2
    return 2
  fi
}

server_init() {
  prepare_server_runtime_env
  ensure_server_binary
  local server_bin
  server_bin="$(server_binary_path)"
  reject_debug_server_binary "${server_bin}"
  "${server_bin}" init
  mkdir -p \
    "${AIT_LOG_DIR}" \
    "${AIT_NATIVE_SERVER_CI_TMP_ROOT}" \
    "${AIT_PATCHSET_CI_TMPDIR}" \
    "${AIT_REPO_CI_TMPDIR}" \
    "${AIT_LAND_MAIN_SEED_TMPDIR}"
}

server_probe() {
  prepare_server_runtime_env
  ensure_server_binary
  local server_bin
  server_bin="$(server_binary_path)"
  reject_debug_server_binary "${server_bin}"
  "${server_bin}" probe
}

server_run() {
  server_init
  local server_bin
  server_bin="$(server_binary_path)"
  reject_debug_server_binary "${server_bin}"
  exec "${server_bin}" run
}

server_start() {
  server_init
  local server_bin
  server_bin="$(server_binary_path)"
  reject_debug_server_binary "${server_bin}"

  local launcher
  launcher="${AIT_SERVER_LAUNCHER:-foreground}"
  case "${launcher}" in
    foreground)
      exec "${server_bin}" run
      ;;
    screen)
      if ! command -v screen >/dev/null 2>&1; then
        printf 'AIT_SERVER_LAUNCHER=screen requested but screen is unavailable.\n' >&2
        return 2
      fi
      local session_name
      local log_file
      session_name="${AIT_SERVER_SCREEN_NAME:-ait-server}"
      log_file="${AIT_SERVER_LOG_FILE:-${AIT_LOG_DIR}/ait-server.log}"
      screen -dmS "${session_name}" /bin/sh -c 'exec "$1" run >> "$2" 2>&1' sh "${server_bin}" "${log_file}"
      printf 'ait-server started with screen session %s\n' "${session_name}"
      printf 'binary: %s\n' "${server_bin}"
      printf 'log: %s\n' "${log_file}"
      ;;
    *)
      printf 'Unsupported AIT_SERVER_LAUNCHER: %s\n' "${launcher}" >&2
      printf 'Supported launchers: foreground, screen\n' >&2
      return 2
      ;;
  esac
}

server_status() {
  local listen
  listen="${AITSERVER_LISTEN:-127.0.0.1:8088}"
  printf 'AITSERVER_LISTEN=%s\n' "${listen}"
  if command -v lsof >/dev/null 2>&1; then
    local port
    port="${listen##*:}"
    lsof -nP -iTCP:"${port}" -sTCP:LISTEN || true
  fi
  if command -v screen >/dev/null 2>&1; then
    screen -ls 2>/dev/null | sed -n '1,120p' || true
  fi
}

server_command() {
  case "${1:-}" in
    init)
      shift
      server_init "$@"
      ;;
    probe)
      server_probe
      ;;
    run)
      server_run
      ;;
    start)
      server_start
      ;;
    status)
      server_status
      ;;
    --help|-h|"")
      server_usage
      ;;
    *)
      printf 'Unknown server command: %s\n' "$1" >&2
      server_usage >&2
      return 2
      ;;
  esac
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
  if [[ "${force}" != "1" ]] && ! command -v lsof >/dev/null 2>&1; then
    printf 'Refusing to compact Cargo target without lsof active-use verification: %s\n' \
      "${target_dir}" >&2
    return 2
  fi
  if [[ "${force}" != "1" ]] && cargo_cache_in_use "${target_dir}"; then
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
  if [[ "${force}" != "1" ]] && ! command -v lsof >/dev/null 2>&1; then
    printf 'Refusing to compact Cargo build dir without lsof active-use verification: %s\n' \
      "${build_dir}" >&2
    return 2
  fi
  if [[ "${force}" != "1" ]] && cargo_cache_in_use "${build_dir}"; then
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
      --help|-h)
        cat <<'EOF'
Usage:
  ./ait.sh core compact [--dry-run] [--force] [--include-worktrees]

Remove Cargo intermediates from the configured build dir and legacy
intermediate paths from the target dir while leaving final release binaries in
place. With --include-worktrees it also scans managed task worktree caches.

Target selection follows the build/test path:
  1. AIT_SHARED_CARGO_TARGET_DIR, when set, explicitly opts into a shared target.
  2. CARGO_TARGET_DIR, when set, is honored for caller-managed targets.
  3. Otherwise .ait/cargo-target is used.

Build-dir selection follows:
  1. AIT_SHARED_CARGO_BUILD_DIR, when set.
  2. CARGO_BUILD_BUILD_DIR, when set.
  3. Otherwise a workspace-isolated leaf beneath .ait/cargo-build is used.
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
  build_dirs+=("$(cargo_build_dir)")
  if [[ "${include_worktrees}" == "1" ]]; then
    local worktree_target
    for worktree_target in "${ROOT_DIR}"/.ait-worktree-links/*/.ait/cargo-target; do
      [[ -d "${worktree_target}" ]] || continue
      targets+=("${worktree_target}")
    done
    local worktree_build
    for worktree_build in "${ROOT_DIR}"/.ait-worktree-links/*/rust/target \
      "${ROOT_DIR}"/.ait-worktree-links/*/.ait/cargo-build/workspaces/*/*; do
      [[ -d "${worktree_build}" ]] || continue
      build_dirs+=("${worktree_build}")
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
  ./ait.sh core build
  ./ait.sh core compact [--dry-run] [--force] [--include-worktrees]
  ./ait.sh core test
  ./ait.sh server init
  ./ait.sh server probe
  ./ait.sh server run
  ./ait.sh server start
  ./ait.sh server status

`ait-server` is AGPL server-side Rust. The migrated core crate is still named
`ait-server-core` for the first repository cut.
Set AIT_SHARED_CARGO_TARGET_DIR to opt into a shared Cargo target. Otherwise
CARGO_TARGET_DIR is honored when set, then .ait/cargo-target is used.
Set AIT_SHARED_CARGO_BUILD_DIR to opt into shared intermediates. Otherwise
CARGO_BUILD_BUILD_DIR is honored, then Cargo expands a workspace-isolated leaf
beneath .ait/cargo-build. Automatic build-dir reclamation is disabled by default; set a nonzero
AIT_CARGO_BUILD_MAX_BYTES to opt in. Reclamation is rate-limited by
AIT_CARGO_BUILD_GC_INTERVAL_SECONDS (default 3600).
EOF
}

case "${1:-}" in
  core)
    case "${2:-}" in
      build)
        run_cargo build --manifest-path "${ROOT_DIR}/rust/Cargo.toml" --workspace --release
        ;;
      compact)
        shift 2
        compact_cargo_targets "$@"
        ;;
      test)
        run_cargo test --manifest-path "${ROOT_DIR}/rust/Cargo.toml" --workspace --profile ait-ci
        ;;
      *)
        usage >&2
        exit 1
        ;;
    esac
    ;;
  server)
    shift
    server_command "$@"
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
