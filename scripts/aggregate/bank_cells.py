#!/usr/bin/env python3
"""Build TREATED cell tables from the local bank dataset (aggregates only).

Reads ONLY call_center_interactions, complaints and satisfaction_surveys (CSV partitions under a local
data root, never committed) and emits ndjson cells for the Rust `cells` sensor (steps::cells):

    {"metric":"M1","dims":{"reason_category":"Queja","channel":"Phone"},"half":"discovery",
     "numerator":120,"denominator":600}

Guarantees: no identifiers, no free text, no row-level output. Every emitted count (numerator,
complement, denominator) is 0 or >= k (k = 10); violating cells are suppressed and only counted.
Cells carry a `period` (YYYY-MM, contact month); the partial months 2023-06 and 2026-06 are excluded.
Replication design: deterministic customer-hash split A/B 50/50 (A = discovery, B = confirmation) (cross-sectional,
because interaction timestamps are naive, without timezone).

Metrics (all "higher is worse"):
  M1  contact_unresolved_rate        calls with known resolution; by reason_category x channel
  M2  complaint_share_of_contacts    Queja contacts / all contacts; by channel
  M3  complaint_share_of_unresolved  Queja among unresolved contacts; by channel
  M4  pqr_open_rate                  PQR not Resolved/Closed/Rejected / all PQR; by category
  M5  pqr_sla_breach_rate            sla_breached / PQR; by category
  M6  survey_low_score_rate          low score / surveys; by channel
  M6L survey_low_score_rate_linked   same, surveys linkable to an interaction; by reason_category x channel
Low score: CSAT main_score <= 2. Only CSAT is scored (NPS/CES use other scales; pooling would be an artifact).

Usage: python scripts/aggregate/bank_cells.py --data-root D:/.codex/factored/data --out <local path>
"""
import argparse
import csv
import hashlib
import json
import sys
import unicodedata
from pathlib import Path

ALLOWED_TABLES = ("call_center_interactions", "complaints", "satisfaction_surveys")
SPLIT_SALT = "pulso-cells-v1|"
OPEN_STATUSES = {"Open", "In Process", "Escalated"}
SCORED_SURVEY_TYPE = "CSAT"  # other types use different scales; pooling them would fabricate differences
LOW_SCORE_MAX = 2.0
PARTIAL_MONTHS = {"2023-06", "2026-06"}  # data starts 2023-06-17 and ends 2026-06-18


def split_half(customer_id: str) -> str:
    """Audit R1: low bit of SHA-256(salt || customer_id); A = discovery, B = holdout (confirmation)."""
    h = hashlib.sha256((SPLIT_SALT + customer_id).encode("utf-8")).digest()
    return "discovery" if h[-1] & 1 == 0 else "holdout"


def norm(s: str) -> str:
    """Closed ASCII vocabulary: strip accents, trim."""
    return "".join(c for c in unicodedata.normalize("NFD", s.strip()) if unicodedata.category(c) != "Mn")


def read_table(root: Path, table: str):
    for f in sorted((root / table).rglob("*.csv")):
        raw = f.read_bytes()
        try:
            text = raw.decode("utf-8-sig")
        except UnicodeDecodeError:
            text = raw.decode("cp1252")
        yield from csv.DictReader(text.splitlines())


def row_key(r):
    return (r["metric"], sorted(r["dims"].items()), r["half"], r["period"])


class Acc:
    def __init__(self):
        self.cells = {}

    def add(self, metric, dims, half, period, num, den=1):
        c = self.cells.setdefault((metric, tuple(sorted(dims.items())), half, period), [0, 0])
        c[0] += num
        c[1] += den


def k_ok(num, den, k):
    return den >= k and (num == 0 or num >= k) and (den - num == 0 or den - num >= k)


