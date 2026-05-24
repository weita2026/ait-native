# ait Public Release License Summary
Revision date: 2026-05-25.  

> This file is a release-facing summary, not a final commercial agreement and not legal advice.

## 1. Why this file exists

The repository root `LICENSE` intentionally grants Apache-2.0 only for:

- `ait` CLI
- `aitk` local history-browser companion
- `ait-agent`
- `ait_protocol`
- `ait_storage`
- `ait_chat`

That root file does **not** automatically grant the same license to the rest of
the repository.

This summary exists so an external agent or contributor can quickly answer:

- which surfaces are currently intended to be public;
- which shared-control-plane surfaces are reciprocal or commercial;
- which docs/examples are reusable versus still internal.

The current monorepo build can expose multiple console scripts (`ait`, `ait-agent`, `ait-server`, `ait-worker`, `aitk`) without collapsing them into one uniform public license grant. The companion machine-readable map for those command/package boundaries lives in `docs/legal/public_package_surface_map.json`.

Fast boundary lookup now also exists in two release-facing marker layers:

- module-local `LICENSE` marker files under the current `src/**` package roots;
- [docs/legal/module_license_map.md](./module_license_map.md) for mixed or compatibility surfaces such as `src/ait_native/**` and `site/**`.

Official website / brand note:

- the current public umbrella brand is `ait-native`;
- the official website domain is `https://ait-native.dev`; and
- code/content licenses do **not** by themselves grant trademark rights in
  `ait-native`, `ait`, related logos, or confusingly similar domains.

## 2. Release-facing surface summary

| Surface | Current release-facing posture | Notes |
| --- | --- | --- |
| `ait` CLI (`src/ait/**`) | `Apache-2.0` | Covered by the repository root `LICENSE`. |
| `aitk` local history browser (`src/ait_tk/**` plus `src/ait/aitk_*`) | `Apache-2.0` | Covered by the repository root `LICENSE` as a local read-only companion shipped with the local trust layer. |
| `ait-agent` (`src/ait_agent/**`) | `Apache-2.0` | Covered by the repository root `LICENSE` for the baseline transport/runtime layer. |
| `ait_protocol` (`src/ait_protocol/**`) | `Apache-2.0` | Covered by the repository root `LICENSE` as the passive protocol/contracts surface. |
| `ait_storage` (`src/ait_storage/**`) | `Apache-2.0` | Covered by the repository root `LICENSE` as the passive storage support surface. |
| `ait_chat` (`src/ait_chat/**`) | `Apache-2.0` | Covered by the repository root `LICENSE` as the shared reply-runtime seam. |
| Local repository/storage trust layer | `Apache-2.0` when shipped with local `ait` | Includes local workflow/storage surfaces that belong to the inspectable trust layer. |
| `ait-server` | `AGPL-3.0-only OR LicenseRef-AIT-Commercial` | Self-hosted/community posture is reciprocal; commercial exceptions require a separate agreement. |
| `ait-worker` | Same as `ait-server` | Operationally tied to the shared control plane. |
| `ait_native` compatibility package (`src/ait_native/**`) | component-specific compatibility surface | Read this package through [module_license_map.md](./module_license_map.md); local shims follow Apache and server/worker shims follow AGPL/commercial posture. |
| `site/**` official public website source and generated pages | No blanket public software-license grant by default; component-specific release-facing treatment applies | Do not assume the public-site scaffold, generated pages, or official website assets inherit the permissive local CLI grant merely because they live in the repository. |
| `site/LICENSE` | release-facing website boundary notice | Use this file as the fast pointer for the official website surface before inferring any software-license or content-license grant. |
| Official website copy deliberately published for public developer/docs use | `CC-BY-4.0` preferred when intentionally published | Scope the grant to deliberately published release-facing website copy rather than all site assets or all repository docs. |
| Official brand assets, logos, and wordmarks | Trademark policy, not open-source software license | See `docs/TRADEMARK_POLICY.md`; software/content licenses do not imply mark rights. |
| Enterprise extensions | `LicenseRef-AIT-Commercial` | No public grant by default. |
| Release-facing product docs deliberately published for public use | `CC-BY-4.0` preferred | Applies only when a doc is intentionally published as public product/developer/contributor documentation. |
| Public example repos, sample plans, and demo templates deliberately published for reuse | `MIT` or `Apache-2.0` preferred | Choose per example package; do not assume every repo artifact is reusable by default. |
| Deployment templates deliberately published for customer/developer use | `MIT` or `Apache-2.0` preferred | Public deployment templates should stay easy to reuse unless a specific template says otherwise. |
| Internal strategy/legal/planning docs not explicitly published for public reuse | No public reuse grant by default | They are not covered by the root `LICENSE` unless a file or package says otherwise. |

## 2.1 Current package and command boundary map

| Command / surface | Primary source roots | Release-facing posture | Boundary note |
| --- | --- | --- | --- |
| `ait` | `src/ait/**` | `Apache-2.0` | Local CLI and local repository workflow surface. |
| `ait-agent` | `src/ait_agent/**` | `Apache-2.0` | Baseline transport/runtime layer. |
| `ait-server` | `src/ait_server/**` | `AGPL-3.0-only OR LicenseRef-AIT-Commercial` | Shared control-plane authority for self-hosted/community use. |
| `ait-worker` | `src/ait_server/**` | `AGPL-3.0-only OR LicenseRef-AIT-Commercial` | Shared control-plane worker surface; do not treat it as part of the permissive local layer. |
| `aitk` | `src/ait_tk/**`, `src/ait/aitk_*` | `Apache-2.0` | Local read-only history browser companion; not a shared server/web surface. |
| `ait-native` official website (`site/**`) | `site/src/**`, `site/public/**`, generated `site/**/index.html` pages | Component-specific; no blanket public software-license grant by default | Public-site materials should be treated as release-facing website assets with explicit per-surface licensing and trademark boundaries rather than as part of the permissive CLI trust layer by default. |

This table is intentionally narrower than the future full package-target extraction plan. Its job is to stop an external agent from inferring that one monorepo checkout or one combined build means one public license boundary.

## 3. Important boundary rules

- The root `LICENSE` is intentionally narrow; do not describe it as a whole-repository Apache grant.
- The root `LICENSE` now covers the Apache-facing local/runtime roots (`src/ait/**`, `src/ait_agent/**`, `src/ait_tk/**`, `src/ait_protocol/**`, `src/ait_storage/**`, and `src/ait_chat/**`) and still does **not** create a whole-repository Apache grant.
- `LicenseRef-AIT-Commercial` names non-public commercial surfaces that require a separate written agreement.
- The official website domain is `https://ait-native.dev`; domain control does not replace trademark clearance or filing work.
- Trademark rights in `ait-native`, `ait`, related logos, and official website branding are governed by `docs/TRADEMARK_POLICY.md`, not by source-code licenses alone.
- Public examples or docs must be explicitly released as such; internal planning/legal material should not be assumed public just because it lives in the repository.
- If a package or file does not clearly state a public grant, treat this summary, [component_license_matrix.md](./component_license_matrix.md), and [module_license_map.md](./module_license_map.md) as the current source of truth.

## 4. Related files

- repository root `LICENSE`
- `LICENSES/AGPL-3.0-only.txt`
- `LICENSES/LicenseRef-AIT-Commercial.txt`
- `site/LICENSE`
- `docs/TRADEMARK_POLICY.md`
- `docs/legal/public_package_surface_map.json`
- `docs/legal/component_license_matrix.md`
- `docs/legal/module_license_map.md`
- `docs/legal/commercial_license_terms.md`
