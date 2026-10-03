"""PL-01: login_accounts (and the other credential tables) are refused BEFORE any query; the allow-list is explicit."""

import pytest

from platform_exporter.policy import ALLOWED_TABLES, DENIED_TABLES, AccessDenied, assert_table_allowed


@pytest.mark.parametrize("table", ["login_accounts", "mfa_challenges", "staff_sessions", "LOGIN_ACCOUNTS", '"login_accounts"'])
def test_credential_tables_refused_by_construction(table):
    with pytest.raises(AccessDenied):
        assert_table_allowed(table)


def test_allow_list_is_exactly_the_conversation_tables_plus_dimensions():
    assert set(ALLOWED_TABLES) == {"cases", "turns", "assignments", "customer_case_slots", "customers", "event_log",
                                   "staff"}
    assert not set(ALLOWED_TABLES) & set(DENIED_TABLES)
    assert {"login_accounts", "mfa_challenges", "staff_sessions"} <= set(DENIED_TABLES)


def test_unknown_table_is_denied_by_default():
    with pytest.raises(AccessDenied):
        assert_table_allowed("suggestions")  # demo data is not on the allow-list either


def test_select_builder_refuses_state_columns_and_denied_tables_before_building_sql():
    from platform_exporter.policy import select_sql

    with pytest.raises(AccessDenied):
        select_sql("login_accounts", ["staff_id"])
    with pytest.raises(AccessDenied):
        select_sql("cases", ["case_id", "status"])  # mutable state is not readable: events are the truth
    with pytest.raises(AccessDenied):
        select_sql("staff", ["staff_id", "email"])
    with pytest.raises(AccessDenied):
        select_sql("event_log", ["sequence"], where="sequence > ? AND password_hash IS NULL")
    assert select_sql("event_log", ["sequence", "event_id"], where="sequence > ?", order_by="sequence",
                      limit=5) == "SELECT sequence, event_id FROM event_log WHERE sequence > ? ORDER BY sequence LIMIT 5"


def test_sqlite_engine_guard_denies_credential_tables_state_columns_and_writes(tmp_path):
    import sqlite3

    from platform_exporter.policy import install_sqlite_guard
    from tests.platform_db import make_db

    make_db(tmp_path / "p.db").close()
    conn = sqlite3.connect(str(tmp_path / "p.db"))
    install_sqlite_guard(conn)
    for sql in ("SELECT password_hash FROM login_accounts", "SELECT * FROM mfa_challenges", "SELECT status FROM cases",
                "SELECT name FROM staff", "DELETE FROM event_log", "INSERT INTO event_log(sequence) VALUES(1)",
                "SELECT text FROM turns"):
        with pytest.raises(sqlite3.DatabaseError):
            conn.execute(sql).fetchall()
    assert conn.execute("SELECT case_id, customer_id FROM cases").fetchall()  # dimension columns stay readable
