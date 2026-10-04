"""G2: bump-readiness report. Recorded gh responses only; no network."""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import bump_readiness as br  # noqa: E402

G = HERE / "golden"
PIN = "c814c2bad9f154d10c092326558815dca9562be7"
MAIN = "79233c4884f5854b6955f2f982f538e4bd60d281"
DIGEST = "a" * 64
M23 = "5705e58766292680ac1eda54357c73feb92dd8e2"
M24 = "19e7a105f33052a5ff71810bf9b50fb8366442c0"
M28 = "3499f94a653f257d51aab2936598c96171a31cb8"
REPO = "repos/pulso-factored/agent-core"


def rec(name):
    return json.loads((G / name).read_text(encoding="utf-8"))


def make_repo(tmp_path, digest_in_adr=True):
    wire = tmp_path / "core-bridge/wire/agent_core@c814c2b"
    wire.mkdir(parents=True)
    (wire / "MANIFEST.json").write_text('{"sha": "%s"}' % PIN, encoding="utf-8")
    real = br.sha256_file(wire / "MANIFEST.json")
    adr = tmp_path / "core-bridge/docs/adr"
    adr.mkdir(parents=True)
    line = f"**MANIFEST digest: `{real}`**" if digest_in_adr else "no digest here"
    (adr / "0012-agent-core-pin-c814c2b.md").write_text(f"# ADR 0012 c814c2b\n{line}\n", encoding="utf-8")
    facts = tmp_path / "docs/reports/gates"
    facts.mkdir(parents=True)
    prs = {"23": {"state": "MERGED", "merge_sha": M23}, "24": {"state": "MERGED", "merge_sha": M24},
           "28": {"state": "MERGED", "merge_sha": M28}}
    (facts / "facts.json").write_text(json.dumps({
        "collected_at": "2026-10-03",
        "agent_core": {"pin_sha": PIN, "main_sha": MAIN, "main_ahead_of_pin_by": 2, "prs": prs,
                       "contract_version": "1.3.0"}}), encoding="utf-8")
    return tmp_path


def fake_gh(overrides=None):
    table = {
        f"{REPO}/compare/{PIN}...main": rec("compare_pin_main.json"),
        f"{REPO}/pulls/23": rec("pull_23.json"),
        f"{REPO}/pulls/24": rec("pull_24.json"),
        f"{REPO}/pulls/28": rec("pull_28.json"),
        f"{REPO}/compare/{M23}...main": rec("compare_merge_in_main_ahead.json"),
        f"{REPO}/compare/{M24}...main": rec("compare_merge_in_main_ahead.json"),
        f"{REPO}/contents/contracts/VERSION?ref=main": rec("contents_version_main.json"),
    }
    table.update(overrides or {})

    def gh(ep):
        if ep not in table:
            raise br.GhError(f"unrecorded {ep}")
        return table[ep]
    return gh


def test_pin_gate_requires_manifest_digest(tmp_path):
    pin = br.read_pin(make_repo(tmp_path, digest_in_adr=False))
    assert pin.sha == PIN
    assert "manifest digest not recorded" in " ".join(pin.problems)


def test_pin_gate_detects_digest_drift(tmp_path):
    repo = make_repo(tmp_path)
    (repo / "core-bridge/docs/adr/0012-agent-core-pin-c814c2b.md").write_text(
        f"MANIFEST digest: `{DIGEST}`\n", encoding="utf-8")
    assert any("digest mismatch" in p for p in br.read_pin(repo).problems)


def test_online_report_commits_prs_and_version(tmp_path):
    r = br.build_report(make_repo(tmp_path), fake_gh(), sleep=lambda s: None)
    assert r["source"] == "online"
    assert r["platform"] == "n/a" and r["pin"]["sha"] == PIN
    assert r["commits_ahead"] == 2 and r["main_sha"] == MAIN
    assert r["contract_version"] == {"pin": "1.3.0", "main": "1.4.0", "changed": True}
    prs = {p["number"]: p for p in r["prs"]}
    assert prs[24]["on_main"] is True and prs[23]["on_main"] is True
    assert prs[28]["on_main"] is False and prs[28]["state"] == "open"
    assert r["readiness"] == "blocked"
    assert any("PR 28" in b for b in r["blockers"]) and any("contract VERSION" in b for b in r["blockers"])


def test_ready_when_all_prs_in_main_and_version_same(tmp_path):
    gh = fake_gh({
        f"{REPO}/pulls/28": {**rec("pull_24.json"), "number": 28, "merge_commit_sha": M28},
        f"{REPO}/compare/{M28}...main": rec("compare_merge_in_main_ahead.json"),
        f"{REPO}/contents/contracts/VERSION?ref=main": {"encoding": "base64", "content": "MS4zLjAK"},
    })
    r = br.build_report(make_repo(tmp_path), gh, sleep=lambda s: None)
    assert r["readiness"] == "ready", r["blockers"]


def test_gh_failure_falls_back_offline(tmp_path):
    def dead(ep):
        raise br.GhError("network down")
    r = br.build_report(make_repo(tmp_path), dead, sleep=lambda s: None)
    assert r["source"] == "offline-last-known" and r["facts_collected_at"] == "2026-10-03"
    assert r["commits_ahead"] == 2
    assert {p["number"] for p in r["prs"]} == {23, 24, 28}
    assert r["readiness"] == "unknown"  # offline never claims ready


def test_rate_limit_retries_with_backoff():
    calls, sleeps = [], []

    def flaky(ep):
        calls.append(ep)
        if len(calls) < 3:
            raise br.RateLimited("API rate limit exceeded")
        return {"ok": 1}
    assert br.with_retry(flaky, "x", sleep=sleeps.append) == {"ok": 1}
    assert len(sleeps) == 2 and sleeps[1] > sleeps[0]


def test_rate_limit_exhausted_raises_gh_error():
    def always(ep):
        raise br.RateLimited("rate limit")
    with pytest.raises(br.GhError):
        br.with_retry(always, "x", sleep=lambda s: None, attempts=3)


def test_digest_problem_blocks_even_when_everything_else_ok(tmp_path):
    r = br.build_report(make_repo(tmp_path, digest_in_adr=False), fake_gh(), sleep=lambda s: None)
    assert r["readiness"] == "blocked" and any("manifest digest" in b for b in r["blockers"])


def test_markdown_and_cli_offline(tmp_path, capsys):
    repo = make_repo(tmp_path)
    md = br.render_markdown(br.build_report(repo, fake_gh(), sleep=lambda s: None))
    assert "blocked" in md and "PR 28" in md and "1.4.0" in md
    code = br.main(["--repo-root", str(repo), "--offline", "--json"])
    out = json.loads(capsys.readouterr().out)
    assert out["source"] == "offline-last-known" and code == 0
