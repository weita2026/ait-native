#!/usr/bin/env bash

set -euo pipefail

PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"

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

export AIT_PYTHON_BIN="${AIT_PYTHON_BIN:-$(resolve_homebrew_python)}"
export AIT_CONSOLE_BIN_DIR="${AIT_CONSOLE_BIN_DIR:-$(python_user_script_dir "$AIT_PYTHON_BIN" 2>/dev/null || true)}"
AITK_BIN="${AITK_BIN:-${AIT_CONSOLE_BIN_DIR}/aitk}"

usage_hint() {
  cat >&2 <<'EOF'
aitk.sh: no usable Tcl/Tk wish executable was found.

Install Homebrew Tcl/Tk, then retry:

  brew install tcl-tk
  ./aitk.sh

Or pin a specific wish executable:

  AITK_WISH=/opt/homebrew/opt/tcl-tk/bin/wish9.0 ./aitk.sh

For JSON-only export, no Tk runtime is needed:

  ./aitk.sh --json-only --output /tmp/aitk.json
EOF
}

arg_present() {
  local expected="$1"
  shift

  local arg
  for arg in "$@"; do
    if [[ "$arg" == "$expected" ]]; then
      return 0
    fi
  done

  return 1
}

user_supplied_wish() {
  local arg
  for arg in "$@"; do
    case "$arg" in
      --wish|--wish=*)
        return 0
        ;;
    esac
  done

  return 1
}

resolve_command_or_path() {
  local value="$1"

  if [[ "$value" == */* ]]; then
    printf '%s\n' "$value"
    return 0
  fi

  command -v "$value" 2>/dev/null || true
}

is_usable_wish() {
  local candidate="$1"

  [[ -n "$candidate" && -x "$candidate" ]] || return 1

  # The macOS system wish can fail on newer installs with Tk.framework errors.
  # Prefer Homebrew or an explicitly pinned non-system build.
  case "$candidate" in
    /usr/bin/wish|/System/*)
      return 1
      ;;
  esac

  return 0
}

find_homebrew_prefix() {
  if command -v brew >/dev/null 2>&1; then
    brew --prefix tcl-tk 2>/dev/null || true
  fi
}

find_wish() {
  local candidate
  local prefix
  local prefixes=()

  if [[ -n "${AITK_WISH:-}" ]]; then
    candidate="$(resolve_command_or_path "${AITK_WISH}")"
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi

    echo "aitk.sh: AITK_WISH is not executable: ${AITK_WISH}" >&2
    return 1
  fi

  prefix="$(find_homebrew_prefix)"
  if [[ -n "$prefix" ]]; then
    prefixes+=("$prefix")
  fi
  prefixes+=("/opt/homebrew/opt/tcl-tk" "/usr/local/opt/tcl-tk")

  for prefix in "${prefixes[@]}"; do
    [[ -d "$prefix/bin" ]] || continue
    for candidate in "$prefix"/bin/wish "$prefix"/bin/wish[0-9]*; do
      if is_usable_wish "$candidate"; then
        printf '%s\n' "$candidate"
        return 0
      fi
    done
  done

  candidate="$(command -v wish 2>/dev/null || true)"
  if is_usable_wish "$candidate"; then
    printf '%s\n' "$candidate"
    return 0
  fi

  return 1
}

main() {
  if [[ ! -x "$AITK_BIN" ]]; then
    echo "aitk.sh: missing executable: ${AITK_BIN}" >&2
    echo "Refresh Homebrew console scripts with: ${AIT_PYTHON_BIN:-python3} -m pip install --user --break-system-packages -e ${ROOT_DIR}" >&2
    return 1
  fi

  if arg_present "--help" "$@" || arg_present "-h" "$@" || \
    arg_present "--json-only" "$@" || arg_present "--no-open" "$@" || \
    user_supplied_wish "$@"; then
    exec "$AITK_BIN" "$@"
  fi

  local wish
  if ! wish="$(find_wish)"; then
    usage_hint
    return 1
  fi

  exec "$AITK_BIN" --wish "$wish" "$@"
}

main "$@"
