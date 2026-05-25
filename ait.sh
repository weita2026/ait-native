#!/usr/bin/env bash

set -euo pipefail

PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

resolve_homebrew_python() {
  local candidate
  for candidate in /opt/homebrew/bin/python3.14 /opt/homebrew/bin/python3 /usr/local/bin/python3; do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  command -v python3 2>/dev/null || true
}

python_user_script_dir() {
  local python_bin="$1"
  "$python_bin" - <<'PY'
import sysconfig

for scheme in ("osx_framework_user", "posix_user", "nt_user"):
    try:
        path = sysconfig.get_path("scripts", scheme=scheme)
    except Exception:
        continue
    if path:
        print(path)
        raise SystemExit(0)
raise SystemExit(1)
PY
}

export AIT_REPO_ROOT="${AIT_REPO_ROOT:-${ROOT_DIR}}"
export AIT_PYTHON_BIN="${AIT_PYTHON_BIN:-$(resolve_homebrew_python)}"
export AIT_CONSOLE_BIN_DIR="${AIT_CONSOLE_BIN_DIR:-$(python_user_script_dir "$AIT_PYTHON_BIN")}"
export PATH="${AIT_CONSOLE_BIN_DIR}:${PATH}"
export AIT_RUNTIME_ROOT="${AIT_RUNTIME_ROOT:-/Volumes/lyravo/ait-runtime}"
export AIT_NATIVE_SERVER_DATA="${AIT_NATIVE_SERVER_DATA:-${AIT_RUNTIME_ROOT}/server-data}"
export AIT_NATIVE_SERVER_DB_BACKEND="${AIT_NATIVE_SERVER_DB_BACKEND:-postgres}"
export AIT_NATIVE_SERVER_POSTGRES_DSN="${AIT_NATIVE_SERVER_POSTGRES_DSN:-postgresql://weita@127.0.0.1:5432/ait_native}"
export AIT_LOG_DIR="${AIT_LOG_DIR:-${AIT_RUNTIME_ROOT}/logs}"
export AIT_SERVER_PID_FILE="${AIT_SERVER_PID_FILE:-${AIT_LOG_DIR}/ait-server.pid}"
export AIT_WEB_PID_FILE="${AIT_WEB_PID_FILE:-${AIT_LOG_DIR}/ait-web.pid}"
export AIT_COMMUNITY_WEB_PID_FILE="${AIT_COMMUNITY_WEB_PID_FILE:-${AIT_LOG_DIR}/ait-web-community.pid}"
export AIT_AGENT_CONFIG_PATH="${AIT_AGENT_CONFIG_PATH:-${AIT_REPO_ROOT}/.ait/agent-workers.json}"
export AIT_TELEGRAM_PID_FILE="${AIT_TELEGRAM_PID_FILE:-${AIT_LOG_DIR}/ait-agent-telegram-main.pid}"
AIT_DEFAULT_TELEGRAM_ENV_PATH="${AIT_REPO_ROOT}/.ait/agent-runtime/telegram.env"
AIT_LEGACY_TELEGRAM_ENV_PATH="${AIT_REPO_ROOT}/telegram-bot/.env"
if [[ -n "${AIT_TELEGRAM_ENV_PATH_OVERRIDE:-}" ]]; then
  export AIT_TELEGRAM_ENV_PATH="${AIT_TELEGRAM_ENV_PATH_OVERRIDE}"
elif [[ -z "${AIT_TELEGRAM_ENV_PATH:-}" || "${AIT_TELEGRAM_ENV_PATH}" == "${AIT_LEGACY_TELEGRAM_ENV_PATH}" ]]; then
  export AIT_TELEGRAM_ENV_PATH="${AIT_DEFAULT_TELEGRAM_ENV_PATH}"
else
  export AIT_TELEGRAM_ENV_PATH
fi

export AIT_NATIVE_QUEUE_MODE="${AIT_NATIVE_QUEUE_MODE:-inline}"
export AIT_NATIVE_SERVER_HOST="${AIT_NATIVE_SERVER_HOST:-127.0.0.1}"
export AIT_NATIVE_SERVER_PORT="${AIT_NATIVE_SERVER_PORT:-8088}"
export AIT_NATIVE_WEB_HOST="${AIT_NATIVE_WEB_HOST:-0.0.0.0}"
export AIT_NATIVE_WEB_PORT="${AIT_NATIVE_WEB_PORT:-8000}"
export AIT_NATIVE_COMMUNITY_WEB_HOST="${AIT_NATIVE_COMMUNITY_WEB_HOST:-${AIT_NATIVE_WEB_HOST}}"
export AIT_NATIVE_COMMUNITY_WEB_PORT="${AIT_NATIVE_COMMUNITY_WEB_PORT:-8100}"
export AIT_NATIVE_COMMUNITY_WEB_SURFACE="${AIT_NATIVE_COMMUNITY_WEB_SURFACE:-community_hosted}"

