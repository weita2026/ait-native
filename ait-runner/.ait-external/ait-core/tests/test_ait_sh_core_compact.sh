#!/usr/bin/env bash

set -euo pipefail

# The launcher exports its selected Cargo directories before invoking this
# regression from `core test`.  Fixtures below exercise repository defaults,
# so inherited caller selections must not escape into their isolated roots.
unset AIT_SHARED_CARGO_TARGET_DIR AIT_SHARED_CARGO_BUILD_DIR
unset CARGO_TARGET_DIR CARGO_BUILD_BUILD_DIR

SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEMP_ROOT="$(mktemp -d)"
trap 'rm -rf -- "${TEMP_ROOT}"' EXIT

write_fixture() {
  local path="$1"
  local contents="$2"
  mkdir -p "$(dirname "${path}")"
  printf '%s\n' "${contents}" > "${path}"
}

copy_launcher() {
  local fixture_root="$1"
  mkdir -p "${fixture_root}"
  cp "${SOURCE_ROOT}/ait.sh" "${fixture_root}/ait.sh"
  chmod +x "${fixture_root}/ait.sh"
}

assert_absent() {
  local path="$1"
  if [[ -e "${path}" ]]; then
    printf 'Expected compacted path to be absent: %s\n' "${path}" >&2
    exit 1
  fi
}

assert_present() {
  local path="$1"
  if [[ ! -e "${path}" ]]; then
    printf 'Expected retained path to remain: %s\n' "${path}" >&2
    exit 1
  fi
}

CANONICAL_FIXTURE="${TEMP_ROOT}/canonical"
copy_launcher "${CANONICAL_FIXTURE}"

OLD_HASH_LEAF="${CANONICAL_FIXTURE}/.ait/cargo-build/workspaces/aa/aaaaaaaaaaaaaa"
NEW_HASH_LEAF="${CANONICAL_FIXTURE}/.ait/cargo-build/workspaces/bb/bbbbbbbbbbbbbb"
CANONICAL_BUILD="${CANONICAL_FIXTURE}/.ait/cargo-build/canonical"
TASK_LEAF="${CANONICAL_FIXTURE}/.ait/cargo-build/task-workspaces/lct-fixture"
TASK_TARGET="${CANONICAL_FIXTURE}/.ait/cargo-target/task-workspaces/lct-fixture"
LEGACY_BUILD="${CANONICAL_FIXTURE}/.ait/cargo-build-rct-9999"
RUNNER_BUILD="${CANONICAL_FIXTURE}/.ait/generated/runner/cargo-build"
INVALID_HASH_LEAF="${CANONICAL_FIXTURE}/.ait/cargo-build/workspaces/not-a-shard/not-a-cache"

write_fixture "${OLD_HASH_LEAF}/ait-ci/deps/old.rlib" "old hash cache"
write_fixture "${NEW_HASH_LEAF}/release/deps/new.rlib" "new hash cache"
write_fixture "${CANONICAL_BUILD}/release/deps/canonical.rlib" "canonical cache"
write_fixture "${TASK_LEAF}/ait-ci/deps/task.rlib" "task cache"
write_fixture "${LEGACY_BUILD}/release/deps/legacy.rlib" "legacy build cache"
write_fixture "${RUNNER_BUILD}/aa/aaaaaaaaaaaaaa/ait-ci/deps/runner.rlib" "runner build cache"
write_fixture "${INVALID_HASH_LEAF}/must-remain" "not a Cargo hash leaf"

for target in \
  "${CANONICAL_FIXTURE}/.ait/cargo-target" \
  "${CANONICAL_FIXTURE}/rust/target" \
  "${CANONICAL_FIXTURE}/.ait-runtime/rct-0001-cargo-target" \
  "${CANONICAL_FIXTURE}/.ait/generated/runner/cargo-target"; do
  write_fixture "${target}/debug/deps/intermediate.rlib" "debug intermediate"
  write_fixture "${target}/release/deps/intermediate.rlib" "release intermediate"
  write_fixture "${target}/release/build/build-script/output" "build intermediate"
  write_fixture "${target}/release/.fingerprint/fingerprint" "fingerprint"
  write_fixture "${target}/release/ait-cli" "final release binary"
