"""Small synthetic bank dataset (no PII, fully generated, deterministic). Nothing here names a 'winning' finding:
the structure is latent in the rows and the scout/verifier/judge must measure it through SQL."""

from __future__ import annotations

import random
import sqlite3

FLOWS = {  # flow -> (steps, base abandon probability)
    "card_replacement": (["identify", "otp_verify", "confirm"], 0.10),
    "address_change": (["identify", "otp_verify", "confirm"], 0.08),
    "transfer_limit": (["identify", "otp_verify", "confirm"], 0.09),
    "statement_request": (["identify", "confirm"], 0.06),
}
DEVICES = ("mobile", "web")
SCHEMA = """create table sessions(session_id integer primary key, flow text, step text, device text, cohort text,
  retries_allowed integer, retry_exhausted integer, extra_needed integer, risky integer, outcome text, week integer)"""


def build(n: int = 8000, seed: int = 20260101, plant: bool = True) -> sqlite3.Connection:
    rnd = random.Random(seed)
    conn = sqlite3.connect(":memory:")
    conn.execute(SCHEMA)
    rows = []
    for sid in range(1, n + 1):
        flow = rnd.choices(list(FLOWS), weights=[3, 2, 3, 2])[0]
        steps, base = FLOWS[flow]
        device = rnd.choices(DEVICES, weights=[6, 4])[0]
        cohort = "holdout" if rnd.random() < 0.3 else "main"
        # latent mechanics (weeks 3-4 had an incident that only hit card_replacement on mobile: a one-off, not a lever)
        week = rnd.randint(1, 8)
        # latent mechanics (hidden from the agents): OTP verification in transfer_limit expires too fast, so users run
        # out of retries; card_replacement looks bad only because it is mostly mobile (device mix, not the flow).
        p_abandon = base + (0.55 if device == "mobile" and flow == "card_replacement" and week in (3, 4) else 0.0)
        step, exhausted, extra, risky = steps[-1], 0, 0, int(rnd.random() < 0.06)
        if plant and flow == "transfer_limit" and rnd.random() < 0.34:
            step, exhausted, extra = "otp_verify", 1, rnd.choices([1, 2, 3, 5], weights=[5, 3, 1, 1])[0]
            p_abandon = 0.78
        elif rnd.random() < p_abandon * 0.5 and "otp_verify" in steps:
            step = "otp_verify"
        elif rnd.random() < p_abandon:
            step = rnd.choice(steps)
        abandoned = rnd.random() < p_abandon
        rows.append((sid, flow, step, device, cohort, 2, exhausted, extra, risky, "abandoned" if abandoned else "completed", week))
    conn.executemany("insert into sessions values (?,?,?,?,?,?,?,?,?,?,?)", rows)
    return conn
