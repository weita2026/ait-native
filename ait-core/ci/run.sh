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

cleanup_owned_root=0
if [ -n "${AIT_RUNNER_ATTEMPT_ROOT:-}" ]; then
  owned_root=$AIT_RUNNER_ATTEMPT_ROOT
else
  owned_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-core-ci.XXXXXX")
  cleanup_owned_root=1
fi

cleanup() {
  if [ "$cleanup_owned_root" -eq 1 ]; then
    rm -rf -- "$owned_root"
  fi
}
trap cleanup 0 1 2 15

mkdir -p \
  "$owned_root/tmp" \
  "$owned_root/test-outside" \
  "$owned_root/cargo-target" \
  "$owned_root/cargo-build"

export TMPDIR="$owned_root/tmp"
export CARGO_TARGET_DIR="$owned_root/cargo-target"
export CARGO_BUILD_BUILD_DIR="$owned_root/cargo-build/{workspace-path-hash}"
export CARGO_INCREMENTAL=0
export AIT_TEST_DISABLE_GLOBAL_HOST_RAM_ROOT_CLEANUP=1
export AIT_TEST_OUTSIDE_REPO_TMP="$owned_root/test-outside"

cd "$repo_root"

cargo fmt --manifest-path rust/Cargo.toml --all -- --check

python_file=$(
  find . -type f -name '*.py' \
    -not -path './.ait/*' \
    -not -path './.git/*' \
    -not -path './target/*' \
    -print -quit
)
if [ -n "$python_file" ]; then
  printf 'zero-Python boundary violation: %s\n' "$python_file" >&2
  exit 1
fi

cargo test \
  --manifest-path rust/Cargo.toml \
  --profile ait-ci \
  -p ait-agent-core \
  -p ait-py \
  -p ait-cli \
  --lib \
  --bin ait-cli \
  --test patchset_ci_smoke_cli \
  --no-run

cargo test --manifest-path rust/Cargo.toml --profile ait-ci -p ait-agent-core --lib
cargo test --manifest-path rust/Cargo.toml --profile ait-ci -p ait-py --lib
cargo test \
  --manifest-path rust/Cargo.toml \
  --profile ait-ci \
  -p ait-cli \
  --test patchset_ci_smoke_cli