def build(root, k=10, tables=ALLOWED_TABLES):
    root = Path(root)
    for t in tables:
        if t not in ALLOWED_TABLES:
            raise ValueError(f"table {t!r} is outside the allowlist {ALLOWED_TABLES}")
    acc = Acc()
    stats = {"k": k, "split": "customer_hash_A_B_50_50", "rows_read": {}}
    reason_of = {}
    stats["partial_month_rows_excluded"] = {}

    if "call_center_interactions" in tables:
        n = skipped = 0
        for r in read_table(root, "call_center_interactions"):
            period = r["interaction_date"][:7]
            if period in PARTIAL_MONTHS:
                skipped += 1
                reason_of[r["interaction_id"]] = None
                continue
            n += 1
            reason, chan = norm(r["reason_category"]), norm(r["channel"])
            half = split_half(r["customer_id"])
            reason_of[r["interaction_id"]] = (reason, r["was_resolved"].strip(), period)
            res = r["was_resolved"].strip()
            queja = int(reason == "Queja")
            acc.add("M2", {"channel": chan}, half, period, queja)
            if res in ("True", "False"):
                unresolved = int(res == "False")
                acc.add("M1", {"reason_category": reason, "channel": chan}, half, period, unresolved)
                if unresolved:
                    acc.add("M3", {"channel": chan}, half, period, queja)
        stats["rows_read"]["call_center_interactions"] = n
        stats["partial_month_rows_excluded"]["call_center_interactions"] = skipped

    if "complaints" in tables:
        n = skipped = 0
        for r in read_table(root, "complaints"):
            period = r["creation_date"][:7]
            if period in PARTIAL_MONTHS:
                skipped += 1
                continue
            n += 1
            cat, half = norm(r["category"]), split_half(r["customer_id"])
            acc.add("M4", {"category": cat}, half, period, int(r["status"].strip() in OPEN_STATUSES))
            sla = r["sla_breached"].strip()
            if sla in ("True", "False"):
                acc.add("M5", {"category": cat}, half, period, int(sla == "True"))
        stats["rows_read"]["complaints"] = n
        stats["partial_month_rows_excluded"]["complaints"] = skipped

    if "satisfaction_surveys" in tables:
        n = linked = scored = 0
        for r in read_table(root, "satisfaction_surveys"):
            n += 1
            st = r["survey_type"].strip()
            try:
                score = float(r["main_score"])
            except ValueError:
                continue
            if st != SCORED_SURVEY_TYPE:
                continue
            low = int(score <= LOW_SCORE_MAX)
            chan, half = norm(r["send_channel"]), split_half(r["customer_id"])
            scored += 1
            link = reason_of.get(r["interaction_id"])
            if link is not None:
                linked += 1
                dims = {"reason_category": link[0], "channel": chan}
                acc.add("M6", dims, half, link[2], low)
                if link[1] in ("True", "False"):
                    acc.add("M6R" if link[1] == "True" else "M6U", dims, half, link[2], low)
        stats["rows_read"]["satisfaction_surveys"] = n
        stats["surveys"] = {"total": n, "scored_csat": scored, "linked_scored": linked,
                            "linked_share": round(linked / scored, 4) if scored else 0.0}

    # Complementary suppression: a published margin must not let a reader subtract a suppressed cell.
    #  - M3 denominator = sum of M1 numerators of the channel: hide M3 when an M1 cell of that channel/half/period
    #    with a positive numerator is suppressed.
    #  - M6 = M6R + M6U (+ unknown resolution): hide the partner when M6R or M6U is suppressed.
    hide = set()
    for (metric, dims, half, period), (num, den) in acc.cells.items():
        if k_ok(num, den, k):
            continue
        d = dict(dims)
        if metric == "M1" and num > 0:
            hide.add(("M3", (("channel", d["channel"]),), half, period))
        elif metric in ("M6R", "M6U"):
            hide.add(("M6U" if metric == "M6R" else "M6R", dims, half, period))

    rows, suppressed = [], {}
    for (metric, dims, half, period), (num, den) in acc.cells.items():
        if k_ok(num, den, k) and (metric, dims, half, period) not in hide:
            rows.append({"metric": metric, "dims": dict(dims), "half": half, "period": period, "numerator": num, "denominator": den})
        else:
            suppressed[metric] = suppressed.get(metric, 0) + 1
    rows.sort(key=row_key)
    stats["suppressed_cells"] = sum(suppressed.values())
    stats["suppressed_by_metric"] = dict(sorted(suppressed.items()))
    per = {}
    for r in rows:
        m = per.setdefault(r["metric"], {"cells": 0, "numerator": 0, "denominator": 0})
        m["cells"] += 1
        m["numerator"] += r["numerator"]
        m["denominator"] += r["denominator"]
    stats["metrics"] = {m: per[m] for m in sorted(per)}
    return rows, stats


def to_ndjson(rows):
    return "".join(json.dumps(r, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n" for r in rows)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--data-root", required=True, help="local bank dataset root (never committed)")
    ap.add_argument("--out", required=True, help="ndjson output path (keep outside the repo)")
    ap.add_argument("--k", type=int, default=10)
    a = ap.parse_args(argv)
    rows, stats = build(a.data_root, k=a.k)
    out = Path(a.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(to_ndjson(rows), encoding="utf-8", newline="\n")
    print(json.dumps(stats, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
