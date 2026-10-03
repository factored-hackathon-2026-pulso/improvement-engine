"""First RED of the demo package: findings must DERIVE from data through recorded SQL, never be precomputed."""
from pulso_demo import analysis, dataset


def test_scout_derives_hypotheses_from_sql_over_the_dataset():
    conn = dataset.build()
    scout = analysis.scout(conn)
    assert scout["queries"] and all(q["sql"].lower().startswith("select") and q["rows_digest"] for q in scout["queries"])
    keys = [h["key"] for h in scout["hypotheses"]]
    assert keys[0] == "transfer_limit/otp_verify"
    assert "card_replacement/mobile" in keys  # a plausible decoy the verifier must still test


def test_without_the_latent_mechanism_the_scout_does_not_invent_it():
    scout = analysis.scout(dataset.build(plant=False))
    assert "transfer_limit/otp_verify" not in [h["key"] for h in scout["hypotheses"]]


def test_verifier_supports_the_real_effect_and_refutes_the_confounded_one():
    conn = dataset.build()
    hyps = analysis.scout(conn)["hypotheses"]
    ver = analysis.verify(conn, hyps)
    verdicts = {a["key"]: a["verdict"] for a in ver["assessments"]}
    assert verdicts["transfer_limit/otp_verify"] == "supported"
    assert verdicts["card_replacement/mobile"] == "refuted"
    assert any(a["counterevidence"] for a in ver["assessments"] if a["verdict"] == "refuted")


def test_first_candidate_fails_a_gate_then_automatic_revision_passes_both():
    conn = dataset.build()
    c1 = analysis.first_candidate()
    r1 = analysis.judge(conn, c1)
    assert r1["native_proxy"]["status"] == "pass" and r1["improvement"]["status"] == "fail"
    assert r1["improvement"]["reason_code"] == "guard_breach"
    c2 = analysis.revise(conn, c1, r1)
    assert c2["revision_of"] == c1["id"] and c2["scope"] != c1["scope"]
    r2 = analysis.judge(conn, c2)
    assert r2["improvement"]["status"] == "pass" and r2["improvement"]["lift_lo"] > 0


def test_alternatives_always_include_do_nothing_with_a_measured_cost():
    alts = analysis.alternatives(dataset.build(), analysis.first_candidate())
    kinds = [a["kind"] for a in alts]
    assert "do_nothing" in kinds and next(a for a in alts if a["kind"] == "do_nothing")["expected_abandoned"] > 0
