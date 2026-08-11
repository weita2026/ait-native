#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  printf '%s\n' \
    'usage: release_monorepo_export.sh <family-manifest> <source-bundles-root> <destination> <evidence-output>' >&2
  exit 64
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
family_manifest=$1
source_bundles_root=$2
destination=$3
evidence_output=$4
template_root=${repo_root}/release/monorepo
readme_template=${template_root}/README.template
transform_tool=${repo_root}/ci/release_monorepo_transform.mjs
product_document=${repo_root}/docs/distribution.md
if [[ ! -e ${product_document} && ! -L ${product_document} ]]; then
  product_document=${repo_root}/../docs/distribution.md
fi
protected_workflow=${repo_root}/.github/workflows/ait-release-component-receipts.yml
coordinator_snapshot=${AIT_RELEASE_COORDINATOR_SNAPSHOT:?AIT_RELEASE_COORDINATOR_SNAPSHOT is required}
coordinator_manifest_hash=${AIT_RELEASE_COORDINATOR_MANIFEST_HASH:?AIT_RELEASE_COORDINATOR_MANIFEST_HASH is required}
coordinator_created_at=${AIT_RELEASE_COORDINATOR_CREATED_AT:?AIT_RELEASE_COORDINATOR_CREATED_AT is required}

if [[ ! ${coordinator_snapshot} =~ ^SNP-[0-9A-F]{12}$ ]]; then
  printf 'coordinator Snapshot identity is invalid\n' >&2
  exit 64
fi
if [[ ! ${coordinator_manifest_hash} =~ ^[0-9a-f]{64}$ ||
  ! ${coordinator_created_at} =~ ^(0|[1-9][0-9]*)$ ]]; then
  printf 'coordinator Snapshot manifest or creation time is invalid\n' >&2
  exit 64
fi

for command in jq node tar; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    printf 'required command is unavailable: %s\n' "${command}" >&2
    exit 69
  fi
done
if [[ ! -f ${family_manifest} || -L ${family_manifest} ]]; then
  printf 'family manifest must be a regular file\n' >&2
  exit 66
fi
if [[ ! -d ${source_bundles_root} || -L ${source_bundles_root} ]]; then
  printf 'source bundle root must be a real directory\n' >&2
  exit 66
