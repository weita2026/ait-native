# PyPI Publishing

This repository publishes `ait-native` to PyPI from the clean public GitHub
repository: `weita2026/ait-native`.

The release automation lives at `.github/workflows/pypi-publish.yml` and is
designed for PyPI `Trusted Publisher` flow instead of a long-lived API token in
CI.

## First-time setup

1. In PyPI account settings, add a pending publisher for project `ait-native`
   if the project does not exist yet.
2. For the GitHub publisher fields, use:
   - owner: `weita2026`
   - repository: `ait-native`
   - workflow: `.github/workflows/pypi-publish.yml`
   - environment: `pypi`
3. If the project already exists on PyPI, add the same GitHub workflow as a
   normal project-level publisher instead of a pending publisher.

## Publish from the public repo

1. Push the clean public release commit to `weita2026/ait-native`.
2. Push the matching `v*` tag, for example `v0.10.5`.
3. Let `.github/workflows/pypi-publish.yml` start automatically from that tag
   push, build the wheel/sdist, run
   `twine check`, smoke install the wheel, and publish to PyPI.
4. If the same public release also needs GitHub Release assets for Homebrew or
   direct downloads, follow [GitHub Release Publishing](./GITHUB_RELEASE_PUBLISHING.md).
5. If the tag-triggered PyPI run needs recovery, use `workflow_dispatch` from
   GitHub Actions instead of creating a GitHub Release first.
6. Verify the release with:

```bash
python -m pip index versions ait-native
python -m pip install ait-native==<version>
```

## Manual fallback

Prefer trusted publishing. If PyPI or GitHub Actions needs an emergency manual
upload path, use a project-scoped PyPI token only for the fallback upload:

```bash
python -m pip install --upgrade build twine
python -m build
python -m twine check dist/*
TWINE_USERNAME=__token__ TWINE_PASSWORD=<token> python -m twine upload dist/*
```

Do not publish from the internal/private repository tree. Publish only from the
clean public repo that already excludes governance files, `ait-web`, and other
non-public surfaces.
