"""Q1r live window: the thread talks to the roleplay shim over real HTTP; the queue is answered by an EXTERNAL responder.

Here the external responder is a test thread that plays the lane (reads requests/, writes responses/); in the real
window the lane is fresh-context subagents. The thread must run, count calls and rejections, and pass G1 `check()`.
"""
import importlib.util
import json
import os
import sys
import threading
import time
from pathlib import Path

from claude_standin import thread01 as T

ROOT = Path(__file__).resolve().parents[3]
EXE = os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe")


def _er():
    return T._er()


class Lane(threading.Thread):
    """Plays the responder lane: answers each queued request once with the scripted answer (labelled per role)."""

    def __init__(self, queue, corrupt_first=False):
        super().__init__(daemon=True)
        self.q, self.stop, self.corrupt, self.done = Path(queue), False, corrupt_first, set()

    def run(self):
        while not self.stop:
            for p in sorted((self.q / "requests").glob("*.json")):
                req = json.loads(p.read_text(encoding="utf-8"))
                key = req["key"]
                resp = self.q / "responses" / f"{key}.json"
                if resp.exists():
                    continue
                stage = next(s for s, t in T.SYSTEMS.items() if t == req["system_prompt"])
                doc = {"protocol": "roleplay-queue/1", "key": key, "provenance": "agent_roleplay",
                       "quality_claims": "forbidden", "responder": {"id": f"lane-{stage}", "role": stage},
                       "content": T.scripted_responder(stage, req["inputs"])}
                if self.corrupt:
                    doc["confidence"] = 0.9  # invalid: a quality-claiming field; the shim must reject it once
                    self.corrupt = False
                self.done.add(key)
                resp.write_text(json.dumps(doc), encoding="utf-8")
            time.sleep(0.02)


def _live(tmp_path, corrupt=False):
    q = tmp_path / "queue"
    (q / "requests").mkdir(parents=True)
    (q / "responses").mkdir(parents=True)
    lane = Lane(q, corrupt)
    lane.start()
    try:
        return T.run_thread(T.ThreadConfig(workdir=tmp_path / "w", exe=EXE, queue_dir=q, mode="live",
                                           live_hold_s=0.5)), q
    finally:
        lane.stop = True


def test_live_window_runs_over_http_and_passes_g1(tmp_path):
    res, q = _live(tmp_path)
    steps = {(s["n"], s["id"]): s for s in res["steps"]}
    assert all(s["status"] != "red" for s in res["steps"]), [(s["n"], s.get("error")) for s in res["steps"]]
    assert steps[(3, "scout")]["status"] == "agent_roleplay" and steps[(4, "opportunity")]["status"] == "agent_roleplay"
    assert steps[(3, "scout")]["model"] == "agent_roleplay:lane-scout"  # the responder id of the answer file
    assert steps[(3, "verifier")]["model"] == "agent_roleplay:lane-verifier"
    rep = res["report"]
    assert rep["mode"] == "live" and rep["quality_claims"] == "forbidden"
    assert {p["port"]: p["provenance"] for p in rep["ports"]}["llm_gateway"] == "roleplay-shim:live"
    assert _er().check(rep) == [] or not [v for v in _er().check(rep)]
    live = res["live"]
    assert live["responder_calls"] == 5 and live["scanner_rejections"] == 0 and live["rejected_responses"] == 0
    assert live["wall_minutes"] > 0 and (q / "ledger.jsonl").exists()


def test_live_summary_is_aggregate_only_and_carries_g1(tmp_path):
    res, _ = _live(tmp_path)
    sm = T.live_summary(res)
    assert sm["g1_violations"] == [] and sm["mode"] == "live" and sm["quality_claims"] == "forbidden"
    assert sm["live"]["responder_calls"] == 5 and [x["n"] for x in sm["steps"]] == [1, 2, 3, 3, 4, 5, 6, 7, 8, 9, 10]
    assert set(sm["steps"][0]) == {"n", "id", "status", "data_class", "model"}


def test_live_invalid_response_is_rejected_by_the_shim_and_reasked_once(tmp_path):
    res, q = _live(tmp_path, corrupt=True)
    assert all(s["status"] != "red" for s in res["steps"]), [(s["n"], s.get("error")) for s in res["steps"]]
    assert res["live"]["rejected_responses"] == 1 and res["live"]["reasks"] == 1
    assert list((q / "responses").glob("*.rejected.json"))


def test_live_second_invalid_answer_is_not_retried_forever(tmp_path):
    q = tmp_path / "queue"
    (q / "requests").mkdir(parents=True)
    (q / "responses").mkdir(parents=True)

    class Bad(Lane):
        def run(self):
            while not self.stop:
                for p in sorted((q / "requests").glob("*.json")):
                    key = json.loads(p.read_text(encoding="utf-8"))["key"]
                    r = q / "responses" / f"{key}.json"
                    if not r.exists():
                        r.write_text(json.dumps({"protocol": "roleplay-queue/1", "key": key, "provenance": "x"}))
                time.sleep(0.02)
    lane = Bad(q)
    lane.start()
    try:
        res = T.run_thread(T.ThreadConfig(workdir=tmp_path / "w", exe=EXE, queue_dir=q, mode="live", live_hold_s=0.5))
    finally:
        lane.stop = True
    scout = next(s for s in res["steps"] if s["n"] == 3 and s["id"] == "scout")
    assert scout["status"] == "red" and "invalid_output" in scout["error"]
    assert res["live"]["rejected_responses"] == 2 and res["live"]["reasks"] == 1


def test_tool_schema_tells_a_responder_the_lab_query_arguments(tmp_path):
    """A responder that sees only the request must be able to form the lab_query call (metric_id, window_id)."""
    res, q = _live(tmp_path)
    req = json.loads(next((q / "requests").glob("*.json")).read_text(encoding="utf-8"))
    sch = req["inputs"]["tools"][0]["args_schema"]
    assert set(sch["required"]) == {"metric_id", "window_id"}
    assert sch["properties"]["metric_id"]["enum"] == ["recurrence_rate"] and sch["properties"]["window_id"]["enum"] == ["w1"]
