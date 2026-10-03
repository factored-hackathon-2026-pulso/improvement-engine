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


def run_offline(tmp_path, *extra, hook=None):
    assert driver.main(["--out", str(tmp_path), "--offline", *extra], hook=hook) == 0
    rep = json.loads((tmp_path / "demo-report.json").read_text("utf-8"))
    return rep, json.loads((tmp_path / "world.json").read_text("utf-8"))


def nodes(world):
    return {n["node_id"]: n for n in world["runs"]["run-demo"]["nodes"]}


def test_offline_run_writes_honest_report_and_console_world(tmp_path):
    rep, world = run_offline(tmp_path)
    ids = {d["id"] for d in rep["doubles"]}
    assert {"stand_in_engine", "scripted_llm", "fixture_api", "offline_core", "human_decision", "offline_human"} <= ids
    st = {s["n"]: (s["status"], s["outcome"]) for s in rep["steps"]}
    assert all(v[0] in ("real", "stand-in", "simulated") for v in st.values()) and len(st) == 10
    assert st[8] == ("simulated", "shown") and st[9] == ("simulated", "shown") and st[7] == ("stand-in", "shown")
    assert rep["mode"] == "offline" and rep["claims"].startswith("Codex stand-in")
    assert world["gates"]["combined"]["decision"] == "needs_human"
    assert all(n["status"] == "planned" for n in json.loads((tmp_path / "replay.json").read_text("utf-8"))["runs"]["run-demo"]["nodes"])


def test_scripted_human_is_reported_as_simulated_and_staging_is_confirmed_but_prod_is_not_exposed(tmp_path):
    rep, world = run_offline(tmp_path)
    assert rep["human"]["simulated_human"] is True and "SIMULATED" in rep["human"]["label"] and rep["human"]["stage"] == "staging_confirmed"
    assert rep["human"]["promoted"] is False and rep["human"]["prod_alias"] != rep["human"]["staging_alias"]
    assert [e["event"] for e in rep["decision_trail"]] == ["decision_requested", "human_decided", "approved", "published", "staging_confirmed"]
    n = nodes(world)
    assert (n["approve"]["status"], n["publish"]["status"], n["release"]["status"]) == ("complete", "complete", "waiting_dependency")
    assert any("offline" in d["what"].lower() for d in rep["doubles"] if d["id"] == "human_decision")
    assert "canary" not in json.dumps(world).lower()


def test_explicit_promote_flag_is_the_only_way_prod_moves(tmp_path):
    rep, world = run_offline(tmp_path, "--promote")
    assert rep["human"]["stage"] == "promoted" and rep["human"]["prod_alias"] == rep["human"]["staging_alias"]
    assert nodes(world)["release"]["status"] == "complete"


def test_step_ten_second_batch_contradicts_memory_and_starts_a_successor(tmp_path):
    rep, world = run_offline(tmp_path)
    assert rep["successor"]["successor_investigation"].startswith("address_change")
    assert any(u["status"] == "contradicted" and u["key"] == "transfer_limit/otp_verify" for u in rep["successor"]["memory_updates"])
    assert world["runs"]["run-demo-successor"]["state"] == "running"
    assert {m["status"] for m in world["memory"]} >= {"contradicted", "published"}
    assert {s["n"]: s["status"] for s in rep["steps"]}[10] == "simulated"  # offline: nothing real was exported


def test_pending_hook_keeps_the_human_step_pending_and_never_publishes(tmp_path):
    from pulso_demo.decision_hook import PendingHook
    rep, world = run_offline(tmp_path, hook=PendingHook())
    st = {s["n"]: (s["status"], s["outcome"]) for s in rep["steps"]}
    assert st[8] == ("stand-in", "pending") and st[9][1] == "not_run" and st[10][1] == "not_run"
    assert nodes(world)["decision"]["reason_code"] == "human_decision_pending" and rep["successor"] is None


