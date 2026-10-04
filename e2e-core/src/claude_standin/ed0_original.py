"""ED0b: loader for the original bank CSV dataset (hive-partitioned `<table>/year=/month=/day=/*.csv`).

Read at runtime only (env `ED0_ORIGINAL_PATH`), never copied or committed. Only the two caller-named columns are
kept; every other column (free text included) is dropped per row. The caller must name the table and columns:
this module does not decide which original table means "case" or "group" (no invented semantics). Recurrence is
the same rule as ED0F: the case appears in at least two rows. Case ids leave only as salted HMAC keys; the raw
group is a transient in-memory value that the lab hashes. Errors never carry a cell value.
"""
import csv
import os
import re
from collections import Counter
from pathlib import Path

from claude_standin.ed0_feed import WINDOW, _case_key, lab_salt  # noqa: F401  (reused, not duplicated)

_NAME = re.compile(r"[A-Za-z0-9_]{1,64}")


def original_path() -> str:
    p = os.environ.get("ED0_ORIGINAL_PATH")
    if not p:
        raise RuntimeError("ED0_ORIGINAL_PATH is not set (path of the local original CSV dataset, read at runtime only)")
    return p


def _rows(root: Path, case_col: str, group_col: str):
    files = sorted(root.rglob("*.csv"))
    if not files:
        raise ValueError("no csv files for the table")
    for f in files:
        with open(f, newline="", encoding="utf-8-sig") as fh:
            rd = csv.reader(fh)
            head = next(rd, None) or []
            if case_col not in head or group_col not in head:
                raise ValueError("mapped column missing in csv header")
            if head.count(case_col) != 1 or head.count(group_col) != 1:
                raise ValueError("mapped column is ambiguous in csv header")
            ci, gi = head.index(case_col), head.index(group_col)
            for r in rd:
                if len(r) > max(ci, gi):
                    yield r[ci], r[gi]


def feed(root: str, salt: bytes, table: str, case_col: str, group_col: str, window: str = WINDOW):
    """Iterable of (case_key, group, window, outcome), one per (case, group); same shape as ed0_feed.feed."""
    if not all(_NAME.fullmatch(x or "") for x in (table, case_col, group_col)):
        raise ValueError("table and column names must be plain identifiers")
    if case_col == group_col:
        raise ValueError("case and group columns must differ")
    pairs = [(c, g) for c, g in _rows(Path(root) / table, case_col, group_col) if c and g]
    per_case = Counter(c for c, _g in pairs)
    for case_id, grp in sorted(set(pairs)):
        yield (_case_key(salt, case_id), grp, window, per_case[case_id] >= 2)
