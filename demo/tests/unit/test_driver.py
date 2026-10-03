import json

from pulso_demo import analysis, dataset, driver


def test_inline_sql_binds_parameters_and_is_executable_on_the_dataset():
    conn = dataset.build()
    ver = analysis.verify(conn, analysis.scout(conn)["hypotheses"])
    q = ver["queries"][0]
    sql = driver.inline_sql(q)
    assert "?" not in sql and conn.execute(sql).fetchall() == [tuple(r) for r in q["rows"]]


def test_scripted_model_answers_derive_from_analysis_not_constants():
    conn = dataset.build()
    sc = analysis.scout(conn)
    ver = analysis.verify(conn, sc["hypotheses"])
    ref = {"id": "r", "digest": "d", "media_type": "application/json"}
    outs = driver.model_outputs(sc, ver, analysis.alternatives(conn, analysis.first_candidate()), ref)
    h = outs[driver.RESEARCH]["hypotheses"]
    assert h[0]["statement"].startswith("transfer_limit otp_verify")
    verdicts = {a["hypothesis_id"]: a["verdict"] for a in outs[driver.VERIFY]["assessments"]}
    assert verdicts["h1"] == "supported" and "refuted" in verdicts.values()
    assert {a["kind"] for a in outs[driver.DESIGN]["alternatives"]} == {"do_nothing", "proposed_change"}
    flat = dataset.build(plant=False)
    sc2 = analysis.scout(flat)
    assert driver.model_outputs(sc2, analysis.verify(flat, sc2["hypotheses"]), [], ref)[driver.RESEARCH]["hypotheses"][:1] != h[:1]


def test_offline_run_writes_honest_report_and_console_world(tmp_path):
    assert driver.main(["--out", str(tmp_path), "--offline"]) == 0
    rep = json.loads((tmp_path / "demo-report.json").read_text("utf-8"))
    ids = {d["id"] for d in rep["doubles"]}
    assert {"stand_in_engine", "scripted_llm", "fixture_api", "offline_core", "human_decision"} <= ids
    st = {s["n"]: s["status"] for s in rep["steps"]}
    assert st[8] == "hook_pending" and st[9] == "not_run" and st[7] == "shown"
    assert rep["mode"] == "offline" and rep["claims"].startswith("Codex stand-in")
    world = json.loads((tmp_path / "world.json").read_text("utf-8"))
    assert world["gates"]["combined"]["decision"] == "needs_human"
    assert all(n["status"] == "planned" for n in json.loads((tmp_path / "replay.json").read_text("utf-8"))["runs"]["run-demo"]["nodes"])
