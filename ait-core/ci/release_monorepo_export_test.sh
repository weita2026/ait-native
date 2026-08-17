#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
product_document=${repo_root}/docs/distribution.md
if [[ ! -e ${product_document} && ! -L ${product_document} ]]; then
  product_document=${repo_root}/../docs/distribution.md
fi
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-monorepo-export-test.XXXXXX")
selftest_mode=0
if [[ ${1:-} == --public-layout-selftest ]]; then
  selftest_mode=1
  shift
fi
if (( $# != 0 )); then
  printf 'usage: %s [--public-layout-selftest]\n' "$0" >&2
  exit 64
fi
export AIT_RELEASE_COORDINATOR_SNAPSHOT=SNP-AAAAAAAAAAAA
export AIT_RELEASE_COORDINATOR_MANIFEST_HASH=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
export AIT_RELEASE_COORDINATOR_CREATED_AT=1700000000

node --test "${repo_root}/release/monorepo/build-release.test.mjs"

cleanup() {
  case "${temporary_root}" in
    "${TMPDIR:-/tmp}"/ait-monorepo-export-test.*)
      rm -rf -- "${temporary_root}"
      ;;
    *)
      printf 'refusing to remove unexpected monorepo test path: %s\n' "${temporary_root}" >&2
      return 1
      ;;
  esac
}
trap cleanup EXIT HUP INT TERM

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" 2>"${temporary_root}/${label}.stderr"; then
    printf 'expected monorepo export failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

if (( selftest_mode == 0 )); then
  public_layout=${temporary_root}/public-layout
  public_core=${public_layout}/ait-core
  mkdir -p \
    "${public_core}/ci" \
    "${public_core}/release" \
    "${public_core}/release/oci" \
    "${public_core}/.github/workflows" \
    "${public_layout}/docs"
  cp "${repo_root}/ait-release-family.json" "${public_core}/ait-release-family.json"
  cp "${repo_root}/LICENSE" "${repo_root}/NOTICE" "${public_core}/"
  cp "${repo_root}/ci/release_monorepo_export.sh" \
    "${repo_root}/ci/release_monorepo_export_test.sh" \
    "${repo_root}/ci/native_bootstrap_matrix.jq" \
    "${repo_root}/ci/native_bootstrap_matrix.json" \
    "${repo_root}/ci/release_clean_host.mjs" \
    "${repo_root}/ci/release_clean_host_phase.mjs" \
    "${repo_root}/ci/release_clean_host_probe.mjs" \
    "${repo_root}/ci/release_clean_host_test.sh" \
    "${repo_root}/ci/release_endpoint_publication.sh" \
    "${repo_root}/ci/release_endpoint_remote.sh" \
    "${repo_root}/ci/release_latest_alias.sh" \
    "${repo_root}/ci/release_operator.sh" \
    "${repo_root}/ci/release_protected_promotion.sh" \
    "${repo_root}/ci/release_receipt_matrix.jq" \
    "${repo_root}/ci/release_receipt_matrix_test.sh" \
    "${repo_root}/ci/release_repository_authorities.json" \
    "${repo_root}/ci/release_monorepo_transform.mjs" \
    "${public_core}/ci/"
  cp -R "${repo_root}/release/monorepo" "${public_core}/release/monorepo"
  cp "${repo_root}/release/endpoint-publication.defaults.json" \
    "${public_core}/release/endpoint-publication.defaults.json"
  cp "${repo_root}/release/oci/ait-server.Dockerfile" \
    "${repo_root}/release/oci/ait-runner.Dockerfile" \
    "${public_core}/release/oci/"
  cp "${repo_root}/.github/workflows/ait-release-component-receipts.yml" \
    "${public_core}/.github/workflows/ait-release-component-receipts.yml"
  cp "${repo_root}/.github/workflows/ait-release-clean-host.yml" \
    "${public_core}/.github/workflows/ait-release-clean-host.yml"
  cp "${repo_root}/.github/workflows/ait-release-latest-alias.yml" \
    "${public_core}/.github/workflows/ait-release-latest-alias.yml"
  cp "${repo_root}/.github/workflows/ait-release-protected-promotion.yml" \
    "${public_core}/.github/workflows/ait-release-protected-promotion.yml"
  cp "${repo_root}/.github/workflows/pypi-publish.yml" \
    "${public_core}/.github/workflows/pypi-publish.yml"
  cp "${product_document}" "${public_layout}/docs/distribution.md"
  bash "${public_core}/ci/release_monorepo_export_test.sh" \
    --public-layout-selftest >/dev/null
fi

write_common_source() {
  local root=$1
  local repository=$2
  mkdir -p \
    "${root}/ci" \
    "${root}/.ait" \
    "${root}/.ait-external/materialized" \
    "${root}/.ait-runtime" \
    "${root}/.ait-worktree-links/task" \
    "${root}/.git"
  if [[ ${repository} == ait-server ]]; then
    printf '%s\n' \
      'GNU AFFERO GENERAL PUBLIC LICENSE' \
      'Version 3, 19 November 2007' \
      'Fixture terms for release topology validation.' \
      'END OF TERMS AND CONDITIONS' >"${root}/LICENSE"
  else
    cp "${repo_root}/LICENSE" "${root}/LICENSE"
  fi
  printf '%s notice\n' "${repository}" >"${root}/NOTICE"
  printf '#!/usr/bin/env bash\nprintf "fixture entrypoint\\n"\n' \
    >"${root}/ci/fixture-entrypoint.sh"
  chmod 0755 "${root}/ci/fixture-entrypoint.sh"
  printf 'non-executable fixture\n' >"${root}/ci/fixture-data.txt"
  chmod 0644 "${root}/ci/fixture-data.txt"
  printf 'excluded\n' >"${root}/.ait/config.json"
  printf 'excluded\n' >"${root}/.ait-external/materialized/file"
  printf 'excluded\n' >"${root}/.ait-runtime/cache.json"
  printf 'excluded\n' >"${root}/.ait-worktree-links/task/file"
  printf 'excluded\n' >"${root}/.git/config"
}

