# Third-Party Notices

Authority: command layer under [docs/plan.md](./plan.md), the legal-layer governance documents, and [docs/legal/component_license_matrix.md](./legal/component_license_matrix.md).

Status: provisional release-candidate notice inventory for the current `ait`
repository package surfaces. This file is not a substitute for a final SBOM,
full transitive dependency review, or counsel-reviewed public distribution
notices.
Scope: provisional third-party notice inventory for the current release-facing repository package surfaces.

## Direct runtime dependencies from `pyproject.toml`

| Package | Scope | Observed license metadata |
| --- | --- | --- |
| `typer` | runtime | `MIT` |
| `rich` | runtime | `MIT` classifier |
| `fastapi` | runtime | `MIT` |
| `uvicorn` | runtime | `BSD-3-Clause` |
| `httpx` | runtime | `BSD-3-Clause` |
| `websockets` | runtime | `BSD-3-Clause` |
| `cryptography` | runtime | `Apache-2.0 OR BSD-3-Clause` |

## Optional dependencies

| Package | Scope | Observed license metadata |
| --- | --- | --- |
| `psycopg[binary]` / `psycopg-binary` | optional `postgres` extra | `LGPL-3.0-only` |

## Development and test dependencies

| Package | Scope | Observed license metadata |
| --- | --- | --- |
| `pytest` | `test` extra | `MIT` classifier |

## Notes

- The table above is based on locally observed installed-package metadata plus
  the dependency declarations in `pyproject.toml`.
- Final release readiness still requires a reviewed dependency inventory,
  transitive-license check, and any additional notices required by bundled
  assets or future release artifacts.
