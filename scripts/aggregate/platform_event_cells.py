"""EVT1: platform events -> treated cell table (aggregates only, k >= 10) for the metric-generic cells sensor.

Input (already past the exporter allow-list; never raw payload text):
  events: rows `{"sequence", "event_type", "case_id", "event_time", "payload"}` (payload = allow-listed enum/bounded keys)
  cases : rows `{"case_id", "customer_id"?, "channel", "language", "case_type"}` (the exporter's `cases` columns)
Output: the six-field rows of `steps::cells` (`{metric, dims, half, period, numerator, denominator}`), `period` in
ALL / W1 / W2 only (full-period + two R2 windows; no month rows, no margins), plus a local stats summary.

Metrics (every rate is "higher = worse", the only direction the sensor flags; acceptance is published as its complement):
  P_DRAFT_REJECT      copilot.suggestion_decided (subject reply): discarded or ignored / all four decisions
  P_DRAFT_HEAVY_EDIT  decided used|edited with edit_distance_permille: heavy bucket (>= 500) / all (bucket edges fixed here)
  P_SUGG_NONE         copilot.suggestion_none / (ready + none)
  P_SUGG_FAILED       copilot.suggestion_failed / (ready + none + failed)   (level risk in the sensor)
  P_TOOL_USE          cases where the analyst used tool T / cases with copilot activity (dims case_type|channel x tool)
  P_TYPE_REASSIGN     cases whose type was changed away from a real type / cases that had a real type
  P_ASSIST_ESCALATION assistant.ended result escalated / ended sessions

Privacy rules (same family as bank_cells.py, DET1 hierarchy kept):
  - a row is published only when den >= k and numerator and complement are each 0 or >= k;
  - NO margin rows (no row without both dims of its signature), so a suppressed cell cannot be read off a published total;
  - ALL > W1/W2 hierarchy: if ALL is withheld both windows are; if one window is withheld the other is;
  - cross-signature rule: signatures of one metric are partitions of the same population, so two signatures that share a
    marginal could difference each other. `publish` runs a differencing attacker (`find_leaks`) over the withheld cells and
    repairs every determined combination (a cell or a sum of up to 3 cells) that fails k by withholding one more published
    cell, until none is left. See tests/test_platform_event_suppression.py (independent attacker oracle + mutation checks).
  - dims are a closed vocabulary; values are checked against a safe pattern and an id-prefix deny list; no ids, no text.
"""
from __future__ import annotations

import argparse
import itertools
import json
import re
import sys
from collections import defaultdict
from fractions import Fraction
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bank_cells as bc  # noqa: E402  (split_half, k_ok: one split and one k rule for every cell table)

K_DEFAULT = 10
DIMS = ("case_type", "channel", "language", "release", "agent", "tool")
SIGNATURES = {
    "P_DRAFT_REJECT": (("case_type", "channel"), ("language", "channel"), ("release", "agent")),
    "P_DRAFT_HEAVY_EDIT": (("case_type", "channel"), ("language", "channel"), ("release", "agent")),
    "P_SUGG_NONE": (("case_type", "channel"), ("language", "channel"), ("release", "agent")),
    "P_SUGG_FAILED": (("channel", "language"),),
    "P_TOOL_USE": (("case_type", "tool"), ("channel", "tool")),
    "P_TYPE_REASSIGN": (("case_type", "channel"), ("language", "channel")),
    "P_ASSIST_ESCALATION": (("case_type", "channel"), ("language", "channel"), ("release", "agent")),
}
# Sensor-facing names of the user's families (docs and mapping use them).
FAMILY = {
    "P_DRAFT_REJECT": "draft_acceptance_rate (complement)",
    "P_DRAFT_HEAVY_EDIT": "draft_edit_distance bucket (heavy)",
    "P_SUGG_NONE": "suggestion_none_rate",
    "P_SUGG_FAILED": "suggestion_failed_rate",
    "P_TOOL_USE": "copilot tool_used mix",
    "P_TYPE_REASSIGN": "case type reassignment rate",
    "P_ASSIST_ESCALATION": "assistant escalation rate",
}
HEAVY_EDIT_PERMILLE = 500  # buckets: light < 200, medium 200..499, heavy >= 500; only the heavy share is a metric