SERVER_PORT="${AIT_NATIVE_SERVER_PORT}"
WEB_PORT="${AIT_NATIVE_WEB_PORT}"
COMMUNITY_WEB_PORT="${AIT_NATIVE_COMMUNITY_WEB_PORT}"
SERVER_LOG_FILE="${AIT_LOG_DIR}/ait-server.log"
WEB_LOG_FILE="${AIT_LOG_DIR}/ait-web.log"
COMMUNITY_WEB_LOG_FILE="${AIT_LOG_DIR}/ait-web-community.log"
TELEGRAM_LOG_FILE="${AIT_LOG_DIR}/ait-agent-telegram.log"

export AIT_CLI_BIN="${AIT_CLI_BIN:-${AIT_CONSOLE_BIN_DIR}/ait}"
export AIT_SERVER_BIN="${AIT_SERVER_BIN:-${AIT_CONSOLE_BIN_DIR}/ait-server}"
export AIT_WEB_BIN="${AIT_WEB_BIN:-${AIT_CONSOLE_BIN_DIR}/ait-web}"
export AIT_AGENT_BIN="${AIT_AGENT_BIN:-${AIT_CONSOLE_BIN_DIR}/ait-agent}"
export AIT_SITE_HELPER_SCRIPT="${AIT_SITE_HELPER_SCRIPT:-${AIT_REPO_ROOT}/scripts/official_site_https.py}"
export AIT_SITE_ENV_PATH="${AIT_SITE_ENV_PATH:-${AIT_REPO_ROOT}/deploy/site/macos-nginx/site.env}"
export AIT_SITE_RENDER_DIR="${AIT_SITE_RENDER_DIR:-${AIT_REPO_ROOT}/deploy/site/nginx-rendered}"
export AIT_SITE_PREVIEW_DIR="${AIT_SITE_PREVIEW_DIR:-${AIT_REPO_ROOT}/site/dist}"
export AIT_SITE_PREVIEW_HOST="${AIT_SITE_PREVIEW_HOST:-192.168.1.106}"
export AIT_SITE_PREVIEW_PORT="${AIT_SITE_PREVIEW_PORT:-1234}"

usage() {
  cat <<'USAGE'
Usage:
  ./ait.sh {start|restart|stop|status}
  ./ait.sh server {start|restart|stop|status}
  ./ait.sh web {start|restart|stop|status}
  ./ait.sh community-web {start|restart|stop|status}
  ./ait.sh community {start|restart|stop|status}  # alias for community-web
  ./ait.sh agent {start|restart|stop|status}
  ./ait.sh telegram {start|restart|stop|status}  # alias for agent
  ./ait.sh site {release|doctor|render|preview}

Services:
  start      Start ait-server, ait-web, and configured ait-agent workers
  stop       Stop configured ait-agent workers, ait-web, and ait-server
  restart    Restart configured ait-agent workers, ait-web, and ait-server
  status     Show backend service status with indicator lights

Site helper:
  release    Build the macOS-native official site output and refresh rendered nginx assets
  doctor     Validate deploy/site/macos-nginx/site.env and nginx template prerequisites
  render     Refresh only the rendered nginx config/helper output
  preview    Build the public site into the local preview directory and serve it on the standard preview port
USAGE
}

log() {
  printf '[%s] %s\n' "$(/bin/date '+%Y-%m-%d %H:%M:%S')" "$*"
}

ensure_runtime_dir() {
  mkdir -p "${AIT_NATIVE_SERVER_DATA}" "${AIT_LOG_DIR}"
}

ensure_runtime_ready() {
  ensure_runtime_volume_root_available

  ensure_runtime_dir
  /usr/bin/touch "${AIT_LOG_DIR}/launchd-access.ok"
}

ensure_server_bin_ready() {
  if [[ ! -x "${AIT_SERVER_BIN}" ]]; then
    log "Missing executable: ${AIT_SERVER_BIN}"
    log "Refresh Homebrew console scripts with: ${AIT_PYTHON_BIN} -m pip install --user --break-system-packages -e ${AIT_REPO_ROOT}"
    exit 1
  fi
}

