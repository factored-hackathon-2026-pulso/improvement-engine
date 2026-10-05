#!/usr/bin/env python3
"""Build TREATED cell tables from the local bank dataset (aggregates only).

Reads ONLY call_center_interactions, complaints, satisfaction_surveys, digital_events, campaign_sends and
transactions (CSV partitions under a local data root, never committed), plus two reference files read by a column
allowlist (customers.csv: customer_id, segment, accepts_marketing; marketing_campaigns.csv: campaign_id,
campaign_type) and emits ndjson cells for the Rust `cells` sensor (steps::cells):

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
AG2 metrics (exact definitions and limits: docs/data/bank-cells-metrics.md):
  M7  digital_error_rate             Error events / events, identified customers; by action x channel (descriptive)
  M8  send_to_nonconsenting_rate     sends to accepts_marketing=false customers / sends; by campaign_type x channel (RISK)
  M9  tx_decline_rate                Declined / transactions; by channel x customer_segment (expected FLAT: non-finding)
  M10 handle_time_unresolved_share   unresolved handled HOURS / handled hours; by reason_category x channel
                                     (re-expression of M1: the sensor flags it depends_on M1)
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

ALLOWED_TABLES = ("call_center_interactions", "complaints", "satisfaction_surveys",
                  "digital_events", "campaign_sends", "transactions")
# Reference files (not partitioned): read by name and by column allowlist; PII columns are never indexed.
REF_COLUMNS = {"customers.csv": ("customer_id", "segment", "accepts_marketing"),
               "marketing_campaigns.csv": ("campaign_id", "campaign_type")}
# Actions with no Error events by construction (audit F10): excluded so they do not deflate the comparison baseline.
STRUCTURAL_ACTIONS = {"", "login", "logout", "view_product"}
HANDLE_TIME_UNIT_SECONDS = 3600  # M10 is emitted in whole hours: a conservative effective n (seconds would overstate it)
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


def _decode(raw: bytes) -> str:
    try:
        return raw.decode("utf-8-sig")
    except UnicodeDecodeError:
        return raw.decode("cp1252")


def _project(text: str, cols):
    rd = csv.reader(text.splitlines())
    header = next(rd, None)
    if header is None:
        return
    idx = [header.index(c) for c in cols]
    top = max(idx)
    for row in rd:
        if len(row) > top:
            yield tuple(row[i] for i in idx)


def read_cols(root: Path, table: str, cols):
    """Fast column-subset reader: yields tuples of the requested columns only (a missing column raises)."""
    for f in sorted((root / table).rglob("*.csv")):
        yield from _project(_decode(f.read_bytes()), cols)


def read_reference(root: Path, name: str):
    """Reference file restricted to its allowlisted columns (REF_COLUMNS); yields tuples in that order."""
    path = Path(root) / name
    if path.exists():
        yield from _project(_decode(path.read_bytes()), REF_COLUMNS[name])


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
    absent = [t for t in tables if not (root / t).is_dir()]
    tables = tuple(t for t in tables if t not in absent)
    stats["tables_absent"] = absent
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
                dur = (r.get("duration_seconds") or "").strip()
                if dur:
                    try:
                        secs = int(float(dur))
                    except ValueError:
                        secs = -1
                    if secs >= 0:
                        acc.add("M10S", {"reason_category": reason, "channel": chan}, half, period, secs * unresolved, secs)
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

    # --- AG2 tables -------------------------------------------------------------------------------------------
    halves, cache, customers, campaigns = {}, {}, None, None

    def half_of(cid):
        h = halves.get(cid)
        if h is None:
            h = halves[cid] = split_half(cid)
        return h

    def vocab(v):
        n = cache.get(v)
        if n is None:
            n = cache[v] = norm(v)
        return n

    def load_customers():
        return {c: (norm(sg), a.strip()) for c, sg, a in read_reference(root, "customers.csv")}

    if "digital_events" in tables:
        n = skipped = anon = structural = 0
        for event_date, cid, etype, chan, action in read_cols(
                root, "digital_events", ("event_date", "customer_id", "event_type", "channel", "action")):
            period = event_date[:7]
            if period in PARTIAL_MONTHS:
                skipped += 1
                continue
            n += 1
            if not cid:
                anon += 1  # anonymous events cannot be split A/B and are never used
                continue
            action = action.strip()
            if action in STRUCTURAL_ACTIONS:
                structural += 1
                continue
            acc.add("M7", {"action": vocab(action), "channel": vocab(chan)}, half_of(cid), period,
                    int(etype.strip() == "Error"))
        stats["rows_read"]["digital_events"] = n
        stats["partial_month_rows_excluded"]["digital_events"] = skipped
        stats["digital_events"] = {"anonymous_excluded": anon, "structural_actions_excluded": structural}

    if "campaign_sends" in tables:
        customers = load_customers()
        campaigns = {c: norm(t) for c, t in read_reference(root, "marketing_campaigns.csv")}
        n = skipped = unk_consent = unk_campaign = 0
        for send_date, cid, camp, chan in read_cols(
                root, "campaign_sends", ("send_date", "customer_id", "campaign_id", "send_channel")):
            period = send_date[:7]
            if period in PARTIAL_MONTHS:
                skipped += 1
                continue
            n += 1
            consent = customers.get(cid, ("", ""))[1]
            if consent not in ("True", "False"):
                unk_consent += 1
                continue
            ctype = campaigns.get(camp)
            if not ctype:
                unk_campaign += 1
                continue
            acc.add("M8", {"campaign_type": ctype, "channel": vocab(chan)}, half_of(cid), period, int(consent == "False"))
        stats["rows_read"]["campaign_sends"] = n
        stats["partial_month_rows_excluded"]["campaign_sends"] = skipped
        stats["campaign_sends"] = {"consent_unknown_excluded": unk_consent, "campaign_unknown_excluded": unk_campaign}

    if "transactions" in tables:
        if customers is None:
            customers = load_customers()
        n = skipped = unk = 0
        for tdate, cid, chan, status in read_cols(
                root, "transactions", ("transaction_date", "customer_id", "channel", "transaction_status")):
            period = tdate[:7]
            if period in PARTIAL_MONTHS:
                skipped += 1
                continue
            n += 1
            seg, status = customers.get(cid, ("", ""))[0], status.strip()
            if not seg or not status:
                unk += 1
                continue
            acc.add("M9", {"channel": vocab(chan), "customer_segment": seg}, half_of(cid), period, int(status == "Declined"))
        stats["rows_read"]["transactions"] = n
        stats["partial_month_rows_excluded"]["transactions"] = skipped
        stats["transactions"] = {"segment_or_status_unknown_excluded": unk}

    # M10 is accumulated in seconds (M10S) and emitted in whole hours (conservative effective n).
    cells = {}
    for (metric, dims, half, period), (num, den) in acc.cells.items():
        if metric == "M10S":
            metric, num, den = "M10", num // HANDLE_TIME_UNIT_SECONDS, den // HANDLE_TIME_UNIT_SECONDS
        cells[(metric, dims, half, period)] = [num, den]

    # Complementary suppression: a published margin must not let a reader subtract a suppressed cell.
    #  - M3 denominator = sum of M1 numerators of the channel: hide M3 when an M1 cell of that channel/half/period
    #    with a positive numerator is suppressed.
    #  - M6 = M6R + M6U (+ unknown resolution): hide the partner when M6R or M6U is suppressed.
    hide = set()
    for (metric, dims, half, period), (num, den) in cells.items():
        if k_ok(num, den, k):
            continue
        d = dict(dims)
        if metric == "M1" and num > 0:
            hide.add(("M3", (("channel", d["channel"]),), half, period))
        elif metric in ("M6R", "M6U"):
            hide.add(("M6U" if metric == "M6R" else "M6R", dims, half, period))

    rows, suppressed = [], {}
    for (metric, dims, half, period), (num, den) in cells.items():
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