CASE_TYPES = {"none", "unrecognized_charge", "undue_charge", "app_issue", "branch_service", "service_quality", "virtual_card"}
CHANNELS = {"app_chat", "web_chat", "chat_app", "chat_web", "phone_inbound", "phone_outbound", "email"}
CHANNEL_ALIAS = {"chat_app": "app_chat", "chat_web": "web_chat"}  # 1.1.0 names, same channel
LANGUAGES = {"es", "pt"}
DECISIONS = {"used", "edited", "discarded", "ignored", "accepted"}
SUBJECTS = {"reply", "escalation"}
ASSISTANT_RESULTS = {"resolved", "escalated", "ended", "failed", "released"}
SAFE_VALUE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:@+-]{0,63}$")
ID_PREFIX = re.compile(r"^(CASE|CUS|CUST|STF|STAFF|EVT|TRN|QST|AST|ANA|SUP|AGT|RUN|TR)-", re.I)

# Allow-list of payload keys per event type read here (everything else is dropped, never stored).
PAYLOAD_KEYS = {
    "copilot.suggestion_ready": ("agent", "release"),
    "copilot.suggestion_none": ("agent", "release"),
    "copilot.suggestion_failed": (),
    "copilot.suggestion_decided": ("subject", "decision", "edit_distance_permille", "agent", "release"),
    "copilot.tool_used": ("tool",),
    "case.type_changed": ("from", "to"),
    "assistant.turn_answered": ("agent", "release"),
    "assistant.ended": ("result",),
    "case.opened": (),
}
USED_TYPES = frozenset(PAYLOAD_KEYS)


def safe_value(v) -> str | None:
    """A dimension value that is a bounded identifier-like token and not a platform id; anything else is rejected."""
    if not isinstance(v, str) or not SAFE_VALUE.match(v) or ID_PREFIX.match(v):
        return None
    return v


def _parse_time(s) -> datetime | None:
    try:
        return datetime.fromisoformat(str(s).replace("Z", "+00:00")).replace(tzinfo=None)
    except ValueError:
        return None


class Discards:
    """Named discards; a published count below k is replaced by null (the count itself is aggregate information)."""

    def __init__(self):
        self.n = defaultdict(int)

    def add(self, name, c=1):
        self.n[name] += c

    def publish(self, k):
        return {name: (n if n >= k else None) for name, n in sorted(self.n.items())}


def _clean_payload(etype, payload, disc):
    out = {}
    if not isinstance(payload, dict):
        disc.add("payload_not_object")
        return out
    for key in PAYLOAD_KEYS[etype]:
        if key not in payload or payload[key] is None:
            continue
        v = payload[key]
        if key == "subject":
            ok = v in SUBJECTS
        elif key == "decision":
            ok = v in DECISIONS
        elif key == "result":
            ok = v in ASSISTANT_RESULTS
        elif key in ("from", "to"):
            ok = v in CASE_TYPES
        elif key == "edit_distance_permille":
            ok = isinstance(v, int) and not isinstance(v, bool) and 0 <= v <= 1000
        else:  # agent, release, tool: bounded identifier tokens
            ok = safe_value(v) is not None
        if ok:
            out[key] = v
        else:
            disc.add(f"unsafe_payload_value:{etype}:{key}")
    return out


