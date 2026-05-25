# ait Homebrew Tap

## What this guide is for

Use this guide when:

- you want a macOS convenience install path for the public `ait-native` package;
- you do not want to start from an editable repository checkout first; or
- you want `brew upgrade` / `brew uninstall` ergonomics for the public CLI/server surfaces.

This guide is about the current Homebrew tap path only. It does **not** create a
new package boundary or a new license boundary; it installs the same
release-facing `ait-native` distribution.

For the current surface split, also read:

- [PACKAGE_TARGETS.md](./PACKAGE_TARGETS.md)
- [LOCAL_QUICKSTART.md](./LOCAL_QUICKSTART.md)
- [SELF_HOSTED_TEAM_DEPLOYMENT.md](./SELF_HOSTED_TEAM_DEPLOYMENT.md)

## 1. Current tap contract

Current first official tap path:

- tap source repo: `https://github.com/weita2026/ait-native`
- formula name: `ait-native`
- installed console scripts:
  - `ait`
  - `ait-agent`
  - `ait-server`
  - `ait-worker`
  - `aitk`

Important boundary note:

- `ait-web` is **not** part of the Homebrew tap surface.

## 2. Install

Add the tap explicitly:

```bash
brew tap weita2026/ait-native https://github.com/weita2026/ait-native
```

Install `ait-native`:

```bash
brew install weita2026/ait-native/ait-native
```

Verify the installed commands:

```bash
command -v ait
command -v ait-agent
command -v ait-server
command -v ait-worker
command -v aitk
ait --help
ait-agent --help
```

Important runtime note:

- `ait-server` and `ait-worker` are installed by the tap, but they still need
  the self-hosted runtime configuration described in
  [SELF_HOSTED_TEAM_DEPLOYMENT.md](./SELF_HOSTED_TEAM_DEPLOYMENT.md).

## 3. Upgrade and uninstall

Upgrade:

```bash
brew upgrade weita2026/ait-native/ait-native
```

Uninstall:

```bash
brew uninstall ait-native
```

Remove the tap when you no longer need it:

```bash
brew untap weita2026/ait-native
```

## 4. What this tap does not do

- it does not auto-start services;
- it does not replace the repository-editable contributor path;
- it does not publish `ait-web`; and
- it does not imply `homebrew/core` support for this first flag-planting wave.
