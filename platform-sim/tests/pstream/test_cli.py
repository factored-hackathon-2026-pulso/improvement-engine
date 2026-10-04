import time
import json
import sqlite3

import pytest

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from product_stream.cli import main


def ev_count(p):
    con = sqlite3.connect(p)
    n = con.execute("select count(*) from event_log").fetchone()[0]
    con.close()
    return n


def test_backfill_writes_exactly_n_events_and_manifest(tmp_path, capsys):
    db, mf = tmp_path / "p.sqlite", tmp_path / "m.json"
    rc = main(["--sqlite", str(db), "--backfill", "500", "--seed", "9", "--scenario", "escalation_rise",
               "--manifest", str(mf)], sleep=lambda s: None)
    assert rc == 0
    assert ev_count(db) == 500
    m = json.loads(mf.read_text())
    assert m["scenario"] == "escalation_rise" and m["horizon_events"] == 500 and m["realised"]["last_sequence"] == 500
    out = capsys.readouterr().out
    assert "synthetic product-sim" in out


def test_same_seed_same_file_content(tmp_path):
    a, b = tmp_path / "a.sqlite", tmp_path / "b.sqlite"
    for p in (a, b):
        main(["--sqlite", str(p), "--backfill", "300", "--seed", "4"], sleep=lambda s: None)
    q = "select * from event_log order by sequence"
    ra = sqlite3.connect(a).execute(q).fetchall()
    rb = sqlite3.connect(b).execute(q).fetchall()
    assert ra == rb


def test_refuses_existing_stream_without_overwrite(tmp_path):
    db = tmp_path / "p.sqlite"
    main(["--sqlite", str(db), "--backfill", "50"], sleep=lambda s: None)
    assert main(["--sqlite", str(db), "--backfill", "50"], sleep=lambda s: None) == 2
    assert main(["--sqlite", str(db), "--backfill", "50", "--overwrite"], sleep=lambda s: None) == 0
    assert ev_count(db) == 50


def test_follow_paces_by_rate_and_stops_on_stop_file(tmp_path):
    db, stop = tmp_path / "p.sqlite", tmp_path / "STOP"
    sleeps = []

    def sleep(s):
        sleeps.append(s)
        if len(sleeps) == 3:
            stop.write_text("x")

    rc = main(["--sqlite", str(db), "--follow", "--rate", "20", "--batch", "10", "--stop-file", str(stop)],
              sleep=sleep)
    assert rc == 0
    assert all(abs(s - 0.5) < 1e-9 for s in sleeps)  # batch 10 at 20 events/s
    assert ev_count(db) == 30


def test_follow_stops_on_stdin_eof(tmp_path):
    import threading
    db = tmp_path / "p.sqlite"
    closed = threading.Event()

    class Stdin:
        def read(self):
            closed.wait(5)
            return ""

    calls = []

    def sleep(s):
        calls.append(s)
        if len(calls) == 2:
            closed.set()  # the parent closes the pipe
            time.sleep(0.2)

    rc = main(["--sqlite", str(db), "--follow", "--rate", "100", "--batch", "10", "--stop-on-stdin-eof"],
              sleep=sleep, stdin=Stdin())
    assert rc == 0
    assert 20 <= ev_count(db) <= 30


def test_backfill_then_follow(tmp_path):
    db, stop = tmp_path / "p.sqlite", tmp_path / "STOP"
    n = []

    def sleep(s):
        n.append(1)
        if len(n) == 2:
            stop.write_text("x")

    main(["--sqlite", str(db), "--backfill", "100", "--follow", "--rate", "50", "--batch", "10",
          "--stop-file", str(stop)], sleep=sleep)
    assert ev_count(db) == 120


def test_postgres_path_uses_env_dsn_and_never_prints_it(monkeypatch, capsys):
    dsn = "postgresql://u:topsecret@h.invalid/db"
    monkeypatch.setenv("PULSO_PRODUCT_SIM_PG_DSN", dsn)
    import test_postgres_sink as t
    conn = t.FakeConn()
    rc = main(["--postgres", "--backfill", "60"], sleep=lambda s: None, connect=lambda d: conn)
    assert rc == 0
    cap = capsys.readouterr()
    assert "topsecret" not in cap.out + cap.err
    assert conn.commits >= 2 and conn.closed


def test_postgres_missing_dsn_exit_code_no_secret(monkeypatch, capsys):
    monkeypatch.delenv("PULSO_PRODUCT_SIM_PG_DSN", raising=False)
    assert main(["--postgres", "--backfill", "10"], sleep=lambda s: None) == 2
    assert "PULSO_PRODUCT_SIM_PG_DSN" in capsys.readouterr().err


def test_requires_a_mode_and_a_target():
    with pytest.raises(SystemExit):
        main(["--sqlite", "x.db"])
    with pytest.raises(SystemExit):
        main(["--backfill", "5"])
