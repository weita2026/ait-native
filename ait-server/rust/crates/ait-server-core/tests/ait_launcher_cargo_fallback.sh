#!/usr/bin/env bash
# Fixtures intentionally isolate state and override indirectly called helpers.
# shellcheck disable=SC2030,SC2031,SC2329
set -euo pipefail

source_script="$1"
# shellcheck source=/dev/null
source "$source_script"
unset AIT_SHARED_CARGO_TARGET_DIR AIT_SHARED_CARGO_BUILD_DIR CARGO_TARGET_DIR CARGO_BUILD_BUILD_DIR
unset AIT_CARGO_BUILD_MAX_BYTES AIT_CARGO_BUILD_GC_INTERVAL_SECONDS
temporary_root="$(mktemp -d)"
temporary_root="$(cd "$temporary_root" && pwd -P)"
trap 'rm -rf -- "$temporary_root"' EXIT HUP INT TERM
ROOT_DIR="$temporary_root/repo"
ram="$temporary_root/ram space"
primary="$ram/ait-runtime/cargo-build/fixture"
fallback="$ROOT_DIR/.ait/cargo-build-ssd"
mkdir -p "$ROOT_DIR/.ait" "$primary" "$ROOT_DIR/.ait/worktrees"
printf '{"repo_name":"fixture","task_worktree":{"memory_root":{"root":"%s"}}}\n' "$ram" >"$ROOT_DIR/.ait/config.json"
ln -s "$primary" "$ROOT_DIR/.ait/cargo-build"
printf keep >"$primary/other-build"
test_capacity=8589934592
cargo_available_bytes() {
  test "$1" = "$primary"
  [[ "$test_capacity" != unknown ]] || return 1
  printf '%s\n' "$test_capacity"
}

(
  AIT_CARGO_BUILD_MAX_BYTES=0
  test "$(cargo_ram_headroom)" = 4294967296
  test "$(cargo_build_dir)" = "$primary/canonical"
  test_capacity=1
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test_capacity=unknown
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test_capacity=8589934592
  AIT_CARGO_BUILD_MAX_BYTES=9663676416
  test "$(cargo_build_dir)" = "$fallback/canonical"
  printf '{"worktree_name":"lset-fixture"}\n' >"$ROOT_DIR/.ait-worktree.json"
  test "$(cargo_build_dir)" = "$fallback/task-workspaces/lset-fixture"
  test "$(cargo_target_dir)" = "$ROOT_DIR/.ait/cargo-target/task-workspaces/lset-fixture"
  rm "$ROOT_DIR/.ait-worktree.json"
  test "$(readlink "$ROOT_DIR/.ait/cargo-build")" = "$primary"
  test "$(cat "$primary/other-build")" = keep
)
(
  cargo_memory_mount_available() { return 1; }
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test "$(readlink "$ROOT_DIR/.ait/cargo-build")" = "$primary"
)
(
  printf '{"task_worktree":{"memory_root":{"root":"%s","kind":"linux_tmpfs"}}}\n' "$ram" >"$ROOT_DIR/.ait/config.json"
  test_memory_kind=ext4
  df() { printf 'Filesystem Type 1K-blocks Used Available Use%% Mounted\nfixture %s 16777216 0 8388608 0%% /\n' "$test_memory_kind"; }
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test_memory_kind=tmpfs
  test "$(cargo_build_dir)" = "$primary/canonical"
  printf '{"task_worktree":{"memory_root":{"root":"%s","kind":"macos_ram_volume"}}}\n' "$ram" >"$ROOT_DIR/.ait/config.json"
  test_mount=/
  df() { printf 'Filesystem 1K-blocks Used Available Use%% Mounted\nfixture 16777216 0 8388608 0%% %s\n' "$test_mount"; }
  if cargo_memory_mount_available '/Volumes/fixture volume'; then exit 65; fi
  test_mount='/Volumes/fixture volume'
  cargo_memory_mount_available "$test_mount"
)
printf '{"task_worktree":{"memory_root":{"root":"%s"}}}\n' "$ram" >"$ROOT_DIR/.ait/config.json"
(
  rm "$ROOT_DIR/.ait/cargo-build"
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test ! -e "$ROOT_DIR/.ait/cargo-build"
  test ! -L "$ROOT_DIR/.ait/cargo-build"
  ln -s "$temporary_root/unavailable-ram/build" "$ROOT_DIR/.ait/cargo-build"
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test ! -e "$temporary_root/unavailable-ram"
  rm "$ROOT_DIR/.ait/cargo-build"
  mkdir -p "$ROOT_DIR/.ait/cargo-build"
  printf keep >"$ROOT_DIR/.ait/cargo-build/ssd-cache"
  test_capacity=unknown
  test "$(cargo_build_dir)" = "$ROOT_DIR/.ait/cargo-build/canonical"
  test "$(cat "$ROOT_DIR/.ait/cargo-build/ssd-cache")" = keep
  rm -rf "$ROOT_DIR/.ait/cargo-build"
  ln -s "$primary" "$ROOT_DIR/.ait/cargo-build"
)
(
  CARGO_BUILD_BUILD_DIR="$temporary_root/explicit-ssd"
  CARGO_TARGET_DIR="$temporary_root/explicit-target"
  test_capacity=unknown
  test "$(cargo_build_dir)" = "$CARGO_BUILD_BUILD_DIR"
  test "$(cargo_target_dir)" = "$CARGO_TARGET_DIR"
  AIT_SHARED_CARGO_BUILD_DIR="$temporary_root/shared-ssd"
  AIT_SHARED_CARGO_TARGET_DIR="$temporary_root/shared-target"
  test "$(cargo_build_dir)" = "$AIT_SHARED_CARGO_BUILD_DIR"
  test "$(cargo_target_dir)" = "$AIT_SHARED_CARGO_TARGET_DIR"
)
(
  test_capacity=1
  mkdir -p "$temporary_root/foreign"
  ln -s "$temporary_root/foreign" "$fallback"
  if cargo_build_dir; then exit 65; fi
  test "$(readlink "$fallback")" = "$temporary_root/foreign"
  rm "$fallback"
  mkdir -p "$fallback/task-workspaces"
  ln -s "$temporary_root/foreign" "$fallback/task-workspaces/lset-fixture"
  printf '{"worktree_name":"lset-fixture"}\n' >"$ROOT_DIR/.ait-worktree.json"
  if cargo_build_dir; then exit 65; fi
  rm "$fallback/task-workspaces/lset-fixture" "$ROOT_DIR/.ait-worktree.json"
  rm -rf "$fallback"
)
(
  printf '{"task_worktree":{"memory_root":{"r\\u006fot":"%s"}}}\n' "$ram" >"$ROOT_DIR/.ait/config.json"
  test "$(cargo_memory_root)" = "$ram"
  printf '{"task_worktree":{"memory_root":{"root":"%s"}}}\n' "$ROOT_DIR" >"$ROOT_DIR/.ait/config.json"
  test_capacity=1
  if cargo_build_dir; then exit 65; fi
)
printf '{"task_worktree":{"memory_root":{"root":"%s"}}}\n' "$ram" >"$ROOT_DIR/.ait/config.json"

