"""Decision feedback (FDBK1): what happened to the engine's proposals after delivery.

Pull-only reader of agent-core. Reads `GET /v1/export/registry-events` (role `exporter`, persisted cursor) and
`GET /v1/registry/proposals/{pid}` (role `builder`, read), folds the lifecycle of every proposal whose origin is
`auto_detect` (optionally also created by the engine principal) and appends one `pulso.decision/1` record per state
change to a JSONL sink. Also pure helpers: closed-vocabulary reason mapping, aggregate metrics, the suppression
rule and the dossier history line. See docs/dev/DECISION_FEEDBACK.md.

Safety: reason text is UNTRUSTED. It is never stored, logged or returned: it is mapped to the closed vocabulary
(or `unknown`) and only its length is kept. Nothing from here goes into a model prompt except the closed-vocabulary
counts of `history_line`. Only loopback hosts are contacted. Tokens come from the environment or a local JSON file
and are never printed or stored.

Usage (values never printed):
  python decision_feedback.py poll --once --state s.json --out decisions.jsonl --registry-token-env X --export-token-env Y
  python decision_feedback.py metrics --in decisions.jsonl
  python decision_feedback.py suppress --in decisions.jsonl --target template:t/x --family M4
  python decision_feedback.py history --in decisions.jsonl --target template:t/x --family M4 --lang es
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import statistics
import sys
import time
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCHEMA = "pulso.decision/1"
METRICS_SCHEMA = "pulso.decision_metrics/1"
REASONS = ("insufficient_evidence", "wrong_target", "risk", "duplicate", "policy_conflict", "wording", "other")
BLOCKING_REASONS = frozenset({"wrong_target", "duplicate", "policy_conflict", "risk"})
DECIDED = frozenset({"approved", "published", "rejected"})
POSITIVE = frozenset({"approved", "published"})
LOOPBACK = {"127.0.0.1", "localhost", "::1"}
LEDGER_EVENT_TYPES = frozenset({"proposal_created", "draft_updated", "frozen", "evaluated", "approved", "rejected",
                                "reopened", "proposal_staled", "published"})
LEDGER_CAP = 60
EXPIRE_DAYS = 14
SEEN_CAP = 5000

# Free-text mapping (Spanish, Portuguese, English). Ordered; two different matches -> `other` (never blocking).
_KEYWORDS = (
    ("wrong_target", ("artefacto equivocado", "objetivo equivocado", "agente equivocado", "otro agente", "wrong target",
                      "wrong agent", "alvo errado", "agente errado", "target equivocado")),
    ("duplicate", ("duplicad", "duplicate", "ya existe", "ja existe", "repetid")),
    ("policy_conflict", ("politica", "policy", "normativ", "regulat", "compliance", "cumplimiento", "conformidade")),
    ("risk", ("riesgo", "arriesgad", "risky", "risk", "risco", "perigo", "inseguro")),
    ("insufficient_evidence", ("evidencia", "evidence", "insuficiente", "insufficient", "pocos casos", "evidencia")),
    ("wording", ("redaccion", "wording", "tono", "redacao", "frase")),
)
_FINDING = re.compile(r"\[finding:\s*([A-Za-z0-9_.:|=/ -]{1,120}?)\s*\]")
_TITLE_METRIC = re.compile(r"\s-\s([A-Za-z]\d{1,3})\b")
_FAMILY_OK = re.compile(r"^[A-Za-z0-9_.:-]{1,40}$")


class SinkError(RuntimeError):
    pass


# ---------------------------------------------------------------------------------------------- pure helpers

def _ascii(text: str) -> str:
    return unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode().lower()


def parse_ts(s: str) -> datetime:
    return datetime.fromisoformat(s.replace("Z", "+00:00")).astimezone(timezone.utc)


def fmt_ts(d: datetime) -> str:
    return d.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def normalize_reason(src: dict) -> tuple[str, str, int]:
    """(code, source, note_len). `reason_code` is the structured field (closed vocabulary); `reason` / `reason_note`
    is untrusted free text that is mapped, never kept. source: structured | mapped | none."""
    note = src.get("reason_note") or src.get("reason") or ""
    note_len = len(note) if isinstance(note, str) else 0
    raw = src.get("reason_code")
    if isinstance(raw, str) and raw.strip():
        code = raw.strip().lower()
        return (code if code in REASONS else "other"), "structured", note_len
    if isinstance(note, str) and note.strip():
        text = _ascii(note)
        hits = [code for code, words in _KEYWORDS if any(w in text for w in words)]
        if len(hits) == 1:
            return hits[0], "mapped", note_len
        if len(hits) > 1:
            return "other", "mapped", note_len
    return "unknown", "none", note_len


def finding_key(rationale: str, changelog: str, title: str = "") -> str | None:
    """Finding key from the header tag `[finding: <key>]` in rationale, then changelog; else the metric id in the
    title (`<target> - M4: ...`). Hostile or oversized headers yield None."""
    for text in (rationale, changelog):
        m = _FINDING.search(text or "")
        if m:
            return m.group(1)
    m = _TITLE_METRIC.search(title or "")
    return m.group(1) if m else None


def family_of(key: str | None) -> str:
    if not key:
        return "unknown"
    fam = key.split("|", 1)[0].strip()
    return fam if _FAMILY_OK.match(fam) else "unknown"


def fold_events(events: list, now: str, expire_days: int = EXPIRE_DAYS) -> dict:
    """Lifecycle of one proposal from its registry events: [type, at, reason_code?, source?, note_len?]."""
    state, created, last, first_dec, dec_at = "draft", None, None, None, None
    code, source, note_len = None, None, 0
    for e in events:
        typ, at = e[0], e[1]
        created = created or at
        last = at
        if typ == "frozen":
            state = "candidate"
        elif typ == "evaluated":
            state = "evaluated"
        elif typ in ("approved", "rejected"):
            state, dec_at = typ, at
            first_dec = first_dec or at
            if typ == "rejected":
                code = e[2] if len(e) > 2 and e[2] else "unknown"
                source = (e[3] if len(e) > 3 and e[3] else "structured" if code != "unknown" else "none")
                note_len = e[4] if len(e) > 4 and e[4] else 0
        elif typ == "published":
            state, dec_at = "published", dec_at or at
        elif typ in ("reopened", "proposal_staled"):
            state = "draft"
        # proposal_created / draft_updated: no state change (a rejected proposal stays rejected while reworked)
    if state in ("draft", "candidate", "evaluated") and last:
        limit = parse_ts(last) + timedelta(days=expire_days)
        if parse_ts(now) > limit:
            state, dec_at = "expired", fmt_ts(limit)
    ttd = None
    if first_dec and created:
        ttd = int((parse_ts(first_dec) - parse_ts(created)).total_seconds())
    return {"state": state, "created_at": created, "last_at": last, "decision_at": dec_at,
            "time_to_decision_secs": ttd,
            "reason_code": code if state == "rejected" else None,
            "reason_source": source if state == "rejected" else None,
            "reason_note_len": note_len if state == "rejected" else 0}


def _latest(records: list[dict]) -> list[dict]:
    last: dict[str, dict] = {}
    for r in records:
        last[r["proposal_id"]] = r
    return list(last.values())


def aggregate(records: list[dict]) -> dict:
    """Headline numbers over the latest record of each proposal."""
    rows = _latest(records)

    def counts(rs: list[dict]) -> dict:
        c = {"made": len(rs), "approved": 0, "rejected": 0, "expired": 0, "pending": 0}
        for r in rs:
            s = r["state"]
            c["approved" if s in POSITIVE else "rejected" if s == "rejected" else "expired" if s == "expired"
              else "pending"] += 1
        return c

    out = counts(rows)
    decided = [r["time_to_decision_secs"] for r in rows if r["state"] in DECIDED and r.get("time_to_decision_secs") is not None]
    med = statistics.median(decided) if decided else None
    out["median_time_to_decision_secs"] = int(med) if med is not None and float(med).is_integer() else med
    d = out["approved"] + out["rejected"]
    out["approval_rate"] = out["approved"] / d if d else None
    reasons: dict[str, int] = {}
    for r in rows:
        if r["state"] == "rejected":
            code = (r.get("reason") or {}).get("code", "unknown")
            reasons[code] = reasons.get(code, 0) + 1
    out["reasons"] = reasons
    kinds = sorted({r.get("kind") or "unknown" for r in rows})
    out["by_kind"] = {k: counts([r for r in rows if (r.get("kind") or "unknown") == k]) for k in kinds}
    return {"schema": METRICS_SCHEMA, **out}


def suppress(records: list[dict], target: str, family: str, now: str, cooldown_days: int = 30,
             allow_mapped: bool = False) -> dict:
    """Do not re-propose `target` for `family` inside the cooldown after a rejection whose reason is in
    BLOCKING_REASONS. Only structured reasons count unless `allow_mapped`. A later approval lifts it."""
    rows = [r for r in _latest(records) if r.get("target") == target and r.get("family") == family]
    blocking = [r for r in rows if r["state"] == "rejected" and r.get("decision_at")
                and (r.get("reason") or {}).get("code") in BLOCKING_REASONS
                and ((r.get("reason") or {}).get("source") == "structured" or allow_mapped)]
    if not blocking:
        return {"suppress": False}
    top = max(blocking, key=lambda r: parse_ts(r["decision_at"]))
    until = parse_ts(top["decision_at"]) + timedelta(days=cooldown_days)
    lifted = any(r["state"] in POSITIVE and r.get("decision_at") and parse_ts(r["decision_at"]) > parse_ts(top["decision_at"])
                 for r in rows)
    if lifted or parse_ts(now) >= until:
        return {"suppress": False}
    return {"suppress": True, "reason": top["reason"]["code"], "proposal_id": top["proposal_id"], "until": fmt_ts(until),
            "target": target, "family": family}


def history(records: list[dict], target: str, family: str) -> dict | None:
    """Closed-vocabulary history for the dossier: same target+family, else same family. None when empty."""
    rows = [r for r in _latest(records) if r.get("family") == family and r["state"] in DECIDED]
    scope = "target_family"
    sel = [r for r in rows if r.get("target") == target]
    if not sel:
        scope, sel = "family", rows
    if not sel:
        return None
    reasons: dict[str, int] = {}
    for r in sel:
        code = (r.get("reason") or {}).get("code", "unknown")
        if r["state"] == "rejected" and code != "unknown":
            reasons[code] = reasons.get(code, 0) + 1
    top = max(reasons, key=lambda k: (reasons[k], k)) if reasons else None
    return {"scope": scope, "target": target, "family": family,
            "approved": sum(r["state"] in POSITIVE for r in sel), "rejected": sum(r["state"] == "rejected" for r in sel),
            "reasons": dict(sorted(reasons.items(), key=lambda kv: (-kv[1], kv[0]))), "top_reason": top}


def history_line(h: dict | None, lang: str = "es") -> str | None:
    if h is None:
        return None
    a, r, top = h["approved"], h["rejected"], h["top_reason"]
    same = h["scope"] == "target_family"
    t = {
        "es": (f"Historial: {a} aprobadas y {r} rechazadas entre propuestas similares"
               + ("" if same else " (misma familia, otros artefactos)"),
               f" (motivo más frecuente: {top})"),
        "pt": (f"Histórico: {a} aprovadas e {r} rejeitadas entre propostas similares"
               + ("" if same else " (mesma família, outros artefatos)"),
               f" (motivo mais frequente: {top})"),
        "en": (f"History: {a} approved and {r} rejected among similar proposals"
               + ("" if same else " (same family, other artifacts)"),
               f" (most frequent reason: {top})"),
    }[lang]
    return t[0] + (t[1] if top else "") + "."


# ---------------------------------------------------------------------------------------------- transport

def require_loopback(url: str) -> str:
    u = urllib.parse.urlparse(url)
    if u.scheme not in ("http", "https") or (u.hostname or "") not in LOOPBACK:
        raise ValueError("only loopback hosts are allowed")
    return url


def redact(text: str, secrets: list[str]) -> str:
    for s in secrets:
        if s:
            text = text.replace(s, "[redacted]")
    return re.sub(r"(?i)(bearer\s+)\S+", r"\1[redacted]", text)


class HttpFetch:
    """GET JSON from agent-core (loopback only). Export paths use the exporter token, registry paths the builder one."""

    def __init__(self, base_url: str, registry_token: str, export_token: str):
        self.base = require_loopback(base_url).rstrip("/")
        self.tokens = {"/v1/export/": export_token, "/v1/registry/": registry_token}

    def __call__(self, path: str, params: dict) -> dict:
        tok = next(t for p, t in self.tokens.items() if path.startswith(p))
        q = urllib.parse.urlencode({k: v for k, v in params.items() if v is not None})
        req = urllib.request.Request(f"{self.base}{path}?{q}", headers={"Authorization": f"Bearer {tok}"})
        try:
            with urllib.request.urlopen(req, timeout=15) as r:  # noqa: S310 (loopback enforced)
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            raise RuntimeError(f"agent-core HTTP {e.code} on {path.split('?')[0].rsplit('/', 1)[0]}") from None
        except OSError as e:
            raise RuntimeError(redact(f"agent-core unreachable: {type(e).__name__}", list(self.tokens.values()))) from None


class JsonlSink:
    """Append-only; `record_key` is the dedupe identity, so a restart never repeats a line."""

    def __init__(self, path: Path):
        self.path = Path(path)

    def _keys(self) -> set[str]:
        if not self.path.exists():
            return set()
        return {json.loads(x)["record_key"] for x in self.path.read_text(encoding="utf-8").splitlines() if x.strip()}

    def send(self, rec: dict) -> None:
        if rec["record_key"] in self._keys():
            return
        with self.path.open("a", encoding="utf-8") as f:
            f.write(json.dumps(rec, sort_keys=True, ensure_ascii=False) + "\n")


def load_records(path: Path) -> list[dict]:
    p = Path(path)
    if not p.exists():
        return []
    return [json.loads(x) for x in p.read_text(encoding="utf-8").splitlines() if x.strip()]


# ---------------------------------------------------------------------------------------------- reader

def _now_iso() -> str:
    return fmt_ts(datetime.now(timezone.utc))


def record_key(pid: str, state: str, decision_at: str | None, reason: str | None) -> str:
    raw = json.dumps([pid, state, decision_at, reason], separators=(",", ":"))
    return "sha256:" + hashlib.sha256(raw.encode()).hexdigest()


class DecisionReader:
    def __init__(self, *, fetch: Callable[[str, dict], dict], sink, state_path: Path, origin: str = "auto_detect",
                 created_by: str | None = None, expire_days: int = EXPIRE_DAYS, page_limit: int = 200,
                 clock: Callable[[], str] = _now_iso):
        self.fetch, self.sink, self.state_path = fetch, sink, Path(state_path)
        self.origin, self.created_by, self.expire_days = origin, created_by, expire_days
        self.page_limit, self.clock = page_limit, clock
        self.state = {"registry_after": 0, "ledger": {}, "meta": {}, "ignored": [], "emitted": []}
        if self.state_path.exists():
            self.state.update(json.loads(self.state_path.read_text(encoding="utf-8")))

    def _save(self, st: dict) -> None:
        st["emitted"] = st["emitted"][-SEEN_CAP:]
        tmp = self.state_path.with_suffix(".tmp")
        tmp.write_text(json.dumps(st, sort_keys=True), encoding="utf-8")
        os.replace(tmp, self.state_path)

    def _meta(self, pid: str) -> dict | None:
        d = self.fetch(f"/v1/registry/proposals/{pid}", {})
        p = d["proposal"]
        if p.get("origin") != self.origin or (self.created_by and p.get("created_by") != self.created_by):
            return None
        changes = [c for c in d.get("changes", []) if c.get("kind") not in ("eval_suite", "release_settings")] \
            or d.get("changes", [])
        c0 = changes[0] if changes else {}
        docs = c0.get("docs") or {}
        key = finding_key(docs.get("rationale", ""), docs.get("changelog", ""), p.get("title", ""))
        eid = (c0.get("content") or {}).get("id")
        return {"agent_id": p.get("agent_id"), "kind": c0.get("kind"), "target": f"{c0['kind']}:{eid}" if c0 else None,
                "n_changes": len(changes), "finding_key": key, "family": family_of(key), "rev": p.get("rev")}

    def poll_once(self) -> int:
        st = json.loads(json.dumps(self.state))  # work on a copy: the cursor and ledger commit together
        after, touched = st["registry_after"], set()
        while True:
            page = self.fetch("/v1/export/registry-events", {"after": after, "limit": self.page_limit})
            for e in page["items"]:
                pid = e.get("proposal_id")
                if pid and e.get("origin") == self.origin and e.get("type") in LEDGER_EVENT_TYPES:
                    entry = [e["type"], e["at"]]
                    if e["type"] == "rejected":
                        code, source, n = normalize_reason(e)
                        entry += [code, source, n]
                    led = st["ledger"].setdefault(pid, [])
                    if not any(x[0] == entry[0] and x[1] == entry[1] for x in led):
                        led.append(entry)
                        led.sort(key=lambda x: x[1])
                        del led[:-LEDGER_CAP]
                        touched.add(pid)
            nxt = page["next_after"]
            if nxt == after or not page["items"]:
                break
            after = nxt
        st["registry_after"] = max(after, st["registry_after"])
        now, sent, seen = self.clock(), 0, set(st["emitted"])
        for pid, led in st["ledger"].items():
            if pid in st["ignored"]:
                continue
            if pid in touched or pid not in st["meta"]:
                meta = self._meta(pid)
                if meta is None:
                    st["ignored"].append(pid)
                    continue
                st["meta"][pid] = meta
            f, m = fold_events(led, now, self.expire_days), st["meta"][pid]
            reason = {"code": f["reason_code"] or "unknown", "source": f["reason_source"] or "none",
                      "note_len": f["reason_note_len"]} if f["state"] == "rejected" else None
            key = record_key(pid, f["state"], f["decision_at"], reason["code"] if reason else None)
            if key in seen:
                continue
            rec = {"schema": SCHEMA, "record_key": key, "proposal_id": pid, "agent_id": m["agent_id"],
                   "origin": self.origin, "finding_key": m["finding_key"], "family": m["family"],
                   "target": m["target"], "kind": m["kind"], "n_changes": m["n_changes"], "state": f["state"],
                   "created_at": f["created_at"], "decision_at": f["decision_at"],
                   "time_to_decision_secs": f["time_to_decision_secs"],
                   "reason": reason or {"code": "unknown", "source": "none", "note_len": 0}, "observed_at": now}
            self.sink.send(rec)
            seen.add(key)
            st["emitted"].append(key)
            sent += 1
        self.state = st
        self._save(st)
        return sent


# ---------------------------------------------------------------------------------------------- CLI

def _tok(args, which: str) -> str:
    if args.token_file:
        return json.loads(Path(args.token_file).read_text(encoding="utf-8"))[getattr(args, f"{which}_token_key")]
    return os.environ.get(getattr(args, f"{which}_token_env"), "")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("cmd", choices=["poll", "metrics", "suppress", "history"])
    ap.add_argument("--core-url", default="http://127.0.0.1:8001")
    ap.add_argument("--out", help="decisions JSONL sink (poll)")
    ap.add_argument("--in", dest="inp", help="decisions JSONL to read (metrics/suppress/history)")
    ap.add_argument("--state", default="decisions-state.json")
    ap.add_argument("--origin", default="auto_detect")
    ap.add_argument("--created-by")
    ap.add_argument("--expire-days", type=int, default=EXPIRE_DAYS)
    ap.add_argument("--registry-token-env", default="AGENTCORE_BUILDER_TOKEN")
    ap.add_argument("--export-token-env", default="AGENTCORE_EXPORT_TOKEN")
    ap.add_argument("--token-file", help="JSON file with local dev tokens; never printed")
    ap.add_argument("--registry-token-key", default="builder")
    ap.add_argument("--export-token-key", default="admin")
    ap.add_argument("--once", action="store_true")
    ap.add_argument("--interval-secs", type=int, default=60)
    ap.add_argument("--target")
    ap.add_argument("--family")
    ap.add_argument("--cooldown-days", type=int, default=30)
    ap.add_argument("--allow-mapped", action="store_true")
    ap.add_argument("--lang", default="es", choices=["es", "pt", "en"])
    ap.add_argument("--now", default=None)
    a = ap.parse_args(argv)
    toks = []
    try:
        if a.cmd == "poll":
            if not a.out:
                print("error: --out is required for poll", file=sys.stderr)
                return 2
            toks = [_tok(a, "registry"), _tok(a, "export")]
            rd = DecisionReader(fetch=HttpFetch(a.core_url, *toks), sink=JsonlSink(Path(a.out)), state_path=Path(a.state),
                                origin=a.origin, created_by=a.created_by, expire_days=a.expire_days)
            while True:
                print(f"emitted={rd.poll_once()} registry_after={rd.state['registry_after']}")
                if a.once:
                    break
                time.sleep(a.interval_secs)
            return 0
        recs = load_records(Path(a.inp)) if a.inp else []
        now = a.now or _now_iso()
        if a.cmd == "metrics":
            print(json.dumps(aggregate(recs), indent=2, sort_keys=True))
        elif a.cmd == "suppress":
            print(json.dumps(suppress(recs, a.target, a.family, now, a.cooldown_days, a.allow_mapped), sort_keys=True))
        else:
            h = history(recs, a.target, a.family)
            print(json.dumps({"history": h, "line": history_line(h, a.lang)}, ensure_ascii=False, sort_keys=True))
    except (RuntimeError, SinkError, ValueError, KeyError) as e:
        print(f"error: {redact(str(e), toks)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