fi
for output in "${destination}" "${evidence_output}"; do
  case "${output}" in
    /*) ;;
    *)
      printf 'monorepo export outputs must use absolute paths\n' >&2
      exit 64
      ;;
  esac
  if [[ -e ${output} || -L ${output} ]]; then
    printf 'monorepo export output must not already exist: %s\n' "${output}" >&2
    exit 73
  fi
  if [[ ! -d $(dirname -- "${output}") ]]; then
    printf 'monorepo export output parent must already exist: %s\n' "${output}" >&2
    exit 73
  fi
done
if [[ ! -d ${template_root} || -L ${template_root} ||
  ! -f ${readme_template} || -L ${readme_template} ||
  ! -f ${transform_tool} || -L ${transform_tool} ||
  ! -f ${product_document} || -L ${product_document} ||
  ! -f ${protected_workflow} || -L ${protected_workflow} ]]; then
  printf 'monorepo release templates or transform tool are unavailable\n' >&2
  exit 66
fi

if ! jq -e '
  . as $root |
  .schema == "ait.release.family/v3" and
  .family.name == "ait-native" and
  .family.version == "1.0.0-rc.1" and
  .family.tag == "v1.0.0-rc.1" and
  .public_source.model == "release-monorepo" and
  .public_source.identity == "weita2026/ait-native" and
  .public_source.product_document == "docs/distribution.md" and
  .public_source.family_manifest == "ait-release-family.json" and
  .public_source.mapping_manifest == "ait-monorepo-source.json" and
  .public_source.build_entrypoints == {
    unix: "build-release.sh",
    windows: "build-release.ps1",
    implementation: "build-release.mjs"
  } and
  (.public_source.subtrees | length) == 5 and
  ([.public_source.subtrees[].source_repository] | sort) ==
    ["ait-core", "ait-node", "ait-python", "ait-runner", "ait-server"] and
  all(.public_source.subtrees[];
    .path == .source_repository and
    if .source_repository == "ait-runner" then
      .transforms == ["runner-core-path/v1"]
    elif .source_repository == "ait-python" then
      .transforms == ["python-core-path/v1"]
    else
      .transforms == []
    end) and
  .public_source.transforms == [
    {
      id: "runner-core-path/v1",
      source_repository: "ait-runner",
      path: "Cargo.toml",
      from: ".ait-external/ait-core/rust/crates/ait-core",
      to: "../ait-core/rust/crates/ait-core"
    },
    {
      id: "python-core-path/v1",
      source_repository: "ait-python",
      path: "pyproject.toml",
      from: ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml",
      to: "../ait-core/rust/crates/ait-py/Cargo.toml"
    }
  ] and
  ([.distributions[] | select(.channel == "github")] | length) == 1 and
  (.distributions[] | select(.channel == "github") |
    .role == "product" and
    .identity == "weita2026/ait-native" and
    ([.components[]] | sort) == ([ $root.components[].id ] | sort) and
    ([.targets[]] | sort) == ($root.targets | sort))
' "${family_manifest}" >/dev/null; then
  printf 'family manifest does not match the exact public monorepo export contract\n' >&2
  exit 65
fi

sha256_file() {
  local path=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${path}" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${path}" | awk '{print $1}'
  else
    printf 'no SHA-256 utility is available\n' >&2
    return 69
  fi
}

tree_digest() {
  local root=$1
  local label=$2
  local list=${temporary_root}/${label}.files
  local inventory=${temporary_root}/${label}.inventory
  local -a paths=()
  local file relative size digest
  while IFS= read -r -d '' file; do
    relative=${file#"${root}"/}
    if [[ ${relative} == *$'\n'* || ${relative} == *$'\r'* || ${relative} == *$'\t'* ]]; then
      printf 'source tree contains a control character in a path\n' >&2
      return 65
    fi
    paths+=("${relative}")
  done < <(find "${root}" -type f -print0)
  if [[ ${#paths[@]} -eq 0 ]]; then
    printf 'source tree is empty: %s\n' "${root}" >&2
    return 65
  fi
  printf '%s\n' "${paths[@]}" | LC_ALL=C sort >"${list}"
  : >"${inventory}"
  while IFS= read -r relative; do
    file=${root}/${relative}
    size=$(wc -c <"${file}" | tr -d '[:space:]')
    digest=$(sha256_file "${file}")
    printf '%s\t%s\t%s\n' "${size}" "${digest}" "${relative}" \
      >>"${inventory}"
  done <"${list}"
  sha256_file "${inventory}"
}

validate_archive() {
  local archive=$1
  local label=$2
  local members=${temporary_root}/${label}.members
  local verbose=${temporary_root}/${label}.verbose
  local member normalized line entry_type
  tar -tzf "${archive}" >"${members}"
  while IFS= read -r member; do
    if [[ ${member} == "." || ${member} == "./" ]]; then
      continue
    fi
    normalized=${member#./}
    if [[ -z ${normalized} || ${normalized} == /* || ${normalized} == ".." ||
      ${normalized} == ../* || ${normalized} == */../* || ${normalized} == */.. ||
      ${normalized} == *\\* || ${normalized} == *$'\r'* || ${normalized} == *$'\t'* ]]; then
      printf 'source archive contains an unsafe member: %s\n' "${member}" >&2
      return 65
    fi
  done <"${members}"
  tar -tvzf "${archive}" >"${verbose}"
  while IFS= read -r line; do
    entry_type=${line:0:1}
    if [[ ${entry_type} != '-' && ${entry_type} != 'd' ]]; then
      printf 'source archive contains a link or special member\n' >&2
      return 65
    fi
  done <"${verbose}"
}

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-monorepo-export.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-monorepo-export.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected monorepo export path: %s\n' "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

