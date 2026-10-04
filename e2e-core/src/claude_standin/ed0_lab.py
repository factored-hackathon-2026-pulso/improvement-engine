"""ED0L: treated lab of k-anonymous aggregates (sqlite) with deterministic recompute.

Raw cases are folded into per-(group, window) aggregates in memory and are never stored. Groups
below k are dropped. Group keys are salted truncated HMAC-SHA256 hashes (`g_*`, 16 hex) as the
treated-payload scanner expects. Each stored row carries a digest over its fields so a tampered
numerator is detected; the verifier recomputes the rate from numerator and count with exact
integer arithmetic (the same function the scout uses to produce its figure).
"""
import hashlib
import hmac
import sqlite3
from collections import defaultdict
from decimal import ROUND_HALF_EVEN, Decimal
from pathlib import Path

K = 10
METRIC = "recurrence_rate"
GROUP_FIELD = "group"


def group_hash(salt, field, value):
    return hmac.new(salt, f"{field}\0{value}".encode(), hashlib.sha256).hexdigest()[:16]


def rate_of(numerator, count):
    """Exact, deterministic rate rounded half-even to 2 decimals (no float division)."""
    q = (Decimal(numerator) / Decimal(count)).quantize(Decimal("0.01"), rounding=ROUND_HALF_EVEN)
    return float(q)


def _digest(salt, ref, metric, window, ghash, numerator, count):
    """Keyed digest (HMAC, salt not stored in the lab) binding ref, fields, numerator and count."""
    return hmac.new(salt, f"digest|{ref}|{metric}|{window}|{ghash}|{numerator}|{count}".encode(),
                    hashlib.sha256).hexdigest()


def _ref(metric, window, ghash):
    return "ev_" + hashlib.sha256(f"{metric}|{window}|{ghash}".encode()).hexdigest()[:16]


def _relations(agg, window_parts, group_parts):
    """Additive relations among published cells: [(total_cell, [part_cells])], from the declared overlaps."""
    rels = []
    for total, parts in (window_parts or {}).items():
        for g in sorted({g for g, _w in agg}):
            cells = [c for c in [(g, total)] + [(g, p) for p in parts] if c in agg]  # an absent cell is a known zero
            if len(cells) >= 2:
                rels.append(cells)
    for total, parts in (group_parts or {}).items():
        for w in sorted({w for _g, w in agg}):
            cells = [c for c in [(total, w)] + [(p, w) for p in parts] if c in agg]
            if len(cells) >= 2:
                rels.append(cells)
    return rels


def complementary_suppression(agg, hidden, window_parts=None, group_parts=None):
    """Grow `hidden` until no relation has exactly one hidden cell (that cell would be total minus the others).
    Hides the smallest published cell of the relation (ties by key) and iterates to a fixed point. Returns the extra cells."""
    hidden, extra = set(hidden), set()
    rels = _relations(agg, window_parts, group_parts)
    changed = True
    while changed:
        changed = False
        for cells in rels:
            gone = [c for c in cells if c in hidden]
            if len(gone) == 1:
                pick = min((c for c in cells if c not in hidden), key=lambda c: (agg[c][1], c))
                hidden.add(pick)
                extra.add(pick)
                changed = True
    return extra


