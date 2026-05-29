# GitHub Release Publishing

Authority: command layer under [plan.md](./plan.md), the applicable legal-layer governance documents, and [ait_release_readiness.md](./ait_release_readiness.md).
Status: current GitHub Release publishing guide for the public `ait-native` repository.
Scope: GitHub Release asset ref convention, asset publication, and release-asset recovery routing.

This repository publishes GitHub Release assets for `ait-native` from the clean
public GitHub repository: `weita2026/ait-native`.

The public automation lives at `.github/workflows/github-release-publish.yml`.
It creates or updates the real GitHub Release for one `v*` tag and uploads the
prepared wheel, sdist, manifest, and checksum files by using the repository
`GITHUB_TOKEN`. The local operator path does not need `gh auth login`.

## Asset ref convention

The workflow reads release assets from a matching asset ref:

- real release tag: `v0.10.6`
- asset ref tag: `release-assets-v0.10.6`
- asset payload directory: `releases/v0.10.6/`

That asset payload should contain:

- `ait_native-<version>-py3-none-any.whl`
- `ait-native-<version>.tar.gz`
- `ait-release-<version>.manifest.json`
- `ait-release-<version>.sha256`
- optional `release-notes.md`

## Publish from the public repo

1. Push the clean public release commit to `weita2026/ait-native` on `main`,
   including `.github/workflows/github-release-publish.yml`.
2. Rewrite the public Homebrew formula to the final GitHub Release wheel URL:

```bash
scripts/github_release_publish.sh rewrite-formula \
  --version 0.10.6 \
  --formula Formula/ait-native.rb
```

3. Publish the prepared asset payload to the matching `release-assets-v*` ref:

```bash
scripts/github_release_publish.sh publish-assets-ref \
  --version 0.10.6 \
  --remote-url git@github.com:weita2026/ait-native.git
```

4. If you have explicit release notes, pass them through `--notes-file <path>`;
   otherwise the helper writes a default `release-notes.md`.
5. Push the matching real release tag, for example `v0.10.6`, on the public
   repo `main` commit.
6. Let `.github/workflows/github-release-publish.yml` create or update the real
   GitHub Release and upload the wheel, sdist, manifest, and checksum assets.

## Manual recovery

If the tag-triggered run needs recovery:

1. Open GitHub Actions for `weita2026/ait-native`.
2. Run `.github/workflows/github-release-publish.yml` with
   `workflow_dispatch`.
3. Provide the real release tag in `version`, for example `v0.10.6`.
4. Provide `assets-ref` only when the assets live somewhere other than the
   default `release-assets-v*` ref.

The workflow uploads assets with `--clobber`, so reruns replace stale assets
without requiring local GitHub CLI authentication.