staging=${temporary_root}/public-source
mapping_rows=${temporary_root}/mapping-rows.jsonl
mkdir -p "${staging}"
: >"${mapping_rows}"

repositories=(ait-core ait-server ait-runner ait-python ait-node)
for repository in "${repositories[@]}"; do
  bundle=${source_bundles_root}/ait-release-source-${repository}
  archive=${bundle}/source-cache.tar.gz
  evidence=${bundle}/source-cache.evidence.json
  if [[ ! -f ${archive} || -L ${archive} || ! -f ${evidence} || -L ${evidence} ]]; then
    printf 'source cache bundle is incomplete for %s\n' "${repository}" >&2
    exit 66
  fi
  snapshot=$(jq -er --arg repository "${repository}" '
    [.components[] | select(.source_repository == $repository) | .source_snapshot]
      | unique | if length == 1 then .[0] else error("Snapshot conflict") end
  ' "${family_manifest}")
  license=$(jq -er --arg repository "${repository}" '
    [.components[] | select(.source_repository == $repository) | .license]
      | unique | if length == 1 then .[0] else error("license conflict") end
  ' "${family_manifest}")
  if ! jq -e \
    --arg repository "${repository}" \
    --arg snapshot "${snapshot}" \
    --arg license "${license}" '
      .contract == "ait.release.source-cache/v1" and
      .status == "ready" and
      .repo_name == $repository and
      .source_snapshot == $snapshot and
      (.source_manifest_hash | type == "string" and test("^[0-9a-f]{64}$")) and
      (.source_snapshot_created_at | type == "string" and test("^(0|[1-9][0-9]*)$")) and
      .license == $license and
      .source_authority == "ait_remote_snapshot_boundary" and
      .workspace_clean == true and
      .remote_coordinates_embedded == false and
      .public_publish == false
    ' "${evidence}" >/dev/null; then
    printf 'source cache evidence does not match %s at %s\n' \
      "${repository}" "${snapshot}" >&2
    exit 65
  fi
  validate_archive "${archive}" "${repository}"
  extracted=${temporary_root}/extracted-${repository}
  subtree=${staging}/${repository}
  mkdir -p "${extracted}" "${subtree}"
  tar -xzf "${archive}" -C "${extracted}"
  if find "${extracted}" \( -type l -o \! -type f -a \! -type d \) -print -quit |
    grep -q .; then
    printf 'source cache contains a link or special file for %s\n' "${repository}" >&2
    exit 65
  fi
  tar -cf - -C "${extracted}" \
    --exclude='.ait' --exclude='*/.ait' \
    --exclude='.ait-external' --exclude='*/.ait-external' \
    --exclude='.ait-runtime' --exclude='*/.ait-runtime' \
    --exclude='.ait-worktree-links' --exclude='*/.ait-worktree-links' \
    --exclude='.git' --exclude='*/.git' . |
    tar -xf - -C "${subtree}"
  if find "${subtree}" \
    \( -name .ait -o -name .ait-external -o -name .ait-runtime -o -name .ait-worktree-links -o -name .git -o -name .gitmodules \) \
    -print -quit | grep -q .; then
    printf 'forbidden operational or Git path escaped export filtering\n' >&2
    exit 65
  fi
  find "${subtree}" -type d -exec chmod 0755 {} +
  while IFS= read -r -d '' file; do
    if [[ -x ${file} ]]; then chmod 0755 "${file}"; else chmod 0644 "${file}"; fi
  done < <(find "${subtree}" -type f -print0)
  source_content_sha256=$(tree_digest "${subtree}" "${repository}-source")
  source_manifest_hash=$(jq -r '.source_manifest_hash' "${evidence}")
  source_snapshot_created_at=$(jq -r '.source_snapshot_created_at' "${evidence}")

  while IFS=$'\t' read -r transform_id transform_path transform_from transform_to; do
    [[ -n ${transform_id} ]] || continue
    node "${transform_tool}" \
      "${subtree}/${transform_path}" "${transform_from}" "${transform_to}"
  done < <(jq -r --arg repository "${repository}" '
    .public_source.transforms[] |
    select(.source_repository == $repository) |
    [.id, .path, .from, .to] | @tsv
  ' "${family_manifest}")
  exported_content_sha256=$(tree_digest "${subtree}" "${repository}-exported")
  evidence_sha256=$(sha256_file "${evidence}")
  jq -cn \
    --arg repository "${repository}" \
    --arg snapshot "${snapshot}" \
    --arg source_manifest_hash "${source_manifest_hash}" \
    --arg source_snapshot_created_at "${source_snapshot_created_at}" \
    --arg license "${license}" \
    --arg source_content_sha256 "${source_content_sha256}" \
    --arg exported_content_sha256 "${exported_content_sha256}" \
    --arg source_cache_evidence_sha256 "${evidence_sha256}" \
    --argjson components "$(jq -c --arg repository "${repository}" \
      '[.components[] | select(.source_repository == $repository) | .id] | sort' \
      "${family_manifest}")" \
    --argjson transforms "$(jq -c --arg repository "${repository}" '
      [.public_source.transforms[] | select(.source_repository == $repository) | .id]
    ' "${family_manifest}")" '
      {
        source_repository: $repository,
        source_snapshot: $snapshot,
        source_manifest_hash: $source_manifest_hash,
        source_snapshot_created_at: $source_snapshot_created_at,
        path: $repository,
        license: $license,
        components: $components,
        transforms: $transforms,
        source_cache_evidence_sha256: $source_cache_evidence_sha256,
        source_content_sha256: $source_content_sha256,
        exported_content_sha256: $exported_content_sha256
      }
    ' >>"${mapping_rows}"
done

mkdir -p \
  "${staging}/.github/workflows" \
  "${staging}/docs" \
  "${staging}/LICENSES"
cp "${readme_template}" "${staging}/README.md"
cp "${template_root}/NOTICE" "${staging}/NOTICE"
cp "${template_root}/.gitignore" "${staging}/.gitignore"
cp "${template_root}/build-release.sh" "${staging}/build-release.sh"
cp "${template_root}/build-release.ps1" "${staging}/build-release.ps1"
cp "${template_root}/build-release.mjs" "${staging}/build-release.mjs"
cp "${family_manifest}" "${staging}/ait-release-family.json"
cp "${product_document}" "${staging}/docs/distribution.md"
cp "${staging}/ait-core/LICENSE" "${staging}/LICENSES/Apache-2.0.txt"
cp "${staging}/ait-server/LICENSE" "${staging}/LICENSES/AGPL-3.0-only.txt"
root_workflow=${staging}/.github/workflows/ait-release-component-receipts.yml
cp "${protected_workflow}" "${root_workflow}"
node "${transform_tool}" \
  "${root_workflow}" \
  $'permissions:\n  contents: read\n\nconcurrency:' \
  $'permissions:\n  contents: read\n\ndefaults:\n  run:\n    working-directory: ait-core\n\nconcurrency:'
node "${transform_tool}" \
  "${root_workflow}" \
  '          path: release-receipt-matrix.json' \
  '          path: ait-core/release-receipt-matrix.json'
find "${staging}" -type d -exec chmod 0755 {} +
chmod 0644 \
  "${staging}/README.md" \
  "${staging}/NOTICE" \
  "${staging}/.gitignore" \
  "${staging}/build-release.ps1" \
  "${staging}/ait-release-family.json" \
  "${staging}/docs/distribution.md" \
  "${staging}/LICENSES/Apache-2.0.txt" \
  "${staging}/LICENSES/AGPL-3.0-only.txt" \
  "${root_workflow}"
chmod 0755 "${staging}/build-release.sh" "${staging}/build-release.mjs"

content_sha256=$(tree_digest "${staging}" monorepo-content)
family_manifest_sha256=$(sha256_file "${staging}/ait-release-family.json")
product_document_sha256=$(sha256_file "${staging}/docs/distribution.md")
subtrees=$(jq -s 'sort_by(.source_repository)' "${mapping_rows}")
mapping=${temporary_root}/ait-monorepo-source.json
jq -n \
  --arg schema 'ait.release.monorepo-source/v1' \
  --arg identity 'weita2026/ait-native' \
  --arg coordinator_snapshot "${coordinator_snapshot}" \
  --arg coordinator_manifest_hash "${coordinator_manifest_hash}" \
  --arg coordinator_created_at "${coordinator_created_at}" \
  --arg family_version '1.0.0-rc.1' \
  --arg family_tag 'v1.0.0-rc.1' \
  --arg family_manifest_sha256 "${family_manifest_sha256}" \
  --arg product_document_sha256 "${product_document_sha256}" \
  --arg content_sha256 "${content_sha256}" \
  --argjson subtrees "${subtrees}" '
    {
      schema: $schema,
      public_source_identity: $identity,
      coordinator_snapshot: $coordinator_snapshot,
      coordinator_manifest_hash: $coordinator_manifest_hash,
      coordinator_created_at: $coordinator_created_at,
      family_version: $family_version,
      family_tag: $family_tag,
      family_manifest_sha256: $family_manifest_sha256,
      product_document_sha256: $product_document_sha256,
      content_digest_contract: "size-sha256-path/v1; excludes ait-monorepo-source.json",
      content_sha256: $content_sha256,
      subtrees: $subtrees,
      excluded_operational_roots: [".ait", ".ait-external", ".ait-runtime", ".ait-worktree-links", ".git"],
      git_commit_created: false,
      public_publish: false
    }
  ' >"${mapping}"
cp "${mapping}" "${staging}/ait-monorepo-source.json"
chmod 0644 "${staging}/ait-monorepo-source.json"

node --check "${staging}/build-release.mjs"
node "${staging}/build-release.mjs" --validate-only
if find "${staging}" \( -type l -o -name .ait -o -name .ait-external -o -name .ait-runtime -o -name .ait-worktree-links -o -name .git -o -name .gitmodules \) \
  -print -quit | grep -q .; then
  printf 'final public source validation found a forbidden path\n' >&2
  exit 65
fi

mapping_sha256=$(sha256_file "${mapping}")
evidence=${temporary_root}/ait-monorepo-source.evidence.json
jq -n \
  --arg contract 'ait.release.monorepo-source-export/v1' \
  --arg identity 'weita2026/ait-native' \
  --arg coordinator_snapshot "${coordinator_snapshot}" \
  --arg coordinator_manifest_hash "${coordinator_manifest_hash}" \
  --arg coordinator_created_at "${coordinator_created_at}" \
  --arg family_version '1.0.0-rc.1' \
  --arg family_tag 'v1.0.0-rc.1' \
  --arg mapping_sha256 "${mapping_sha256}" \
  --arg content_sha256 "${content_sha256}" \
  --argjson subtrees "${subtrees}" '
    {
      contract: $contract,
      status: "ready",
      public_source_identity: $identity,
      coordinator_snapshot: $coordinator_snapshot,
      coordinator_manifest_hash: $coordinator_manifest_hash,
      coordinator_created_at: $coordinator_created_at,
      family_version: $family_version,
      family_tag: $family_tag,
      source_cache_count: ($subtrees | length),
      mapping_sha256: $mapping_sha256,
      content_sha256: $content_sha256,
      subtrees: $subtrees,
      git_commit_created: false,
      tag_created: false,
      registry_write: false,
      public_publish: false
    }
  ' >"${evidence}"

mv "${staging}" "${destination}"
mv "${evidence}" "${evidence_output}"
printf '%s\n' "${evidence_output}"
