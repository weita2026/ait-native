#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PATH="${CARGO_HOME:-${HOME}/.cargo}/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"
DEFAULT_REPOSITORY_CARGO_BUILD_MAX_BYTES=4294967296
CANONICAL_CARGO_BUILD_DIRNAME="canonical"
MANAGED_WORKTREE_CARGO_TARGET_DIRNAME="task-workspaces"

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
  local primary shared ram available floor reason=""
  primary="$(repository_cargo_registered_build_root)" || return
  shared="$(repository_ait_dir)/cargo-build"
  ram="$(cargo_memory_root)" || return
  if [[ ! -d "$shared" ]]; then
    reason="registered RAM build root is unavailable"
  elif [[ ! -L "$shared" && ( -z "$ram" || ( "$ram" != / && "$primary" != "$ram" && "$primary" != "$ram/"* ) ) ]]; then
    printf '%s\n' "$primary"
    return
  elif [[ -n "$ram" && ! -d "$ram" ]]; then
    reason="configured RAM mount is unavailable"
  elif [[ -n "$ram" ]] && ! cargo_memory_mount_available "$ram"; then
    reason="configured RAM mount is unavailable"
  else
    floor="$(cargo_ram_headroom)"
    available="$(cargo_available_bytes "$primary")" || available=""
    if [[ -z "$available" ]]; then
      reason="RAM filesystem capacity is unavailable"
    elif ! awk -v free="$available" -v floor="$floor" 'BEGIN {exit !(free>=floor)}'; then
      reason="available RAM storage is below required build headroom"
    fi
  fi
  if [[ -n "$reason" ]]; then
    local fallback
    fallback="$(repository_cargo_ssd_build_root)" || return
    printf 'Cargo RAM-to-SSD fallback: %s; %s\n' "$reason" "$fallback" >&2
    printf '%s\n' "$fallback"
    return
  fi
  printf '%s\n' "$primary"
}

repository_ait_dir() {
  local path="$ROOT_DIR/.ait"
  mkdir -p "$path"
  (cd "$path" && pwd -P)
}