mkdir -p "$temporary_root/bin"
cat >"$temporary_root/bin/cargo" <<'CARGO'
#!/usr/bin/env bash
set -euo pipefail
printf '%s|%s|%s|' "$CARGO_BUILD_BUILD_DIR" "$CARGO_TARGET_DIR" "$CARGO_INCREMENTAL" >>"$TEST_CARGO_LOG"
printf '%q ' "$@" >>"$TEST_CARGO_LOG"
printf '\n' >>"$TEST_CARGO_LOG"
printf 'original cargo stdout\n'
printf 'ordinary stdout mentions ENOSPC and No space left on device\n'
[[ -z "$TEST_CARGO_DIAGNOSTIC" ]] || printf '%s\n' "$TEST_CARGO_DIAGNOSTIC" >&2
if [[ "$CARGO_BUILD_BUILD_DIR" == "$TEST_CARGO_RAM_BUILD_DIR" ]]; then
  exit "$TEST_CARGO_RAM_STATUS"
fi
exit "$TEST_CARGO_SSD_STATUS"
CARGO
chmod +x "$temporary_root/bin/cargo"
resolve_cargo() { printf '%s\n' "$temporary_root/bin/cargo"; }
resolve_rust_tool() { :; }
export TEST_CARGO_LOG="$temporary_root/cargo.log"
export TEST_CARGO_RAM_BUILD_DIR="$primary/canonical"
export TEST_CARGO_DIAGNOSTIC TEST_CARGO_RAM_STATUS TEST_CARGO_SSD_STATUS