ensure_web_bin_ready() {
  if [[ ! -x "${AIT_WEB_BIN}" ]]; then
    log "Missing executable: ${AIT_WEB_BIN}"
    log "Refresh Homebrew console scripts with: ${AIT_PYTHON_BIN} -m pip install --user --break-system-packages -e ${AIT_REPO_ROOT}"
    exit 1
  fi
}

ensure_service_start_ready() {
  local service="$1"

  ensure_runtime_ready

  case "$service" in
    server)
      ensure_server_bin_ready
      ;;
    web|community-web)
      ensure_web_bin_ready
      ;;
  esac
}

ensure_agent_ready() {
  ensure_runtime_ready

  if [[ ! -x "${AIT_AGENT_BIN}" ]]; then
    log "Missing executable: ${AIT_AGENT_BIN}"
    log "Refresh Homebrew console scripts with: ${AIT_PYTHON_BIN} -m pip install --user --break-system-packages -e ${AIT_REPO_ROOT}"
    exit 1
  fi
}

ensure_runtime_volume_root_available() {
  local volume_root

  if [[ "${AIT_RUNTIME_ROOT}" == /Volumes/* ]]; then
    volume_root="/$(printf '%s\n' "${AIT_RUNTIME_ROOT}" | cut -d/ -f2-3)"
    if [[ ! -d "${volume_root}" ]]; then
      log "Runtime volume is not mounted at ${volume_root}"
      exit 1
    fi
  fi
}

ensure_site_helper_ready() {
  if [[ ! -x "${AIT_PYTHON_BIN}" ]]; then
    log "Missing Python executable: ${AIT_PYTHON_BIN}"
    exit 1
  fi

  if [[ ! -f "${AIT_SITE_HELPER_SCRIPT}" ]]; then
    log "Missing official site helper script: ${AIT_SITE_HELPER_SCRIPT}"
    exit 1
  fi
}

site_release() {
  ensure_site_helper_ready
  (
    cd "$AIT_REPO_ROOT"
    "${AIT_PYTHON_BIN}" "${AIT_SITE_HELPER_SCRIPT}" release-nginx \
      --env-file "${AIT_SITE_ENV_PATH}" \
      --output-dir "${AIT_SITE_RENDER_DIR}"
  )
}

site_doctor() {
  ensure_site_helper_ready
  (
    cd "$AIT_REPO_ROOT"
    "${AIT_PYTHON_BIN}" "${AIT_SITE_HELPER_SCRIPT}" doctor-nginx \
      --env-file "${AIT_SITE_ENV_PATH}"
  )
}

site_render() {
  ensure_site_helper_ready
  (
    cd "$AIT_REPO_ROOT"
    "${AIT_PYTHON_BIN}" "${AIT_SITE_HELPER_SCRIPT}" render-nginx \
      --env-file "${AIT_SITE_ENV_PATH}" \
      --output-dir "${AIT_SITE_RENDER_DIR}"
  )
}

site_preview() {
  ensure_site_helper_ready

  local preview_dir="${AIT_SITE_PREVIEW_DIR}"
  local preview_host="${AIT_SITE_PREVIEW_HOST}"
  local preview_port="${AIT_SITE_PREVIEW_PORT}"

  (
    cd "$AIT_REPO_ROOT"
    "${AIT_PYTHON_BIN}" "${AIT_SITE_HELPER_SCRIPT}" build --output "${preview_dir}"
  )

  log "Official site preview ready at http://${preview_host}:${preview_port}"
  if [[ "${preview_host}" == "0.0.0.0" ]]; then
    log "Same-LAN preview is also available at http://<your-lan-ip>:${preview_port}"
  fi
  log "Serving ${preview_dir} in the foreground. Press Ctrl-C to stop."

  exec "${AIT_PYTHON_BIN}" -m http.server "${preview_port}" \
    --bind "${preview_host}" \
    --directory "${preview_dir}"
}

load_telegram_env_if_present() {
  if [[ -f "${AIT_TELEGRAM_ENV_PATH}" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "${AIT_TELEGRAM_ENV_PATH}"
    set +a
  fi
}

telegram_token_configured() {
  load_telegram_env_if_present
  [[ -n "${AIT_TELEGRAM_BOT_TOKEN:-${BOT_TOKEN:-}}" ]]
}

spawn_detached_process() {
  if [ "$#" -lt 2 ]; then
    echo "spawn_detached_process requires a log file and command." >&2
    return 1
  fi

  local log_file="$1"
  shift

  if [[ -x "${AIT_PYTHON_BIN}" ]]; then
    DETACHED_PID="$(
      "${AIT_PYTHON_BIN}" - "$log_file" "$@" <<'PY'
import os
import subprocess
import sys

log_path = sys.argv[1]
command = sys.argv[2:]

with open(log_path, "ab", buffering=0) as log_handle:
    process = subprocess.Popen(
        command,
        cwd=os.environ["AIT_REPO_ROOT"],
        env=os.environ.copy(),
        stdin=subprocess.DEVNULL,
        stdout=log_handle,
        stderr=subprocess.STDOUT,
        start_new_session=True,
        close_fds=True,
    )

print(process.pid)
PY
    )"
    export DETACHED_PID
    return 0
  fi

  if command -v nohup >/dev/null 2>&1; then
    nohup "$@" </dev/null >>"$log_file" 2>&1 &
  else
    "$@" </dev/null >>"$log_file" 2>&1 &
  fi

  DETACHED_PID="$!"
  export DETACHED_PID
  disown %1 2>/dev/null || true
}

read_pid_from_file() {
  local pid_file="$1"

  if [ ! -f "$pid_file" ]; then
    return 1
  fi

  local pid
  pid="$(tr -d '[:space:]' < "$pid_file")"
  if [ -z "$pid" ]; then
    return 1
  fi

  echo "$pid"
}

listener_pid_for_port() {
  local port="$1"

  if ! command -v lsof >/dev/null 2>&1; then
    return 1
  fi

  lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null | head -n 1
}

process_cwd_for_pid() {
  local pid="$1"

  if ! command -v lsof >/dev/null 2>&1; then
    return 1
  fi

  lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -n 1
}

process_command_for_pid() {
  local pid="$1"

  ps -p "$pid" -o command= 2>/dev/null | sed 's/^ *//' || true
}

pid_looks_alive() {
  local pid="$1"

  if [ -z "$pid" ]; then
    return 1
  fi

  if kill -0 "$pid" 2>/dev/null; then
    return 0
  fi

  if command -v lsof >/dev/null 2>&1; then
    lsof -p "$pid" >/dev/null 2>&1 && return 0
  fi

  return 1
}

is_ait_server_pid() {
  local pid="$1"
  local cwd
  local command

  cwd="$(process_cwd_for_pid "$pid" || true)"
  if [ "$cwd" != "$AIT_REPO_ROOT" ]; then
    return 1
  fi

  command="$(process_command_for_pid "$pid" || true)"
  case "$command" in
    *"${AIT_SERVER_BIN}"*|*".venv/bin/ait-server"*)
      return 0
      ;;
  esac

  return 1
}

is_ait_web_pid() {
  local pid="$1"
  local cwd
  local command

  cwd="$(process_cwd_for_pid "$pid" || true)"
  if [ "$cwd" != "$AIT_REPO_ROOT" ]; then
    return 1
  fi

  command="$(process_command_for_pid "$pid" || true)"
  case "$command" in
    *"${AIT_WEB_BIN}"*|*".venv/bin/ait-web"*)
      return 0
      ;;
  esac

  return 1
}

