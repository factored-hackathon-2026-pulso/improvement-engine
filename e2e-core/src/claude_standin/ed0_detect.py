"""ED0: local detection on an E0-shaped package by the EXISTING Rust local-sim sensor.

The sensor (crates/runner, binary improvement-engine) does the detection; this module
only (1) builds SYNTHETIC E0-shaped packages for tests, (2) invokes the sensor on any
package path, (3) projects its result to an aggregate-only detection record. No raw E0
row is read, logged or copied; data_origin is always generated_sample.
"""
import json
import os
import subprocess

from .parquet_min import BOOL, INT32, INT64, STRING, TS_MICROS_UTC, write_parquet

FAMILY = "e0_recurring_copilot_query_cases"
_AT = 1_750_000_000_000_000


def write_synthetic_e0(root, labels, cases=60, arranque=30):
    """Synthetic package. Group A (22 Arranque + 20 replay cases) is larger than
    group B (12 + 10); labels maps group -> query_signature string."""
    os.makedirs(os.path.join(root, "datos"), exist_ok=True)
    os.makedirs(os.path.join(root, "contratos"), exist_ok=True)
    with open(os.path.join(root, "contratos", "platform_history.json"), "w") as f:
        f.write('{"fixture":"synthetic"}')
    ids = [f"private-case-{i}" for i in range(1, cases + 1)]
    ts = lambda off, k=0: [_AT + o * 1_000_000 + k for o in range(off)]  # noqa: E731
    d = os.path.join(root, "datos")
    write_parquet(os.path.join(d, "case.parquet"), [
        ("case_id", STRING, ids), ("opened_at", TS_MICROS_UTC, ts(cases)),
        ("channel", STRING, ["chat"] * cases), ("language", STRING, ["es"] * cases),
        ("topic", STRING, ["support"] * cases), ("priority", STRING, ["normal"] * cases)])
    write_parquet(os.path.join(d, "tool_call.parquet"), [
        ("case_id", STRING, ids), ("event_time", TS_MICROS_UTC, ts(cases, 10)),
        ("call_id", STRING, [f"private-call-{i}" for i in range(1, cases + 1)]),
        ("actor_role", STRING, ["tree"] * cases), ("tool_id", STRING, ["status_lookup"] * cases),
        ("permission_level", STRING, ["read"] * cases), ("status", STRING, ["ok"] * cases),
        ("verified", BOOL, [True] * cases), ("state_change", STRING, ["{}"] * cases),
        ("retry_count", INT32, [0] * cases), ("latency_ms", INT64, [20] * cases)])
    q_cases, sigs = [], []
    for grp, arr, rep in (("A", 22, 20), ("B", 12, 10)):
        for i in range(1, arr + 1):
            q_cases.append(ids[i - 1]); sigs.append(labels[grp])
        for i in range(arranque + 1, arranque + rep + 1):
            q_cases.append(ids[i - 1]); sigs.append(labels[grp])
    n = len(q_cases)
    write_parquet(os.path.join(d, "copilot_query.parquet"), [
        ("query_id", STRING, [f"private-query-{i}" for i in range(1, n + 1)]),
        ("case_id", STRING, q_cases), ("event_time", TS_MICROS_UTC, ts(n, 20)),
        ("query_signature", STRING, sigs), ("answered_by", STRING, ["tool:status_lookup"] * n)])


def run_sensor(exe, package, out, cutoff="2025-07-01T00:00:00Z", arranque=30, min_support=5):
    cmd = [exe, "local-sim", "--mode", "local-simulation", "--source", "e0", "--input", package,
           "--output", out, "--tenant-id", "pulso_local", "--observed-cutoff", cutoff,
           "--arranque-cases", str(arranque), "--min-recurring-query-cases", str(min_support)]
    done = subprocess.run(cmd, capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError("sensor failed: " + done.stderr[-300:])  # stderr is sanitized by the sensor
    run_dir = os.path.join(out, os.listdir(out)[0])
    with open(os.path.join(run_dir, "result.json"), "rb") as f:
        return json.load(f)


def project(result):
    """Aggregate-only detection record from the sensor result (no rows, no labels)."""
    sig = result["signal"]
    if sig.get("metric_id") != FAMILY:
        raise ValueError("sensor admitted an unexpected signal family")
    discards = [{"metric_id": s["metric_id"], "numerator": s["numerator"],
                 "minimum_support": s["minimum_support"], "reason": "not_selected_or_below_support"}
                for s in result["signals"] if s["digest"] != sig["digest"]]
    hold = result.get("e0_recurrence_holdout") or {}
    return {
        "admitted_family": sig["metric_id"], "winner": sig["pattern_ref"],
        "winner_support": sig["numerator"], "denominator": sig["denominator"],
        "discards": discards, "holdout_status": hold.get("status"),
        "data_origin": "generated_sample", "providers": ["local"], "producer": "rust_local_sim_sensor",
        "run_id": result["run_id"],
    }


def detect(exe, package, out, **kw):
    return project(run_sensor(exe, package, out, **kw))


def main(argv=None):
    """python -m claude_standin.ed0_detect --exe EXE --input E0_DIR --output NEW_DIR [--arranque N]
    The E0 path is read only by the sensor at runtime; the printed record is aggregate-only."""
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=os.environ.get("ED0_RUNNER_EXE"))
    ap.add_argument("--input", default=os.environ.get("ED0_E0_PATH"))
    ap.add_argument("--output", required=True)
    ap.add_argument("--arranque", type=int, default=200)
    ap.add_argument("--min-support", type=int, default=20)
    a = ap.parse_args(argv)
    print(json.dumps(detect(a.exe, a.input, a.output, arranque=a.arranque, min_support=a.min_support), indent=2))


if __name__ == "__main__":
    main()
