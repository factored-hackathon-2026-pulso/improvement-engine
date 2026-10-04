import re
import sys
from pathlib import Path

import catalog
import gen_ddl

ROOT = Path(__file__).resolve().parents[2]


def _exporter_policy():
    sys.path.insert(0, str(ROOT / "platform-exporter" / "src"))
    from platform_exporter import policy
    return policy


def test_committed_sql_equals_render():
    for name, text in gen_ddl.render_all().items():
        assert (ROOT / "db" / "sql" / name).read_text(encoding="utf-8") == text, name


def test_product_allow_list_matches_exporter_exactly():
    pol = _exporter_policy()
    prod = {t: tuple(c for c, _ in cols) for t, cols in catalog.PRODUCT.items()}
    assert prod == dict(pol.ALLOWED_COLUMNS)
    for denied in pol.DENIED_TABLES:
        assert denied not in catalog.PRODUCT


def test_forbidden_tables_absent_everywhere():
    sql = "".join(gen_ddl.render_all().values())
    for schema, tables in catalog.SCHEMAS.items():
        for t in tables:
            bare = t.split("_", 1)[1] if t.startswith(("e0_", "bank_")) else t
            assert bare not in catalog.FORBIDDEN_TABLES, (schema, t)
    for f in catalog.FORBIDDEN_TABLES:
        assert not re.search(rf"create\s+table\s+\S*{f}\b", sql, re.I), f


def test_lineage_columns_on_every_table_and_not_in_catalog():
    for schema, tables in catalog.SCHEMAS.items():
        for t, cols in tables.items():
            assert not any(n in [c for c, _ in cols] for n in catalog.LINEAGE_NAMES), (schema, t)
    files = gen_ddl.render_all()
    for schema, name in (("raw", "010_raw.sql"), ("augmented", "020_augmented.sql"), ("product", "030_product.sql")):
        assert files[name].count("_batch_id text") == len(catalog.SCHEMAS[schema])
        assert files[name].count("_ingested_at timestamptz NOT NULL DEFAULT now()") == len(catalog.SCHEMAS[schema])


def test_e0_raw_has_nine_tables_and_no_evaluator():
    assert len([t for t in catalog.SCHEMAS["raw"] if t.startswith("e0_")]) == 9
    assert not {"e0_labels", "e0_timeline"} & set(catalog.SCHEMAS["raw"])


def test_roles_one_reader_per_schema_read_only():
    sql = gen_ddl.render_all()["001_roles_schemas.sql"]
    for r in catalog.READER_ROLE.values():
        assert re.search(rf"ALTER ROLE {r} SET default_transaction_read_only = on", sql)
    assert len(set(catalog.READER_ROLE.values())) == 3
    assert "CREATE SCHEMA IF NOT EXISTS pulso" in sql


def test_grants_are_column_level_and_exclude_sensitive():
    sql = gen_ddl.render_all()["090_grants.sql"]
    assert "ALL TABLES" not in sql.upper()
    turn = re.search(r"GRANT SELECT \(([^)]*)\) ON raw\.e0_turn TO pulso_raw_ro", sql)
    assert turn
    cols = [c.strip() for c in turn.group(1).split(",")]
    assert "turn_id" in cols and "text" not in cols
    assert "pulso_product_ro" in sql and "login_accounts" not in sql
    assert not re.search(r"ON raw\.\S+ TO pulso_product_ro", sql)
    assert not re.search(r"(INSERT|UPDATE|DELETE|TRUNCATE)[^;]*TO pulso_\w+_ro", sql)
