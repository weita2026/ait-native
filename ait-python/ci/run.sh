#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mode=${1:-patchset}

case "$mode" in
  patchset | repo | all)
    ;;
  *)
    printf '%s\n' "usage: ./ci/run.sh {patchset|repo|all}" >&2
    exit 64
    ;;
esac

runtime_parent=${AIT_RUNNER_ATTEMPT_ROOT:-${TMPDIR:-/tmp}}
mkdir -p "$runtime_parent"
ci_root=$(mktemp -d "$runtime_parent/ait-python-ci.XXXXXX")

cleanup() {
  rm -rf -- "$ci_root"
}
trap cleanup 0 1 2 15

mkdir -p \
  "$ci_root/tmp" \
  "$ci_root/cache/pip" \
  "$ci_root/cache/cargo" \
  "$ci_root/cache/python" \
  "$ci_root/cargo-target" \
  "$ci_root/cargo-build"

export TMPDIR="$ci_root/tmp"
export TMP="$ci_root/tmp"
export TEMP="$ci_root/tmp"
export XDG_CACHE_HOME="$ci_root/cache"
export PIP_CACHE_DIR="$ci_root/cache/pip"
export PIP_NO_CACHE_DIR=1
export PIP_DISABLE_PIP_VERSION_CHECK=1
export PYTHONPYCACHEPREFIX="$ci_root/cache/python"
export PYTHONDONTWRITEBYTECODE=1
export CARGO_HOME="$ci_root/cache/cargo"
export CARGO_TARGET_DIR="$ci_root/cargo-target"
export CARGO_BUILD_BUILD_DIR="$ci_root/cargo-build/{workspace-path-hash}"
export CARGO_INCREMENTAL=0

external_root="$repo_root/.ait-external/ait-core"
marker="$external_root/.ait-external-marker.json"
test -f "$marker"
test -f "$external_root/rust/crates/ait-py/Cargo.toml"

declared_external=${AIT_EXTERNAL_CORE_REPO_ROOT:-$external_root}
declared_external=$(CDPATH= cd -- "$declared_external" && pwd -P)
materialized_external=$(CDPATH= cd -- "$external_root" && pwd -P)
if [ "$declared_external" != "$materialized_external" ]; then
  printf '%s\n' "AIT_EXTERNAL_CORE_REPO_ROOT does not match .ait-external/ait-core" >&2
  exit 1
fi

python3 - "$repo_root/ait-external.lock" "$marker" <<'PY'
import json
import pathlib
import sys
import tomllib

lock_path = pathlib.Path(sys.argv[1])
marker_path = pathlib.Path(sys.argv[2])
lock = tomllib.loads(lock_path.read_text(encoding="utf-8"))
marker = json.loads(marker_path.read_text(encoding="utf-8"))
nodes = lock.get("node", [])
if len(nodes) != 1:
    raise SystemExit("ait-external.lock must contain exactly one node")
node = nodes[0]
for field in ("name", "repo_name", "repository_index", "snapshot", "materialize_to"):
    if marker.get(field) != node.get(field):
        raise SystemExit(f"external marker field {field!r} does not match lock")
PY

cd "$repo_root"
python3 -m venv "$ci_root/venv"
"$ci_root/venv/bin/python" -m pip install --no-cache-dir '.[test]'
"$ci_root/venv/bin/python" -m pytest -p no:cacheprovider
"$ci_root/venv/bin/python" -m pip check
