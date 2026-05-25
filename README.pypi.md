# ait-native

`ait-native` is the first public Python distribution for the `ait` workflow family.

It packages these command surfaces today:

- `ait` for the local trust-layer CLI
- `ait-agent` for transport/runtime helper flows
- `ait-server` for the shared workflow control plane
- `ait-worker` for the async shared-control-plane worker
- `aitk` for the local read-only history companion

## Install

```bash
pip install ait-native
```

If you plan to run the shared self-hosted control plane, install the PostgreSQL extra:

```bash
pip install "ait-native[postgres]"
```

## Quick start

- Local-first path: https://ait-native.dev
- Self-hosted guide: https://ait-native.dev
- Source: https://github.com/weita2026/ait-native

## Important license boundary

`ait-native` is a combined public distribution with multiple release-facing license surfaces.

- Local CLI and local companion surfaces remain Apache-2.0.
- Public self-hosted `ait-server` / `ait-worker` surfaces follow AGPL-3.0-only.

Read the release-facing summary before relying on a broader grant:

- https://ait-native.dev
- https://github.com/weita2026/ait-native
