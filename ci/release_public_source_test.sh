#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-public-source.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-public-source.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing unexpected cleanup path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM
remote=${temporary_root}/remote.git
public=${temporary_root}/public
prior=${temporary_root}/prior
final=${temporary_root}/final
records=${temporary_root}/records
git init --bare "${remote}" >/dev/null
git clone "${remote}" "${public}" >/dev/null 2>&1
git -C "${public}" config user.name 'AIT Release Test'
git -C "${public}" config user.email 'release-test@localhost'
write_readmes() {
  local root=$1 source_version=$2 name notice
  for name in README.md README_CN.md README_ZH.md; do
    case ${name} in
      README.md) notice='AIT 1.2.0 is in development and has not been released.' ;;
      README_CN.md) notice='AIT 1.2.0 仍在开发中，尚未发布。' ;;
      README_ZH.md) notice='AIT 1.2.0 仍在開發中，尚未發布。' ;;
    esac
    printf '# AIT\n%s\nsource %s\nreleases/tag/v1.1.4\nait-native==1.1.4\n@wa120/ait-native@1.1.4\n' \
      "${notice}" "${source_version}" >"${root}/${name}"
  done
}
check_published_readmes() {
  local export_root=$1 name
  for name in README.md README_CN.md README_ZH.md; do
    cmp -s "${export_root}/${name}" "${public}/${name}"
    git -C "${public}" show "HEAD:${name}" | cmp -s - "${public}/${name}"
  done
}
write_readmes "${public}" 1.1.4
git -C "${public}" add -- README.md README_CN.md README_ZH.md
git -C "${public}" commit -m base >/dev/null
git -C "${public}" branch -M main
git -C "${public}" push -u origin main >/dev/null 2>&1
mkdir -p "${prior}/ci" "${final}/ci" "${records}"
for root in "${prior}" "${final}"; do
  cat >"${root}/build-release.mjs" <<'JS'
#!/usr/bin/env node
if (!process.argv.includes("--validate-only")) process.exit(64);
JS
  cat >"${root}/ci/release_pre_rc_delta.mjs" <<'JS'
#!/usr/bin/env node
process.stdout.write('{"decision":"pass"}\n');
JS
done
cat >"${prior}/ait-release-family.json" <<'JSON'
{"family":{"version":"1.1.1","tag":"v1.1.1"}}
JSON
cat >"${final}/ait-release-family.json" <<'JSON'
{"family":{"version":"1.1.2","tag":"v1.1.2"}}
JSON
write_readmes "${prior}" 1.1.1
write_readmes "${final}" 1.1.2
for root in "${prior}" "${final}"; do
  printf '#!/bin/sh\nexit 0\n' >"${root}/guard.sh"
  chmod 0755 "${root}/guard.sh"
  touch -t 202001010000 "${root}/guard.sh"
  ln -s README.md "${root}/readme-link"
done

"${repo_root}/ci/release_public_source.sh" publish-qualification \
  "${public}" "${prior}" "${records}/qualification" >/dev/null
"${repo_root}/ci/release_public_source.sh" probe-qualification \
  "${public}" "${prior}" "${records}/qualification"
qualification_head=$(cat "${records}/qualification/head.txt")
test "$(git -C "${public}" rev-parse HEAD)" = "${qualification_head}"
check_published_readmes "${prior}"

"${repo_root}/ci/release_public_source.sh" publish-final \
  "${public}" "${final}" "${records}/qualification/head.txt" \
  "${records}/final" >/dev/null
"${repo_root}/ci/release_public_source.sh" probe-final \
  "${public}" "${final}" "${records}/qualification/head.txt" \
  "${records}/final"
final_head=$(cat "${records}/final/head.txt")
test "$(git -C "${public}" rev-parse HEAD^)" = "${qualification_head}"
check_published_readmes "${final}"

# Re-exporting identical Git content later must neither reject the probe nor
# create a metadata-only final commit. Check source mtimes as well as bytes.
timestamp_export=${temporary_root}/timestamp-export
cp -Rp "${final}" "${timestamp_export}"
node --input-type=module - "${timestamp_export}" <<'JS'
import fs from 'node:fs';import path from 'node:path';
function age(file){const s=fs.lstatSync(file);if(s.isSymbolicLink()){fs.lutimesSync(file,946684800,946684800);return;}if(s.isDirectory())for(const n of fs.readdirSync(file))age(path.join(file,n));fs.utimesSync(file,946684800,946684800);}
age(process.argv[2]);
JS
snapshot_trees() {
  node --input-type=module - "${public}" "${timestamp_export}" <<'JS'
import fs from 'node:fs';import path from 'node:path';import{createHash}from'node:crypto';
const rows=[];
function walk(root,file){const s=fs.lstatSync(file);rows.push([file,s.mode,s.mtimeMs,s.isSymbolicLink()?fs.readlinkSync(file):s.isFile()?createHash('sha256').update(fs.readFileSync(file)).digest('hex'):null]);if(s.isDirectory())for(const n of fs.readdirSync(file).sort())if(n!=='.git')walk(root,path.join(file,n));}
for(const root of process.argv.slice(2))walk(root,root);process.stdout.write(JSON.stringify(rows));
JS
}
snapshot_trees >"${temporary_root}/trees-before.json"
"${repo_root}/ci/release_public_source.sh" probe-final "${public}" "${timestamp_export}" \
  "${records}/qualification/head.txt" "${records}/final"
