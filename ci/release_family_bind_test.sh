#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
bind=${repo_root}/ci/release_family_bind.mjs
overlay=${repo_root}/ci/release_version_overlay.mjs
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-family-bind.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-family-bind.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing unexpected cleanup path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM

expect_failure() {
  local label=$1
  shift
  if "$@" >"${temporary_root}/${label}.stdout" 2>"${temporary_root}/${label}.stderr"; then
    printf 'expected family bind failure: %s\n' "${label}" >&2
    return 1
  fi
  test -s "${temporary_root}/${label}.stderr"
}

# The template is the canonical family with every selector unbound. Until the
# canonical manifest itself becomes the template, derive it here.
template=${temporary_root}/template.json
jq '.components |= map(.source_snapshot = "unbound")' \
  "${repo_root}/ait-release-family.json" >"${template}"
version=$(jq -er '.family.version' "${template}")

binding=${temporary_root}/binding.json
jq -n --arg version "${version}" '
  {
    contract: "ait.release.family-binding/v1",
    version: $version,
    tag: ("v" + $version),
    components: {
      "ait-core": "SNP-AAAAAAAAAAAA",
      "ait-server": "SNP-BBBBBBBBBBBB",
      "ait-runner": "SNP-CCCCCCCCCCCC",
      "ait-python": "SNP-DDDDDDDDDDDD",
      "ait-node": "SNP-EEEEEEEEEEEE"
    }
  }
' >"${binding}"

bound=${temporary_root}/bound.json
node "${bind}" --template "${template}" --binding "${binding}" --output "${bound}" >/dev/null
jq -e --arg version "${version}" '
  .schema == "ait.release.family/v3" and .family.version == $version and
  .family.tag == ("v" + $version) and
  ([.components[] | select(.source_repository == "ait-core") | .source_snapshot] | unique) == ["SNP-AAAAAAAAAAAA"] and
  ([.components[] | select(.source_repository == "ait-node") | .source_snapshot] | unique) == ["SNP-EEEEEEEEEEEE"] and
  ([.components[].source_snapshot] | all(test("^SNP-[0-9A-F]{12}$")))
' "${bound}" >/dev/null
jq 'del(.components[].source_snapshot)' "${template}" >"${temporary_root}/template-shape.json"
jq 'del(.components[].source_snapshot)' "${bound}" >"${temporary_root}/bound-shape.json"
cmp "${temporary_root}/template-shape.json" "${temporary_root}/bound-shape.json"
bound_repeat=${temporary_root}/bound-repeat.json
node "${bind}" --template "${template}" --binding "${binding}" --output "${bound_repeat}" >/dev/null
cmp "${bound}" "${bound_repeat}"
expect_failure existing-output node "${bind}" --template "${template}" --binding "${binding}" --output "${bound}"

IFS=. read -r major minor patch <<<"${version}"
if (( patch > 0 )); then prior=${major}.${minor}.$((patch - 1)); else prior=${major}.$((minor - 1)).0; fi
prior_bound=${temporary_root}/prior-bound.json
node "${bind}" --template "${template}" --binding "${binding}" --output "${prior_bound}" --version "${prior}" >/dev/null
jq -e --arg prior "${prior}" '
  .family.version == $prior and .family.tag == ("v" + $prior) and
  ([.components[].version] | unique) == [$prior] and
  ([.components[] | select(.source_repository == "ait-core") | .source_snapshot] | unique) == ["SNP-AAAAAAAAAAAA"]
' "${prior_bound}" >/dev/null
rc_bound=${temporary_root}/rc-bound.json
node "${bind}" --template "${template}" --binding "${binding}" --output "${rc_bound}" --version "${version}-rc.3" >/dev/null
jq -e --arg version "${version}" '
  .family.version == ($version + "-rc.3") and
  ([.components[] | select(.version_scheme == "pep440") | .version] | unique) == [$version + "rc3"] and
  ([.components[] | select(.version_scheme == "family") | .version] | unique) == [$version + "-rc.3"]
' "${rc_bound}" >/dev/null

expect_failure bound-template node "${bind}" --template "${bound}" --binding "${binding}" --output "${temporary_root}/rebound.json"
grep -F 'must be unbound' "${temporary_root}/bound-template.stderr" >/dev/null
jq 'del(.components["ait-node"])' "${binding}" >"${temporary_root}/incomplete.json"
expect_failure incomplete-binding node "${bind}" --template "${template}" --binding "${temporary_root}/incomplete.json" --output "${temporary_root}/incomplete-bound.json"
grep -F 'exactly the template repositories' "${temporary_root}/incomplete-binding.stderr" >/dev/null
jq '.components["ait-node"] = .components["ait-core"]' "${binding}" >"${temporary_root}/duplicate.json"
expect_failure duplicate-binding node "${bind}" --template "${template}" --binding "${temporary_root}/duplicate.json" --output "${temporary_root}/duplicate-bound.json"
jq '.version = "9.9.9" | .tag = "v9.9.9"' "${binding}" >"${temporary_root}/version-drift.json"
expect_failure version-drift node "${bind}" --template "${template}" --binding "${temporary_root}/version-drift.json" --output "${temporary_root}/version-drift-bound.json"
grep -F 'differs from the template version' "${temporary_root}/version-drift.stderr" >/dev/null
expect_failure bad-overlay node "${bind}" --template "${template}" --binding "${binding}" --output "${temporary_root}/bad-overlay.json" --version "${version}.1"

