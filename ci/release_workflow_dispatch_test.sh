#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
temporary_parent=$(CDPATH='' cd -- "${TMPDIR:-/tmp}" && pwd)
temporary_root=$(mktemp -d "${temporary_parent}/ait-release-workflow-dispatch.XXXXXX")
cleanup() {
  case "${temporary_root}" in
    "${temporary_parent}"/ait-release-workflow-dispatch.*) rm -rf -- "${temporary_root}" ;;
    *) printf 'refusing unexpected cleanup path: %s\n' "${temporary_root}" >&2 ;;
  esac
}
trap cleanup EXIT HUP INT TERM
mkdir "${temporary_root}/bin"
cat >"${temporary_root}/bin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" >"${AIT_WORKFLOW_DISPATCH_ARGS:?}"
GH
chmod +x "${temporary_root}/bin/gh"
jq -S -n '{
  release: {repository: "owner/repository"},
  dispatch: {
    workflow: "release.yml", ref: "main",
    inputs: {zeta: 2, alpha: "value", enabled: true}
  }
}' >"${temporary_root}/record.json"
export AIT_WORKFLOW_DISPATCH_ARGS=${temporary_root}/args.txt
PATH="${temporary_root}/bin:${PATH}" node "${repo_root}/ci/release_workflow_dispatch.mjs" \
  --record "${temporary_root}/record.json" --selector dispatch \
  >"${temporary_root}/stdout"
cat >"${temporary_root}/expected.txt" <<'EOF'
workflow
run
release.yml
--repo
owner/repository
--ref
main
-f
alpha=value
-f
enabled=true
-f
zeta=2
EOF
diff -u "${temporary_root}/expected.txt" "${temporary_root}/args.txt"
grep -F 'release.yml: dispatched' "${temporary_root}/stdout" >/dev/null
if PATH="${temporary_root}/bin:${PATH}" node "${repo_root}/ci/release_workflow_dispatch.mjs" \
  --record "${temporary_root}/record.json" --selector missing \
  >"${temporary_root}/missing.stdout" 2>"${temporary_root}/missing.stderr"; then
  printf 'workflow dispatch accepted a missing selector\n' >&2
  exit 65
fi

jq -S -n '{
  release:{id:"REL-FAM-0123456789ABCDEF"},
  publisher:{repository:"owner/repository",workflow:"pypi-publish.yml"},
  protected_authorization:{workflow_run_id:12345}
}' >"${temporary_root}/endpoints.json"
PATH="${temporary_root}/bin:${PATH}" node "${repo_root}/ci/release_workflow_dispatch.mjs" \
  --record "${temporary_root}/endpoints.json" --selector endpoint_publication \
  >"${temporary_root}/endpoint.stdout"
grep -Fx 'pypi-publish.yml' "${temporary_root}/args.txt" >/dev/null
grep -Fx 'release_id=REL-FAM-0123456789ABCDEF' "${temporary_root}/args.txt" >/dev/null
grep -Fx 'protected_run_id=12345' "${temporary_root}/args.txt" >/dev/null
grep -Fx 'publish_exact_frozen_bytes=true' "${temporary_root}/args.txt" >/dev/null
grep -E '^endpoint_config_b64=[A-Za-z0-9+/=]+$' "${temporary_root}/args.txt" >/dev/null
grep -F 'pypi-publish.yml: dispatched' "${temporary_root}/endpoint.stdout" >/dev/null

jq -S -n '{status:"published_readback_complete",release:{id:"REL-FAM-0123456789ABCDEF"}}' \
  >"${temporary_root}/operator-status.json"
PATH="${temporary_root}/bin:${PATH}" node "${repo_root}/ci/release_workflow_dispatch.mjs" \
  --record "${temporary_root}/endpoints.json" --status-record "${temporary_root}/operator-status.json" \
  --selector latest_alias >"${temporary_root}/latest.stdout"
grep -Fx 'ait-release-latest-alias.yml' "${temporary_root}/args.txt" >/dev/null
grep -Fx 'promote_exact_release=true' "${temporary_root}/args.txt" >/dev/null
grep -E '^operator_status_b64=[A-Za-z0-9+/=]+$' "${temporary_root}/args.txt" >/dev/null
grep -F 'ait-release-latest-alias.yml: dispatched' "${temporary_root}/latest.stdout" >/dev/null

printf 'release workflow-dispatch tests passed\n'