"${repo_root}/ci/release_public_source.sh" publish-final "${public}" "${timestamp_export}" \
  "${records}/qualification/head.txt" "${records}/final-reuse" >/dev/null
snapshot_trees >"${temporary_root}/trees-after.json"
cmp "${temporary_root}/trees-before.json" "${temporary_root}/trees-after.json"
test "$(cat "${records}/final-reuse/head.txt")" = "${final_head}"
test "$(git -C "${public}" rev-parse HEAD)" = "${final_head}"
test "$(git -C "${public}" ls-remote origin refs/heads/main | awk 'NR == 1 {print $1}')" = "${final_head}"
test -z "$(git -C "${public}" status --porcelain --untracked-files=all)"

for drift in bytes added deleted renamed type symlink mode; do
  drift_export=${temporary_root}/drift-${drift}
  cp -Rp "${timestamp_export}" "${drift_export}"
  case ${drift} in
    bytes)
      node --input-type=module - "${drift_export}/guard.sh" "${public}/guard.sh" <<'JS'
import fs from 'node:fs';import assert from 'node:assert/strict';
const[file,reference]=process.argv.slice(2),before=fs.readFileSync(reference),changed=Buffer.from(before),s=fs.statSync(reference);
changed[changed.length-2]^=1;fs.writeFileSync(file,changed);fs.utimesSync(file,s.atime,s.mtime);
assert.equal(fs.statSync(file).size,s.size);assert.equal(fs.statSync(file).mtimeMs,s.mtimeMs);assert.notDeepEqual(fs.readFileSync(file),before);
JS
      ;;
    added) printf 'extra\n' >"${drift_export}/added.txt" ;;
    deleted) rm "${drift_export}/README_ZH.md" ;;
    renamed) mv "${drift_export}/README_ZH.md" "${drift_export}/renamed.md" ;;
    type) rm "${drift_export}/guard.sh"; mkdir "${drift_export}/guard.sh" ;;
    symlink) rm "${drift_export}/readme-link"; ln -s README_CN.md "${drift_export}/readme-link" ;;
    mode) chmod 0644 "${drift_export}/guard.sh" ;;
  esac
  if "${repo_root}/ci/release_public_source.sh" probe-final "${public}" "${drift_export}" \
    "${records}/qualification/head.txt" "${records}/final"; then
    printf 'public-source probe accepted %s drift\n' "${drift}" >&2
    exit 1
  fi
done
printf '%s\n' "${final_head}" >"${records}/wrong-qualified-parent.txt"
if "${repo_root}/ci/release_public_source.sh" probe-final "${public}" "${timestamp_export}" \
  "${records}/wrong-qualified-parent.txt" "${records}/final"; then
  printf 'public-source probe accepted the wrong qualified parent\n' >&2; exit 1
fi
git --git-dir="${remote}" update-ref refs/heads/main "${qualification_head}" "${final_head}"
if "${repo_root}/ci/release_public_source.sh" probe-final "${public}" "${timestamp_export}" \
  "${records}/qualification/head.txt" "${records}/final"; then
  printf 'public-source probe accepted remote F identity drift\n' >&2; exit 1
fi
git --git-dir="${remote}" update-ref refs/heads/main "${final_head}" "${qualification_head}"
if "${repo_root}/ci/release_public_source.sh" probe-final "${public}" "${temporary_root}/missing-export" \
  "${records}/qualification/head.txt" "${records}/final"; then
  printf 'public-source probe accepted a missing export\n' >&2; exit 1
fi
printf 'unreviewed README change\n' >>"${public}/README_CN.md"
if "${repo_root}/ci/release_public_source.sh" probe-final "${public}" "${final}" \
  "${records}/qualification/head.txt" "${records}/final"; then
  printf 'public-source probe accepted README drift\n' >&2
  exit 1
