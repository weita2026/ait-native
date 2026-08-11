#!/usr/bin/env sh
set -eu

release_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec node "${release_root}/build-release.mjs" "$@"