cargo_json_string() {
  local config
  config="$1"
  [[ -f "$config" ]] || return 0
  LC_ALL=C awk -v wanted="$2" '
    function bad() { failed=1; exit 2 }
    function ws() { while (substr(s,p,1) ~ /[ \t\r\n]/ && p<=length(s)) p++ }
    function hex4(  t,n,i,c) {
      t=substr(s,p,4); if (length(t)!=4 || t ~ /[^0-9a-fA-F]/) bad()
      p+=4; n=0
      for(i=1;i<=4;i++) { c=index("0123456789abcdef",tolower(substr(t,i,1)))-1; n=n*16+c }
      return n
    }
    function utf8(n) {
      if(n<1 || n>1114111 || (n>=55296 && n<=57343)) bad()
      if(n<128) return sprintf("%c",n)
      if(n<2048) return sprintf("%c%c",192+int(n/64),128+n%64)
      if(n<65536) return sprintf("%c%c%c",224+int(n/4096),128+int(n/64)%64,128+n%64)
      return sprintf("%c%c%c%c",240+int(n/262144),128+int(n/4096)%64,128+int(n/64)%64,128+n%64)
    }
    function string(  out,c,n,low) {
      if(substr(s,p++,1)!="\"") bad()
      out=""
      while(p<=length(s)) {
        c=substr(s,p++,1)
        if(c=="\"") return out
        if(c ~ /[[:cntrl:]]/) bad()
        if(c!="\\") { out=out c; continue }
        c=substr(s,p++,1)
        if(c=="u") {
          n=hex4()
          if(n>=55296 && n<=56319) {
            if(substr(s,p,2)!="\\u") bad()
            p+=2; low=hex4()
            if(low<56320 || low>57343) bad()
            n=65536+(n-55296)*1024+low-56320
          }
          out=out utf8(n)
        } else if(c=="\"" || c=="\\" || c=="/") out=out c
        else if(c=="n") out=out "\n"
        else if(c=="r") out=out "\r"
        else if(c=="t") out=out "\t"
        else if(c=="b") out=out sprintf("%c",8)
        else if(c=="f") out=out sprintf("%c",12)
        else bad()
      }
      bad()
    }
    function value(depth,  c,key,text,start,selected) {
      if(depth>64) bad()
      ws(); c=substr(s,p,1)
      selected=(depth==3 && names[1]=="task_worktree" && names[2]=="memory_root" && ((wanted=="memory" && names[3]=="root") || (wanted=="memory_kind" && names[3]=="kind"))) || (wanted!="memory" && wanted!="memory_kind" && depth==1 && names[1]==wanted)
      if(selected) result=""
      if(c=="{") {
        p++; ws(); if(substr(s,p,1)=="}") { p++; return }
        while(1) {
          ws(); key=string(); names[depth+1]=key
          if((wanted=="memory" || wanted=="memory_kind") && ((depth==0 && key=="task_worktree") || (depth==1 && names[1]=="task_worktree" && key=="memory_root"))) result=""
          ws(); if(substr(s,p++,1)!=":") bad()
          value(depth+1); delete names[depth+1]; ws(); c=substr(s,p++,1)
          if(c=="}") return
          if(c!=",") bad()
        }
      }
      if(c=="[") {
        p++; ws(); if(substr(s,p,1)=="]") { p++; return }
        while(1) { value(depth+1); ws(); c=substr(s,p++,1); if(c=="]") return; if(c!=",") bad() }
      }
      if(c=="\"") { text=string(); if(selected) result=text; return }
      start=p
      while(p<=length(s) && substr(s,p,1) !~ /[ \t\r\n,}\]]/) p++
      text=substr(s,start,p-start)
      if(text !~ /^(true|false|null|-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?)$/) bad()
    }
    { s=s $0 "\n" }
    END {
      if(failed) exit 2
      p=1; value(0); ws(); if(p<=length(s)) bad()
      sub(/^[ \t\r\n]+/,"",result); sub(/[ \t\r\n]+$/,"",result)
      printf "%s",result
    }
  ' "$config"
}


cargo_config_string() {
  local key="$1"
  [[ "$key" != repo ]] || key=repo_name
  cargo_json_string "$(repository_ait_dir)/config.json" "$key"
}

physical_cargo_path_without_creation() {
  local path="$1" suffix="" parent
  while [[ ! -d "$path" ]]; do
    parent="$(dirname "$path")"
    [[ "$parent" != "$path" ]] || return 1
    suffix="/$(basename "$path")$suffix"
    path="$parent"
  done
  printf '%s%s\n' "$(cd "$path" && pwd -P)" "$suffix" | awk -F/ '
    { for(i=1;i<=NF;i++) { if($i=="" || $i==".") continue; if($i=="..") { if(n>0) n-- } else parts[++n]=$i }
      if(n==0) { print "/"; next }
      for(i=1;i<=n;i++) printf "/%s",parts[i]; print ""
    }'
}


