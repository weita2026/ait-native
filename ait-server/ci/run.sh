#!/bin/sh
set -eu

mode="${1:-patchset}"
case "$mode" in
  patchset|repo|all) ;;
  *)
    echo "usage: ./ci/run.sh {patchset|repo|all}" >&2
    exit 2
    ;;
esac

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -n "${AIT_RUNNER_ATTEMPT_ROOT:-}" ]; then
  owned_root="${AIT_RUNNER_ATTEMPT_ROOT%/}/repository-ci"
  case "$owned_root" in
    */repository-ci) ;;
    *)
      echo "refusing unsafe repository CI root: $owned_root" >&2
      exit 2
      ;;
  esac
  mkdir -p "$owned_root"
else
  owned_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-server-ci.XXXXXX")
fi

cleanup() {
  case "$owned_root" in
    */repository-ci|*/ait-server-ci.*) rm -rf -- "$owned_root" ;;
    *) echo "refusing unsafe repository CI cleanup: $owned_root" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$owned_root/tmp" "$owned_root/cargo-target" "$owned_root/cargo-build"
export TMPDIR="$owned_root/tmp"
export CARGO_TARGET_DIR="$owned_root/cargo-target"
export CARGO_BUILD_BUILD_DIR="$owned_root/cargo-build"
export CARGO_INCREMENTAL=0
export AIT_TEST_DISABLE_GLOBAL_HOST_RAM_ROOT_CLEANUP=1

cd "$repo_root"

cargo fmt --manifest-path rust/Cargo.toml --all -- --check

python_file=$(
  find . -type f -name '*.py' \
    -not -path './.ait/*' \
    -not -path './.ait-runtime/*' \
    -not -path './.ait-external/*' \
    -not -path './rust/target/*' \
    -print -quit
)
if [ -n "$python_file" ]; then
  echo "Python source is forbidden in ait-server: $python_file" >&2
  exit 1
fi

run_patchset_gate() {
  cargo test \
    --manifest-path rust/Cargo.toml \
    --locked \
    --profile ait-ci \
    --no-run \
    -p ait-server-core \
    -p ait-server \
    --lib \
    --test seam_contract_direct_tests \
    --features ait-server-core/patch-ci-harness

  cargo test \
    --manifest-path rust/Cargo.toml \
    --locked \
    --profile ait-ci \
    -p ait-server-core \
    -p ait-server \
    --lib \
    --test seam_contract_direct_tests \
    --features ait-server-core/patch-ci-harness \
    --no-fail-fast
}

run_workspace_gate() {
  cargo check \
    --manifest-path rust/Cargo.toml \
    --locked \
    --workspace \
    --all-targets \
    --all-features

  cargo clippy \
    --manifest-path rust/Cargo.toml \
    --locked \
    --workspace \
    --all-targets \
    --all-features \
    -- \
    -D warnings

  cargo test \
    --manifest-path rust/Cargo.toml \
    --locked \
    --workspace \
    --all-targets \
    --no-fail-fast

  cargo test \
    --manifest-path rust/Cargo.toml \
    --locked \
    --workspace \
    --doc \
    --no-fail-fast

  cargo build \
    --manifest-path rust/Cargo.toml \
    --locked \
    --workspace \
    --release
}

case "$mode" in
  patchset)
    run_patchset_gate
    ;;
  repo)
    run_workspace_gate
    ;;
  all)
    run_patchset_gate
    run_workspace_gate
    ;;
esac
