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
  temporary_parent=$(cd "${TMPDIR:-/tmp}" && pwd -P)
  owned_root=$(mktemp -d "$temporary_parent/ait-core-ci.XXXXXX")
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
  "$owned_root/cargo-target" \
  "$owned_root/cargo-build"

export TMPDIR="$owned_root/tmp"
export CARGO_TARGET_DIR="$owned_root/cargo-target"
export CARGO_BUILD_BUILD_DIR="$owned_root/cargo-build/{workspace-path-hash}"
export CARGO_INCREMENTAL=0

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

run_patchset_tests() {
  cargo test \
    --manifest-path rust/Cargo.toml \
    --profile ait-ci \
    --locked \
    --all-features \
    -p ait-core \
    -p ait-cli \
    -p ait-agent-core \
    -p ait-agent-worker \
    -p ait-benchmark \
    -p ait-napi \
    -p ait-py \
    --lib \
    --test server_source_ownership \
    --test patchset_ci_runner \
    --no-run

  cargo test \
    --manifest-path rust/Cargo.toml \
    --profile ait-ci \
    --locked \
    --all-features \
    -p ait-core \
    -p ait-cli \
    -p ait-agent-core \
    -p ait-agent-worker \
    -p ait-benchmark \
    -p ait-napi \
    -p ait-py \
    --lib

  cargo test \
    --manifest-path rust/Cargo.toml \
    --profile ait-ci \
    --locked \
    --all-features \
    -p ait-core \
    -p ait-cli \
    --test server_source_ownership \
    --test patchset_ci_runner

  # Markdown is Plan lineage and is intentionally absent from remote Snapshot
  # materialization. Canonical source still carries the sole protected authority.
  if [ -f "$repo_root/docs/binary_db_v0.md" ]; then
    cargo test \
      --manifest-path rust/Cargo.toml \
      --profile ait-ci \
      --locked \
      -p ait-core \
      --test binary_db_schema_authority
  else
    printf '%s\n' "skipping binary_db_schema_authority: lineage-only Markdown is unavailable in this Snapshot"
  fi
}

run_repo_tests() {
  cargo test \
    --manifest-path rust/Cargo.toml \
    --profile ait-ci \
    --workspace \
    --all-targets \
    --all-features \
    --locked
}

run_clippy() {
  cargo clippy \
    --manifest-path rust/Cargo.toml \
    --workspace \
    --all-targets \
    --all-features \
    --locked \
    -- \
    -D warnings
}

case "$mode" in
  patchset)
    run_patchset_tests
    ;;
  repo)
    run_repo_tests
    ;;
  all)
    run_repo_tests
    run_clippy
    ;;
esac