# Exercise the production helper through run_cargo for every diagnostic.
# The Server card retains bare ENOSPC write failures and requires complete
# tokens for all alternatives, including the established no-space phrase.
diagnostic_cases=(
  'no_space|1|error: failed to build archive: No space left on device'
  'os_error28|1|error: failed to write archive (os error 28)'
  'os_error_punctuation|1|error: failed to create archive (os error 28), stopping'
  'enospc|1|error: failed to write artifact: ENOSPC'
  'enospc_punctuation|1|error: failed to create artifact (ENOSPC), stopping'
  'darwin|1|cargo:warning=write64() failed, returned -1 expected 919616, errno=28'
  'darwin_direct|1|write64() failed, returned -1 expected 919616, errno=28'
  'darwin_whitespace|1|cargo:warning=write64()   failed,  returned  -1  expected  919616,  errno=28)'
  'os_error280|0|error: failed to write archive (os error 280)'
  'os_error281|0|error: failed to write archive (os error 281)'
  'os_error128|0|error: failed to write archive (os error 128)'
  'os_error1280|0|error: failed to write archive (os error 1280)'
  'os_error_symbol|0|error: failed to write archive (os error 28_fixture)'
  'os_error_prefix|0|error: failed to write archive (xos error 28)'
  'errno280|0|cargo:warning=write64() failed, returned -1 expected 919616, errno=280'
  'errno281|0|cargo:warning=write64() failed, returned -1 expected 919616, errno=281'
  'errno128|0|cargo:warning=write64() failed, returned -1 expected 919616, errno=128'
  'errno1280|0|cargo:warning=write64() failed, returned -1 expected 919616, errno=1280'
  'errno_symbol|0|cargo:warning=write64() failed, returned -1 expected 919616, errno=28_fixture'
  'errno_other|0|cargo:warning=write64() failed, returned -1 expected 919616, errno=5'
  'darwin_unrelated_errno|0|error: fixture expected errno=28 from write64()'
  'darwin_quoted_failure|0|error: fixture expected write64() failed with errno=28'
  'darwin_wrong_return|0|cargo:warning=write64() failed, returned 0 expected 919616, errno=28'
  'darwin_wrong_function|0|cargo:warning=write() failed, returned -1 expected 919616, errno=28'
  'darwin_prefix|0|xcargo:warning=write64() failed, returned -1 expected 919616, errno=28'
  'enospc_suffix|0|error: failed to write artifact: ENOSPC280'
  'enospc_prefix|0|error: failed to write artifact: XENOSPC'
  'enospc_symbol|0|error: failed to write artifact: ENOSPC_TEST'
  'no_space_suffix|0|error: failed to write archive: No space left on devices'
  'no_space_prefix|0|error: failed to write archive: XNo space left on device'
  'compiler_symbol|0|error: cannot find value ENOSPC in this scope'
  'ordinary_test|0|error: assertion failed: expected 28, actual 29'
  'exit28_only|0|'
)

exercise_cargo_case() (
    local expected_attempts="$1" expected_status="$2"
    local expected_build="${3:-$primary/canonical}" status=0 attempts expected_args
    export AIT_CARGO_BUILD_MAX_BYTES=0
    : >"$TEST_CARGO_LOG"
    run_cargo build --profile release --features 'one two' --locked >"$temporary_root/stdout" 2>"$temporary_root/stderr" || status=$?
    attempts="$(wc -l <"$TEST_CARGO_LOG" | tr -d ' ')"
    test "$(grep -c '^original cargo stdout$' "$temporary_root/stdout")" = "$attempts"
    if grep -F 'original cargo stdout' "$temporary_root/stderr"; then exit 65; fi
    if [[ -n "$TEST_CARGO_DIAGNOSTIC" ]]; then
      test "$(grep -Fxc -- "$TEST_CARGO_DIAGNOSTIC" "$temporary_root/stderr")" = "$attempts"
    else
      test ! -s "$temporary_root/stderr"
    fi
    expected_args="$(printf '%q ' build --profile release --features 'one two' --locked)"
    test "$(sed -n '1p' "$TEST_CARGO_LOG" | cut -d '|' -f 4-)" = "$expected_args"
    test "$(sed -n '1p' "$TEST_CARGO_LOG" | cut -d '|' -f 2,3)" = "$ROOT_DIR/.ait/cargo-target|0"
    if [[ "$status" != "$expected_status" || "$attempts" != "$expected_attempts" ]]; then
      printf '%s: expected status=%s attempts=%s, got status=%s attempts=%s\n' \
        "$case_name" "$expected_status" "$expected_attempts" "$status" "$attempts" >&2
      exit 65
    fi
    test "$(sed -n '1p' "$TEST_CARGO_LOG" | cut -d '|' -f 1)" = "$expected_build"
    test "$(cut -d '|' -f 2- "$TEST_CARGO_LOG" | sort -u | wc -l | tr -d ' ')" = 1
    if [[ "$attempts" = 2 ]]; then
      sed -n '1p' "$TEST_CARGO_LOG" | grep -F "$primary/canonical|" >/dev/null
      sed -n '2p' "$TEST_CARGO_LOG" | grep -F "$fallback/canonical|" >/dev/null
    fi
    test "$(readlink "$ROOT_DIR/.ait/cargo-build")" = "$primary"
    test "$(cat "$primary/other-build")" = keep
)