fi
git -C "${public}" restore -- README_CN.md
printf 'unreviewed source change\n' >"${public}/unexpected.txt"
if "${repo_root}/ci/release_public_source.sh" probe-final "${public}" "${final}" \
  "${records}/qualification/head.txt" "${records}/final"; then
  printf 'public-source probe accepted source drift\n' >&2
  exit 1
fi
rm -- "${public}/unexpected.txt"
cat >"${records}/candidate.json" <<JSON
{"release":{"tag":"v1.1.2","version":"1.1.2","source_commit":"${final_head}"}}
JSON
"${repo_root}/ci/release_public_source.sh" publish-tag \
  "${public}" "${records}/candidate.json" "${records}/tag" >/dev/null
"${repo_root}/ci/release_public_source.sh" probe-tag \
  "${public}" "${records}/candidate.json" "${records}/tag"
test "$(git -C "${public}" ls-remote origin 'refs/tags/v1.1.2^{}' | awk 'NR == 1 {print $1}')" = "${final_head}"

# Runner's actual nested ignore rule defeats the public root exception. Both
# publication paths must retain the complete validated dependency in Git.
runner_prior=${temporary_root}/runner-prior
runner_final=${temporary_root}/runner-final
for spec in "${runner_prior} 1.1.4" "${runner_final} 1.2.0"; do
  read -r root version <<<"${spec}"
  mkdir -p "${root}/ci" "${root}/ait-core" "${root}/ait-runner/.ait-external/ait-core/ci" \
    "${root}/ait-runner/.ait-external/ait-core/rust"
  write_readmes "${root}" "${version}"
  cp "${prior}/build-release.mjs" "${root}/build-release.mjs"
  cp "${prior}/ci/release_pre_rc_delta.mjs" "${root}/ci/"
  cp "${repo_root}/release/monorepo/.gitignore" "${root}/.gitignore"
  mkdir -p "${root}/.ait/generated"
  printf 'ignored local state\n' >"${root}/.ait/generated/state"
  printf '{"family":{"version":"%s","tag":"v%s"}}\n' "${version}" "${version}" >"${root}/ait-release-family.json"
  printf 'main producer E053\n' >"${root}/ait-core/source.rs"
  printf '/target/\n.ait-external/\n.DS_Store\n' >"${root}/ait-runner/.gitignore"
  printf '[package]\nname = "ait-runner"\nversion = "%s"\n[dependencies]\nait-core = { path = ".ait-external/ait-core/rust/crates/ait-core" }\n' \
    "${version}" >"${root}/ait-runner/Cargo.toml"
  for item in 'ait-external.toml external' 'ait-external.lock node'; do
    read -r filename table <<<"${item}"
    printf '[[%s]]\nname = "ait-core"\nrepo_name = "ait-core"\nrepository_index = 0\nsnapshot = "SNP-B1E5120FA441"\nmaterialize_to = ".ait-external/ait-core"\nlicense = "Apache-2.0"\nversion = "1.1.4"\n' \
      "${table}" >"${root}/ait-runner/${filename}"
  done
  dependency=${root}/ait-runner/.ait-external/ait-core
  printf '[workspace]\n' >"${dependency}/rust/Cargo.toml"
  printf 'literal source\n' >"${dependency}/literal [glob]*.rs"
  printf 'unicode source\n' >"${dependency}/source λ.rs"
  printf '\000\012\015\377' >"${dependency}/binary.dat"
  printf '#!/bin/sh\nexit 0\n' >"${dependency}/ci/check.sh"
  chmod 0755 "${dependency}/ci/check.sh"
  jq -n '{contract:"ait.release.source-cache/v1",status:"ready",repo_name:"ait-core",repository_index:0,
    source_snapshot:"SNP-B1E5120FA441",source_manifest_hash:("b" * 64),source_snapshot_created_at:"1700000000",
    version:"1.1.4",license:"Apache-2.0",source_authority:"ait_remote_snapshot_boundary",workspace_clean:true,
    remote_coordinates_embedded:false,public_publish:false,materialize_to:".ait-external/ait-core"}' \
    >"${dependency}/source-cache.evidence.json"
done

ordinary=${temporary_root}/ordinary
cp -R "${runner_prior}" "${ordinary}"
git -C "${ordinary}" init -q
git -C "${ordinary}" add -A
if git -C "${ordinary}" ls-files --error-unmatch 'ait-runner/.ait-external/ait-core/rust/Cargo.toml' >/dev/null 2>&1; then
  printf 'ordinary staging unexpectedly retained the nested-ignore fixture\n' >&2
  exit 1