done
write_fixture "${TASK_TARGET}/debug/deps/intermediate.rlib" "Task debug intermediate"
write_fixture "${TASK_TARGET}/release/deps/intermediate.rlib" "Task release intermediate"
write_fixture "${TASK_TARGET}/release/ait-cli" "Task final release binary"

write_fixture "${CANONICAL_FIXTURE}/.ait/binary-db/authority.bin" "binary db authority"
write_fixture "${CANONICAL_FIXTURE}/.ait/objects/packs/pack.bin" "object pack authority"
write_fixture "${CANONICAL_FIXTURE}/.ait/generated/runner/tmp/must-remain" "unrelated generated data"
write_fixture "${CANONICAL_FIXTURE}/.ait-runtime/must-remain" "unrelated runtime data"
cp "${CANONICAL_FIXTURE}/.ait/binary-db/authority.bin" "${TEMP_ROOT}/authority.expected"
cp "${CANONICAL_FIXTURE}/.ait/objects/packs/pack.bin" "${TEMP_ROOT}/pack.expected"

DRY_RUN_OUTPUT="$("${CANONICAL_FIXTURE}/ait.sh" core compact --dry-run)"
printf '%s\n' "${DRY_RUN_OUTPUT}" | grep -F "${CANONICAL_BUILD}" >/dev/null
if printf '%s\n' "${DRY_RUN_OUTPUT}" | grep -F "${OLD_HASH_LEAF}" >/dev/null; then
  printf 'Former workspace-hash caches must remain explicit legacy inventory.\n' >&2
  exit 1
fi
if printf '%s\n' "${DRY_RUN_OUTPUT}" | grep -F "${TASK_LEAF}" >/dev/null; then
  printf 'Managed Task caches must remain opt-in during canonical compaction.\n' >&2
  exit 1
fi
if printf '%s\n' "${DRY_RUN_OUTPUT}" | grep -F "${LEGACY_BUILD}" >/dev/null; then
  printf 'Legacy caches must remain opt-in during canonical compaction.\n' >&2
  exit 1
fi
assert_present "${OLD_HASH_LEAF}/ait-ci/deps/old.rlib"
assert_present "${CANONICAL_BUILD}/release/deps/canonical.rlib"
assert_present "${CANONICAL_FIXTURE}/.ait/cargo-target/debug/deps/intermediate.rlib"
assert_present "${TASK_TARGET}/debug/deps/intermediate.rlib"

"${CANONICAL_FIXTURE}/ait.sh" core compact \
  --force --include-worktrees --include-legacy >/dev/null

assert_absent "${OLD_HASH_LEAF}/ait-ci/deps/old.rlib"
assert_absent "${NEW_HASH_LEAF}/release/deps/new.rlib"
assert_absent "${CANONICAL_BUILD}/release/deps/canonical.rlib"
assert_absent "${TASK_LEAF}/ait-ci/deps/task.rlib"
assert_absent "${LEGACY_BUILD}/release/deps/legacy.rlib"
assert_absent "${RUNNER_BUILD}/aa/aaaaaaaaaaaaaa/ait-ci/deps/runner.rlib"
assert_present "${OLD_HASH_LEAF}/.ait-gc-marker"
assert_present "${CANONICAL_BUILD}/.ait-gc-marker"
assert_present "${TASK_LEAF}/.ait-gc-marker"
assert_present "${INVALID_HASH_LEAF}/must-remain"
assert_absent "${TASK_TARGET}/debug"
assert_absent "${TASK_TARGET}/release/deps"
assert_present "${TASK_TARGET}/release/ait-cli"

for target in \
  "${CANONICAL_FIXTURE}/.ait/cargo-target" \
  "${CANONICAL_FIXTURE}/rust/target" \
  "${CANONICAL_FIXTURE}/.ait-runtime/rct-0001-cargo-target" \
  "${CANONICAL_FIXTURE}/.ait/generated/runner/cargo-target"; do
  assert_absent "${target}/debug"
  assert_absent "${target}/release/deps"
  assert_absent "${target}/release/build"
  assert_absent "${target}/release/.fingerprint"
  assert_present "${target}/release/ait-cli"
done

