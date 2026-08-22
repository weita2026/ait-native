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

runtime_parent=${AIT_RUNNER_ATTEMPT_ROOT:-${TMPDIR:-/tmp}}
mkdir -p "$runtime_parent"
ci_root=$(mktemp -d "$runtime_parent/ait-node-ci.XXXXXX")

cleanup() {
  rm -rf -- "$ci_root"
}
trap cleanup 0 1 2 15

mkdir -p \
  "$ci_root/tmp" \
  "$ci_root/cache/npm" \
  "$ci_root/project"

export TMPDIR="$ci_root/tmp"
export TMP="$ci_root/tmp"
export TEMP="$ci_root/tmp"
export XDG_CACHE_HOME="$ci_root/cache"
export npm_config_cache="$ci_root/cache/npm"
export npm_config_audit=false
export npm_config_fund=false
export npm_config_update_notifier=false

project_root="$ci_root/project"
cp -R \
  "$repo_root/package.json" \
  "$repo_root/ait-release.json" \
  "$repo_root/ait-external.toml" \
  "$repo_root/ait-external.lock" \
  "$repo_root/LICENSE" \
  "$repo_root/NOTICE" \
  "$repo_root/bin" \
  "$repo_root/lib" \
  "$repo_root/release" \
  "$repo_root/scripts" \
  "$repo_root/src" \
  "$repo_root/test" \
  "$repo_root/ci" \
  "$project_root/"

cd "$project_root"
external_core=${AIT_EXTERNAL_CORE_REPO_ROOT:-$repo_root/.ait-external/ait-core}
if [ ! -f "$external_core/.ait-external-marker.json" ]; then
  printf '%s\n' "ait-node CI requires the exact materialized ait-core external" >&2
  exit 1
fi
node -e 'const fs=require("node:fs");const marker=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));if(marker.name!=="ait-core"||marker.snapshot!=="SNP-7F95342C97E7"){throw new Error("ait-core external marker identity drift")}' "$external_core/.ait-external-marker.json"
mkdir -p .ait-external
ln -s "$external_core" .ait-external/ait-core
bash ci/generate_notice.sh --check
npm run native:build
npm test
npm run check
node release/release-adapter.mjs build portable 1.0.1
node release/release-adapter.mjs smoke portable 1.0.1
