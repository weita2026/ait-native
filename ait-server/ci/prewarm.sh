#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
PATH="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"

cd "$ROOT_DIR/rust"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${AIT_SHARED_CARGO_TARGET_DIR:-${ROOT_DIR}/.ait/cargo-target}}"
cargo patch-ci-build