is_ait_telegram_pid() {
  local pid="$1"
  local cwd
  local command

  cwd="$(process_cwd_for_pid "$pid" || true)"
  if [ "$cwd" != "$AIT_REPO_ROOT" ]; then
    return 1
  fi

  command="$(process_command_for_pid "$pid" || true)"
  case "$command" in
    *"ait_agent.telegram.app"*)
      return 0
      ;;
  esac

  return 1
}

service_pid_matches() {
  local service="$1"
  local pid="$2"

  case "$service" in
    server)
      is_ait_server_pid "$pid"
      ;;
    web)
      is_ait_web_pid "$pid"
      ;;
    community-web)
      is_ait_web_pid "$pid"
      ;;
    telegram)
      is_ait_telegram_pid "$pid"
      ;;
    *)
      return 1
      ;;
  esac
}

adopt_running_service() {
  local service="$1"
  local port="$2"
  local pid_file="$3"
  local listener_pid

  listener_pid="$(listener_pid_for_port "$port" || true)"
  if [ -z "$listener_pid" ]; then
    return 1
  fi

  if ! service_pid_matches "$service" "$listener_pid"; then
    return 1
  fi

  echo "$listener_pid" > "$pid_file"
  return 0
}

service_is_running() {
  local service="$1"
  local port="$2"
  local pid_file="$3"
  local pid

  pid="$(read_pid_from_file "$pid_file" 2>/dev/null || true)"
  if [ -n "$pid" ] && pid_looks_alive "$pid" && service_pid_matches "$service" "$pid"; then
    echo "$pid" > "$pid_file"
    return 0
  fi

  rm -f "$pid_file"
  adopt_running_service "$service" "$port" "$pid_file"
}

