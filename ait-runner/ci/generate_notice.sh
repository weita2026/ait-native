#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd "$script_dir/.." && pwd -P)
generator="$repo_root/.ait-external/ait-core/ci/generate_rust_notice.sh"

if [[ ! -f "$generator" ]]; then
  echo "materialized ait-core notice generator is missing: $generator" >&2
  echo "run 'ait external update --locked --validate' first" >&2
  exit 1
fi

exec bash "$generator" \
  --manifest "$repo_root/Cargo.toml" \
  --notice "$repo_root/NOTICE" \
  --project ait-runner \
  "$@"
