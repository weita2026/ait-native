from __future__ import annotations

import sys
from pathlib import Path

import pytest

from tests.postgres_fake import restore_real_psycopg

ROOT = Path(__file__).resolve().parents[1]
SRC = str(ROOT / "src")

while SRC in sys.path:
    sys.path.remove(SRC)
sys.path.insert(0, SRC)


@pytest.fixture(autouse=True)
def restore_postgres_driver_after_test():
    yield
    restore_real_psycopg()