assert_port_available() {
  local service="$1"
  local port="$2"
  local pid_file="$3"
  local listener_pid

  listener_pid="$(listener_pid_for_port "$port" || true)"
  if [ -z "$listener_pid" ]; then
    return 0
  fi

  if service_pid_matches "$service" "$listener_pid"; then
    echo "$listener_pid" > "$pid_file"
    return 0
  fi

  echo "Error: Port $port is already in use by PID $listener_pid." >&2
  return 1
}

wait_for_service() {
  local service="$1"
  local pid="$2"
  local port="$3"
  local attempt

  for attempt in $(seq 1 20); do
    if ! pid_looks_alive "$pid"; then
      return 1
    fi

    if command -v lsof >/dev/null 2>&1; then
      local listener_pid
      listener_pid="$(listener_pid_for_port "$port" || true)"
      if [ "$listener_pid" = "$pid" ] && service_pid_matches "$service" "$pid"; then
        return 0
      fi
    fi

    sleep 0.5
  done

  pid_looks_alive "$pid"
}

wait_for_pid_exit() {
  local pid="$1"
  local attempts="${2:-20}"
  local sleep_seconds="${3:-0.5}"
  local attempt

  for attempt in $(seq 1 "$attempts"); do
    if ! pid_looks_alive "$pid"; then
      return 0
    fi
    sleep "$sleep_seconds"
  done

  ! pid_looks_alive "$pid"
}

stop_pid_gracefully() {
  local pid="$1"
  local signal="${2:-TERM}"

  if ! pid_looks_alive "$pid"; then
    return 2
  fi

  if kill "-$signal" "$pid" 2>/dev/null; then
    return 0
  fi

  if ! pid_looks_alive "$pid"; then
    return 2
  fi

  return 1
}

start_service() {
  local service="$1"
  local label="$2"
  local port="$3"
  local pid_file="$4"
  local log_file="$5"
  shift 5
  local cmd=( "$@" )

  ensure_service_start_ready "$service"

  if service_is_running "$service" "$port" "$pid_file"; then
    local pid
    pid="$(read_pid_from_file "$pid_file")"
    echo "${label} is already running in daemon mode."
    echo "PID: $pid"
    echo "Log: $log_file"
    return 0
  fi

  assert_port_available "$service" "$port" "$pid_file"

  echo "Starting ${label} in daemon mode..."
  (
    cd "$AIT_REPO_ROOT"
    spawn_detached_process "$log_file" "${cmd[@]}"
    echo "$DETACHED_PID" > "$pid_file"
  )

  local pid
  pid="$(read_pid_from_file "$pid_file")"
  if ! wait_for_service "$service" "$pid" "$port"; then
    echo "Failed to start ${label}." >&2
    echo "Last log lines:" >&2
    tail -n 20 "$log_file" 2>/dev/null || true
    rm -f "$pid_file"
    return 1
  fi

  echo "${label} started."
  echo "PID: $pid"
  echo "Log: $log_file"
}

stop_service() {
  local service="$1"
  local label="$2"
  local port="$3"
  local pid_file="$4"

  ensure_runtime_ready

  if ! service_is_running "$service" "$port" "$pid_file"; then
    rm -f "$pid_file"
    echo "${label} is not running."
    return 0
  fi

  local pid
  pid="$(read_pid_from_file "$pid_file")"
  echo "Stopping ${label} (PID: $pid)..."

  if stop_pid_gracefully "$pid" TERM; then
    if wait_for_pid_exit "$pid" 20 0.5; then
      rm -f "$pid_file"
      echo "${label} stopped."
      return 0
    fi

    echo "Process did not stop in time. Forcing shutdown..."
    if stop_pid_gracefully "$pid" KILL; then
      if wait_for_pid_exit "$pid" 10 0.2; then
        rm -f "$pid_file"
        echo "${label} stopped."
        return 0
      fi
    fi
  fi

  if ! pid_looks_alive "$pid"; then
    rm -f "$pid_file"
    echo "${label} was already stopped. Cleared runtime state."
    return 0
  fi

  echo "Error: Unable to stop ${label} (PID: $pid)." >&2
  return 1
}