def build_lab(path, cases, salt, k=K, min_cell=0, window_parts=None, group_parts=None):
    """cases: iterable of (case_id, group, window, outcome). case_id is read and discarded.
    window_parts / group_parts: declared overlaps {total: [parts]} among published windows / groups; a hidden cell
    that a published total minus its sibling cells would reveal makes another cell of that relation hidden too.
    min_cell: also drop a group whose numerator or complement is below it (real data passes K; the rate would expose it)."""
    if not isinstance(salt, bytes) or len(salt) < 16:
        raise ValueError("salt must be at least 16 bytes")
    agg = defaultdict(lambda: [0, 0])
    for _case_id, group, window, outcome in cases:
        a = agg[(group, window)]
        a[1] += 1
        a[0] += 1 if outcome else 0
    path = Path(path)
    if path.exists():
        path.unlink()
    con = sqlite3.connect(path)
    con.execute("create table lab_rows (evidence_ref text primary key, metric_id text, window_id text,"
                " g_group text, numerator integer, count integer, digest text)")
    con.execute("create table lab_meta (key text primary key, value text)")
    primary = {c for c, (num, cnt) in agg.items() if cnt < k or num < min_cell or cnt - num < min_cell}
    extra = complementary_suppression(agg, primary, window_parts, group_parts)
    for (group, window), (num, cnt) in sorted(agg.items()):
        if (group, window) in primary or (group, window) in extra:  # small cell in either side of the rate is a disclosure
            continue
        gh = group_hash(salt, GROUP_FIELD, group)
        ref = _ref(METRIC, window, gh)
        con.execute("insert into lab_rows values (?,?,?,?,?,?,?)",
                    (ref, METRIC, window, gh, num, cnt, _digest(salt, ref, METRIC, window, gh, num, cnt)))
    con.executemany("insert into lab_meta values (?,?)", [("k", str(k))])
    con.commit()
    con.close()
    return str(path)


def _fetch(db, ref):
    con = sqlite3.connect(db)
    try:
        return con.execute("select evidence_ref, metric_id, window_id, g_group, numerator, count, digest"
                           " from lab_rows where evidence_ref = ?", (ref,)).fetchone()
    finally:
        con.close()


def resolve_ref(db, ref):
    r = _fetch(db, ref)
    return None if r is None else {"evidence_ref": r[0], "metric_id": r[1], "window_id": r[2]}


def lab_query(db, metric_id, window_id):
    """Scanner-shaped treated rows; the numerator never leaves the lab."""
    con = sqlite3.connect(db)
    try:
        rows = con.execute("select evidence_ref, metric_id, window_id, g_group, numerator, count from lab_rows"
                           " where metric_id = ? and window_id = ? order by evidence_ref",
                           (metric_id, window_id)).fetchall()
    finally:
        con.close()
    return {"rows": [{"metric_id": m, "window_id": w, "g_group": g, "count": c, "rate": rate_of(n, c),
                      "evidence_ref": e} for e, m, w, g, n, c in rows]}


def lab_groups(db, metric_id=METRIC, window_id="w1"):
    """Treated group view for the orchestrator: hashed group key -> evidence ref, numerator, count (never raw labels)."""
    con = sqlite3.connect(db)
    try:
        rows = con.execute("select g_group, evidence_ref, numerator, count from lab_rows where metric_id = ? and"
                           " window_id = ? order by g_group", (metric_id, window_id)).fetchall()
    finally:
        con.close()
    return {g: {"evidence_ref": e, "numerator": n, "count": c} for g, e, n, c in rows}


def scout_figure(db, ref):
    r = _fetch(db, ref)
    if r is None:
        raise KeyError("unresolved evidence ref")
    return {"evidence_ref": r[0], "rate": rate_of(r[4], r[5]), "count": r[5]}


def verify_claim(db, claim, salt):
    """Independent recompute from the stored numerator; fails on unresolved ref, digest or figure mismatch."""
    r = _fetch(db, claim.get("evidence_ref"))
    if r is None:
        return {"ok": False, "reasons": ["evidence ref does not resolve in the lab"], "recomputed": None}
    ref, metric, window, gh, num, cnt, digest = r
    reasons = []
    if ref != _ref(metric, window, gh):
        reasons.append("evidence ref does not match row identity")
    if digest != _digest(salt, ref, metric, window, gh, num, cnt):
        reasons.append("row digest mismatch (numerator tampered)")
    if cnt < K:
        reasons.append("row count below k")
    recomputed = rate_of(num, cnt)
    if claim.get("count") != cnt:
        reasons.append("claimed count differs from lab")
    if claim.get("rate") != recomputed:
        reasons.append("claimed rate differs from recompute")
    return {"ok": not reasons, "reasons": reasons, "recomputed": recomputed}
