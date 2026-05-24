from __future__ import annotations

import importlib.util
import sys
from pathlib import Path


def test_pytest_prefers_repo_src() -> None:
    repo_src = (Path(__file__).resolve().parents[1] / "src").resolve()

    import_roots = []
    for index, raw_path in enumerate(sys.path):
        path = Path(raw_path or ".").resolve()
        if (path / "ait" / "__init__.py").exists():
            import_roots.append((index, path))

    assert import_roots
    assert import_roots[0][1] == repo_src

    spec = importlib.util.find_spec("ait")
    assert spec is not None
    assert spec.origin is not None
    assert Path(spec.origin).resolve().is_relative_to(repo_src)