start_server() {
  start_service "server" "ait-server" "$SERVER_PORT" "$AIT_SERVER_PID_FILE" "$SERVER_LOG_FILE" "${AIT_SERVER_BIN}"
}

assert_distinct_web_ports() {
  if [ "$WEB_PORT" = "$COMMUNITY_WEB_PORT" ]; then
    echo "Error: AIT_NATIVE_WEB_PORT and AIT_NATIVE_COMMUNITY_WEB_PORT must be different." >&2
    return 1
  fi
}

start_web() {
  start_service "web" "ait-web" "$WEB_PORT" "$AIT_WEB_PID_FILE" "$WEB_LOG_FILE" "${AIT_WEB_BIN}" "--host" "$AIT_NATIVE_WEB_HOST" "--port" "$AIT_NATIVE_WEB_PORT"
}

start_community_web() {
  assert_distinct_web_ports
  start_service \
    "community-web" \
    "ait-web community" \
    "$COMMUNITY_WEB_PORT" \
    "$AIT_COMMUNITY_WEB_PID_FILE" \
    "$COMMUNITY_WEB_LOG_FILE" \
    "env" \
    "AIT_NATIVE_WEB_FIRST_USE_SURFACE=${AIT_NATIVE_COMMUNITY_WEB_SURFACE}" \
    "AIT_NATIVE_WEB_PORT=${AIT_NATIVE_COMMUNITY_WEB_PORT}" \
    "${AIT_WEB_BIN}" \
    "--host" \
    "$AIT_NATIVE_COMMUNITY_WEB_HOST" \
    "--port" \
    "$AIT_NATIVE_COMMUNITY_WEB_PORT"
}

agent_status_json() {
  ensure_agent_ready
  load_telegram_env_if_present

  (
    cd "$AIT_REPO_ROOT"
    "${AIT_AGENT_BIN}" telegram supervisor status --json
  )
}

json_field() {
  local payload="$1"
  local field="$2"
  local default="${3:-}"

  JSON_PAYLOAD="$payload" "${AIT_PYTHON_BIN}" - "$field" "$default" <<'PY'
import json
import os
import sys

field = sys.argv[1]
default = sys.argv[2]

try:
    payload = json.loads(os.environ.get("JSON_PAYLOAD") or "{}")
except json.JSONDecodeError:
    print(default)
    raise SystemExit(0)

value = payload.get(field, default)
if value is None:
    value = default
print(value)
PY
}

telegram_worker_count() {
  local payload
  payload="$(agent_status_json 2>/dev/null || true)"
  if [ -z "$payload" ]; then
    echo 0
    return 0
  fi
  json_field "$payload" "worker_count" "0"
}

telegram_running_count() {
  local payload
  payload="$(agent_status_json 2>/dev/null || true)"
  if [ -z "$payload" ]; then
    echo 0
    return 0
  fi
  json_field "$payload" "running_count" "0"
}

telegram_workers_configured() {
  local count
  count="$(telegram_worker_count)"
  [ "${count:-0}" -gt 0 ]
}

telegram_is_running() {
  local count
  count="$(telegram_running_count)"
  [ "${count:-0}" -gt 0 ]
}

start_telegram() {
  ensure_agent_ready
  load_telegram_env_if_present

  if ! telegram_workers_configured; then
    echo "No ait-agent Telegram workers are configured."
    echo "Configure one with:"
    echo "  ${AIT_AGENT_BIN} telegram add main --token ... --username ..."
    return 1
  fi

  if telegram_is_running; then
    echo "ait-agent Telegram workers are already running."
    agent_status_json
    return 0
  fi

  echo "Starting configured ait-agent Telegram workers..."
  (
    cd "$AIT_REPO_ROOT"
    "${AIT_AGENT_BIN}" telegram supervisor start --json
  )
}

stop_server() {
  stop_service "server" "ait-server" "$SERVER_PORT" "$AIT_SERVER_PID_FILE"
}

stop_web() {
  stop_service "web" "ait-web" "$WEB_PORT" "$AIT_WEB_PID_FILE"
}

stop_community_web() {
  assert_distinct_web_ports
  stop_service "community-web" "ait-web community" "$COMMUNITY_WEB_PORT" "$AIT_COMMUNITY_WEB_PID_FILE"
}

