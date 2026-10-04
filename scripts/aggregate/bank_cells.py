#!/usr/bin/env python3
"""Build TREATED cell tables from the local bank dataset (aggregates only).

Reads ONLY call_center_interactions, complaints and satisfaction_surveys (CSV partitions under a local
data root, never committed) and emits ndjson cells for the Rust `cells` sensor (steps::cells):

    {"metric":"M1","dims":{"reason_category":"Queja","channel":"Phone"},"half":"discovery",
     "numerator":120,"denominator":600}

Guarantees: no identifiers, no free text, no row-level output. Every emitted count (numerator,
complement, denominator) is 0 or >= k (k = 10); violating cells are suppressed and only counted.
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
    return (r["metric"], sorted(r["dims"].items()), r["half"])


class Acc:
    def __init__(self):
        self.cells = {}

    def add(self, metric, dims, half, num, den=1):
        c = self.cells.setdefault((metric, tuple(sorted(dims.items())), half), [0, 0])
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

    if "call_center_interactions" in tables:
        n = 0
        for r in read_table(root, "call_center_interactions"):
            n += 1
            reason, chan = norm(r["reason_category"]), norm(r["channel"])
            half = split_half(r["customer_id"])
            reason_of[r["interaction_id"]] = (reason, r["was_resolved"].strip())
            res = r["was_resolved"].strip()
            queja = int(reason == "Queja")
            acc.add("M2", {"channel": chan}, half, queja)
            if res in ("True", "False"):
                unresolved = int(res == "False")
                acc.add("M1", {"reason_category": reason, "channel": chan}, half, unresolved)
                if unresolved:
                    acc.add("M3", {"channel": chan}, half, queja)
        stats["rows_read"]["call_center_interactions"] = n

    if "complaints" in tables:
        n = 0
        for r in read_table(root, "complaints"):
            n += 1
            cat, half = norm(r["category"]), split_half(r["customer_id"])
            acc.add("M4", {"category": cat}, half, int(r["status"].strip() in OPEN_STATUSES))
            sla = r["sla_breached"].strip()
            if sla in ("True", "False"):
                acc.add("M5", {"category": cat}, half, int(sla == "True"))
        stats["rows_read"]["complaints"] = n

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
            if link is not None and link[1] in ("True", "False"):
                linked += 1
                metric = "M6R" if link[1] == "True" else "M6U"
                acc.add(metric, {"reason_category": link[0], "channel": chan}, half, low)
        stats["rows_read"]["satisfaction_surveys"] = n
        stats["surveys"] = {"total": n, "scored_csat": scored, "linked_scored": linked,
                            "linked_share": round(linked / scored, 4) if scored else 0.0}

    rows, suppressed = [], {}
    for (metric, dims, half), (num, den) in acc.cells.items():
        if k_ok(num, den, k):
            rows.append({"metric": metric, "dims": dict(dims), "half": half, "numerator": num, "denominator": den})
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
