"""Replay of the first LIVE synthetic window (answers written by fresh-context subagents, labels agent_roleplay)."""
import json
from pathlib import Path

from claude_standin import thread01 as T

Q = Path(__file__).resolve().parents[1] / "fixtures" / "thread01_live_queue"
EXE = "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe"


def test_live_window_fixtures_replay_with_zero_misses_and_pass_g1(tmp_path):
    res = T.run_thread(T.ThreadConfig(workdir=tmp_path, exe=EXE, queue_dir=Q, mode="replay"))
    assert res["replay"]["misses"] == 0 and res["replay"]["calls"] == 6
    assert all(s["status"] != "red" for s in res["steps"]), [(s["n"], s.get("error")) for s in res["steps"]]
    assert T.live_summary(res)["g1_violations"] == []
    roles = {json.loads(p.read_text())["responder"]["role"] for p in (Q / "responses").glob("*.json")}
    assert roles == {"scout", "verifier", "builder"}
    ids = {json.loads(p.read_text())["responder"]["id"] for p in (Q / "responses").glob("*.json")}
    assert len(ids) == 6 and all(i.startswith("resp-") for i in ids)
