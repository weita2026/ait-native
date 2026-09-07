#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
usage:
  release_public_source.sh publish-qualification <public-root> <export-root> <absolute-output-dir>
  release_public_source.sh probe-qualification <public-root> <export-root> <absolute-output-dir>
  release_public_source.sh publish-final <public-root> <export-root> <qualification-head-file> <absolute-output-dir>
  release_public_source.sh probe-final <public-root> <export-root> <qualification-head-file> <absolute-output-dir>
  release_public_source.sh publish-tag <public-root> <candidate-json> <absolute-output-dir>
  release_public_source.sh probe-tag <public-root> <candidate-json> <absolute-output-dir>
USAGE
  exit 64
}

mode=${1:-}
shift || true
case "${mode}" in
  publish-qualification | probe-qualification) [[ $# -eq 3 ]] || usage ;;
  publish-final | probe-final) [[ $# -eq 4 ]] || usage ;;
  publish-tag | probe-tag) [[ $# -eq 3 ]] || usage ;;
  *) usage ;;
esac
public_root=$1
input_root=$2
if [[ ${mode} == *final ]]; then
  qualification_head_file=$3
  output=$4
else
  qualification_head_file=
  output=$3
fi
for input in "${public_root}" "${input_root}" "${output}"; do
  [[ ${input} == /* ]] || {
    printf 'release public-source paths must be absolute\n' >&2
    exit 64
  }
done
[[ -d ${public_root} && ! -L ${public_root} ]] || {
  printf 'release public-source checkout must be a real directory\n' >&2
  exit 66
}
for command in git jq node rsync; do
  command -v "${command}" >/dev/null 2>&1 || {
    printf 'required command is unavailable: %s\n' "${command}" >&2
    exit 69
  }
done

remote_main() {
  git -C "${public_root}" ls-remote --exit-code origin refs/heads/main | awk 'NR == 1 {print $1}'
}

tree_matches() {
  local export_root=$1
  local comparison
  comparison=$(mktemp)
  # Check content as well as size and mtime. Version-only releases commonly
  # replace same-size files created within the same timestamp granularity.
  rsync -acin --delete --exclude '.git/' "${export_root}/" "${public_root}/" >"${comparison}"
  if [[ -s ${comparison} ]]; then
    rm -f -- "${comparison}"
    return 1
  fi
  rm -f -- "${comparison}"
}

valid_receipt() {
  local kind=$1
  [[ -d ${output} && ! -L ${output} && -f ${output}/receipt.json &&
    -f ${output}/head.txt && ! -L ${output}/receipt.json && ! -L ${output}/head.txt ]] || return 1
  local head
  head=$(cat "${output}/head.txt")
  [[ ${head} =~ ^[0-9a-f]{40}$ ]] || return 1
  jq -e --arg kind "${kind}" --arg head "${head}" '
    .contract == "ait.release.public-source/v1" and .status == "complete" and
    .kind == $kind and .head_sha == $head
  ' "${output}/receipt.json" >/dev/null || return 1
  [[ $(git -C "${public_root}" rev-parse HEAD) == "${head}" && $(remote_main) == "${head}" ]] || return 1
}

probe_source() {
  local kind=$1 export_root=$2
  valid_receipt "${kind}" && tree_matches "${export_root}"
}

write_source_receipt() {
  local directory=$1 kind=$2 head=$3 parent=$4
  printf '%s\n' "${head}" >"${directory}/head.txt"
  jq -S -n --arg kind "${kind}" --arg head "${head}" --arg parent "${parent}" '
    {
      contract: "ait.release.public-source/v1",
      status: "complete",
      kind: $kind,
      head_sha: $head,
      parent_sha: (if $parent == "" then null else $parent end),
      remote: "origin",
      branch: "main"
    }
  ' >"${directory}/receipt.json"
}

publish_source() {
  local kind=$1 export_root=$2 qualified=${3:-}
  [[ -d ${export_root} && ! -L ${export_root} && -f ${export_root}/build-release.mjs ]] || {
    printf 'release public-source export is unavailable\n' >&2
    exit 66
  }
  [[ ! -e ${output} && ! -L ${output} ]] || {
    printf 'release public-source output already exists\n' >&2
    exit 73
  }
  local parent temporary head
  parent=$(git -C "${public_root}" rev-parse HEAD)
  [[ -z $(git -C "${public_root}" status --porcelain --untracked-files=all) ]] || {
    printf 'release public-source checkout is dirty\n' >&2
    exit 65
  }
  if tree_matches "${export_root}"; then
    head=${parent}
  else
    if [[ ${kind} == final && ${parent} != "${qualified}" ]]; then
      printf 'release final source is not based on the qualification commit\n' >&2
      exit 65
    fi
    rsync -ac --delete --exclude '.git/' "${export_root}/" "${public_root}/"
    git -C "${public_root}" add -A
    if [[ ${kind} == qualification ]]; then
      git -C "${public_root}" commit -m 'chore(release): qualify accepted stable source' >/dev/null
    else
      version=$(jq -er '.family.version' "${public_root}/ait-release-family.json")
      git -C "${public_root}" commit -m "release: AIT Native ${version} accepted source" >/dev/null
    fi
    head=$(git -C "${public_root}" rev-parse HEAD)
  fi
  if [[ ${kind} == final ]]; then
    [[ $(git -C "${public_root}" rev-parse "${head}^") == "${qualified}" ]] || {
      printf 'release final commit is not the direct child of qualification\n' >&2
      exit 65
    }
  fi
  node "${public_root}/build-release.mjs" --validate-only --git-commit "${head}" >/dev/null
  temporary=$(mktemp -d "$(dirname -- "${output}")/.$(basename -- "${output}").partial.XXXXXX")
  if [[ ${kind} == final ]]; then
    node "${public_root}/ci/release_pre_rc_delta.mjs" \
      --repository "${public_root}" --qualified-commit "${qualified}" --release-commit "${head}" \
      >"${temporary}/delta.json"
  fi
  git -C "${public_root}" push origin HEAD:main >/dev/null
  write_source_receipt "${temporary}" "${kind}" "${head}" "${parent}"
  mv "${temporary}" "${output}"
}

case "${mode}" in
  probe-qualification)
    probe_source qualification "${input_root}"
    ;;
  publish-qualification)
    publish_source qualification "${input_root}"
    ;;
  probe-final)
    [[ -f ${qualification_head_file} && ! -L ${qualification_head_file} ]] || exit 1
    qualification_head=$(cat "${qualification_head_file}")
    [[ ${qualification_head} =~ ^[0-9a-f]{40}$ ]] || exit 1
    probe_source final "${input_root}" &&
      [[ $(git -C "${public_root}" rev-parse HEAD^) == "${qualification_head}" ]]
    ;;
  publish-final)
    [[ -f ${qualification_head_file} && ! -L ${qualification_head_file} ]] || {
      printf 'release qualification head receipt is unavailable\n' >&2
      exit 66
    }
    qualification_head=$(cat "${qualification_head_file}")
    [[ ${qualification_head} =~ ^[0-9a-f]{40}$ ]] || {
      printf 'release qualification head receipt is invalid\n' >&2
      exit 65
    }
    publish_source final "${input_root}" "${qualification_head}"
    ;;
  probe-tag | publish-tag)
    candidate=${input_root}
    [[ -f ${candidate} && ! -L ${candidate} ]] || {
      [[ ${mode} == probe-tag ]] && exit 1
      printf 'release candidate binding is unavailable\n' >&2
      exit 66
    }
    IFS=$'\t' read -r tag version source_commit < <(
      jq -er '[.release.tag, .release.version, .release.source_commit] | @tsv' "${candidate}"
    )
    [[ ${tag} == "v${version}" && ${source_commit} =~ ^[0-9a-f]{40}$ ]] || {
      [[ ${mode} == probe-tag ]] && exit 1
      printf 'release candidate tag identity is invalid\n' >&2
      exit 65
    }
    remote_tag=$(git -C "${public_root}" ls-remote origin "refs/tags/${tag}^{}" | awk 'NR == 1 {print $1}')
    if [[ ${mode} == probe-tag ]]; then
      valid_receipt tag && [[ ${remote_tag} == "${source_commit}" ]]
      exit
    fi
    [[ ! -e ${output} && ! -L ${output} ]] || {
      printf 'release public-source output already exists\n' >&2
      exit 73
    }
    [[ $(git -C "${public_root}" rev-parse HEAD) == "${source_commit}" && $(remote_main) == "${source_commit}" ]] || {
      printf 'release tag source is not the exact public main head\n' >&2
      exit 65
    }
    if [[ -n ${remote_tag} && ${remote_tag} != "${source_commit}" ]]; then
      printf 'release tag already points to a different commit\n' >&2
      exit 65
    fi
    if [[ -z ${remote_tag} ]]; then
      git -C "${public_root}" tag -a "${tag}" -m "Release ${version}"
      git -C "${public_root}" push origin "refs/tags/${tag}" >/dev/null
      remote_tag=$(git -C "${public_root}" ls-remote origin "refs/tags/${tag}^{}" | awk 'NR == 1 {print $1}')
    fi
    [[ ${remote_tag} == "${source_commit}" ]] || {
      printf 'release tag remote readback failed\n' >&2
      exit 65
    }
    temporary=$(mktemp -d "$(dirname -- "${output}")/.$(basename -- "${output}").partial.XXXXXX")
    write_source_receipt "${temporary}" tag "${source_commit}" "$(git -C "${public_root}" rev-parse HEAD^)"
    mv "${temporary}" "${output}"
    ;;
esac
