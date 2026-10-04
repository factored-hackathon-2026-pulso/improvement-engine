"""ED0F: pyarrow feeder from an E0 parquet package into the ED0L lab.

Real E0 is read only at runtime from `ED0_E0_PATH` (never copied or committed). Only `case_id` and
`query_signature` are read from `datos/copilot_query.parquet`; free text columns are never loaded.
Group = query signature, outcome = recurrence (the same signature asked at least twice in one case).
Case ids leave this module only as salted HMAC keys; the raw signature is a transient in-memory group
that the lab hashes and discards. The salt is the env `ED0_LAB_SALT` or ephemeral, and is never stored.
"""
import hashlib
import hmac
import os
from collections import Counter

WINDOW = "w1"
QUERY_FILE = os.path.join("datos", "copilot_query.parquet")
MIN_SALT = 16


def e0_path() -> str:
    p = os.environ.get("ED0_E0_PATH")
    if not p:
        raise RuntimeError("ED0_E0_PATH is not set (path of the local E0 package, read at runtime only)")
    return p


def lab_salt() -> bytes:
    env = os.environ.get("ED0_LAB_SALT")
    salt = env.encode() if env else os.urandom(32)
    if len(salt) < MIN_SALT:
        raise ValueError("ED0_LAB_SALT must be at least 16 bytes")
    return salt


def _case_key(salt: bytes, case_id: str) -> str:
    return hmac.new(salt, f"case\0{case_id}".encode(), hashlib.sha256).hexdigest()[:16]


def feed(package: str, salt: bytes, window: str = WINDOW):
    """Iterable of (case_key, group, window, outcome), one per (case, signature)."""
    import pyarrow.parquet as pq
    t = pq.read_table(os.path.join(package, QUERY_FILE), columns=["case_id", "query_signature"])
    pairs = Counter(zip(t.column("case_id").to_pylist(), t.column("query_signature").to_pylist()))
    del t
    for (case_id, sig), n in sorted(pairs.items(), key=lambda kv: (str(kv[0][0]), str(kv[0][1]))):
        if case_id is None or sig is None:
            continue
        yield (_case_key(salt, case_id), sig, window, n >= 2)