fi
for kind in qualification final; do
  if [[ ${kind} == qualification ]]; then
    "${repo_root}/ci/release_public_source.sh" publish-qualification "${public}" "${runner_prior}" "${records}/runner-qualification" >/dev/null
  else
    "${repo_root}/ci/release_public_source.sh" publish-final "${public}" "${runner_final}" "${records}/runner-qualification/head.txt" "${records}/runner-final" >/dev/null
  fi
  checked=${temporary_root}/fresh-${kind}
  git clone -q --single-branch --branch main "${remote}" "${checked}"
  exported=${runner_prior}
  if [[ ${kind} == final ]]; then exported=${runner_final}; fi
  check_published_readmes "${exported}"
  node --input-type=module - "${repo_root}" "${exported}" "${checked}" <<'JS'
import { pathToFileURL } from 'node:url';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
const [root, exported, checkout] = process.argv.slice(2);
const { verifyPublicRunnerDependencyCommit } = await import(pathToFileURL(root + '/ci/release_runner_dependency.mjs'));
verifyPublicRunnerDependencyCommit(exported, checkout, execFileSync('git', ['-C', checkout, 'rev-parse', 'HEAD'], {encoding:'utf8'}).trim());
assert.throws(() => verifyPublicRunnerDependencyCommit(exported, checkout, undefined), /commit identity is invalid/);
JS
  test -x "${checked}/ait-runner/.ait-external/ait-core/ci/check.sh"
  test ! -e "${checked}/.ait"
  diff -r "${exported}/ait-runner/.ait-external/ait-core" "${checked}/ait-runner/.ait-external/ait-core"
done

# Inject changes after rsync/ordinary add but before the narrowly forced add.
# Unexpected files, byte/mode drift and operational state must stop before push.
wrapper=${temporary_root}/git-wrapper
mkdir "${wrapper}"
source_git_bin=$(command -v git)
cat >"${wrapper}/git" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} == -C && ${2:-} == "${AIT_PUBLIC_TEST_CHECKOUT}" && ${3:-} == add && ${4:-} == -A ]]; then
  dependency=${AIT_PUBLIC_TEST_CHECKOUT}/ait-runner/.ait-external/ait-core
  case "${AIT_PUBLIC_TEST_TAMPER}" in
    extra) printf 'not in exported inventory\n' >"${dependency}/unlisted-secret.env" ;;
    bytes) printf 'different source bytes\n' >"${dependency}/rust/Cargo.toml" ;;
    mode) chmod 0644 "${dependency}/ci/check.sh" ;;
    state) mkdir -p "${dependency}/.ait"; printf 'state\n' >"${dependency}/.ait/state" ;;
    cache) mkdir -p "${dependency}/rust/target"; printf 'cache\n' >"${dependency}/rust/target/cache" ;;
    gitstate) mkdir -p "${dependency}/.git"; printf 'git state\n' >"${dependency}/.git/config" ;;
    link) ln -s "${AIT_PUBLIC_TEST_OUTSIDE}" "${dependency}/outside-link" ;;
  esac
fi
if [[ " $* " == *' --force '* ]]; then printf 'forced\n' >>"${AIT_PUBLIC_TEST_FORCE_LOG}"; fi
exec "${AIT_PUBLIC_TEST_GIT}" "$@"
SH
chmod 0755 "${wrapper}/git"
printf 'outside fixture\n' >"${temporary_root}/outside"
for tamper in extra bytes mode state cache gitstate link; do
  negative=${temporary_root}/negative-${tamper}
  git clone -q --single-branch --branch main "${remote}" "${negative}"
  git -C "${negative}" config user.name 'AIT Release Test'
  git -C "${negative}" config user.email 'release-test@localhost'
  before=$(git -C "${negative}" rev-parse HEAD)
  force_log=${temporary_root}/${tamper}-force.log
  if PATH="${wrapper}:${PATH}" AIT_PUBLIC_TEST_GIT="${source_git_bin}" AIT_PUBLIC_TEST_CHECKOUT="${negative}" \
    AIT_PUBLIC_TEST_TAMPER="${tamper}" AIT_PUBLIC_TEST_OUTSIDE="${temporary_root}/outside" AIT_PUBLIC_TEST_FORCE_LOG="${force_log}" \
    "${repo_root}/ci/release_public_source.sh" publish-qualification "${negative}" "${runner_prior}" "${records}/negative-${tamper}" \
      >"${temporary_root}/${tamper}.stdout" 2>"${temporary_root}/${tamper}.stderr"; then
    printf 'unexpected public-source acceptance after %s drift\n' "${tamper}" >&2
    exit 1
  fi
  test -s "${temporary_root}/${tamper}.stderr"
  test ! -e "${force_log}"
  test "$(git -C "${negative}" rev-parse HEAD)" = "${before}"
  test "$(git -C "${negative}" ls-remote origin refs/heads/main | awk 'NR == 1 {print $1}')" = "${before}"
done

printf 'release public-source tests passed\n'
