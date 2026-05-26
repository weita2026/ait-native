#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DEFAULT_DIST_DIR="${AIT_GITHUB_RELEASE_DIST_DIR:-${WORKSPACE_ROOT}/dist}"
DEFAULT_REMOTE_URL="${AIT_GITHUB_RELEASE_REMOTE_URL:-git@github.com:weita2026/ait-native.git}"
DEFAULT_REMOTE_NAME="${AIT_GITHUB_RELEASE_REMOTE_NAME:-origin}"
DEFAULT_BASE_BRANCH="${AIT_GITHUB_RELEASE_BASE_BRANCH:-main}"
DEFAULT_ASSET_BRANCH="${AIT_GITHUB_RELEASE_ASSET_BRANCH:-release-assets}"
DEFAULT_REPO_FULL_NAME="${AIT_GITHUB_RELEASE_REPO_FULL_NAME:-weita2026/ait-native}"
DEFAULT_FORMULA_PATH="${AIT_GITHUB_RELEASE_FORMULA_PATH:-Formula/ait-native.rb}"
DEFAULT_PYTHON_BIN="${AIT_GITHUB_RELEASE_PYTHON:-python3}"

usage() {
  cat <<EOF
Usage:
  $(basename "$0") metadata --version <version> [--dist-dir <dir>] [--repo-full-name <owner/repo>]
  $(basename "$0") rewrite-formula --version <version> --formula <path> [--dist-dir <dir>] [--repo-full-name <owner/repo>]
  $(basename "$0") publish-assets-ref --version <version> [--dist-dir <dir>] [--notes-file <path>] [--remote-url <url>] [--remote <name>] [--base-branch <branch>] [--branch <branch>] [--ref-tag <tag>] [--message <message>] [--repo-full-name <owner/repo>] [--force] [--keep-temp]

Notes:
  - metadata reports the exact filenames, release tag, asset ref tag, and wheel checksum for one prepared release.
  - rewrite-formula patches one Homebrew formula to the GitHub Releases wheel URL plus the local wheel checksum.
  - publish-assets-ref pushes the prepared wheel, sdist, manifest, checksum, and release-notes payload to a dedicated asset ref without requiring local \`gh auth login\`.
  - The paired public workflow \`.github/workflows/github-release-publish.yml\` consumes the matching \`release-assets-v*\` ref when the real \`v*\` tag is pushed on the public repo.
EOF
}

python_cmd() {
  "${DEFAULT_PYTHON_BIN}" "$@"
}

die() {
  echo "$*" >&2
  exit 1
}

normalize_release_tag() {
  local raw="${1:-}"
  [[ -n "${raw}" ]] || die "Missing release version."
  if [[ "${raw}" == v* ]]; then
    printf '%s\n' "${raw}"
  else
    printf 'v%s\n' "${raw}"
  fi
}

package_version_from_release_tag() {
  local release_tag
  release_tag="$(normalize_release_tag "${1:-}")"
  printf '%s\n' "${release_tag#v}"
}

require_file() {
  local path="$1"
  [[ -f "${path}" ]] || die "Required file is missing: ${path}"
}

trim_trailing_slash() {
  local path="$1"
  printf '%s\n' "${path%/}"
}

release_metadata() {
  local version="$1"
  local dist_dir="$2"
  local repo_full_name="$3"

  RELEASE_TAG="$(normalize_release_tag "${version}")"
  PACKAGE_VERSION="$(package_version_from_release_tag "${RELEASE_TAG}")"
  DIST_DIR="$(trim_trailing_slash "${dist_dir}")"
  REPO_FULL_NAME="${repo_full_name}"
  RELEASE_DIR="releases/${RELEASE_TAG}"
  ASSET_REF_TAG="release-assets-${RELEASE_TAG}"
  WHEEL_NAME="ait_native-${PACKAGE_VERSION}-py3-none-any.whl"
  SDIST_NAME="ait-native-${PACKAGE_VERSION}.tar.gz"
  MANIFEST_NAME="ait-release-${PACKAGE_VERSION}.manifest.json"
  CHECKSUM_NAME="ait-release-${PACKAGE_VERSION}.sha256"
  RELEASE_NOTES_NAME="release-notes.md"
  WHEEL_PATH="${DIST_DIR}/${WHEEL_NAME}"
  SDIST_PATH="${DIST_DIR}/${SDIST_NAME}"
  MANIFEST_PATH="${DIST_DIR}/${MANIFEST_NAME}"
  CHECKSUM_PATH="${DIST_DIR}/${CHECKSUM_NAME}"
  WHEEL_RELEASE_URL="https://github.com/${REPO_FULL_NAME}/releases/download/${RELEASE_TAG}/${WHEEL_NAME}"

  require_file "${WHEEL_PATH}"
  require_file "${SDIST_PATH}"
  require_file "${MANIFEST_PATH}"
  require_file "${CHECKSUM_PATH}"

  WHEEL_SHA256="$(shasum -a 256 "${WHEEL_PATH}" | awk '{print $1}')"
}

emit_metadata() {
  cat <<EOF
release_tag=${RELEASE_TAG}
package_version=${PACKAGE_VERSION}
repo_full_name=${REPO_FULL_NAME}
release_dir=${RELEASE_DIR}
asset_ref_tag=${ASSET_REF_TAG}
wheel_name=${WHEEL_NAME}
sdist_name=${SDIST_NAME}
manifest_name=${MANIFEST_NAME}
checksum_name=${CHECKSUM_NAME}
wheel_path=${WHEEL_PATH}
sdist_path=${SDIST_PATH}
manifest_path=${MANIFEST_PATH}
checksum_path=${CHECKSUM_PATH}
wheel_sha256=${WHEEL_SHA256}
wheel_release_url=${WHEEL_RELEASE_URL}
EOF
}

rewrite_formula_file() {
  local formula_path="$1"
  require_file "${formula_path}"
  python_cmd - "${formula_path}" "${WHEEL_RELEASE_URL}" "${WHEEL_SHA256}" <<'PY'
from __future__ import annotations

from pathlib import Path
import re
import sys

formula_path = Path(sys.argv[1])
url = sys.argv[2]
sha256 = sys.argv[3]
text = formula_path.read_text(encoding="utf-8")
updated = re.sub(r'^(\s*url\s+)"[^"]+"', rf'\1"{url}"', text, count=1, flags=re.MULTILINE)
updated = re.sub(r'^(\s*sha256\s+)"[^"]+"', rf'\1"{sha256}"', updated, count=1, flags=re.MULTILINE)
if updated == text:
    raise SystemExit(f"Formula {formula_path} did not contain updatable url/sha256 lines.")
formula_path.write_text(updated, encoding="utf-8")
PY
}

resolve_git_identity() {
  GIT_AUTHOR_NAME_VALUE="${AIT_GITHUB_RELEASE_GIT_NAME:-$(git config --get user.name 2>/dev/null || git config --global --get user.name 2>/dev/null || true)}"
  GIT_AUTHOR_EMAIL_VALUE="${AIT_GITHUB_RELEASE_GIT_EMAIL:-$(git config --get user.email 2>/dev/null || git config --global --get user.email 2>/dev/null || true)}"
  [[ -n "${GIT_AUTHOR_NAME_VALUE}" ]] || die "Git author name is not configured. Set AIT_GITHUB_RELEASE_GIT_NAME or git user.name."
  [[ -n "${GIT_AUTHOR_EMAIL_VALUE}" ]] || die "Git author email is not configured. Set AIT_GITHUB_RELEASE_GIT_EMAIL or git user.email."
}

write_release_notes() {
  local destination="$1"
  local notes_file="${2:-}"
  if [[ -n "${notes_file}" ]]; then
    require_file "${notes_file}"
    cp "${notes_file}" "${destination}"
    return
  fi
  cat >"${destination}" <<EOF
# ${RELEASE_TAG}

Automated GitHub Release publication for ${RELEASE_TAG}.

- Assets were prepared locally and pushed through ${ASSET_REF_TAG}.
- The public workflow \`.github/workflows/github-release-publish.yml\` uploads this wheel, sdist, manifest, and checksum payload to the real GitHub Release for ${RELEASE_TAG}.
EOF
}

publish_assets_ref() {
  local version="$1"
  local dist_dir="$2"
  local notes_file="$3"
  local remote_url="$4"
  local remote_name="$5"
  local base_branch="$6"
  local asset_branch="$7"
  local ref_tag="$8"
  local message="$9"
  local repo_full_name="${10}"
  local force_push="${11}"
  local keep_temp="${12}"

  release_metadata "${version}" "${dist_dir}" "${repo_full_name}"
  resolve_git_identity

  local temp_dir
  temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/ait-github-release.XXXXXX")"
  if [[ "${keep_temp}" != "1" ]]; then
    trap "rm -rf \"${temp_dir}\"" EXIT
  fi

  local clone_dir="${temp_dir}/repo"
  git clone --branch "${base_branch}" --single-branch "${remote_url}" "${clone_dir}" >/dev/null
  git -C "${clone_dir}" config user.name "${GIT_AUTHOR_NAME_VALUE}"
  git -C "${clone_dir}" config user.email "${GIT_AUTHOR_EMAIL_VALUE}"
  git -C "${clone_dir}" checkout -B "${asset_branch}" "${remote_name}/${base_branch}" >/dev/null

  local target_dir="${clone_dir}/${RELEASE_DIR}"
  mkdir -p "${target_dir}"
  cp "${WHEEL_PATH}" "${target_dir}/${WHEEL_NAME}"
  cp "${SDIST_PATH}" "${target_dir}/${SDIST_NAME}"
  cp "${MANIFEST_PATH}" "${target_dir}/${MANIFEST_NAME}"
  cp "${CHECKSUM_PATH}" "${target_dir}/${CHECKSUM_NAME}"
  write_release_notes "${target_dir}/${RELEASE_NOTES_NAME}" "${notes_file}"

  git -C "${clone_dir}" add -- "${RELEASE_DIR}"
  if ! git -C "${clone_dir}" diff --cached --quiet; then
    git -C "${clone_dir}" commit -m "${message}" >/dev/null
  fi

  if git -C "${clone_dir}" rev-parse --verify --quiet "refs/tags/${ref_tag}" >/dev/null; then
    git -C "${clone_dir}" tag -d "${ref_tag}" >/dev/null
  fi
  git -C "${clone_dir}" tag -a "${ref_tag}" -m "${message}" >/dev/null

  if [[ "${force_push}" == "1" ]]; then
    git -C "${clone_dir}" push --force-with-lease "${remote_name}" "HEAD:${asset_branch}" >/dev/null
    git -C "${clone_dir}" push --force "${remote_name}" "refs/tags/${ref_tag}" >/dev/null
  else
    git -C "${clone_dir}" push --force-with-lease "${remote_name}" "HEAD:${asset_branch}" >/dev/null
    git -C "${clone_dir}" push "${remote_name}" "refs/tags/${ref_tag}" >/dev/null
  fi

  emit_metadata
  printf 'asset_branch=%s\n' "${asset_branch}"
  printf 'asset_remote=%s\n' "${remote_url}"
  if [[ "${keep_temp}" == "1" ]]; then
    printf 'temp_repo=%s\n' "${clone_dir}"
  fi
}

cmd="${1:-}"
if [[ -z "${cmd}" ]]; then
  usage
  exit 0
fi
shift

case "${cmd}" in
  metadata)
    version=""
    dist_dir="${DEFAULT_DIST_DIR}"
    repo_full_name="${DEFAULT_REPO_FULL_NAME}"
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --version)
          version="$2"
          shift 2
          ;;
        --dist-dir)
          dist_dir="$2"
          shift 2
          ;;
        --repo-full-name)
          repo_full_name="$2"
          shift 2
          ;;
        *)
          die "Unknown metadata argument: $1"
          ;;
      esac
    done
    [[ -n "${version}" ]] || die "`metadata` requires --version."
    release_metadata "${version}" "${dist_dir}" "${repo_full_name}"
    emit_metadata
    ;;
  rewrite-formula)
    version=""
    formula_path="${DEFAULT_FORMULA_PATH}"
    dist_dir="${DEFAULT_DIST_DIR}"
    repo_full_name="${DEFAULT_REPO_FULL_NAME}"
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --version)
          version="$2"
          shift 2
          ;;
        --formula)
          formula_path="$2"
          shift 2
          ;;
        --dist-dir)
          dist_dir="$2"
          shift 2
          ;;
        --repo-full-name)
          repo_full_name="$2"
          shift 2
          ;;
        *)
          die "Unknown rewrite-formula argument: $1"
          ;;
      esac
    done
    [[ -n "${version}" ]] || die "`rewrite-formula` requires --version."
    release_metadata "${version}" "${dist_dir}" "${repo_full_name}"
    rewrite_formula_file "${formula_path}"
    emit_metadata
    printf 'formula_path=%s\n' "${formula_path}"
    ;;
  publish-assets-ref)
    version=""
    dist_dir="${DEFAULT_DIST_DIR}"
    notes_file=""
    remote_url="${DEFAULT_REMOTE_URL}"
    remote_name="${DEFAULT_REMOTE_NAME}"
    base_branch="${DEFAULT_BASE_BRANCH}"
    asset_branch="${DEFAULT_ASSET_BRANCH}"
    repo_full_name="${DEFAULT_REPO_FULL_NAME}"
    ref_tag=""
    message=""
    force_push="0"
    keep_temp="0"
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --version)
          version="$2"
          shift 2
          ;;
        --dist-dir)
          dist_dir="$2"
          shift 2
          ;;
        --notes-file)
          notes_file="$2"
          shift 2
          ;;
        --remote-url)
          remote_url="$2"
          shift 2
          ;;
        --remote)
          remote_name="$2"
          shift 2
          ;;
        --base-branch)
          base_branch="$2"
          shift 2
          ;;
        --branch)
          asset_branch="$2"
          shift 2
          ;;
        --repo-full-name)
          repo_full_name="$2"
          shift 2
          ;;
        --ref-tag)
          ref_tag="$2"
          shift 2
          ;;
        --message)
          message="$2"
          shift 2
          ;;
        --force)
          force_push="1"
          shift
          ;;
        --keep-temp)
          keep_temp="1"
          shift
          ;;
        *)
          die "Unknown publish-assets-ref argument: $1"
          ;;
      esac
    done
    [[ -n "${version}" ]] || die "`publish-assets-ref` requires --version."
    if [[ -z "${ref_tag}" ]]; then
      ref_tag="release-assets-$(normalize_release_tag "${version}")"
    fi
    if [[ -z "${message}" ]]; then
      message="Publish $(normalize_release_tag "${version}") release assets"
    fi
    publish_assets_ref \
      "${version}" \
      "${dist_dir}" \
      "${notes_file}" \
      "${remote_url}" \
      "${remote_name}" \
      "${base_branch}" \
      "${asset_branch}" \
      "${ref_tag}" \
      "${message}" \
      "${repo_full_name}" \
      "${force_push}" \
      "${keep_temp}"
    ;;
  *)
    die "Unknown command: ${cmd}"
    ;;
esac
