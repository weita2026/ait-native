#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
product_document=${repo_root}/docs/distribution.md
if [[ ! -e ${product_document} && ! -L ${product_document} ]]; then
  product_document=${repo_root}/../docs/distribution.md
fi
temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/ait-monorepo-export-test.XXXXXX")
export AIT_RELEASE_COORDINATOR_SNAPSHOT=SNP-AAAAAAAAAAAA
export AIT_RELEASE_COORDINATOR_MANIFEST_HASH=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
export AIT_RELEASE_COORDINATOR_CREATED_AT=1700000000

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

if [[ ${AIT_RELEASE_MONOREPO_PUBLIC_LAYOUT_SELFTEST:-0} == 0 ]]; then
  public_layout=${temporary_root}/public-layout
  public_core=${public_layout}/ait-core
  mkdir -p \
    "${public_core}/ci" \
    "${public_core}/release" \
    "${public_core}/.github/workflows" \
    "${public_layout}/docs"
  cp "${repo_root}/ait-release-family.json" "${public_core}/ait-release-family.json"
  cp "${repo_root}/ci/release_monorepo_export.sh" \
    "${repo_root}/ci/release_monorepo_export_test.sh" \
    "${repo_root}/ci/release_monorepo_transform.mjs" \
    "${public_core}/ci/"
  cp -R "${repo_root}/release/monorepo" "${public_core}/release/monorepo"
  cp "${repo_root}/.github/workflows/ait-release-component-receipts.yml" \
    "${public_core}/.github/workflows/ait-release-component-receipts.yml"
  cp "${product_document}" "${public_layout}/docs/distribution.md"
  AIT_RELEASE_MONOREPO_PUBLIC_LAYOUT_SELFTEST=1 \
    bash "${public_core}/ci/release_monorepo_export_test.sh" >/dev/null
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
  printf '%s license\n' "${repository}" >"${root}/LICENSE"
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
      printf '[workspace]\nmembers = ["crates/ait-py"]\n' >"${source}/rust/Cargo.toml"
      printf '[package]\nname = "ait-py"\nversion = "1.0.0-rc.1"\n' \
        >"${source}/rust/crates/ait-py/Cargo.toml"
      cp "${product_document}" "${source}/docs/distribution.md"
      ;;
    ait-server)
      mkdir -p "${source}/rust"
      printf '[workspace]\nmembers = []\n' >"${source}/rust/Cargo.toml"
      ;;
    ait-runner)
      printf '[package]\nname = "ait-runner"\nversion = "1.0.0-rc.1"\n\n[dependencies]\nait-core = { path = ".ait-external/ait-core/rust/crates/ait-core" }\n' \
        >"${source}/Cargo.toml"
      ;;
    ait-python)
      printf '[build-system]\nrequires = ["maturin==1.13.3"]\nbuild-backend = "maturin"\n\n[tool.maturin]\nmanifest-path = ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml"\n' \
        >"${source}/pyproject.toml"
      ;;
    ait-node)
      mkdir -p "${source}/release"
      printf '{"name":"ait-native","version":"1.0.0-rc.1","type":"module","exports":{},"scripts":{}}\n' \
        >"${source}/package.json"
      printf 'export {};\n' >"${source}/release/npm-payload-package.mjs"
      printf '%s\n' \
        'import { existsSync, mkdirSync, writeFileSync } from "node:fs";' \
        'const [phase, target, version] = process.argv.slice(2);' \
        'if (target !== "portable" || version !== "1.0.0-rc.1") process.exit(64);' \
        'const artifact = "dist/ait-native-1.0.0-rc.1.tgz";' \
        'if (phase === "build") {' \
        '  mkdirSync("dist", { recursive: true });' \
        '  writeFileSync(artifact, "fixture npm envelope\n");' \
        '} else if (phase === "smoke" && !existsSync(artifact)) {' \
        '  process.exit(65);' \
        '} else if (phase !== "check" && phase !== "smoke") {' \
        '  process.exit(64);' \
        '}' >"${source}/release/fixture-receipt.mjs"
      jq -n '
        {
          schema: "ait.release.adapter/v1",
          package: {
            name: "ait-native",
            version: "1.0.0-rc.1",
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
            dependency_files: ["package.json", "release/fixture-receipt.mjs"],
            commands: {
              test: [["node", "release/fixture-receipt.mjs", "check", "$AIT_RELEASE_TARGET", "$AIT_RELEASE_VERSION"]],
              build: [["node", "release/fixture-receipt.mjs", "build", "$AIT_RELEASE_TARGET", "$AIT_RELEASE_VERSION"]],
              smoke: [["node", "release/fixture-receipt.mjs", "smoke", "$AIT_RELEASE_TARGET", "$AIT_RELEASE_VERSION"]]
            },
            artifacts: [{path: "dist/ait-native-1.0.0-rc.1.tgz", kind: "npm-cli-envelope"}]
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
  --version 1.0.0-rc.1 \
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
  --version 1.0.0-rc.1 \
  --git-commit "${fixture_git_commit}" \
  --out-dir "${temporary_root}/receipt-parent-link/escaped-receipt"
public_readme=${output_one}/README.md
for required_readme_text in \
  'ait init' \
  'AGENTS.md' \
  'ait workflow tier --json' \
  'ait task start' \
  'ait plan sync' \
  'ait snapshot create' \
  'ait task land'; do
  grep -F "${required_readme_text}" "${public_readme}" >/dev/null
done
if grep -F 'mkdir -p docs/sprints' "${public_readme}" >/dev/null; then
  printf 'public README teaches the user a manual sprint bootstrap\n' >&2
  exit 65
fi

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
test -f "${root_workflow}"
test "$(find "${output_one}/.github/workflows" -maxdepth 1 -type f | wc -l | tr -d '[:space:]')" = 1
grep -F '    working-directory: ait-core' "${root_workflow}" >/dev/null
grep -F '          path: ait-core/release-receipt-matrix.json' \
  "${root_workflow}" >/dev/null
if grep -F 'contents: write' "${root_workflow}" >/dev/null ||
  grep -F '          path: release-receipt-matrix.json' \
    "${root_workflow}" >/dev/null; then
  printf 'root protected workflow retained unsafe monorepo execution paths\n' >&2
  exit 65
fi

workflow_drift_output=${temporary_root}/workflow-drift-output
cp -R "${output_one}" "${workflow_drift_output}"
node "${repo_root}/ci/release_monorepo_transform.mjs" \
  "${workflow_drift_output}/.github/workflows/ait-release-component-receipts.yml" \
  '    working-directory: ait-core' \
  '    working-directory: .'
expect_failure workflow-drift node \
  "${workflow_drift_output}/build-release.mjs" --validate-only

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