stop_telegram() {
  ensure_agent_ready
  load_telegram_env_if_present

  if ! telegram_workers_configured; then
    echo "No ait-agent Telegram workers are configured."
    return 0
  fi

  echo "Stopping configured ait-agent Telegram workers..."
  (
    cd "$AIT_REPO_ROOT"
    "${AIT_AGENT_BIN}" telegram supervisor stop --json
  )
}

restart_server() {
  stop_server
  start_server
}

restart_web() {
  stop_web
  start_web
}

restart_community_web() {
  stop_community_web || true
  start_community_web
}

restart_telegram() {
  ensure_agent_ready
  load_telegram_env_if_present

  if ! telegram_workers_configured; then
    echo "No ait-agent Telegram workers are configured."
    return 1
  fi

  echo "Restarting configured ait-agent Telegram workers..."
  (
    cd "$AIT_REPO_ROOT"
    "${AIT_AGENT_BIN}" telegram supervisor restart --json
  )
}

show_status() {
  local green="\033[32m"
  local yellow="\033[33m"
  local red="\033[31m"
  local reset="\033[0m"
  local on="${green}[ok]${reset}"
  local warn="${yellow}[!]${reset}"
  local off="${red}[x]${reset}"

  local server_state="off"
  local web_state="off"
  local telegram_state="off"
  local community_web_state="off"
  local server_pid=""
  local web_pid=""
  local community_web_pid=""
  local telegram_config="not_configured"
  local telegram_status_payload=""
  local telegram_worker_count_value="0"
  local telegram_running_count_value="0"

  ensure_runtime_ready

  telegram_status_payload="$(agent_status_json 2>/dev/null || true)"
  if [ -n "$telegram_status_payload" ]; then
    telegram_worker_count_value="$(json_field "$telegram_status_payload" "worker_count" "0")"
    telegram_running_count_value="$(json_field "$telegram_status_payload" "running_count" "0")"
  fi

  if [ "${telegram_worker_count_value:-0}" -gt 0 ]; then
    telegram_config="configured"
  elif telegram_token_configured; then
    telegram_config="token_only"
  fi

  if service_is_running "server" "$SERVER_PORT" "$AIT_SERVER_PID_FILE"; then
    server_state="on"
    server_pid="$(read_pid_from_file "$AIT_SERVER_PID_FILE")"
  fi

  if service_is_running "web" "$WEB_PORT" "$AIT_WEB_PID_FILE"; then
    web_state="on"
    web_pid="$(read_pid_from_file "$AIT_WEB_PID_FILE")"
  fi

  if [ "$WEB_PORT" != "$COMMUNITY_WEB_PORT" ] && service_is_running "community-web" "$COMMUNITY_WEB_PORT" "$AIT_COMMUNITY_WEB_PID_FILE"; then
    community_web_state="on"
    community_web_pid="$(read_pid_from_file "$AIT_COMMUNITY_WEB_PID_FILE")"
  fi

  if [ "${telegram_running_count_value:-0}" -gt 0 ]; then
    telegram_state="on"
  fi

  local overall_light="$off"
  local overall_label="stopped"
  if [ "$server_state" = "on" ] && { [ "$web_state" = "on" ] || [ "$community_web_state" = "on" ]; }; then
    overall_light="$on"
    overall_label="healthy"
  elif [ "$server_state" = "on" ] || [ "$web_state" = "on" ] || [ "$community_web_state" = "on" ] || [ "$telegram_state" = "on" ]; then
    overall_light="$warn"
    overall_label="partial"
  fi

  echo ""
  echo "  ait Backend Status"
  echo "  -----------------------------"
  printf "  %b  Backend           %s\n" "$overall_light" "$overall_label"

  if [ "$server_state" = "on" ]; then
    printf "  %b  ait-server        PID %-8s http://%s:%s\n" "$on" "$server_pid" "$AIT_NATIVE_SERVER_HOST" "$SERVER_PORT"
  else
    printf "  %b  ait-server        not running\n" "$off"
  fi

  if [ "$web_state" = "on" ]; then
    printf "  %b  ait-web           PID %-8s http://127.0.0.1:%s/inbox\n" "$on" "$web_pid" "$WEB_PORT"
  else
    printf "  %b  ait-web           not running\n" "$off"
  fi

  if [ "$WEB_PORT" = "$COMMUNITY_WEB_PORT" ]; then
    printf "  %b  community-web     port conflict with ait-web (%s)\n" "$warn" "$COMMUNITY_WEB_PORT"
  elif [ "$community_web_state" = "on" ]; then
    printf "  %b  community-web     PID %-8s http://127.0.0.1:%s/\n" "$on" "$community_web_pid" "$COMMUNITY_WEB_PORT"
  else
    printf "  %b  community-web     not running\n" "$off"
  fi

  if [ "$telegram_state" = "on" ]; then
    printf "  %b  ait-agent          telegram workers %s/%s running\n" "$on" "$telegram_running_count_value" "$telegram_worker_count_value"
  elif [ "$telegram_config" = "configured" ]; then
    printf "  %b  ait-agent          telegram workers configured but not running (%s total)\n" "$warn" "$telegram_worker_count_value"
  elif [ "$telegram_config" = "token_only" ]; then
    printf "  %b  ait-agent          token exists but no named worker config (%s)\n" "$warn" "${AIT_AGENT_CONFIG_PATH}"
  else
    printf "  %b  ait-agent          no Telegram worker configured (%s)\n" "$off" "${AIT_AGENT_CONFIG_PATH}"
  fi

  echo ""
  echo "  Runtime root: ${AIT_RUNTIME_ROOT}"
  echo "  Data root:    ${AIT_NATIVE_SERVER_DATA}"
  echo "  Python:       ${AIT_PYTHON_BIN}"
  echo "  Script bin:   ${AIT_CONSOLE_BIN_DIR}"
  echo "  Server log:   ${SERVER_LOG_FILE}"
  echo "  Web log:      ${WEB_LOG_FILE}"
  echo "  Community log:${COMMUNITY_WEB_LOG_FILE}"
  echo "  Agent config: ${AIT_AGENT_CONFIG_PATH}"
  echo "  Agent log:    ${TELEGRAM_LOG_FILE}"
  echo ""
}