core=${temporary_root}/core
mkdir -p "${core}/rust" "${core}/ci"
printf 'version = "%s"\n' "${version}" >"${core}/rust/Cargo.toml"
for name in ait-agent-core ait-agent-worker ait-cli ait-napi ait-py; do
  printf '[[package]]\nname = "%s"\nversion = "%s"\n\n' "${name}" "${version}" >>"${core}/rust/Cargo.lock"
done
printf '{"version":"%s"}\n' "${version}" >"${core}/ait-release.json"
cp "${template}" "${core}/ait-release-family.json"
printf '{"version":"%s"}\n' "${version}" >"${core}/ci/native_bootstrap_matrix.json"
printf '{"family_version":"%s"}\n' "${version}" >"${core}/ci/release_repository_authorities.json"
node "${overlay}" --root "${core}" --repository ait-core --from "${version}" --to "${prior}" >"${temporary_root}/overlay.json"
jq -e --arg prior "${prior}" '
  .contract == "ait.release.version-overlay/v1" and .repository == "ait-core" and
  .to == $prior and (.changed_paths | length) == 6
' "${temporary_root}/overlay.json" >/dev/null
test "$(jq -r '.family.version' "${core}/ait-release-family.json")" = "${prior}"
test "$(jq -r '[.components[].source_snapshot] | unique | join(",")' "${core}/ait-release-family.json")" = unbound
grep -F "version = \"${prior}\"" "${core}/rust/Cargo.toml" >/dev/null
expect_failure overlay-same-version node "${overlay}" --root "${core}" --repository ait-core --from "${prior}" --to "${prior}"
expect_failure overlay-runner-without-core node "${overlay}" --root "${core}" --repository ait-runner --from "${version}" --to "${prior}"

# Gate C overlays Runner's own release version while its declared dependency
# remains immutable, including source text that contains the family version.
runner=${temporary_root}/runner
mkdir -p "${runner}/.ait-external/ait-core" "${runner}/tests"
printf '[package]\nname = "ait-runner"\nversion = "%s"\n\n[dependencies]\nait-core = { path = ".ait-external/ait-core/rust/crates/ait-core" }\n' \
  "${version}" >"${runner}/Cargo.toml"
printf '[[package]]\nname = "ait-runner"\nversion = "%s"\n' "${version}" >"${runner}/Cargo.lock"
printf '{"package":{"version":"%s"}}\n' "${version}" >"${runner}/ait-release.json"
for spec in 'ait-external.toml external' 'ait-external.lock node'; do
  read -r name table <<<"${spec}"
  printf '[[%s]]\nname = "ait-core"\nsnapshot = "SNP-B1E5120FA441"\nversion = "1.1.4"\n' \
    "${table}" >"${runner}/${name}"
done
printf 'const CORE: &str = "SNP-B1E5120FA441";\n' >"${runner}/tests/cli.rs"
printf 'immutable source token %s\n' "${version}" >"${runner}/.ait-external/ait-core/source.rs"
cp -R "${runner}" "${temporary_root}/runner-before"
node "${overlay}" --root "${runner}" --repository ait-runner --from "${version}" --to "${prior}" \
  --core-snapshot SNP-B1E5120FA441 >"${temporary_root}/runner-overlay.json"
jq -e --arg prior "${prior}" '
  .contract == "ait.release.version-overlay/v1" and .repository == "ait-runner" and
  .to == $prior and .changed_paths == ["Cargo.lock", "Cargo.toml", "ait-release.json"]
' "${temporary_root}/runner-overlay.json" >/dev/null
grep -F "version = \"${prior}\"" "${runner}/Cargo.toml" >/dev/null
grep -F "version = \"${prior}\"" "${runner}/Cargo.lock" >/dev/null
test "$(jq -r '.package.version' "${runner}/ait-release.json")" = "${prior}"
for name in ait-external.toml ait-external.lock tests/cli.rs; do
  cmp "${temporary_root}/runner-before/${name}" "${runner}/${name}"
done
diff -r "${temporary_root}/runner-before/.ait-external" "${runner}/.ait-external"

printf 'release family bind tests passed\n'
