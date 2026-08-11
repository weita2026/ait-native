#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
runtime_parent=${AIT_RUNNER_ATTEMPT_ROOT:-${TMPDIR:-/tmp}}
mkdir -p "$runtime_parent"
ci_root=$(mktemp -d "$runtime_parent/ait-runner-ci.XXXXXX")

cleanup() {
  rm -rf -- "$ci_root"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$ci_root/tmp" "$ci_root/cache/cargo" "$ci_root/build"
export TMPDIR="$ci_root/tmp"
export TMP="$ci_root/tmp"
export TEMP="$ci_root/tmp"
export CARGO_HOME="$ci_root/cache/cargo"
export CARGO_TARGET_DIR="$ci_root/build/cargo-target"
export CARGO_BUILD_BUILD_DIR="$ci_root/build/cargo-build"

cd "$repo_root"

run_fmt() {
  cargo fmt --all -- --check
}

run_clippy() {
  cargo clippy --workspace --all-targets -- -D warnings
}

run_tests() {
  cargo test --workspace --all-targets
}

case "${1:-patchset}" in
  fmt)
    run_fmt
    ;;
  clippy)
    run_clippy
    ;;
  test)
    run_tests
    ;;
  patchset|repo|all)
    run_fmt
    run_clippy
    run_tests
    ;;
  *)
    printf 'usage: %s {fmt|clippy|test|patchset|repo|all}\n' "$0" >&2
    exit 64
    ;;
esac