run_service_command() {
  local service="$1"
  local action="$2"

  case "$service:$action" in
    server:start) start_server ;;
    server:stop) stop_server ;;
    server:restart) restart_server ;;
    server:status) show_status ;;
    web:start) start_web ;;
    web:stop) stop_web ;;
    web:restart) restart_web ;;
    web:status) show_status ;;
    community-web:start|community:start) start_community_web ;;
    community-web:stop|community:stop) stop_community_web ;;
    community-web:restart|community:restart) restart_community_web ;;
    community-web:status|community:status) show_status ;;
    agent:start) start_telegram ;;
    agent:stop) stop_telegram ;;
    agent:restart) restart_telegram ;;
    agent:status) show_status ;;
    telegram:start) start_telegram ;;
    telegram:stop) stop_telegram ;;
    telegram:restart) restart_telegram ;;
    telegram:status) show_status ;;
    *) usage; return 1 ;;
  esac
}

run_site_command() {
  local action="$1"

  case "$action" in
    release) site_release ;;
    doctor) site_doctor ;;
    render) site_render ;;
    preview) site_preview ;;
    *) usage; return 1 ;;
  esac
}

main() {
  if [ "$#" -eq 0 ]; then
    usage
    exit 1
  fi

  case "$1" in
    start)
      start_server
      start_web
      if telegram_workers_configured; then
        start_telegram
      elif telegram_token_configured; then
        echo "Skipping ait-agent Telegram startup because token exists but no named worker config is registered."
        echo "Register one with: ${AIT_AGENT_BIN} telegram add main --token ... --username ..."
      else
        echo "Skipping ait-agent Telegram startup because no worker is configured."
      fi
      show_status
      ;;
    stop)
      stop_telegram || true
      stop_web
      stop_server
      show_status
      ;;
    restart)
      stop_telegram || true
      stop_web || true
      stop_server || true
      start_server
      start_web
      if telegram_workers_configured; then
        start_telegram
      elif telegram_token_configured; then
        echo "Skipping ait-agent Telegram startup because token exists but no named worker config is registered."
        echo "Register one with: ${AIT_AGENT_BIN} telegram add main --token ... --username ..."
      else
        echo "Skipping ait-agent Telegram startup because no worker is configured."
      fi
      show_status
      ;;
    status)
      show_status
      ;;
    site)
      if [ "$#" -ne 2 ]; then
        usage
        exit 1
      fi
      run_site_command "$2"
      ;;
    server|web|community-web|community|agent|telegram)
      if [ "$#" -ne 2 ]; then
        usage
        exit 1
      fi
      run_service_command "$1" "$2"
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

main "$@"
