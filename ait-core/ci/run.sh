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

ci_failed=0
stage_status=0
collect() {
  stage=$1
  shift
  printf 'CI RUN %s\n' "$stage"
  if "$@"; then
    stage_status=0
    printf 'CI PASS %s\n' "$stage"
  else
    stage_status=$?
    ci_failed=1
    printf 'CI FAIL %s (exit %s)\n' "$stage" "$stage_status" >&2
    if [ "$stage_status" -gt 128 ]; then
      cancel "$stage_status"
    fi
  fi
}

blocked() {
  ci_failed=1
  printf 'CI BLOCKED %s\n' "$1" >&2
}

cancel() {
  # Cancellation is terminal; only the EXIT trap may clean up after it.
  trap '' 1 2 15
  blocked "remaining Cargo stages: cancelled (exit $1)"
  exit "$1"
}

cleanup_owned_root=0
if [ -n "${AIT_RUNNER_ATTEMPT_ROOT:-}" ]; then
  owned_root=$AIT_RUNNER_ATTEMPT_ROOT
else
  if ! temporary_parent=$(cd "${TMPDIR:-/tmp}" && pwd -P) ||
     ! owned_root=$(mktemp -d "$temporary_parent/ait-core-ci.XXXXXX"); then
    blocked "all Cargo stages: temporary root unavailable"
    exit 1
  fi
  cleanup_owned_root=1
fi

cleanup() {
  trap '' 1 2 15
  if [ "$cleanup_owned_root" -eq 1 ]; then
    cleanup_owned_root=0
    # Fixture seeds can contain readonly directories in this exclusive mktemp tree.
    # Physical find traversal excludes symlinks and leaves their targets unchanged.
    find "$owned_root" -type d -exec chmod u+rwx {} +
    rm -rf -- "$owned_root"
  fi
}
trap cleanup 0
trap 'cancel 129' 1
trap 'cancel 130' 2
trap 'cancel 143' 15

if mkdir -p \
  "$owned_root/tmp" \
  "$owned_root/cargo-target" \
  "$owned_root/cargo-build"; then
  :
else
  blocked "all Cargo stages: filesystem setup failed"
  exit 1
fi

export TMPDIR="$owned_root/tmp"
export CARGO_TARGET_DIR="$owned_root/cargo-target"
export CARGO_BUILD_BUILD_DIR="$owned_root/cargo-build/{workspace-path-hash}"
export CARGO_INCREMENTAL=0
# CI links executable binding tests, even if its caller built an extension.
unset PYO3_BUILD_EXTENSION_MODULE

cd "$repo_root"

collect format cargo fmt --manifest-path rust/Cargo.toml --all -- --check

if ! python_file=$(
  find . -type f -name '*.py' \
    -not -path './.ait/*' \
    -not -path './.git/*' \
    -not -path './target/*' \
    -print -quit
); then
  blocked "all test/clippy stages: zero-Python scan failed"
  exit 1
fi
if [ -n "$python_file" ]; then
  printf 'zero-Python boundary violation: %s\n' "$python_file" >&2
  blocked "all test/clippy stages: zero-Python boundary violation"
  exit 1
fi

run_patchset_tests() {
  collect patchset-build cargo test \
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
    --test workflow_recovery_cli \
    --test snapshot_task_binding_cli \
    --test workflow_friction_cli \
    --test plan_cli \
    --test workflow_package_frozen_candidate_cli \
    --test workflow_package_accepted_retry_cli \
    --test team_session_cli \
    --no-run
  if [ "$stage_status" -eq 0 ]; then
    collect patchset-libraries cargo test \
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
      --no-fail-fast

    collect patchset-integrations cargo test \
      --manifest-path rust/Cargo.toml \
      --profile ait-ci \
      --locked \
      --all-features \
      -p ait-core \
      -p ait-cli \
      --test server_source_ownership \
      --test patchset_ci_runner \
      --test workflow_recovery_cli \
      --test snapshot_task_binding_cli \
      --test workflow_friction_cli \
      --test plan_cli \
      --test workflow_package_frozen_candidate_cli \
      --test workflow_package_accepted_retry_cli \
      --no-fail-fast

    collect team-create cargo test \
      --manifest-path rust/Cargo.toml \
      --profile ait-ci \
      --locked \
      --all-features \
      -p ait-cli \
      --test team_session_cli \
      concurrent_creation_then_retry_does_not_create_duplicate -- --exact
    collect team-recovery cargo test \
      --manifest-path rust/Cargo.toml \
      --profile ait-ci \
      --locked \
      --all-features \
      -p ait-cli \
      --test team_session_cli \
      concurrent_missing_recovery_then_retry_does_not_create_duplicate -- --exact
  else
    blocked "patchset-libraries, patchset-integrations, team-create, team-recovery: patchset-build failed"
  fi

  # Markdown is Plan lineage and is intentionally absent from remote Snapshot
  # materialization. Canonical source still carries the sole protected authority.
  if [ -f "$repo_root/docs/binary_db_v0.md" ]; then
    collect schema-build cargo test \
      --manifest-path rust/Cargo.toml \
      --profile ait-ci \
      --locked \
      -p ait-core \
      --test binary_db_schema_authority \
      --no-run
    if [ "$stage_status" -eq 0 ]; then
      collect schema-tests cargo test \
        --manifest-path rust/Cargo.toml \
        --profile ait-ci \
        --locked \
        -p ait-core \
        --test binary_db_schema_authority
    else
      blocked "schema-tests: schema-build failed"
    fi
  else
    printf '%s\n' "skipping binary_db_schema_authority: lineage-only Markdown is unavailable in this Snapshot"
  fi
}

run_repo_tests() {
  collect repo-build cargo test \
    --manifest-path rust/Cargo.toml \
    --profile ait-ci \
    --workspace \
    --all-targets \
    --all-features \
    --locked \
    --no-run
  if [ "$stage_status" -eq 0 ]; then
    collect repo-tests cargo test \
      --manifest-path rust/Cargo.toml \
      --profile ait-ci \
      --workspace \
      --all-targets \
      --all-features \
      --locked \
      --no-fail-fast
  else
    blocked "repo-tests: repo-build failed"
  fi
}

run_clippy() {
  collect clippy cargo clippy \
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

exit "$ci_failed"
