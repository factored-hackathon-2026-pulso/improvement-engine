import json
import sqlite3
from pathlib import Path

from product_stream import catalog_view as cv
from product_stream.cli import main

FX = Path(__file__).resolve().parents[2] / "product_stream" / "fixtures" / "product_sample.sqlite"


def test_fixture_is_small_and_has_exactly_the_allow_list():
    assert FX.exists() and FX.stat().st_size < 200_000
    con = sqlite3.connect(f"file:{FX}?mode=ro", uri=True)
    tables = {r[0] for r in con.execute("select name from sqlite_master where type='table'")}
    assert tables == set(cv.PRODUCT_COLUMNS) == set(cv.exporter_allowed_columns())
    assert not tables & cv.FORBIDDEN_TABLES
    for t, cols in cv.PRODUCT_COLUMNS.items():
        assert [r[1] for r in con.execute(f"pragma table_info({t})")] == list(cols)
        assert not set(cols) & set(cv.LINEAGE_NAMES)


def test_fixture_is_the_documented_regeneration(tmp_path):
    out = tmp_path / "r.sqlite"
    main(["--sqlite", str(out), "--backfill", "600", "--seed", "7", "--scenario", "escalation_rise",
          "--horizon-events", "600"], sleep=lambda s: None)
    q = {t: f"select * from {t} order by 1, 2" for t in ("event_log", "cases", "turns", "assignments")}
    a, b = sqlite3.connect(FX), sqlite3.connect(out)
    for t, sql in q.items():
        assert a.execute(sql).fetchall() == b.execute(sql).fetchall(), t
    m = json.loads(FX.with_suffix(".manifest.json").read_text())
    assert m["scenario"] == "escalation_rise" and m["data_origin"] == "synthetic product-sim"


def test_fixture_holds_no_personal_looking_values():
    con = sqlite3.connect(FX)
    blob = " ".join(str(v) for t in cv.PRODUCT_COLUMNS for row in con.execute(f"select * from {t}") for v in row)
    assert "@" not in blob and "http" not in blob
    assert con.execute("select count(*) from customers where simulator != 1").fetchone()[0] == 0