def derive_units(events, cases, split_time=None, k=K_DEFAULT):
    """Events + cases -> unit records. Each unit is `(metric, dims, num, den_unit_half_key, time)`; returns (units, discards, info)."""
    disc = Discards()
    cmap = {}
    for c in cases:
        cid = c.get("case_id")
        if safe_value(cid) is None and not (isinstance(cid, str) and cid):
            continue
        ch = CHANNEL_ALIAS.get(c.get("channel"), c.get("channel"))
        cmap[cid] = {
            "customer": c.get("customer_id") or cid,
            "channel": ch if ch in CHANNELS else None,
            "language": c.get("language") if c.get("language") in LANGUAGES else None,
            "case_type": c.get("case_type") if c.get("case_type") in CASE_TYPES else None,
            "opened": _parse_time(c.get("opened_at")),
        }
    seen = set()
    rows = []
    for e in sorted((e for e in events if isinstance(e, dict)), key=lambda e: (e.get("sequence") is None, e.get("sequence") or 0)):
        et = e.get("event_type")
        if et not in USED_TYPES:
            continue
        seq = e.get("sequence")
        if seq in seen:
            disc.add("duplicate_sequence")
            continue
        seen.add(seq)
        t = _parse_time(e.get("event_time"))
        if t is None:
            disc.add("bad_event_time")
            continue
        rows.append((et, e.get("case_id"), t, _clean_payload(et, e.get("payload"), disc)))
    # Window = the window of the CASE (its opened_at): a case and all its decisions stay in one window, so a release change does not
    # leave stragglers of the old release in the new window. The boundary is the MEDIAN case opening time (equal volume per window).
    times = [c["opened"] for c in cmap.values() if c["opened"] is not None] or [r[2] for r in rows]
    if split_time is None and times:
        split_time = sorted(times)[len(times) // 2]
    per = defaultdict(lambda: {"type_changes": [], "tools": set(), "copilot": False, "release": None, "agent": None,
                               "ended": None, "t0": None})
    units = []

    def dims_of(cid, extra):
        c = cmap.get(cid)
        if c is None:
            return None
        d = {"case_type": c["case_type"], "channel": c["channel"], "language": c["language"]}
        d.update(extra)
        return d

    for et, cid, t, p in rows:
        if cid is None:
            disc.add("event_without_case")
            continue
        st = per[cid]
        if st["t0"] is None or t < st["t0"]:
            st["t0"] = t
        if et == "case.type_changed":
            st["type_changes"].append((t, p.get("from"), p.get("to")))
        elif et == "copilot.tool_used":
            st["copilot"] = True
            if "tool" in p:
                st["tools"].add(p["tool"])
        elif et == "assistant.turn_answered":
            if "release" in p:
                st["release"] = p["release"]
            if "agent" in p:
                st["agent"] = p["agent"]
        elif et == "assistant.ended":
            st["ended"] = (t, p.get("result"))
        elif et in ("copilot.suggestion_ready", "copilot.suggestion_none", "copilot.suggestion_failed", "copilot.suggestion_decided"):
            st["copilot"] = True
            extra = {"release": p.get("release"), "agent": p.get("agent")}
            if et == "copilot.suggestion_ready":
                units.append(("P_SUGG_NONE", dims_of(cid, extra), 0, t, cid))
                units.append(("P_SUGG_FAILED", dims_of(cid, {}), 0, t, cid))
            elif et == "copilot.suggestion_none":
                units.append(("P_SUGG_NONE", dims_of(cid, extra), 1, t, cid))
                units.append(("P_SUGG_FAILED", dims_of(cid, {}), 0, t, cid))
            elif et == "copilot.suggestion_failed":
                units.append(("P_SUGG_FAILED", dims_of(cid, {}), 1, t, cid))
            else:
                dec, subj = p.get("decision"), p.get("subject", "reply")
                if subj != "reply" or dec not in ("used", "edited", "discarded", "ignored"):
                    disc.add("decision_not_a_reply_draft")
                    continue
                units.append(("P_DRAFT_REJECT", dims_of(cid, extra), int(dec in ("discarded", "ignored")), t, cid))
                dist = p.get("edit_distance_permille")
                if dec in ("used", "edited") and dist is not None:
                    units.append(("P_DRAFT_HEAVY_EDIT", dims_of(cid, extra), int(dist >= HEAVY_EDIT_PERMILLE), t, cid))
                elif dec in ("used", "edited"):
                    disc.add("edit_distance_missing")
    # case-level units: every case of the `cases` table counts (a case with no event is a case that kept its type)
    vocab = sorted({tool for st in per.values() for tool in st["tools"]})
    for cid in cmap:
        st = per[cid]
        t0 = cmap[cid]["opened"] or st["t0"]
        if t0 is None:
            disc.add("case_without_time")
            continue
        if st["copilot"]:
            for tool in vocab:
                units.append(("P_TOOL_USE", dims_of(cid, {"tool": tool}), int(tool in st["tools"]), t0, cid))
        real = [(t, f, to) for (t, f, to) in sorted(st["type_changes"]) if f != "none"]
        first_real = real[0][1] if real else None
        cur = cmap[cid]["case_type"]
        base = first_real or (cur if cur not in (None, "none") else None)
        if base is None:
            if st["type_changes"] or cur == "none":
                disc.add("type_never_real")
            continue
        d = dims_of(cid, {})
        d["case_type"] = base
        units.append(("P_TYPE_REASSIGN", d, int(first_real is not None), t0, cid))
    for cid, st in list(per.items()):
        if cid in cmap and st["ended"] is not None:
            t, res = st["ended"]
            if res is None:
                disc.add("assistant_result_missing")
                continue
            units.append(("P_ASSIST_ESCALATION", dims_of(cid, {"release": st["release"], "agent": st["agent"]}), int(res == "escalated"), t, cid))
    for (et, cid, t, p) in rows:
        if cid is not None and cid not in cmap:
            disc.add("event_case_not_in_cases")
    info = {"split_time": split_time.isoformat() if split_time else None, "events_used": len(rows), "cases": len(cmap)}
    return units, disc, cmap, split_time, info


def accumulate(units, cmap, split_time, disc):
    """Units -> raw cells keyed (metric, dims tuple, half, period) for ALL and the window of the unit."""
    raw = defaultdict(lambda: [0, 0])
    for metric, dims, num, t, cid in units:
        if dims is None:
            disc.add("no_case_dimension")
            continue
        half = bc.split_half(cmap[cid]["customer"])
        wt = cmap[cid]["opened"] or t
        window = "W1" if (split_time is None or wt < split_time) else "W2"
        for sig in SIGNATURES[metric]:
            vals = [dims.get(d) for d in sig]
            if any(v is None for v in vals):
                disc.add(f"dimension_unknown:{metric}")
                continue
            if any(safe_value(v) is None for v in vals):
                disc.add("unsafe_dimension_value")
                continue
            key = tuple(sorted(zip(sig, vals)))
            for period in ("ALL", window):
                c = raw[(metric, key, half, period)]
                c[0] += num
                c[1] += 1
    return {k: tuple(v) for k, v in raw.items()}


def _subsets(items):
    for r in range(len(items) + 1):
        yield from itertools.combinations(items, r)


def _identities(cells):
    """Linear identities a reader can rely on (each is {key: coef} summing to zero): the signatures of one metric partition the
    same population, so for every marginal Q the group totals of two signatures are equal; and ALL = W1 + W2 per cell."""
    ids = []
    by = defaultdict(lambda: defaultdict(list))
    for key in cells:
        metric, dims, half, period = key
        by[(metric, half, period)][tuple(d for d, _ in dims)].append(key)
    for sigs in by.values():
        for s, s2 in itertools.combinations(sorted(sigs), 2):
            for q in _subsets(sorted(set(s) & set(s2))):
                groups = defaultdict(dict)
                for sig, sign in ((s, 1), (s2, -1)):
                    for key in sigs[sig]:
                        dd = dict(key[1])
                        groups[tuple(dd[x] for x in q)][key] = sign
                ids.extend(groups.values())
    per = defaultdict(dict)
    for key in cells:
        per[(key[0], key[1], key[2])][key[3]] = key
    for p in per.values():
        if len(p) == 3:
            ids.append({p["ALL"]: 1, p["W1"]: -1, p["W2"]: -1})
    return ids


def _rref(rows, ncols):
    """RREF over Fractions -> ({pivot: {nonpivot col: coef}}, pivots)."""
    rows = [[Fraction(x) for x in r] for r in rows]
    used, piv_rows = set(), []
    for j in range(ncols):
        pr = next((i for i, r in enumerate(rows) if i not in used and r[j] != 0), None)
        if pr is None:
            continue
        used.add(pr)
        f = rows[pr][j]
        rows[pr] = [x / f for x in rows[pr]]
        for i, r in enumerate(rows):
            if i != pr and r[j] != 0:
                g = r[j]
                rows[i] = [x - g * y for x, y in zip(r, rows[pr])]
        piv_rows.append((pr, j))
    pivots = {j for _, j in piv_rows}
    return {j: {c: rows[i][c] for c in range(ncols) if c not in pivots and rows[i][c] != 0} for i, j in piv_rows}, pivots


def find_leaks(cells, pub, k=K_DEFAULT, max_subset=3):
    """Differencing attacker (one component = one metric x half): withheld cells, or sums of <= `max_subset` withheld cells, that
    FAIL the k rule and that the published cells plus the identities determine exactly (recovering a k-safe value is harmless). Returns [(leak_keys, [identities touching it])]."""
    leaks = []
    comps = defaultdict(dict)
    for key, v in cells.items():
        comps[(key[0], key[2])][key] = v
    for comp in comps.values():
        unknown = sorted(x for x in comp if x not in pub)
        if not unknown:
            continue
        idx = {x: i for i, x in enumerate(unknown)}
        rows, idents = [], []
        for ident in _identities(comp):
            row = [0] * len(unknown)
            for key, coef in ident.items():
                if key in idx:
                    row[idx[key]] += coef
            if any(row):
                rows.append(row)
                idents.append(ident)
        if not rows:
            continue
        table, pivots = _rref(rows, len(unknown))
        support = sorted({c for r in rows for c, x in enumerate(r) if x != 0})
        members = set(range(len(unknown))) - pivots
        for size in range(1, min(max_subset, len(support)) + 1):
            for subset in itertools.combinations(support, size):
                sset = set(subset)
                acc = {}
                for j in sset & pivots:
                    for c, coef in table[j].items():
                        acc[c] = acc.get(c, 0) + coef
                if all(acc.get(c, 0) == (1 if c in sset else 0) for c in members):
                    n = sum(comp[unknown[i]][0] for i in subset)
                    d = sum(comp[unknown[i]][1] for i in subset)
                    if not bc.k_ok(n, d, k):
                        keys = [unknown[i] for i in subset]
                        leaks.append((keys, [ident for ident in idents if any(x in ident for x in keys)]))
    return leaks


def _repair(cells, pub, leaks):
    """Secondary (complementary) suppression, minimal: for each leak withhold ONE published cell, the smallest one of the finest
    identity that touches the leak. Returns the number of leaks that could not be repaired (no published cell left to hide)."""
    unrepaired, taken = 0, set()
    for keys, idents in sorted(leaks, key=lambda t: (len(t[0]), t[0])):
        options = [[x for x in ident if x in pub and x not in taken] for ident in idents]
        options = [o for o in options if o]
        if not options:
            unrepaired += 1
            continue
        best = min(options, key=len)
        victim = min(best, key=lambda x: (cells[x][1], x))
        taken.add(victim)
    pub.difference_update(taken)
    return unrepaired


def _fixpoint(cells, pub, k):
    changed = True
    while changed:
        before = len(pub)
        # (A) ALL > W1/W2
        by_cell = defaultdict(dict)
        for (metric, dims, half, period) in cells:
            by_cell[(metric, dims, half)][period] = (metric, dims, half, period)
        for (metric, dims, half), per in by_cell.items():
            kk = lambda p: (metric, dims, half, p)  # noqa: E731
            if "ALL" in per and kk("ALL") not in pub:
                for w in ("W1", "W2"):
                    pub.discard(kk(w))
            wins = [w for w in ("W1", "W2") if w in per]
            if any(kk(w) not in pub for w in wins):
                for w in wins:
                    pub.discard(kk(w))
        changed = len(pub) != before


def publish(raw, k=K_DEFAULT):
    """Raw cells -> published keys under the full suppression contract. Returns (pub_set, cells_dict).
    1. k on every cell; 2. ALL > W1/W2 hierarchy; 3. closure: a differencing attacker
    (`find_leaks`, sums of up to 3 withheld cells) runs over the result and every published cell that takes part in a
    determined unsafe combination is repaired by withholding ONE more published cell (the smallest of the finest identity touching it),
    until the attacker finds nothing (pub only shrinks: terminates)."""
    cells = {key: list(v) for key, v in raw.items()}
    pub = {key for key, (n, d) in cells.items() if bc.k_ok(n, d, k)}
    while True:
        _fixpoint(cells, pub, k)
        leaks = find_leaks(cells, pub, k)
        if not leaks:
            return pub, cells
        before = len(pub)
        _repair(cells, pub, leaks)
        if len(pub) == before:  # a leak among withheld cells only: nothing published left to hide (surfaced by the property tests)
            return pub, cells


def build(events, cases, k=K_DEFAULT, split_time=None):
    units, disc, cmap, split, info = derive_units(events, cases, split_time=split_time, k=k)
    raw = accumulate(units, cmap, split, disc)
    pub, cells = publish(raw, k=k)
    rows = []
    suppressed = defaultdict(int)
    for (metric, dims, half, period), (num, den) in sorted(cells.items()):
        if (metric, dims, half, period) in pub:
            rows.append({"metric": metric, "dims": dict(dims), "half": half, "period": period, "numerator": num, "denominator": den})
        else:
            suppressed[metric] += 1
    stats = {"label": "synthetic_or_real_per_input_source", "k": k, "split_time": info["split_time"], "cases": info["cases"],
             "events_used": info["events_used"], "rows_published": len(rows),
             "cells_suppressed_by_metric": dict(sorted(suppressed.items())),
             "discards": disc.publish(k), "families": FAMILY}
    assert_clean(rows, k)
    return rows, stats


def assert_clean(rows, k=K_DEFAULT):
    """Hard output gate: exact six fields, closed dims, safe values, k on every row. Raises, never filters silently."""
    for r in rows:
        assert set(r) == {"metric", "dims", "half", "period", "numerator", "denominator"}, r
        assert r["metric"] in SIGNATURES and r["period"] in ("ALL", "W1", "W2") and r["half"] in ("discovery", "holdout")
        assert set(r["dims"]) <= set(DIMS) and all(safe_value(v) for v in r["dims"].values()), r["dims"]
        assert bc.k_ok(r["numerator"], r["denominator"], k), r


def to_ndjson(rows):
    return bc.to_ndjson(rows)


def read_ndjson(path):
    return [json.loads(line) for line in Path(path).read_text(encoding="utf-8").splitlines() if line.strip()]


def main(argv=None):
    ap = argparse.ArgumentParser(description="platform events -> cell table (aggregates only)")
    ap.add_argument("--events", required=True, help="events.ndjson (exporter-shaped, payload allow-listed)")
    ap.add_argument("--cases", required=True, help="cases.ndjson (case_id, customer_id, channel, language, case_type)")
    ap.add_argument("--out", required=True)
    ap.add_argument("--k", type=int, default=K_DEFAULT)
    a = ap.parse_args(argv)
    rows, stats = build(read_ndjson(a.events), read_ndjson(a.cases), k=a.k)
    out = Path(a.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(to_ndjson(rows), encoding="utf-8", newline="\n")
    print(json.dumps(stats, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
