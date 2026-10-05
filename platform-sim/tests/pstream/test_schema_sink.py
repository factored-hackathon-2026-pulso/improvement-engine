import sqlite3

from product_stream import catalog_view as cv
from product_stream.sinks import SqliteSink


def test_allow_list_mirrors_exporter_and_db_catalog():
    assert cv.PRODUCT_COLUMNS == cv.exporter_allowed_columns()


def test_denylisted_tables_never_in_schema():
    assert not set(cv.PRODUCT_COLUMNS) & cv.FORBIDDEN_TABLES


def test_sqlite_sink_creates_exact_allow_list(tmp_path):
    p = tmp_path / "p.sqlite"
    s = SqliteSink(str(p))
    s.ensure_schema()
    s.close()
    con = sqlite3.connect(p)
    tables = {r[0] for r in con.execute("select name from sqlite_master where type='table'")}
    assert tables == set(cv.PRODUCT_COLUMNS)
    for t, cols in cv.PRODUCT_COLUMNS.items():
        got = [r[1] for r in con.execute(f"pragma table_info({t})")]
        assert got == list(cols)


def test_sink_refuses_unknown_column_and_table(tmp_path):
    s = SqliteSink(str(tmp_path / "p.sqlite"))
    s.ensure_schema()
    import pytest
    with pytest.raises(ValueError):
        s.write_batch({"cases": [{"id": "x", "email": "a@b"}]})
    with pytest.raises(ValueError):
        s.write_batch({"login_accounts": [{"id": "x"}]})
