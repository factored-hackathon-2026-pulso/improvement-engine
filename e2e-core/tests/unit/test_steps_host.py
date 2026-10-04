"""E3b: Python host glue for the Rust step stand-ins (steps_cli) behind the FRZ0 step schema.

Schema-only tests run everywhere. Tests that spawn the Rust CLI skip cleanly unless STEPS_CLI_EXE names an
existing steps_cli binary (e.g. D:/cargo-targets/claude-seams/debug/steps_cli.exe). Parity tests run the same
inputs through the Python reference steps and the Rust steps and require equal results.
"""
import copy
import json
import os
from pathlib import Path

import pytest

from claude_standin import compile_step as cmp
from claude_standin import ed0_lab as lab
from claude_standin import gate_step as gate
from claude_standin import steps_host as H

ROOT = Path(__file__).resolve().parents[3]
FIX = ROOT / "seams" / "crates" / "steps" / "tests" / "fixtures"
EXE = os.environ.get("STEPS_CLI_EXE")
needs_rust = pytest.mark.skipif(not (EXE and os.path.exists(EXE)), reason="STEPS_CLI_EXE not set")
WORLD = cmp.load_world(ROOT / "agent-core-assets" / "worlds" / "seeded-base.world.yaml")
SALT = b"steps-host-test-salt"
VALID_COMPILE_IN = ROOT / "contracts" / "engine-steps" / "samples" / "valid-compile.in.json"


def host(**kw):
    return H.StepsHost(EXE, **kw)


# ---- schema gate (no Rust needed) -------------------------------------------------------------------------------
def test_input_is_validated_against_the_step_schema_before_spawning():
    with pytest.raises(H.StepInputInvalid):
        H.StepsHost("does-not-exist.exe").compile({"step": "compile"})


def test_missing_exe_is_a_config_error_not_a_silent_skip():
    good = json.loads(VALID_COMPILE_IN.read_text("utf-8"))
    with pytest.raises(H.StepsHostConfigError):
        H.StepsHost("does-not-exist.exe").compile(good)
    with pytest.raises(H.StepsHostConfigError):
        H.StepsHost(None).compile(good)


def test_out_schema_violation_is_refused():
    with pytest.raises(H.StepOutputInvalid):
        H.validate_out_or_raise("compile", {"step": "compile"})


# ---- compile parity ----------------------------------------------------------------------------------------------
def _norm_label(out):
    return {**out, "compiler_label": "claude-standin"} if "compiler_label" in out else out


@needs_rust
def test_compile_parity_with_python_reference_on_the_cmp_corpus():
    cases = FIX / "cmp" / "cases"
    h, n = host(), 0
    for p in sorted(cases.glob("*.in.json")):
        stem = p.name[: -len(".in.json")]
        if stem == "raw_trailing_newline_ref":  # documented: Rust follows the schema, Python regex `$` is laxer
            continue
        try:
            doc = json.loads(p.read_text("utf-8"))
        except ValueError:  # not JSON for Python: the raw-text rejection is covered by the cargo parity test
            continue
        try:
            want = cmp.compile_change_spec(doc, copy.deepcopy(WORLD))
        except (ValueError, KeyError, TypeError):
            with pytest.raises(H.StepsHostError):
                h.compile(doc)
            n += 1
            continue
        got, label = h.compile(doc)
        assert label == "claude-standin"
        assert got == _norm_label(want), stem
        n += 1
    assert n >= 25


# ---- gate parity -------------------------------------------------------------------------------------------------
@needs_rust
def test_gate_parity_with_python_reference_on_the_gsi_cases():
    cases = json.loads((FIX / "gsi" / "cases.json").read_text("utf-8"))
    h = host()
    for c in cases:
        env = c["input"]
        try:
            want = gate.gate_verdict(env["gate_in"], env.get("reports") or {}, {"authors": env["world_authors"]})
        except ValueError:  # schema-invalid input: refused by the host before spawning
            with pytest.raises(H.StepInputInvalid):
                h.gate(env["gate_in"], env.get("reports") or {}, env["world_authors"])
            continue
        got, label = h.gate(env["gate_in"], env.get("reports") or {}, env["world_authors"])
        assert label == "claude-standin"
        assert got == want, c["name"]
    assert len(cases) >= 5


# ---- recompute parity --------------------------------------------------------------------------------------------
@needs_rust
def test_recompute_parity_with_ed0l_verify_claim(tmp_path):
    h = host()
    cases = [(f"c{i}", "g", "w1", i < 12) for i in range(40)] + [(f"d{i}", "h", "w1", i < 7) for i in range(25)]
    db = lab.build_lab(tmp_path / "lab.sqlite", cases, SALT, min_cell=0)
    groups = lab.lab_groups(db)
    assert len(groups) == 2
    for row in groups.values():
        want = lab.rate_of(row["numerator"], row["count"])
        for claimed, match in ((want, True), (round(min(1.0, want + 0.05), 2), False)):
            claim = {"hypothesis_id": "h_1", "evidence_ref": row["evidence_ref"], "rate": claimed, "count": row["count"]}
            ref = lab.verify_claim(db, claim, SALT)
            r = h.recompute_claims(db, [claim], SALT, tmp_path / "steps")["h_1"]
            assert r["recomputed"] == want == ref["recomputed"]
            assert r["ok"] is ref["ok"] is match


@needs_rust
def test_recompute_unresolved_ref_is_not_ok(tmp_path):
    db = lab.build_lab(tmp_path / "lab.sqlite", [(f"c{i}", "g", "w1", i < 12) for i in range(40)], SALT)
    claim = {"hypothesis_id": "h_1", "evidence_ref": "ev_" + "0" * 16, "rate": 0.3, "count": 40}
    r = host().recompute_claims(db, [claim], SALT, tmp_path / "steps")["h_1"]
    assert r["ok"] is False and r["recomputed"] is None


@needs_rust
def test_sensor_and_validation_go_through_the_cli(tmp_path):
    from claude_standin import ed0_detect as ed0
    pkg = tmp_path / "snap" / "e0_package"
    ed0.write_synthetic_e0(str(pkg), {"A": "closing_reply_unclear", "B": "followup_wording"})
    runner = os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe")
    if not os.path.exists(runner):
        pytest.skip("sensor runner exe not built")
    det, label = host(runner_exe=runner).detect(str(pkg), arranque=30, min_support=5, run_id="run-test-0001")
    assert label == "claude-standin"
    assert det["admitted_family"] == ed0.FAMILY and det["winner_support"] > 0 and det["discards"]
    assert det["producer"] == "rust_steps_sensor"
