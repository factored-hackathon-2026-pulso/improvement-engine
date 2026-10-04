"""Allow-list view: db/catalog.py is the source of truth, platform-exporter policy the cross-check."""
from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules.setdefault(name, mod)
    spec.loader.exec_module(mod)
    return mod


_db = _load("pstream_db_catalog", ROOT / "db" / "catalog.py")
PRODUCT_COLUMNS: dict[str, tuple[str, ...]] = {t: tuple(c for c, _ in cols) for t, cols in _db.PRODUCT.items()}
PRODUCT_TYPES: dict[str, dict[str, str]] = {t: dict(cols) for t, cols in _db.PRODUCT.items()}
FORBIDDEN_TABLES = frozenset(_db.FORBIDDEN_TABLES)
LINEAGE_NAMES = tuple(_db.LINEAGE_NAMES)


def exporter_allowed_columns() -> dict[str, tuple[str, ...]]:
    pol = _load("pstream_exporter_policy", ROOT / "platform-exporter" / "src" / "platform_exporter" / "policy.py")
    return {t: tuple(c) for t, c in pol.ALLOWED_COLUMNS.items()}