cargo_memory_root() {
  local path
  path="$(cargo_config_string memory)" || {
    printf 'Cannot read configured Cargo RAM root safely.\n' >&2
    return 2
  }
  [[ -n "$path" ]] || return 0
  case "$path" in
    "~") path="$HOME" ;;
    \~/*) path="$HOME/${path#\~/}" ;;
    /*) ;;
    *) path="$(dirname "$(repository_ait_dir)")/$path" ;;
  esac
  physical_cargo_path_without_creation "$path"
}


cargo_ram_headroom() {
  awk -v configured="${AIT_CARGO_BUILD_MAX_BYTES:-}" -v floor="$DEFAULT_REPOSITORY_CARGO_BUILD_MAX_BYTES" \
    'BEGIN { if(configured !~ /^[0-9]+$/ || configured+0<floor) configured=floor; printf "%.0f\n",configured }'
}

cargo_memory_mount_available() {
  local root="$1" kind mounted
  kind="$(cargo_config_string memory_kind)" || return 1
  case "$kind" in
    macos_ram_volume)
      if [[ "$root" == /Volumes/* ]]; then
        mounted="$(LC_ALL=C df -Pk "$root" 2>/dev/null | awk 'NR==2 {line=$0; for(i=0;i<5;i++) sub(/^[^[:space:]]+[[:space:]]+/,"",line); print line}')" || return 1
        [[ "$mounted" == "$root" ]] || return 1
      fi ;;
    linux_*)
      mounted="$(LC_ALL=C df -PT "$root" 2>/dev/null | awk 'NR==2 {print $2}')" || return 1
      [[ "$mounted" == tmpfs || "$mounted" == ramfs ]] || return 1 ;;
  esac
}


cargo_available_bytes() {
  local probe="$1"
  while [[ ! -d "$probe" ]]; do
    [[ "$probe" != / ]] || return 1
    probe="$(dirname "$probe")"
  done
  LC_ALL=C df -Pk "$probe" 2>/dev/null | awk 'NR==2 && $4~/^[0-9]+$/ {printf "%.0f\n",$4*1024}'
}


repository_cargo_ssd_build_root() {
  local path ram resolved child
  path="$(repository_ait_dir)/cargo-build-ssd"
  if [[ -L "$path" || ( -e "$path" && ! -d "$path" ) ]]; then
    printf 'Cargo SSD fallback must be a physical repository cache directory: %s\n' "$path" >&2
    return 2
  fi
  ram="$(cargo_memory_root)" || return
  resolved="$(physical_cargo_path_without_creation "$path")"
  if [[ -n "$ram" && ( "$ram" == / || "$resolved" == "$ram" || "$resolved" == "$ram/"* ) ]]; then
    printf 'Cargo SSD fallback is inside the configured RAM root: %s\n' "$resolved" >&2
    return 2
  fi
  for child in "$path/canonical" "$path/task-workspaces"; do
    if [[ -L "$child" || ( -e "$child" && ! -d "$child" ) ]]; then
      printf 'Cargo SSD cache namespace is not a physical directory: %s\n' "$child" >&2
      return 2
    fi
  done
  printf '%s\n' "$resolved"
}


repository_cargo_registered_build_root() {
  local shared
  shared="$(repository_ait_dir)/cargo-build"
  if [[ -d "$shared" ]]; then
    (cd "$shared" && pwd -P)
  else
    printf '%s\n' "$shared"
  fi
}


repository_cargo_build_leaf() {
  local root="$1" name path parent
  name="$(managed_worktree_name || true)"
  if [[ -n "$name" ]]; then
    parent="$root/task-workspaces"
    path="$parent/$name"
  else
    parent="$root"
    path="$root/$CANONICAL_CARGO_BUILD_DIRNAME"
  fi
  if [[ -L "$parent" || -L "$path" || ( -e "$path" && ! -d "$path" ) ]]; then
    printf 'Managed Cargo build leaf is not a physical directory: %s\n' "$path" >&2
    return 2
  fi
  printf '%s\n' "$path"
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
    return
  fi
  if [[ -n "${CARGO_BUILD_BUILD_DIR:-}" ]]; then
    printf '%s\n' "${CARGO_BUILD_BUILD_DIR}"
    return
  fi
  local build_root
  build_root="$(repository_cargo_build_root)" || return
  repository_cargo_build_leaf "$build_root"
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

invoke_cargo_once() {
  local space_error
  cargo_status=0
  # Preserve both output streams and keep only a bounded in-memory error marker.
  if space_error="$("$cargo_bin" "$@" 2>&1 >&3 | tee /dev/fd/4 | LC_ALL=C awk '
    {
      line=tolower($0)
      diagnostic=(line ~ /(error:|fatal:|failed to (write|create|build archive)|ar:|ld:|rustc:)/)
      # Keep the established Server ENOSPC write form, with complete tokens.
      space=(line ~ /(^|[^[:alnum:]_])(no space left on device|os error 28)([^[:alnum:]_]|$)/ ||
        (line ~ /(^|[^[:alnum:]_])enospc([^[:alnum:]_]|$)/ && line ~ /(errno|i\/o|io error|failed to (write|create|build archive)|write.*failed|filesystem|file system)/))
      darwin_space=(line ~ /(^|[[:space:]])(cargo:warning=)?write64\(\)[[:space:]]+failed,[[:space:]]+returned[[:space:]]+-1[[:space:]]+expected[[:space:]]+[0-9]+,[[:space:]]+errno=28([^[:alnum:]_]|$)/)
      if((diagnostic && space) || darwin_space) found=1
    }
    END {print found ? 1 : 0}
  ')"; then
    cargo_status=0
  else
    cargo_status=$?
  fi
  cargo_space_failure=0
  if [[ "$cargo_status" != 0 && "$space_error" == 1 ]]; then
    cargo_space_failure=1
  fi
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
  local automatic_build=0 managed_ram=0 ram
  if [[ -z "${AIT_SHARED_CARGO_BUILD_DIR:-}" && -z "${CARGO_BUILD_BUILD_DIR:-}" ]]; then
    automatic_build=1
  fi
  target_dir="$(cargo_target_dir)" || return
  build_dir="$(cargo_build_dir)" || return
  ensure_distinct_cargo_dirs "$target_dir" "$build_dir" || return
  if [[ "$automatic_build" == 1 ]]; then
    ram="$(cargo_memory_root)" || return
    if [[ -n "$ram" && ( "$ram" == / || "$build_dir" == "$ram/"* ) ]]; then
      managed_ram=1
    elif [[ -L "$ROOT_DIR/.ait/cargo-build" && "$build_dir" != "$(repository_ait_dir)/cargo-build-ssd/"* ]]; then
      managed_ram=1
    fi
  fi
  export CARGO_TARGET_DIR="${target_dir}"
  export CARGO_BUILD_BUILD_DIR="${build_dir}"
  export CARGO_INCREMENTAL=0
  local cargo_status=0
  local cargo_space_failure=0
  invoke_cargo_once "$@" 3>&1 4>&2
  if [[ "$cargo_status" != 0 && "$cargo_space_failure" == 1 && "$managed_ram" == 1 ]]; then
    local fallback
    fallback="$(repository_cargo_ssd_build_root)" || return
    build_dir="$(repository_cargo_build_leaf "$fallback")" || return
    ensure_distinct_cargo_dirs "$target_dir" "$build_dir" || return
    printf 'Cargo RAM-to-SSD fallback: Cargo reported filesystem space exhaustion; rerunning this invocation once in %s\n' "$build_dir" >&2
    export CARGO_BUILD_BUILD_DIR="$build_dir"
    invoke_cargo_once "$@" 3>&1 4>&2
  fi
  auto_reclaim_cargo_build_dir "$build_dir"
  return "${cargo_status}"
}

server_usage() {
  cat <<'EOF'
Usage:
  ./ait.sh server init
  ./ait.sh server probe [--defer-ci-admission]
  ./ait.sh server run [--listen <ip:port>] [--defer-ci-admission]
  ./ait.sh server start [--listen <ip:port>] [--defer-ci-admission]
  ./ait.sh server status [--listen <ip:port>]

Required:
  AIT_NATIVE_SERVER_DATA=/durable/path/to/server-data

Optional:
  AIT_NATIVE_SERVER_CI_RAM_ROOT=/memory-backed/runtime/root
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
  "${server_bin}" init --data "${AIT_NATIVE_SERVER_DATA}"
  mkdir -p "${AIT_LOG_DIR}"
}

server_probe() {
  prepare_server_runtime_env
  ensure_server_binary
  local server_bin
  server_bin="$(server_binary_path)"
  reject_debug_server_binary "${server_bin}"
  "${server_bin}" probe --data "${AIT_NATIVE_SERVER_DATA}" "$@"
}

server_run() {
  server_init
  local server_bin
  server_bin="$(server_binary_path)"
  reject_debug_server_binary "${server_bin}"
  exec "${server_bin}" --data "${AIT_NATIVE_SERVER_DATA}" "$@"
}

server_start() {
  server_init
  local server_bin
  server_bin="$(server_binary_path)"
  reject_debug_server_binary "${server_bin}"

  local listen
  local listen_set
  local defer_ci_admission
  listen="127.0.0.1:8088"
  listen_set=0
  defer_ci_admission=0
  while (( $# > 0 )); do
    case "$1" in
      --listen)
        if [[ -z "${2:-}" ]]; then
          printf '%s\n' '--listen requires an IP address and port.' >&2
          return 2
        fi
        if [[ "${listen_set}" == "1" ]]; then
          printf '%s\n' '--listen may be supplied only once.' >&2
          return 2
        fi
        listen="$2"
        listen_set=1
        shift 2
        ;;
      --defer-ci-admission)
        defer_ci_admission=1
        shift
        ;;
      *)
        printf 'Unknown server start argument: %s\n' "$1" >&2
        return 2
        ;;
    esac
  done

  local launcher
  launcher="${AIT_SERVER_LAUNCHER:-foreground}"
  case "${launcher}" in
    foreground)
      if [[ "${defer_ci_admission}" == "1" ]]; then
        exec "${server_bin}" --data "${AIT_NATIVE_SERVER_DATA}" --listen "${listen}" --defer-ci-admission
      fi
      exec "${server_bin}" --data "${AIT_NATIVE_SERVER_DATA}" --listen "${listen}"
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
      screen -dmS "${session_name}" /bin/sh -c 'if [ "$4" = "1" ]; then exec "$1" --data "$2" --listen "$3" --defer-ci-admission >> "$5" 2>&1; fi; exec "$1" --data "$2" --listen "$3" >> "$5" 2>&1' sh "${server_bin}" "${AIT_NATIVE_SERVER_DATA}" "${listen}" "${defer_ci_admission}" "${log_file}"
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
  listen="127.0.0.1:8088"
  if [[ "${1:-}" == "--listen" ]]; then
    if [[ -z "${2:-}" ]]; then
      printf '%s\n' '--listen requires an IP address and port.' >&2
      return 2
    fi
    listen="$2"
  elif [[ -n "${1:-}" ]]; then
    printf 'Unknown server status argument: %s\n' "$1" >&2
    return 2
  fi
  printf 'listen=%s\n' "${listen}"
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
      shift
      server_probe "$@"
      ;;
    run)
      shift
      server_run "$@"
      ;;
    start)
      shift
      server_start "$@"
      ;;
    status)
      shift
      server_status "$@"
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

owned_fallback_cargo_build_dirs() {
  local include_worktrees="$1" fallback name current metadata owner recorded canonical
  fallback="$(repository_cargo_ssd_build_root)" || return
  current="$(managed_worktree_name || true)"
  canonical="$(dirname "$(repository_ait_dir)")"
  if [[ -z "$current" || "$include_worktrees" == 1 ]]; then
    [[ ! -d "$fallback/canonical" ]] || printf '%s\n' "$fallback/canonical"
  fi
  if [[ -n "$current" && -d "$fallback/task-workspaces/$current" && ! -L "$fallback/task-workspaces/$current" ]]; then
    printf '%s\n' "$fallback/task-workspaces/$current"
  fi
  [[ "$include_worktrees" == 1 ]] || return 0
  local leaf
  for leaf in "$fallback"/task-workspaces/*; do
    [[ -d "$leaf" && ! -L "$leaf" ]] || continue
    name="$(basename "$leaf")"
    case "$name" in ''|.|..|*[!a-zA-Z0-9._-]*) continue ;; esac
    metadata="$(repository_ait_dir)/worktrees/$name.json"
    [[ -f "$metadata" && ! -L "$metadata" ]] || continue
    recorded="$(cargo_json_string "$metadata" name)" || continue
    owner="$(cargo_json_string "$metadata" repo_root)" || continue
    [[ "$recorded" == "$name" && -n "$owner" ]] || continue
    [[ "$(physical_cargo_path_without_creation "$owner")" == "$canonical" ]] || continue
    printf '%s\n' "$leaf"
  done
  return 0
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
place. With --include-worktrees it also scans managed Task target/build leaves.

Target selection follows the build/test path:
  1. AIT_SHARED_CARGO_TARGET_DIR, when set, explicitly opts into a shared target.
  2. CARGO_TARGET_DIR, when set, is honored for caller-managed targets.
  3. Otherwise a managed Task worktree uses its task-workspaces leaf beneath
     .ait/cargo-target, and the canonical checkout uses .ait/cargo-target.

Build-dir selection follows:
  1. AIT_SHARED_CARGO_BUILD_DIR, when set.
  2. CARGO_BUILD_BUILD_DIR, when set.
  3. Otherwise a managed Task leaf or the fixed canonical directory beneath
     .ait/cargo-build is used when RAM has sufficient free headroom.
     Unavailable or insufficient managed RAM uses .ait/cargo-build-ssd.
Known fallback canonical/registered Task caches are included without adopting
unregistered folders or changing the shared RAM link.
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
  local selected_build fallback_dirs fallback_dir
  selected_build="$(cargo_build_dir)" || return
  build_dirs+=("$selected_build")
  fallback_dirs="$(owned_fallback_cargo_build_dirs "$include_worktrees")" || return
  while IFS= read -r fallback_dir; do
    [[ -z "$fallback_dir" ]] || build_dirs+=("$fallback_dir")
  done <<< "$fallback_dirs"
  if [[ "${include_worktrees}" == "1" ]]; then
    local repository_target_root
    repository_target_root="$(repository_cargo_target_root)"
    local worktree_target
    for worktree_target in \
      "${repository_target_root}/${MANAGED_WORKTREE_CARGO_TARGET_DIRNAME}"/* \
      "${ROOT_DIR}"/.ait-worktree-links/*/rust/target; do
      [[ -d "${worktree_target}" && ! -L "${worktree_target}" ]] || continue
      targets+=("${worktree_target}")
    done
    local repository_build_root
    repository_build_root="$(repository_cargo_registered_build_root)" || return
    local worktree_build
    if [[ "$repository_build_root" != "$(repository_ait_dir)/cargo-build-ssd" ]]; then
      for worktree_build in "${repository_build_root}/task-workspaces"/*; do
        [[ -d "${worktree_build}" && ! -L "${worktree_build}" ]] || continue
        build_dirs+=("${worktree_build}")
      done
    fi
    for worktree_build in "${ROOT_DIR}"/.ait-worktree-links/*/.ait/cargo-build/workspaces/*/*; do
      [[ -d "${worktree_build}" && ! -L "${worktree_build}" ]] || continue
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
CARGO_TARGET_DIR is honored when set, managed Task worktrees use their
task-workspaces leaf beneath .ait/cargo-target, and the canonical checkout uses
.ait/cargo-target.
Set AIT_SHARED_CARGO_BUILD_DIR to opt into shared intermediates. Otherwise
CARGO_BUILD_BUILD_DIR is honored, managed Task worktrees use their dedicated
task-workspaces leaf, and the canonical checkout reuses the fixed
.ait/cargo-build/canonical directory. Managed RAM needs at least 4 GiB free,
raised by a larger positive AIT_CARGO_BUILD_MAX_BYTES. Insufficient/unavailable
RAM selects .ait/cargo-build-ssd with the same canonical/Task isolation.
Proven Cargo filesystem space exhaustion retries only that failed invocation
once on SSD. Shared RAM links/caches and final target placement are preserved.
Automatic build-dir reclamation is disabled by default; set a nonzero
AIT_CARGO_BUILD_MAX_BYTES to opt in. Reclamation is rate-limited by
AIT_CARGO_BUILD_GC_INTERVAL_SECONDS (default 3600).
EOF
}

main() {
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
          return 1
          ;;
      esac
      ;;
    server)
      shift
      server_command "$@"
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