write_bundle() {
  local repository=$1
  local source=$2
  local bundle_root=$3
  local snapshot license bundle
  snapshot=$(jq -er --arg repository "${repository}" '
    [.components[] | select(.source_repository == $repository) | .source_snapshot]
      | unique | .[0]
  ' "${repo_root}/ait-release-family.json")
  license=$(jq -er --arg repository "${repository}" '
    [.components[] | select(.source_repository == $repository) | .license]
      | unique | .[0]
  ' "${repo_root}/ait-release-family.json")
  bundle=${bundle_root}/ait-release-source-${repository}
  mkdir -p "${bundle}"
  tar -czf "${bundle}/source-cache.tar.gz" -C "${source}" .
  jq -n \
    --arg repository "${repository}" \
    --arg snapshot "${snapshot}" \
    --arg source_manifest_hash 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' \
    --arg source_snapshot_created_at '1699999999' \
    --arg license "${license}" '
      {
        contract: "ait.release.source-cache/v1",
        status: "ready",
        repo_name: $repository,
        source_snapshot: $snapshot,
        source_manifest_hash: $source_manifest_hash,
        source_snapshot_created_at: $source_snapshot_created_at,
        license: $license,
        source_authority: "ait_remote_snapshot_boundary",
        workspace_clean: true,
        remote_coordinates_embedded: false,
        public_publish: false
      }
    ' >"${bundle}/source-cache.evidence.json"
}

sources=${temporary_root}/sources
bundles=${temporary_root}/bundles
mkdir -p "${sources}" "${bundles}"
for repository in ait-core ait-server ait-runner ait-python ait-node; do
  source=${sources}/${repository}
  mkdir -p "${source}"
  write_common_source "${source}" "${repository}"
  case "${repository}" in
    ait-core)
      mkdir -p "${source}/rust/crates/ait-py" "${source}/docs"
      cp "${repo_root}/ci/release_protected_promotion.sh" "${source}/ci/"
      jq '.family.version = "1.0.0-rc.2"' \
        "${repo_root}/ait-release-family.json" \
        >"${source}/ait-release-family.json"
      jq '.family_version = "1.0.0-rc.2"' \
        "${repo_root}/ci/release_repository_authorities.json" \
        >"${source}/ci/release_repository_authorities.json"
      printf '[workspace]\nmembers = ["crates/ait-py"]\n' >"${source}/rust/Cargo.toml"
      printf '[package]\nname = "ait-py"\nversion = "1.0.0-rc.8"\n' \
        >"${source}/rust/crates/ait-py/Cargo.toml"
      cp "${product_document}" "${source}/docs/distribution.md"
      ;;
    ait-server)
      mkdir -p "${source}/rust"
      printf '[workspace]\nmembers = []\n' >"${source}/rust/Cargo.toml"
      ;;
    ait-runner)
      printf '[package]\nname = "ait-runner"\nversion = "1.0.0-rc.8"\n\n[dependencies]\nait-core = { path = ".ait-external/ait-core/rust/crates/ait-core" }\n' \
        >"${source}/Cargo.toml"
      ;;
    ait-python)
      printf '[build-system]\nrequires = ["maturin==1.13.3"]\nbuild-backend = "maturin"\n\n[tool.maturin]\nmanifest-path = ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml"\n' \
        >"${source}/pyproject.toml"
      ;;
    ait-node)
      mkdir -p "${source}/bin" "${source}/lib" "${source}/release" \
        "${source}/scripts" "${source}/src"
      jq -n '
        {
          name: "@wa120/ait-native",
          version: "1.0.0-rc.8",
          description: "Agent-first, language-neutral workflow for verified repository changes",
          homepage: "https://ait-native.dev/",
          type: "module",
          bin: {ait: "bin/ait.mjs"},
          exports: {
            ".": {
              types: "./src/index.d.ts",
              import: "./src/index.js",
              default: "./src/index.js"
            }
          },
          types: "./src/index.d.ts",
          optionalDependencies: {
            "@wa120/ait-native-darwin-arm64": "1.0.0-rc.8",
            "@wa120/ait-native-darwin-x64": "1.0.0-rc.8",
            "@wa120/ait-native-linux-arm64": "1.0.0-rc.8",
            "@wa120/ait-native-linux-x64": "1.0.0-rc.8",
            "@wa120/ait-native-win32-arm64": "1.0.0-rc.8",
            "@wa120/ait-native-win32-x64": "1.0.0-rc.8"
          },
          scripts: {}
        }
      ' >"${source}/package.json"
      printf '#!/usr/bin/env node\nexport {};\n' >"${source}/bin/ait.mjs"
      printf 'const addonPath = "native/ait_napi.node";\nconst addon = require(addonPath);\nexport { addon };\n' \
        >"${source}/src/runtime.js"
      printf 'export {};\n' >"${source}/src/index.js"
      printf 'export interface NativeAddon {}\n' >"${source}/src/index.d.ts"
      printf 'export {};\n' >"${source}/scripts/native-build.mjs"
      printf 'export {};\n' >"${source}/release/npm-payload-package.mjs"
      printf '%s\n' \
        '# ait-native' \
        '' \
        'AIT turns an ordinary coding request into an isolated, sprint-bound repository change.' \
        'It is for individual developers and maintainers who use coding agents.' \
        'Official website: <https://ait-native.dev/>' \
        '' \
        '## Install and initialize' \
        'npm install --global @wa120/ait-native@@AIT_NPM_VERSION@' \
        'ait init' \
        '' \
        '## What you have after 90 seconds' \
        'Installation and initialization are complete; arbitrary coding work is not promised.' \
        '' \
        '## Moving from 0.x' \
        'The 0.x requirement to run `ait install` and its task-DAG positioning are retired.' \
        >"${source}/release/npm-readme.txt"
      core_snapshot=$(jq -er '
        [.components[] | select(.source_repository == "ait-core") | .source_snapshot]
          | unique | .[0]
      ' "${repo_root}/ait-release-family.json")
      jq -n --arg snapshot "${core_snapshot}" '
        [
          ["aarch64-apple-darwin", "darwin", "arm64", null],
          ["x86_64-apple-darwin", "darwin", "x64", null],
          ["aarch64-unknown-linux-gnu", "linux", "arm64", "glibc"],
          ["x86_64-unknown-linux-gnu", "linux", "x64", "glibc"],
          ["aarch64-pc-windows-msvc", "win32", "arm64", null],
          ["x86_64-pc-windows-msvc", "win32", "x64", null]
        ] | {
          schema: "ait.node.napi-platform-packages/v2",
          family_version: "1.0.0-rc.8",
          top_level_package: "@wa120/ait-native",
          payloads: map({
            target: .[0],
            os: .[1],
            cpu: .[2],
            libc: .[3],
            component: "ait-node",
            package: ("@wa120/ait-native-" + .[1] + "-" + .[2]),
            version: "1.0.0-rc.8",
            binding_repository: "ait-core",
            binding_snapshot: $snapshot,
            license: "Apache-2.0",
            addon: "native/ait_napi.node"
          })
        }
      ' >"${source}/lib/npm-payload-contract.json"
      printf '%s\n' \
        'import { existsSync, mkdirSync, writeFileSync } from "node:fs";' \
        'import { dirname } from "node:path";' \
        'const [phase, target, version] = process.argv.slice(2);' \
        'const targets = new Map([' \
        '  ["aarch64-apple-darwin", "wa120-ait-native-darwin-arm64"],' \
        '  ["x86_64-apple-darwin", "wa120-ait-native-darwin-x64"],' \
        '  ["aarch64-unknown-linux-gnu", "wa120-ait-native-linux-arm64"],' \
        '  ["x86_64-unknown-linux-gnu", "wa120-ait-native-linux-x64"],' \
        '  ["aarch64-pc-windows-msvc", "wa120-ait-native-win32-arm64"],' \
        '  ["x86_64-pc-windows-msvc", "wa120-ait-native-win32-x64"],' \
        ']);' \
        'if ((target !== "portable" && !targets.has(target)) || version !== "1.0.0-rc.8") process.exit(64);' \
        'const artifact = target === "portable"' \
        '  ? "dist/wa120-ait-native-1.0.0-rc.8.tgz"' \
        '  : `dist/npm-addons/${targets.get(target)}-1.0.0-rc.8.tgz`;' \
        'if (phase === "build") {' \
        '  mkdirSync(dirname(artifact), { recursive: true });' \
        '  writeFileSync(artifact, `fixture direct Node-API ${target}\n`);' \
        '} else if (phase === "smoke" && !existsSync(artifact)) {' \
        '  process.exit(65);' \
        '} else if (phase !== "check" && phase !== "smoke") {' \
        '  process.exit(64);' \
        '}' >"${source}/release/fixture-receipt.mjs"
      jq -n '
        {
          schema: "ait.release.adapter/v1",
          package: {
            name: "@wa120/ait-native",
            version: "1.0.0-rc.8",
            description: "fixture",
            license_files: [
              {path: "LICENSE", role: "license"},
              {path: "NOTICE", role: "notice"}
            ]
          },
          components: [{
            id: "ait-node",
            ecosystem: "node",
            working_directory: ".",
            dependency_files: [
              "package.json",
              "lib/npm-payload-contract.json",
              "src/runtime.js",
              "scripts/native-build.mjs",
              "release/npm-readme.txt",
              "release/npm-payload-package.mjs",
              "release/fixture-receipt.mjs"
            ],
            commands: {
              test: [["node", "release/fixture-receipt.mjs", "check", "$AIT_RELEASE_TARGET", "$AIT_RELEASE_VERSION"]],
              build: [["node", "release/fixture-receipt.mjs", "build", "$AIT_RELEASE_TARGET", "$AIT_RELEASE_VERSION"]],
              smoke: [["node", "release/fixture-receipt.mjs", "smoke", "$AIT_RELEASE_TARGET", "$AIT_RELEASE_VERSION"]]
            },
            artifacts: [
              {path: "dist/wa120-ait-native-1.0.0-rc.8.tgz", kind: "npm-napi-envelope"},
              {path: "dist/npm-addons/wa120-ait-native-darwin-arm64-1.0.0-rc.8.tgz", kind: "npm-napi-addon", target: "aarch64-apple-darwin"},
              {path: "dist/npm-addons/wa120-ait-native-darwin-x64-1.0.0-rc.8.tgz", kind: "npm-napi-addon", target: "x86_64-apple-darwin"},
              {path: "dist/npm-addons/wa120-ait-native-linux-arm64-1.0.0-rc.8.tgz", kind: "npm-napi-addon", target: "aarch64-unknown-linux-gnu"},
              {path: "dist/npm-addons/wa120-ait-native-linux-x64-1.0.0-rc.8.tgz", kind: "npm-napi-addon", target: "x86_64-unknown-linux-gnu"},
              {path: "dist/npm-addons/wa120-ait-native-win32-arm64-1.0.0-rc.8.tgz", kind: "npm-napi-addon", target: "aarch64-pc-windows-msvc"},
              {path: "dist/npm-addons/wa120-ait-native-win32-x64-1.0.0-rc.8.tgz", kind: "npm-napi-addon", target: "x86_64-pc-windows-msvc"}
            ]
          }]
        }
      ' >"${source}/ait-release.json"
      ;;
  esac
  write_bundle "${repository}" "${source}" "${bundles}"
done

output_one=${temporary_root}/output-one
output_two=${temporary_root}/output-two
evidence_one=${temporary_root}/evidence-one.json
evidence_two=${temporary_root}/evidence-two.json
bash "${repo_root}/ci/release_monorepo_export.sh" \
  "${repo_root}/ait-release-family.json" "${bundles}" "${output_one}" "${evidence_one}"
bash "${repo_root}/ci/release_monorepo_export.sh" \
  "${repo_root}/ait-release-family.json" "${bundles}" "${output_two}" "${evidence_two}"

diff -r "${output_one}" "${output_two}"
cmp "${evidence_one}" "${evidence_two}"
cmp "${repo_root}/release/monorepo/.gitattributes" \
  "${output_one}/.gitattributes"
cmp "${repo_root}/release/monorepo/CONTRIBUTING.template" \
  "${output_one}/CONTRIBUTING.md"
cmp "${repo_root}/release/monorepo/SECURITY.template" \
  "${output_one}/SECURITY.md"
grep -F 'ait-native public source license scope' "${output_one}/LICENSE" >/dev/null
grep -F 'sole component exception is `ait-server/**`' \
  "${output_one}/LICENSE" >/dev/null
grep -F 'No commercial or proprietary license applies to a public 1.0 source path' \
  "${output_one}/LICENSE" >/dev/null
cmp "${output_one}/ait-core/LICENSE" \
  "${output_one}/LICENSES/Apache-2.0.txt"
cmp "${output_one}/ait-server/LICENSE" \
  "${output_one}/LICENSES/AGPL-3.0-only.txt"
for apache_repository in ait-core ait-runner ait-python ait-node; do
  test ! -e "${output_one}/${apache_repository}/LICENSES/AGPL-3.0-only.txt"
  test ! -e "${output_one}/${apache_repository}/LICENSES/LicenseRef-AIT-Commercial.txt"
done
if find "${output_one}" -type f -name 'LicenseRef-AIT-Commercial.txt' \
  -print -quit | grep -q .; then
  printf 'commercial license reference survived the public export\n' >&2
  exit 65
fi
jq -e '
  .schema == "ait.release.monorepo-source/v1" and
  .public_source_identity == "weita2026/ait-native" and
  .coordinator_snapshot == "SNP-AAAAAAAAAAAA" and
  .coordinator_manifest_hash == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" and
  .coordinator_created_at == "1700000000" and
  (.subtrees | length) == 5 and
  ([.subtrees[].source_snapshot] | all(test("^SNP-[0-9A-F]{12}$"))) and
  ([.subtrees[].source_manifest_hash] | all(test("^[0-9a-f]{64}$"))) and
  .git_commit_created == false and
  .public_publish == false
' "${output_one}/ait-monorepo-source.json" >/dev/null
jq -e '
  .contract == "ait.release.monorepo-source-export/v1" and
  .status == "ready" and
  .coordinator_snapshot == "SNP-AAAAAAAAAAAA" and
  .coordinator_manifest_hash == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" and
  .coordinator_created_at == "1700000000" and
  .source_cache_count == 5 and
  .git_commit_created == false and
  .tag_created == false and
  .registry_write == false and
  .public_publish == false
' "${evidence_one}" >/dev/null
grep -F 'path = "../ait-core/rust/crates/ait-core"' \
  "${output_one}/ait-runner/Cargo.toml" >/dev/null
grep -F 'manifest-path = "../ait-core/rust/crates/ait-py/Cargo.toml"' \
  "${output_one}/ait-python/pyproject.toml" >/dev/null
if grep -RFl '.ait-external/ait-core/rust/crates' \
  "${output_one}/ait-runner" "${output_one}/ait-python" >/dev/null; then
  printf 'internal external-source path survived the public export\n' >&2
  exit 65
fi
if find "${output_one}" \
  \( -type l -o -name .ait -o -name .ait-external -o -name .ait-runtime -o -name .ait-worktree-links -o -name .git -o -name .gitmodules \) \
  -print -quit | grep -q .; then
  printf 'forbidden source path survived deterministic export\n' >&2
  exit 65
fi
if [[ ! -x ${output_one}/ait-core/ci/fixture-entrypoint.sh ]]; then
  printf 'component executable mode did not survive deterministic export\n' >&2
  exit 65
fi
if [[ -x ${output_one}/ait-core/ci/fixture-data.txt ]]; then
  printf 'component non-executable mode changed during deterministic export\n' >&2
  exit 65
fi
node --check "${output_one}/build-release.mjs"
node "${output_one}/build-release.mjs" --validate-only >/dev/null
missing_public_storefront=${temporary_root}/missing-public-storefront
cp -R "${output_one}" "${missing_public_storefront}"
printf '# ait-native\n' >"${missing_public_storefront}/README.md"
expect_failure missing-public-storefront node \
  "${missing_public_storefront}/build-release.mjs" --validate-only
stale_public_storefront=${temporary_root}/stale-public-storefront
cp -R "${output_one}" "${stale_public_storefront}"
printf '\nJira-like\n' >>"${stale_public_storefront}/README.md"
expect_failure stale-public-storefront node \
  "${stale_public_storefront}/build-release.mjs" --validate-only
unresolved_public_storefront=${temporary_root}/unresolved-public-storefront
cp -R "${output_one}" "${unresolved_public_storefront}"
printf '\n@AIT_PYPI_VERSION@\n' >>"${unresolved_public_storefront}/README.md"
expect_failure unresolved-public-storefront node \
  "${unresolved_public_storefront}/build-release.mjs" --validate-only
missing_npm_storefront=${temporary_root}/missing-npm-storefront
cp -R "${output_one}" "${missing_npm_storefront}"
printf '# ait-native\n' \
  >"${missing_npm_storefront}/ait-node/release/npm-readme.txt"
expect_failure missing-npm-storefront node \
  "${missing_npm_storefront}/build-release.mjs" --validate-only
test "$(jq -r '.family.version' \
  "${output_one}/ait-core/ait-release-family.json")" = "1.0.0-rc.2"
test "$(jq -r '.family_version' \
  "${output_one}/ait-core/ci/release_repository_authorities.json")" = \
  "1.0.0-rc.2"
test "$(jq -r '.family_version' \
  "${output_one}/ci/release_repository_authorities.json")" = "1.0.0-rc.8"
for release_control_path in \
  ci/native_bootstrap_matrix.jq \
  ci/release_clean_host.mjs \
  ci/release_clean_host_phase.mjs \
  ci/release_clean_host_probe.mjs \
  ci/release_clean_host_test.sh \
  ci/release_endpoint_publication.sh \
  ci/release_endpoint_remote.sh \
  ci/release_latest_alias.sh \
  ci/release_operator.sh \
  ci/release_protected_promotion.sh \
  ci/release_receipt_matrix.jq \
  ci/release_receipt_matrix_test.sh; do
  cmp "${repo_root}/${release_control_path}" \
    "${output_one}/${release_control_path}"
done
test "$(jq -r '.version' "${output_one}/ci/native_bootstrap_matrix.json")" = \
  '1.0.0-rc.8'
AIT_RELEASE_FAMILY_MANIFEST="${output_one}/ait-release-family.json" \
  bash "${output_one}/ci/release_receipt_matrix_test.sh" >/dev/null
bash "${output_one}/ci/release_clean_host_test.sh" >/dev/null
expect_failure historical-component-family env \
  AIT_RELEASE_FAMILY_MANIFEST="${output_one}/ait-core/ait-release-family.json" \
  bash "${output_one}/ci/release_receipt_matrix_test.sh"
expect_failure relative-public-family env \
  AIT_RELEASE_FAMILY_MANIFEST=../ait-release-family.json bash \
  "${output_one}/ci/release_receipt_matrix_test.sh"
for required_local_node_adapter_text in \
  'const nodeAdapter = path.join(nodeRoot, "release", "release-adapter.mjs");' \
  '[nodeAdapter, "build", "portable", family.family.version]' \
  'portable npm adapter artifact differs from its reported digest or size'; do
  grep -F "${required_local_node_adapter_text}" \
    "${output_one}/build-release.mjs" >/dev/null
done
if grep -F '"pack", "--ignore-scripts", "--pack-destination", npmOutput' \
  "${output_one}/build-release.mjs" >/dev/null; then
  printf 'local source build bypasses the protected portable npm adapter\n' >&2
  exit 65
fi
git -C "${output_one}" init -q
git -C "${output_one}" config user.name 'AIT release fixture'
git -C "${output_one}" config user.email 'release-fixture@localhost'
for ignored_path in \
  '.ait/config.json' \
  'ait-python/.ait/cargo-target/release/libait_py.dylib' \
  'ait-python/.ait/cargo-build/workspaces/example/output' \
  'ait-core/.ait-external/ait-server/marker.json' \
  'ait-server/.ait-runtime/server.json' \
  'ait-runner/.ait-worktree-links/task/path'; do
  if ! git -C "${output_one}" check-ignore -q --no-index "${ignored_path}"; then
    printf 'public root .gitignore does not cover operational path: %s\n' \
      "${ignored_path}" >&2
    exit 65
  fi
done
mkdir -p \
  "${output_one}/.ait/generated" \
  "${output_one}/ait-python/.ait/cargo-target/release"
printf 'generated root state\n' >"${output_one}/.ait/generated/state.json"
printf 'generated build output\n' \
  >"${output_one}/ait-python/.ait/cargo-target/release/libait_py.dylib"
node "${output_one}/build-release.mjs" --validate-only >/dev/null
git -C "${output_one}" add -A
git -C "${output_one}" commit -qm 'fixture public source commit'
if [[ -n $(git -C "${output_one}" status --porcelain --untracked-files=all) ]]; then
  printf 'ignored operational output dirtied the public Git fixture\n' >&2
  exit 65
fi
fixture_git_commit=$(git -C "${output_one}" rev-parse HEAD)
windows_checkout=${temporary_root}/windows-autocrlf-checkout
git -c core.autocrlf=true clone -q --no-local \
  "${output_one}" "${windows_checkout}"
node "${windows_checkout}/build-release.mjs" --validate-only >/dev/null
fixture_receipt=${temporary_root}/public-git-receipt
node "${output_one}/build-release.mjs" \
  --component-receipt \
  --repository ait-node \
  --target portable \
  --version 1.0.0-rc.8 \
  --git-commit "${fixture_git_commit}" \
  --out-dir "${fixture_receipt}" >/dev/null
jq -e \
  --arg commit "${fixture_git_commit}" '
    .contract == "ait.release.public-git.receipt/v1" and
    .repo_name == "ait-node" and
    .artifact_selection == "portable" and
    .authority.source == "public_git_commit" and
    .authority.git_commit == $commit and
    .status == "built" and
    .check_summary.decision == "pass" and
    ([.artifacts[] | select(.role == "component-artifact")] | length) == 1 and
    .public_publish == false and
    .publishable == false
  ' "${fixture_receipt}/ait-release.receipt.json" >/dev/null

tracked_operational_output=${temporary_root}/tracked-operational-output
cp -R "${output_one}" "${tracked_operational_output}"
printf 'forced tracked state\n' \
  >"${tracked_operational_output}/ait-python/.ait/tracked-state.json"
git -C "${tracked_operational_output}" add -f \
  ait-python/.ait/tracked-state.json
expect_failure tracked-operational node \
  "${tracked_operational_output}/build-release.mjs" --validate-only

gitlink_output=${temporary_root}/gitlink-output
cp -R "${output_one}" "${gitlink_output}"
git -C "${gitlink_output}" update-index --add --cacheinfo \
  "160000,${fixture_git_commit},ait-core-link"
expect_failure gitlink node "${gitlink_output}/build-release.mjs" --validate-only

rm -rf -- "${output_one}/ait-node/dist"
ln -s "${output_one}" "${temporary_root}/receipt-parent-link"
expect_failure receipt-parent-symlink node "${output_one}/build-release.mjs" \
  --component-receipt \
  --repository ait-node \
  --target portable \
  --version 1.0.0-rc.8 \
  --git-commit "${fixture_git_commit}" \
  --out-dir "${temporary_root}/receipt-parent-link/escaped-receipt"
public_readme=${output_one}/README.md
for required_readme_text in \
  'AIT turns an ordinary coding request into an isolated, sprint-bound repository' \
  'individual developers and maintainers' \
  'python -m pip install ait-native==1.0.0rc8' \
  'img.shields.io/github/v/release/weita2026/ait-native' \
  'https://github.com/weita2026/ait-native/discussions' \
  'https://github.com/weita2026/ait-native/issues/new/choose' \
  'ait init' \
  'ait --version' \
  '## What you have after 90 seconds' \
  'https://ait-native.dev/' \
  '## Moving from 0.x' \
  'The 0.x requirement to run `ait install` and its task-DAG positioning are' \
  'AGENTS.md' \
  'ait task start' \
  'ait plan sync' \
  'ait snapshot create' \
  'ait task land' \
  'package-owned `native/ait_napi.node`' \
  'does not locate or launch a child executable' \
  '## License map' \
  '`ait-core/**`, `ait-runner/**`, `ait-python/**`, and' \
  '`ait-server/**`, which is AGPL-3.0-only' \
  'No commercial or proprietary license applies to a public 1.0 source path'; do
  grep -F "${required_readme_text}" "${public_readme}" >/dev/null
done
if grep -E '@AIT_[A-Z0-9_]+@|Jira-like|parallel AI execution|compact task DAG|mkdir -p docs/sprints' \
  "${public_readme}" >/dev/null; then
  printf 'public README contains an unresolved token, stale positioning, or manual sprint bootstrap\n' >&2
  exit 65
fi

grep -F 'deterministic release monorepo' \
  "${output_one}/CONTRIBUTING.md" >/dev/null
grep -F 'material AI assistance' "${output_one}/CONTRIBUTING.md" >/dev/null
grep -F 'security/advisories/new' "${output_one}/SECURITY.md" >/dev/null
grep -F 'Do not open a public issue' "${output_one}/SECURITY.md" >/dev/null

cmp "${repo_root}/release/monorepo/CODE_OF_CONDUCT.template" \
  "${output_one}/CODE_OF_CONDUCT.md"
cmp "${repo_root}/release/monorepo/SUPPORT.template" \
  "${output_one}/SUPPORT.md"
cmp "${repo_root}/release/monorepo/.github/PULL_REQUEST_TEMPLATE.template" \
  "${output_one}/.github/PULL_REQUEST_TEMPLATE.md"

for public_community_path in \
  CITATION.cff \
  .github/ISSUE_TEMPLATE/bug_report.yml \
  .github/ISSUE_TEMPLATE/documentation.yml \
  .github/ISSUE_TEMPLATE/config.yml \
  .github/DISCUSSION_TEMPLATE/q-a.yml \
  .github/DISCUSSION_TEMPLATE/ideas.yml \
  .github/DISCUSSION_TEMPLATE/show-and-tell.yml \
  .github/release.yml \
  .github/social-preview.png; do
  cmp "${repo_root}/release/monorepo/${public_community_path}" \
    "${output_one}/${public_community_path}"
done

community_path_index=0
for required_community_path in \
  CODE_OF_CONDUCT.md \
  SUPPORT.md \
  CITATION.cff \
  .github/ISSUE_TEMPLATE/bug_report.yml \
  .github/ISSUE_TEMPLATE/documentation.yml \
  .github/ISSUE_TEMPLATE/config.yml \
  .github/DISCUSSION_TEMPLATE/q-a.yml \
  .github/DISCUSSION_TEMPLATE/ideas.yml \
  .github/DISCUSSION_TEMPLATE/show-and-tell.yml \
  .github/PULL_REQUEST_TEMPLATE.md \
  .github/release.yml \
  .github/social-preview.png; do
  missing_community_output=${temporary_root}/missing-community-${community_path_index}
  cp -R "${output_one}" "${missing_community_output}"
  rm -- "${missing_community_output}/${required_community_path}"
  expect_failure "missing-community-${community_path_index}" node \
    "${missing_community_output}/build-release.mjs" --validate-only
  community_path_index=$((community_path_index + 1))
done

community_drift_output=${temporary_root}/community-drift-output
cp -R "${output_one}" "${community_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${community_drift_output}/SUPPORT.md" \
  'private vulnerability reporting' \
  'public vulnerability reporting'
expect_failure community-drift node \
  "${community_drift_output}/build-release.mjs" --validate-only

missing_contributing_output=${temporary_root}/missing-contributing-output
cp -R "${output_one}" "${missing_contributing_output}"
rm -- "${missing_contributing_output}/CONTRIBUTING.md"
expect_failure missing-contributing node \
  "${missing_contributing_output}/build-release.mjs" --validate-only

contributing_drift_output=${temporary_root}/contributing-drift-output
cp -R "${output_one}" "${contributing_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${contributing_drift_output}/CONTRIBUTING.md" \
  'deterministic release monorepo' \
  'mutable release monorepo'
expect_failure contributing-drift node \
  "${contributing_drift_output}/build-release.mjs" --validate-only

missing_security_output=${temporary_root}/missing-security-output
cp -R "${output_one}" "${missing_security_output}"
rm -- "${missing_security_output}/SECURITY.md"
expect_failure missing-security node \
  "${missing_security_output}/build-release.mjs" --validate-only

security_drift_output=${temporary_root}/security-drift-output
cp -R "${output_one}" "${security_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${security_drift_output}/SECURITY.md" \
  'Do not open a public issue' \
  'Open a public issue'
expect_failure security-drift node \
  "${security_drift_output}/build-release.mjs" --validate-only

readme_drift_output=${temporary_root}/readme-drift-output
cp -R "${output_one}" "${readme_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${readme_drift_output}/README.md" \
  "does not identify the repository's programming language or project type" \
  "tries to identify the repository's programming language or project type"
expect_failure readme-drift node \
  "${readme_drift_output}/build-release.mjs" --validate-only
byte_policy_drift_output=${temporary_root}/byte-policy-drift-output
cp -R "${output_one}" "${byte_policy_drift_output}"
printf '* text=auto\n' >"${byte_policy_drift_output}/.gitattributes"
expect_failure byte-policy-drift node \
  "${byte_policy_drift_output}/build-release.mjs" --validate-only
root_workflow=${output_one}/.github/workflows/ait-release-component-receipts.yml
clean_host_workflow=${output_one}/.github/workflows/ait-release-clean-host.yml
latest_alias_workflow=${output_one}/.github/workflows/ait-release-latest-alias.yml
promotion_workflow=${output_one}/.github/workflows/ait-release-protected-promotion.yml
endpoint_workflow=${output_one}/.github/workflows/pypi-publish.yml
test -f "${root_workflow}"
test -f "${clean_host_workflow}"
test -f "${latest_alias_workflow}"
test -f "${promotion_workflow}"
test -f "${endpoint_workflow}"
test "$(find "${output_one}/.github/workflows" -maxdepth 1 -type f | wc -l | tr -d '[:space:]')" = 5
grep -F '    working-directory: source/ait-core' "${root_workflow}" >/dev/null
grep -F '          path: release-receipt-matrix.json' \
  "${root_workflow}" >/dev/null
if grep -F 'contents: write' "${root_workflow}" >/dev/null ||
  grep -F '          path: ait-core/release-receipt-matrix.json' \
    "${root_workflow}" >/dev/null; then
  printf 'root protected workflow retained unsafe monorepo execution paths\n' >&2
  exit 65
fi
for required_clean_host_text in \
  'name: ait release clean host' \
  'matrix_sha256:' \
  'release_clean_host_probe.mjs' \
  'release_clean_host_phase.mjs run' \
  'release_clean_host.mjs combine' \
  'release_clean_host.mjs aggregate' \
  'ait-clean-host-${{ inputs.release_id }}'; do
  grep -F -- "${required_clean_host_text}" "${clean_host_workflow}" >/dev/null
done
# shellcheck disable=SC2016
for required_promotion_text in \
  'name: ait release protected promotion' \
  "      name: \${{ format('{0}-promotion', inputs.channel) }}" \
  'artifact-ids: ${{ inputs.dossier_artifact_id }}' \
  '          merge-multiple: true' \
  'source_control_commit:' \
  'bash control/ci/release_protected_promotion.sh' \
  'actions/attest-build-provenance@977bb373ede98d70efdf65b84cb5f73e068dcc2a'; do
  grep -F -- "${required_promotion_text}" "${promotion_workflow}" >/dev/null
done

for endpoint_control in \
  "${output_one}/ci/release_endpoint_publication.sh" \
  "${output_one}/ci/release_endpoint_remote.sh" \
  "${output_one}/ci/release_latest_alias.sh" \
  "${output_one}/ci/release_operator.sh" \
  "${output_one}/release/endpoint-publication.defaults.json"; do
  test -f "${endpoint_control}"
done
grep -F 'endpoint_config_sha256:' "${endpoint_workflow}" >/dev/null
grep -F 'control/ci/release_operator.sh validate-config' \
  "${endpoint_workflow}" >/dev/null
# shellcheck disable=SC2016
for required_latest_alias_text in \
  'name: ait release latest alias' \
  '      name: pypi' \
  'AIT_RELEASE_LATEST_RELEASE_ID: ${{ inputs.release_id }}' \
  'control/ci/release_latest_alias.sh apply' \
  'control/ci/release_latest_alias.sh verify' \
  'AIT_NPM_TOKEN: ${{ secrets.AIT_NPM_TOKEN }}' \
  'packages: write'; do
  grep -F -- "${required_latest_alias_text}" "${latest_alias_workflow}" >/dev/null
done
cmp "${repo_root}/release/endpoint-publication.defaults.json" \
  "${output_one}/release/endpoint-publication.defaults.json"
test ! -e "${output_one}/release/endpoint-publication.rc1.json"
test ! -e "${output_one}/release/endpoint-publication.rc2.json"
cmp "${repo_root}/release/oci/ait-server.Dockerfile" \
  "${output_one}/release/oci/ait-server.Dockerfile"
cmp "${repo_root}/release/oci/ait-runner.Dockerfile" \
  "${output_one}/release/oci/ait-runner.Dockerfile"
for forbidden_promotion_text in \
  'contents: write' \
  'packages: write' \
  'secrets.' \
  'gh release create' \
  'npm publish' \
  'twine upload' \
  'docker push' \
  'oras push'; do
  if grep -F -- "${forbidden_promotion_text}" "${promotion_workflow}" >/dev/null; then
    printf 'protected promotion workflow gained publication authority: %s\n' \
      "${forbidden_promotion_text}" >&2
    exit 65
  fi
done

workflow_drift_output=${temporary_root}/workflow-drift-output
cp -R "${output_one}" "${workflow_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${workflow_drift_output}/.github/workflows/ait-release-component-receipts.yml" \
  '    working-directory: source/ait-core' \
  '    working-directory: .'
expect_failure workflow-drift node \
  "${workflow_drift_output}/build-release.mjs" --validate-only

clean_host_workflow_drift_output=${temporary_root}/clean-host-workflow-drift-output
cp -R "${output_one}" "${clean_host_workflow_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${clean_host_workflow_drift_output}/.github/workflows/ait-release-clean-host.yml" \
  'name: ait release clean host' \
  'name: mutable clean host'
expect_failure clean-host-workflow-drift node \
  "${clean_host_workflow_drift_output}/build-release.mjs" --validate-only

promotion_workflow_drift_output=${temporary_root}/promotion-workflow-drift-output
cp -R "${output_one}" "${promotion_workflow_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${promotion_workflow_drift_output}/.github/workflows/ait-release-protected-promotion.yml" \
  "      name: \${{ format('{0}-promotion', inputs.channel) }}" \
  '      name: unprotected'
expect_failure promotion-workflow-drift node \
  "${promotion_workflow_drift_output}/build-release.mjs" --validate-only

promotion_download_drift_output=${temporary_root}/promotion-download-drift-output
cp -R "${output_one}" "${promotion_download_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${promotion_download_drift_output}/.github/workflows/ait-release-protected-promotion.yml" \
  '          merge-multiple: true' \
  '          merge-multiple: false'
expect_failure promotion-download-drift node \
  "${promotion_download_drift_output}/build-release.mjs" --validate-only

latest_alias_workflow_drift_output=${temporary_root}/latest-alias-workflow-drift-output
cp -R "${output_one}" "${latest_alias_workflow_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${latest_alias_workflow_drift_output}/.github/workflows/ait-release-latest-alias.yml" \
  '      name: pypi' \
  '      name: unprotected'
expect_failure latest-alias-workflow-drift node \
  "${latest_alias_workflow_drift_output}/build-release.mjs" --validate-only

missing_root_license=${temporary_root}/missing-root-license
cp -R "${output_one}" "${missing_root_license}"
rm "${missing_root_license}/LICENSE"
expect_failure missing-root-license node \
  "${missing_root_license}/build-release.mjs" --validate-only

license_mapping_drift=${temporary_root}/license-mapping-drift
cp -R "${output_one}" "${license_mapping_drift}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${license_mapping_drift}/LICENSE" \
  'The sole component exception is `ait-server/**`.' \
  'Every component uses the root default license.'
expect_failure license-mapping-drift node \
  "${license_mapping_drift}/build-release.mjs" --validate-only

validator_apache_agpl=${temporary_root}/validator-apache-agpl
cp -R "${output_one}" "${validator_apache_agpl}"
mkdir -p "${validator_apache_agpl}/ait-node/legal"
printf 'foreign AGPL marker\n' \
  >"${validator_apache_agpl}/ait-node/legal/AGPL-LICENSE.txt"
expect_failure validator-apache-agpl node \
  "${validator_apache_agpl}/build-release.mjs" --validate-only
grep -F 'ait-node contains a foreign AGPL license marker: legal/AGPL-LICENSE.txt' \
  "${temporary_root}/validator-apache-agpl.stderr" >/dev/null

validator_commercial=${temporary_root}/validator-commercial
cp -R "${output_one}" "${validator_commercial}"
mkdir -p "${validator_commercial}/ait-server/legal"
printf 'unauthorized proprietary marker\n' \
  >"${validator_commercial}/ait-server/legal/Proprietary-LICENSE.txt"
expect_failure validator-commercial node \
  "${validator_commercial}/build-release.mjs" --validate-only
grep -F 'ait-server contains an unauthorized commercial license marker: legal/Proprietary-LICENSE.txt' \
  "${temporary_root}/validator-commercial.stderr" >/dev/null

wrong_snapshot_bundles=${temporary_root}/wrong-snapshot-bundles
cp -R "${bundles}" "${wrong_snapshot_bundles}"
jq '.source_snapshot = "SNP-000000000000"' \
  "${wrong_snapshot_bundles}/ait-release-source-ait-core/source-cache.evidence.json" \
  >"${temporary_root}/wrong-snapshot.json"
mv "${temporary_root}/wrong-snapshot.json" \
  "${wrong_snapshot_bundles}/ait-release-source-ait-core/source-cache.evidence.json"
expect_failure wrong-snapshot bash "${repo_root}/ci/release_monorepo_export.sh" \
  "${repo_root}/ait-release-family.json" "${wrong_snapshot_bundles}" \
  "${temporary_root}/wrong-snapshot-output" "${temporary_root}/wrong-snapshot-evidence.json"

symlink_bundles=${temporary_root}/symlink-bundles
cp -R "${bundles}" "${symlink_bundles}"
symlink_source=${temporary_root}/symlink-source
cp -R "${sources}/ait-node" "${symlink_source}"
ln -s package.json "${symlink_source}/package-link.json"
tar -czf "${symlink_bundles}/ait-release-source-ait-node/source-cache.tar.gz" \
  -C "${symlink_source}" .
expect_failure symlink bash "${repo_root}/ci/release_monorepo_export.sh" \
  "${repo_root}/ait-release-family.json" "${symlink_bundles}" \
  "${temporary_root}/symlink-output" "${temporary_root}/symlink-evidence.json"

apache_agpl_bundles=${temporary_root}/apache-agpl-bundles
cp -R "${bundles}" "${apache_agpl_bundles}"
apache_agpl_source=${temporary_root}/apache-agpl-source
cp -R "${sources}/ait-core" "${apache_agpl_source}"
mkdir -p "${apache_agpl_source}/docs/legal"
printf 'foreign AGPL marker\n' \
  >"${apache_agpl_source}/docs/legal/AGPL-LICENSE.txt"
tar -czf "${apache_agpl_bundles}/ait-release-source-ait-core/source-cache.tar.gz" \
  -C "${apache_agpl_source}" .
expect_failure apache-agpl bash "${repo_root}/ci/release_monorepo_export.sh" \
  "${repo_root}/ait-release-family.json" "${apache_agpl_bundles}" \
  "${temporary_root}/apache-agpl-output" "${temporary_root}/apache-agpl-evidence.json"
grep -F 'Apache source subtree ait-core contains a foreign license marker' \
  "${temporary_root}/apache-agpl.stderr" >/dev/null

apache_commercial_bundles=${temporary_root}/apache-commercial-bundles
cp -R "${bundles}" "${apache_commercial_bundles}"
apache_commercial_source=${temporary_root}/apache-commercial-source
cp -R "${sources}/ait-python" "${apache_commercial_source}"
mkdir -p "${apache_commercial_source}/legal/private"
printf 'unauthorized commercial marker\n' \
  >"${apache_commercial_source}/legal/private/Commercial-LICENSE.txt"
tar -czf "${apache_commercial_bundles}/ait-release-source-ait-python/source-cache.tar.gz" \
  -C "${apache_commercial_source}" .
expect_failure apache-commercial bash "${repo_root}/ci/release_monorepo_export.sh" \
  "${repo_root}/ait-release-family.json" "${apache_commercial_bundles}" \
  "${temporary_root}/apache-commercial-output" \
  "${temporary_root}/apache-commercial-evidence.json"
grep -F 'public source subtree ait-python contains an unauthorized commercial license marker' \
  "${temporary_root}/apache-commercial.stderr" >/dev/null

incomplete_server_bundles=${temporary_root}/incomplete-server-bundles
cp -R "${bundles}" "${incomplete_server_bundles}"
incomplete_server_source=${temporary_root}/incomplete-server-source
cp -R "${sources}/ait-server" "${incomplete_server_source}"
printf 'GNU AFFERO GENERAL PUBLIC LICENSE reference only\n' \
  >"${incomplete_server_source}/LICENSE"
tar -czf "${incomplete_server_bundles}/ait-release-source-ait-server/source-cache.tar.gz" \
  -C "${incomplete_server_source}" .
expect_failure incomplete-server bash "${repo_root}/ci/release_monorepo_export.sh" \
  "${repo_root}/ait-release-family.json" "${incomplete_server_bundles}" \
  "${temporary_root}/incomplete-server-output" \
  "${temporary_root}/incomplete-server-evidence.json"
grep -F 'AGPL source subtree ait-server has an invalid or incomplete root LICENSE' \
  "${temporary_root}/incomplete-server.stderr" >/dev/null

jq '.public_source.transforms[0].to = "../../ait-core"' \
  "${repo_root}/ait-release-family.json" >"${temporary_root}/undeclared-transform-family.json"
expect_failure undeclared-transform bash "${repo_root}/ci/release_monorepo_export.sh" \
  "${temporary_root}/undeclared-transform-family.json" "${bundles}" \
  "${temporary_root}/undeclared-transform-output" "${temporary_root}/undeclared-transform-evidence.json"

mkdir "${temporary_root}/existing-output"
expect_failure existing-output bash "${repo_root}/ci/release_monorepo_export.sh" \
  "${repo_root}/ait-release-family.json" "${bundles}" \
  "${temporary_root}/existing-output" "${temporary_root}/existing-output-evidence.json"

printf 'release monorepo export contract: pass\n'
