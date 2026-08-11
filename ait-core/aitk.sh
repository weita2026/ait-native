#!/usr/bin/env bash

set -euo pipefail

cat >&2 <<'EOF'
aitk.sh is unavailable in pure-Rust ait-core.
Use ../ait for Python- or Tk-owned surfaces.
EOF
exit 1
