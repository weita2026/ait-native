# Security Policy

## Supported versions

AIT receives security fixes on the newest maintained release line.

| Release line | Security support |
| --- | --- |
| Latest `1.x` stable release | Supported once published |
| Latest `1.0.0-rc.x` release candidate | Supported until `1.0.0` is published |
| Older release candidates and superseded releases | Not supported |

If a report affects an older version, reproduce it against the newest
supported version when that can be done safely. Published tags and registry
artifacts are immutable; a fix is released under a new version.

## Report a vulnerability privately

Use GitHub's private vulnerability reporting form:

<https://github.com/weita2026/ait-native/security/advisories/new>

Do not open a public issue, pull request, discussion, or social-media thread
for an undisclosed vulnerability. Do not include secrets, personal data, or
third-party confidential material beyond what is necessary to reproduce the
problem.

Include, when available:

- the affected AIT version, component, package, and platform;
- installation source, such as GitHub, PyPI, npm, APT, Homebrew, WinGet, or
  GHCR;
- a minimal reproduction and the observed security impact;
- relevant logs with tokens, credentials, and private paths removed;
- whether the issue is already being exploited or publicly known; and
- any suggested mitigation or disclosure deadline.

The maintainers will confirm receipt, assess severity and affected release
channels, request missing evidence, and coordinate a fix and disclosure. Keep
the report private until a maintainer confirms that disclosure is safe.

## Scope

Security reports may cover:

- the `ait`, `ait-agent`, `ait-server`, and `ait-runner` native executables;
- the direct PyO3 Python and Node-API Node.js bindings;
- release packages, installers, OCI images, signatures, checksums, provenance,
  and protected publication workflows;
- authorization, repository isolation, Remote authority, Task/Change,
  Snapshot, and Binary DB integrity boundaries; and
- vulnerabilities caused by the public source or default configuration.

The aggregate license boundary does not change the reporting path:
`ait-server/**` is AGPL-3.0-only, while the public root and `ait-core/**`,
`ait-runner/**`, `ait-python/**`, and `ait-node/**` are Apache-2.0.

General support questions, feature requests, and non-security bugs belong in
the public issue tracker. Package identities, verification requirements, and
the exact license/source-publication contract are centralized in
[docs/distribution.md](docs/distribution.md).