cmp "${TEMP_ROOT}/authority.expected" "${CANONICAL_FIXTURE}/.ait/binary-db/authority.bin"
cmp "${TEMP_ROOT}/pack.expected" "${CANONICAL_FIXTURE}/.ait/objects/packs/pack.bin"
assert_present "${CANONICAL_FIXTURE}/.ait/generated/runner/tmp/must-remain"
assert_present "${CANONICAL_FIXTURE}/.ait-runtime/must-remain"

TASK_FIXTURE="${TEMP_ROOT}/task"
copy_launcher "${TASK_FIXTURE}"
write_fixture "${TASK_FIXTURE}/.ait-worktree.json" '{"worktree_name":"lct-fixture"}'
mkdir -p "${TASK_FIXTURE}/.ait/cargo-build/task-workspaces/lct-fixture"
mkdir -p "${TASK_FIXTURE}/.ait/cargo-target/task-workspaces/lct-fixture"
SELECTED_TASK_TARGET_DIR="$(bash -c 'source "$1"; cargo_target_dir' _ \
  "${TASK_FIXTURE}/ait.sh")"
EXPECTED_TASK_TARGET_DIR="$(cd "${TASK_FIXTURE}/.ait/cargo-target/task-workspaces/lct-fixture" && pwd -P)"
if [[ "${SELECTED_TASK_TARGET_DIR}" != "${EXPECTED_TASK_TARGET_DIR}" ]]; then
  printf 'Managed Task launcher selected target %s instead of %s.\n' \
    "${SELECTED_TASK_TARGET_DIR}" "${EXPECTED_TASK_TARGET_DIR}" >&2
  exit 1
fi
SELECTED_TASK_BUILD_DIR="$(bash -c 'source "$1"; cargo_build_dir' _ \
  "${TASK_FIXTURE}/ait.sh")"
EXPECTED_TASK_BUILD_DIR="$(cd "${TASK_FIXTURE}/.ait/cargo-build/task-workspaces/lct-fixture" && pwd -P)"
if [[ "${SELECTED_TASK_BUILD_DIR}" != "${EXPECTED_TASK_BUILD_DIR}" ]]; then
  printf 'Managed Task launcher selected %s instead of %s.\n' \
    "${SELECTED_TASK_BUILD_DIR}" "${EXPECTED_TASK_BUILD_DIR}" >&2
  exit 1
fi

SECOND_TASK_FIXTURE="${TEMP_ROOT}/task-two"
copy_launcher "${SECOND_TASK_FIXTURE}"
ln -s "${TASK_FIXTURE}/.ait" "${SECOND_TASK_FIXTURE}/.ait"
write_fixture "${SECOND_TASK_FIXTURE}/.ait-worktree.json" '{"worktree_name":"lct-second"}'
mkdir -p "${SECOND_TASK_FIXTURE}/.ait/cargo-target/task-workspaces/lct-second"
SELECTED_SECOND_TASK_TARGET_DIR="$(bash -c 'source "$1"; cargo_target_dir' _ \
  "${SECOND_TASK_FIXTURE}/ait.sh")"
RESOLVED_TASK_TARGET_DIR="$(cd "${SELECTED_TASK_TARGET_DIR}" && pwd -P)"
RESOLVED_SECOND_TASK_TARGET_DIR="$(cd "${SELECTED_SECOND_TASK_TARGET_DIR}" && pwd -P)"
if [[ "${RESOLVED_TASK_TARGET_DIR}" == "${RESOLVED_SECOND_TASK_TARGET_DIR}" ]]; then
  printf 'Distinct managed Tasks must not share one Cargo target directory.\n' >&2
  exit 1
fi

SELECTED_CANONICAL_BUILD_DIR="$(bash -c 'source "$1"; cargo_build_dir' _ \
  "${CANONICAL_FIXTURE}/ait.sh")"
EXPECTED_CANONICAL_BUILD_DIR="$(cd "${CANONICAL_FIXTURE}/.ait/cargo-build" && pwd -P)/canonical"
if [[ "${SELECTED_CANONICAL_BUILD_DIR}" != "${EXPECTED_CANONICAL_BUILD_DIR}" ]]; then
  printf 'Canonical launcher selected %s instead of %s.\n' \
    "${SELECTED_CANONICAL_BUILD_DIR}" "${EXPECTED_CANONICAL_BUILD_DIR}" >&2
  exit 1
fi
SELECTED_CANONICAL_TARGET_DIR="$(bash -c 'source "$1"; cargo_target_dir' _ \
  "${CANONICAL_FIXTURE}/ait.sh")"