def test_manual_mode_pauses_until_the_cli_approves_and_times_out_into_pending(tmp_path):
    import threading
    import time
    from pulso_demo import decide, human_flow
    rc = []
    t = threading.Thread(target=lambda: rc.append(driver.main(["--out", str(tmp_path), "--offline", "--human-mode", "manual", "--human-timeout", "30"])))
    t.start()
    for _ in range(200):
        if (tmp_path / human_flow.REQUEST_FILE).exists():
            break
        time.sleep(0.05)
    interim = json.loads((tmp_path / "world.json").read_text("utf-8"))  # the console world shows the wait while the person decides
    assert nodes(interim)["decision"]["status"] == "waiting_dependency" and nodes(interim)["approve"]["status"] == "planned"
    assert decide.main(["--out", str(tmp_path), "approve"]) == 0
    t.join(60)
    rep = json.loads((tmp_path / "demo-report.json").read_text("utf-8"))
    assert rc == [0] and rep["human"]["mode"] == "manual" and rep["human"]["simulated_human"] is False and rep["human"]["stage"] == "staging_confirmed"
    tmp2 = tmp_path / "t2"
    assert driver.main(["--out", str(tmp2), "--offline", "--human-mode", "manual", "--human-timeout", "0.2"]) == 0  # nobody decides: stays pending
    rep2 = json.loads((tmp2 / "demo-report.json").read_text("utf-8"))
    assert rep2["human"]["stage"] == "requested" and rep2["successor"] is None


# ---- adversarial review: pipeline output must follow the data, failures must stay visible (never a crash, never a fake pass)
def run_offline_any(tmp_path, monkeypatch=None, **kw):
    rc = driver.main(["--out", str(tmp_path), "--offline"])
    return rc, json.loads((tmp_path / "demo-report.json").read_text("utf-8")), json.loads((tmp_path / "world.json").read_text("utf-8"))


def test_dataset_without_the_mechanism_never_reaches_the_human(tmp_path, monkeypatch):
    real = dataset.build
    monkeypatch.setattr(dataset, "build", lambda *a, **k: real(*a, **{**k, "plant": False}))
    rc, rep, world = run_offline_any(tmp_path)
    st = {s["n"]: s["outcome"] for s in rep["steps"]}
    assert rc == 0 and rep["outcome"] == "no_candidate_passed_gates"  # the unplanted flow still shows a (weaker) real effect; candidate 1 fails, no revision clears
    assert st[7] == "failed" and st[8] == "pending" and st[9] == "not_run" and st[10] == "not_run" and rep["human"] is None
    assert world["gates"]["combined"]["decision"] == "revise"
    assert "transfer_limit/otp_verify" not in json.dumps(world["investigation"])


def test_no_supported_hypothesis_means_no_candidate_and_a_hold(tmp_path, monkeypatch):
    monkeypatch.setattr(analysis, "scout", lambda conn: {"queries": [analysis.run_query(conn, "q-overall", "select count(*) from sessions")], "hypotheses": []})
    rc, rep, world = run_offline_any(tmp_path)
    assert rc == 0 and rep["outcome"] == "no_opportunity" and rep["attempts"] == [] and rep["human"] is None
    assert {s["n"]: s["outcome"] for s in rep["steps"]}[4] != "shown" and world["gates"]["combined"]["decision"] == "hold"


def test_revision_is_only_triggered_by_a_failing_gate(tmp_path, monkeypatch):
    monkeypatch.setattr(analysis, "GUARD_MAX_EXPOSURE", 1.0)  # candidate 1 now clears the guard on the real data
    rc, rep, world = run_offline_any(tmp_path)
    assert len(rep["attempts"]) == 1 and {s["n"]: s["outcome"] for s in rep["steps"]}[7] == "not_triggered"
    assert rep["attempts"][0]["improvement"]["status"] == "pass"


def test_exhausted_revision_is_reported_failed_and_never_reaches_the_human(tmp_path, monkeypatch):
    monkeypatch.setattr(analysis, "revise", lambda *a, **k: (_ for _ in ()).throw(RuntimeError("no revision within bounds clears the gates")))
    rc, rep, world = run_offline_any(tmp_path)
    st = {s["n"]: s["outcome"] for s in rep["steps"]}
    assert rep["outcome"] == "no_candidate_passed_gates" and st[7] == "failed" and st[8] == "pending" and rep["human"] is None
    assert len(rep["attempts"]) == 1 and rep["attempts"][0]["improvement"]["status"] == "fail"


def test_moving_the_mechanism_moves_the_finding(tmp_path, monkeypatch):
    real = dataset.build

    def swapped(*a, **k):
        c = real(*a, **k)
        c.execute("update sessions set flow=case flow when 'transfer_limit' then 'address_change' when 'address_change' then 'transfer_limit' else flow end")
        return c
    monkeypatch.setattr(dataset, "build", swapped)
    rc, rep, world = run_offline_any(tmp_path)
    assert world["investigation"]["run-demo"]["hypothesis"].startswith("address_change") and rep["attempts"][-1]["scope"] == "flow:address_change"