for diagnostic_case in "${diagnostic_cases[@]}"; do
  IFS='|' read -r case_name expected_space TEST_CARGO_DIAGNOSTIC <<< "$diagnostic_case"
  TEST_CARGO_RAM_STATUS=28
  TEST_CARGO_SSD_STATUS=0
  if [[ "$expected_space" == 1 ]]; then
    for ssd_status in 0 28 29; do
      TEST_CARGO_SSD_STATUS="$ssd_status"
      exercise_cargo_case 2 "$ssd_status"
    done
    TEST_CARGO_RAM_STATUS=0 TEST_CARGO_SSD_STATUS=28 exercise_cargo_case 1 0
    CARGO_BUILD_BUILD_DIR="$temporary_root/explicit-ssd" TEST_CARGO_SSD_STATUS=28 \
      exercise_cargo_case 1 28 "$temporary_root/explicit-ssd"
  else
    exercise_cargo_case 1 28
    TEST_CARGO_RAM_STATUS=23 exercise_cargo_case 1 23
  fi
done
(
  test_capacity=1
  lsof() { return 1; }
  mkdir -p "$fallback/canonical" "$fallback/task-workspaces/registered" "$fallback/task-workspaces/foreign"
  printf '{"name":"registered","repo_root":"%s"}\n' "$ROOT_DIR" >"$ROOT_DIR/.ait/worktrees/registered.json"
  compact_cargo_targets --dry-run --include-worktrees >"$temporary_root/compact.out"
  grep -F "$fallback/canonical" "$temporary_root/compact.out" >/dev/null
  grep -F "$fallback/task-workspaces/registered" "$temporary_root/compact.out" >/dev/null
  if grep -F "$fallback/task-workspaces/foreign" "$temporary_root/compact.out"; then exit 65; fi
)
(
  export AIT_CARGO_BUILD_MAX_BYTES=1 AIT_CARGO_BUILD_GC_INTERVAL_SECONDS=0
  lsof() { return 1; }
  printf reclaim >"$fallback/canonical/idle"
  printf keep >"$fallback/task-workspaces/foreign/cache"
  auto_reclaim_cargo_build_dir "$fallback/canonical"
  test ! -e "$fallback/canonical/idle"
  printf active >"$fallback/canonical/active"
  lsof() { printf '%s\n' "$$"; }
  auto_reclaim_cargo_build_dir "$fallback/canonical"
  test "$(cat "$fallback/canonical/active")" = active
  test "$(cat "$fallback/task-workspaces/foreign/cache")" = keep
  test "$(cat "$primary/other-build")" = keep
)

# Invoke the actual command entrypoint with a controlled capacity probe.
cp "$source_script" "$ROOT_DIR/ait.sh"
cat >"$temporary_root/bin/df" <<'DF'
#!/usr/bin/env bash
printf 'Filesystem 1024-blocks Used Available Capacity Mounted\nfixture 16777216 0 8388608 0%% /\n'
DF
printf '#!/usr/bin/env bash\nexit 1\n' >"$temporary_root/bin/rustup"
chmod +x "$temporary_root/bin/df" "$temporary_root/bin/rustup"
mkdir -p "$ROOT_DIR/.ait/cargo-target/release"
printf original >"$ROOT_DIR/.ait/cargo-target/release/ait-server"
: >"$TEST_CARGO_LOG"
TEST_CARGO_DIAGNOSTIC='cargo:warning=write64() failed, returned -1 expected 919616, errno=28' \
  TEST_CARGO_RAM_STATUS=28 TEST_CARGO_SSD_STATUS=0 CARGO_HOME="$temporary_root" AIT_CARGO_BUILD_MAX_BYTES=0 \
  bash "$ROOT_DIR/ait.sh" core build
test "$(wc -l <"$TEST_CARGO_LOG" | tr -d ' ')" = 2
test "$(cat "$ROOT_DIR/.ait/cargo-target/release/ait-server")" = original
test "$(readlink "$ROOT_DIR/.ait/cargo-build")" = "$primary"
printf 'ait-server Cargo RAM/SSD fallback: pass\n'