EXPECTED_CANONICAL_TARGET_DIR="$(cd "${CANONICAL_FIXTURE}/.ait/cargo-target" && pwd -P)"
if [[ "${SELECTED_CANONICAL_TARGET_DIR}" != "${EXPECTED_CANONICAL_TARGET_DIR}" ]]; then
  printf 'Canonical launcher selected target %s instead of %s.\n' \
    "${SELECTED_CANONICAL_TARGET_DIR}" "${EXPECTED_CANONICAL_TARGET_DIR}" >&2
  exit 1
fi

AUTO_FIXTURE="${TEMP_ROOT}/auto"
copy_launcher "${AUTO_FIXTURE}"
AUTO_OLD="${AUTO_FIXTURE}/.ait/cargo-build/workspaces/11/11111111111111"
AUTO_NEW="${AUTO_FIXTURE}/.ait/cargo-build/workspaces/22/22222222222222"
AUTO_CANONICAL="${AUTO_FIXTURE}/.ait/cargo-build/canonical"
AUTO_TARGET_ARTIFACT="${AUTO_FIXTURE}/.ait/cargo-target/release/ait-cli"
mkdir -p "${AUTO_OLD}" "${AUTO_NEW}" "${AUTO_CANONICAL}" "${TEMP_ROOT}/fake-bin"
dd if=/dev/zero of="${AUTO_OLD}/old.bin" bs=4096 count=2 2>/dev/null
dd if=/dev/zero of="${AUTO_NEW}/new.bin" bs=4096 count=2 2>/dev/null
write_fixture "${AUTO_CANONICAL}/current.bin" "current canonical cache"
write_fixture "${AUTO_TARGET_ARTIFACT}" "final binary"
touch -t 202001010000 "${AUTO_OLD}" "${AUTO_OLD}/old.bin"
touch -t 202501010000 "${AUTO_NEW}" "${AUTO_NEW}/new.bin"
write_fixture "${TEMP_ROOT}/fake-bin/lsof" '#!/usr/bin/env bash
exit 1'
chmod +x "${TEMP_ROOT}/fake-bin/lsof"

AIT_CARGO_BUILD_MAX_BYTES=16384 AIT_CARGO_BUILD_GC_INTERVAL_SECONDS=0 \
  bash -c 'source "$1"; PATH="$2:${PATH}"; auto_reclaim_cargo_build_dir "$(cargo_build_dir)"' \
  _ "${AUTO_FIXTURE}/ait.sh" "${TEMP_ROOT}/fake-bin" >/dev/null 2>&1
assert_absent "${AUTO_OLD}/old.bin"
assert_present "${AUTO_OLD}/.ait-gc-marker"
assert_present "${AUTO_NEW}/new.bin"
assert_present "${AUTO_CANONICAL}/current.bin"

dd if=/dev/zero of="${AUTO_CANONICAL}/oversized.bin" bs=4096 count=2 2>/dev/null
AIT_CARGO_BUILD_MAX_BYTES=1024 AIT_CARGO_BUILD_GC_INTERVAL_SECONDS=3600 \
  bash -c 'source "$1"; PATH="$2:${PATH}"; auto_reclaim_cargo_build_dir "$(cargo_build_dir)"' \
  _ "${AUTO_FIXTURE}/ait.sh" "${TEMP_ROOT}/fake-bin" >/dev/null 2>&1
assert_absent "${AUTO_CANONICAL}/oversized.bin"
assert_present "${AUTO_CANONICAL}/.ait-gc-marker"
assert_present "${AUTO_TARGET_ARTIFACT}"

write_fixture "${AUTO_CANONICAL}/opt-out.bin" "explicit opt out"
AIT_CARGO_BUILD_MAX_BYTES=0 AIT_CARGO_BUILD_GC_INTERVAL_SECONDS=0 \
  bash -c 'source "$1"; PATH="$2:${PATH}"; auto_reclaim_cargo_build_dir "$(cargo_build_dir)"' \
  _ "${AUTO_FIXTURE}/ait.sh" "${TEMP_ROOT}/fake-bin" >/dev/null 2>&1
assert_present "${AUTO_CANONICAL}/opt-out.bin"

printf 'ait.sh Cargo compaction regression: ok\n'
