#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 --manifest <Cargo.toml> --notice <NOTICE> --project <name> [--check]" >&2
}

manifest=""
notice=""
project=""
check=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest)
      manifest="${2:-}"
      shift 2
      ;;
    --notice)
      notice="${2:-}"
      shift 2
      ;;
    --project)
      project="${2:-}"
      shift 2
      ;;
    --check)
      check=1
      shift
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$manifest" || -z "$notice" || -z "$project" ]]; then
  usage
  exit 2
fi
if [[ ! -f "$manifest" || ! -f "$notice" ]]; then
  echo "manifest and notice must be regular files" >&2
  exit 1
fi
if ! command -v cargo >/dev/null || ! command -v jq >/dev/null; then
  echo "cargo and jq are required to generate Rust dependency notices" >&2
  exit 1
fi

marker="----- BEGIN GENERATED THIRD-PARTY NOTICES -----"
if ! grep -Fqx -- "$marker" "$notice"; then
  echo "$notice is missing the exact generated-section marker" >&2
  exit 1
fi

notice_dir=$(cd "$(dirname "$notice")" && pwd -P)
temporary_root=$(mktemp -d "$notice_dir/.ait-license-notice.XXXXXX")
cleanup() {
  rm -rf "$temporary_root"
}
trap cleanup EXIT

metadata="$temporary_root/metadata.json"
records="$temporary_root/records.usv"
inventory="$temporary_root/inventory.tsv"
license_rows="$temporary_root/license-rows.tsv"
generated="$temporary_root/generated"
candidates="$temporary_root/candidates"

cargo metadata --manifest-path "$manifest" --locked --format-version 1 >"$metadata"
jq -r '
  .packages[]
  | select(.source != null)
  | [
      .name,
      .version,
      (.license // "NOASSERTION"),
      (.authors | join(", ")),
      (.repository // .homepage // ""),
      .manifest_path,
      (.license_file // "")
    ]
  | join("\u001f")
' "$metadata" | LC_ALL=C sort -u >"$records"
jq -r '
  .packages[]
  | select(.source != null)
  | [
      .name,
      .version,
      (.license // "NOASSERTION"),
      (.authors | join(", ")),
      (.repository // .homepage // "")
    ]
  | @tsv
' "$metadata" | LC_ALL=C sort -u >"$inventory"

: >"$license_rows"
while IFS=$'\x1f' read -r package version expression authors upstream package_manifest license_file; do
  package_dir=$(dirname "$package_manifest")
  : >"$candidates"
  if [[ -n "$license_file" && -f "$package_dir/$license_file" ]]; then
    printf '%s\n' "$package_dir/$license_file" >>"$candidates"
  fi
  find "$package_dir" -maxdepth 1 -type f \
    \( -iname 'license*' -o \
    -iname 'licence*' -o \
    -iname 'copying*' -o \
    -iname 'notice*' -o \
    -iname 'copyright*' -o \
    -iname 'unlicense*' \) -print >>"$candidates"
  LC_ALL=C sort -u "$candidates" | while IFS= read -r legal_file; do
    [[ -n "$legal_file" ]] || continue
    if command -v sha256sum >/dev/null; then
      digest=$(sha256sum "$legal_file" | awk '{print $1}')
    else
      digest=$(shasum -a 256 "$legal_file" | awk '{print $1}')
    fi
    printf '%s\t%s %s\t%s\t%s\n' \
      "$digest" "$package" "$version" "$(basename "$legal_file")" "$legal_file" \
      >>"$license_rows"
  done
done <"$records"
LC_ALL=C sort -u "$license_rows" -o "$license_rows"

awk -v marker="$marker" '
  $0 == marker { exit }
  { print }
' "$notice" >"$generated"
{
  printf '%s\n\n' "$marker"
  printf 'Third-party dependency notices for %s\n\n' "$project"
  printf 'This section is generated from the locked Cargo metadata. '
  printf 'It contains no build-host paths.\n\n'
  printf 'Package\tVersion\tSPDX license expression\tAuthors\tUpstream\n'
  cut -f1-5 "$inventory"
  printf '\nPackages without a standalone root legal file\n\n'
  while IFS=$'\x1f' read -r package version expression authors upstream package_manifest license_file; do
    if ! cut -f2 "$license_rows" | grep -Fqx "$package $version"; then
      printf '%s\t%s\t%s\n' "$package" "$version" "$expression"
    fi
  done <"$records"
  printf '\nComplete deduplicated upstream legal texts\n'
} >>"$generated"

cut -f1 "$license_rows" | uniq | while IFS= read -r digest; do
  [[ -n "$digest" ]] || continue
  first=$(awk -F '\t' -v wanted="$digest" '$1 == wanted { print; exit }' "$license_rows")
  legal_file=$(printf '%s\n' "$first" | cut -f4-)
  {
    printf '\n--- SHA-256 %s ---\n' "$digest"
    printf 'Used by:\n'
    awk -F '\t' -v wanted="$digest" '
      $1 == wanted { print "- " $2 " (" $3 ")" }
    ' "$license_rows" | LC_ALL=C sort -u
    printf '\n'
    sed 's/\r$//' "$legal_file"
    printf '\n'
  } >>"$generated"
done

if grep -Fq "$notice_dir/" "$generated" || grep -Fq '/.cargo/registry/' "$generated"; then
  echo "generated notice contains a build-host path" >&2
  exit 1
fi
if [[ "$check" -eq 1 ]]; then
  if ! cmp -s "$generated" "$notice"; then
    echo "$notice is stale; regenerate it with $0" >&2
    exit 1
  fi
  exit 0
fi
mv "$generated" "$notice"
