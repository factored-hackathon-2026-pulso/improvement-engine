"""Tests for pin_watch.py: fake `gh` output (golden JSON), a local fixture git repo, and the exit-code contract."""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import pin_watch as pw  # noqa: E402

GOLDEN = HERE / "golden"
PIN = "789d6c89b2fca90fc10e2abf157da51dc81c5d51"
NEW = "ccccccc3333333333333333333333333333333cc"
LLM = "9" * 40


def golden(name: str):
    return json.loads((GOLDEN / name).read_text(encoding="utf-8"))


class FakeGh:
    """Replays golden JSON keyed by endpoint substring (first match wins); records calls."""

    def __init__(self, routes: dict[str, object]):
        self.routes, self.calls = routes, []

    def __call__(self, endpoint: str):
        self.calls.append(endpoint)
        for key, val in self.routes.items():
            if key in endpoint:
                return val
        raise pw.GhError(f"no fixture for {endpoint}")


# ---------------------------------------------------------------- pin reading
def make_repo(tmp: Path, sha: str = PIN, wire: bool = True, pin_json: str | None = None) -> Path:
    if wire:
        d = tmp / "core-bridge" / "wire" / f"agent_core@{sha[:7]}"
        d.mkdir(parents=True)
        (d / "MANIFEST.json").write_text(json.dumps({"sha": sha, "contract_version": "1.3.0", "files": []}))
    if pin_json:
        p = tmp / "contracts" / "agent_core"
        p.mkdir(parents=True)
        (p / "pin.json").write_text(pin_json)
    return tmp


def test_read_pin_from_manifest_is_sha_agnostic(tmp_path):
    repo = make_repo(tmp_path, "a" * 40)
    pin = pw.read_pin(repo)
    assert pin.sha == "a" * 40 and pin.source.endswith("MANIFEST.json") and pin.contract_version == "1.3.0"


def test_read_pin_prefers_pin_json(tmp_path):
    repo = make_repo(tmp_path, "a" * 40, pin_json=json.dumps({"sha": "b" * 40, "contract_version": "1.3.0"}))
    assert pw.read_pin(repo).sha == "b" * 40


def test_read_pin_multiple_manifests_picks_newest_and_warns(tmp_path):
    repo = make_repo(tmp_path, "a" * 40)
    d2 = repo / "core-bridge" / "wire" / ("agent_core@" + "c" * 7)
    d2.mkdir()
    (d2 / "MANIFEST.json").write_text(json.dumps({"sha": "c" * 40, "contract_version": "1.3.0"}))
    os.utime(d2 / "MANIFEST.json", (4102444800, 4102444800))
    pin = pw.read_pin(repo)
    assert pin.sha == "c" * 40 and pin.warnings


def test_read_pin_missing_raises(tmp_path):
    with pytest.raises(pw.PinWatchError):
        pw.read_pin(tmp_path)


# ---------------------------------------------------------------- analysis
def test_classify_areas():
    c = pw.classify_path
    assert c("contracts/VERSION") == "contracts"
    assert c("agent_core/registry/x.py") == "registry"
    assert c("agent_core/composition/serve.py") == "composition"
    assert c("agent_core/api/routes.py") == "api"
    assert c("agent_core/identity/k.py") == "identity"
    assert c("agent_core/cli.py") == "cli"
    assert c("Dockerfile") == "dockerfile"
    assert c("migrations/0007.sql") == "migrations"
    assert c("docs/adr/1.md") == "docs"
    assert c("tests/t.py") == "tests"
    assert c("zzz/other.py") == "other"


def test_analyze_compare_signals_and_grouping():
    a = pw.analyze_files(golden("compare.json")["files"])
    kinds = {s["kind"] for s in a["signals"]}
    assert {"contracts_version", "openapi_changed", "schema_changed", "signature_change", "migration_added",
            "problem_code_change", "new_env_var", "new_route"} <= kinds
    assert a["areas"]["contracts"] == 3 and a["areas"]["migrations"] == 1 and a["areas"]["docs"] == 1
    assert any(s["kind"] == "new_env_var" and s["detail"] == "AGENTCORE_NEW_FLAG" for s in a["signals"])
    assert any(s["kind"] == "new_route" and "/v1/things" in s["detail"] for s in a["signals"])
    sig = {s["file"] for s in a["signals"] if s["kind"] == "signature_change"}
    assert "agent_core/composition/serve.py" in sig


def test_docs_only_has_no_signals():
    a = pw.analyze_files([{"filename": "docs/x.md", "status": "modified", "patch": "+x"},
                          {"filename": "tests/t.py", "status": "added", "patch": "+x"}])
    assert a["signals"] == []


# ---------------------------------------------------------------- verdict
def test_verdict_ladder():
    v = pw.decide_verdict
    assert v(changed=False, signals=[], checks={}) == "no-change"
    assert v(changed=True, signals=[], checks={"compat": "pass", "wire_diff": "clean"}) == "additive-safe"
    assert v(changed=True, signals=[{"kind": "new_route"}],
             checks={"compat": "pass", "wire_diff": "clean"}) == "needs-bump-work"
    assert v(changed=True, signals=[], checks={"compat": "pass", "wire_diff": "drift"}) == "needs-bump-work"
    assert v(changed=True, signals=[{"kind": "contracts_version"}], checks={}) == "breaking"
    assert v(changed=True, signals=[], checks={"compat": "fail"}) == "breaking"
    assert v(changed=True, signals=[], checks={"gen_wire": "fail"}) == "breaking"
    assert v(changed=True, signals=[{"kind": "schema_changed"}], checks={}) == "needs-bump-work"
    assert v(changed=True, signals=[], checks={}) == "additive-safe"


def test_exit_codes():
    assert [pw.EXIT[v] for v in ("no-change", "additive-safe", "needs-bump-work", "breaking")] == [0, 10, 20, 30]


def test_route_table_and_wire_diff(tmp_path):
    a, b = tmp_path / "a", tmp_path / "b"
    for d, extra in ((a, {}), (b, {"/new": {"get": {}}})):
        d.mkdir()
        (d / "openapi.json").write_text(json.dumps({"paths": {"/x": {"get": {}, "post": {}}, **extra}}))
        (d / "schemas").mkdir()
        (d / "schemas" / "S.json").write_text("{}" if not extra else '{"a":1}')
        (d / "MANIFEST.json").write_text(json.dumps({"sha": "1" if not extra else "2", "files": []}))
    (b / "golden").mkdir()
    (b / "golden" / "hash_vectors.json").write_text("[]")
    r = pw.diff_wire(a, b)
    assert r["drift"] is True
    assert r["routes_added"] == ["GET /new"] and r["routes_removed"] == []
    assert "schemas/S.json" in r["changed"]["schemas"] and "golden/hash_vectors.json" in r["added"]["golden"]
    assert "MANIFEST.json" not in sum(r["changed"].values(), [])
    assert pw.diff_wire(a, a)["drift"] is False


# ---------------------------------------------------------------- orchestration with fake gh
def routes():
    return {
        "agent-core/commits/main": {"sha": NEW},
        "agent-core/compare/" + PIN: golden("compare.json"),
        "agent-core/pulls?state=open": golden("pulls.json"),
        "agent-core/pulls/31/files": golden("pull_files.json"),
        "llm-gateway/commits/main": {"sha": LLM},
    }


def test_run_watch_report_with_fake_gh(tmp_path):
    repo = make_repo(tmp_path / "repo")
    gh = FakeGh(routes())
    rep = pw.run_watch(repo, gh, run_checks=False, state_path=None, scratch_root=tmp_path / "s")
    assert rep["pin"]["sha"] == PIN and rep["agent_core"]["main_sha"] == NEW
    assert rep["agent_core"]["changed"] is True and rep["verdict"] == "breaking"
    assert len(rep["agent_core"]["commits"]) == 3
    assert rep["agent_core"]["open_prs"][0]["number"] == 31
    assert rep["agent_core"]["open_prs"][0]["signals"][0]["kind"] == "new_route"
    assert "core-bridge/src/pulso_core_runtime/compat.py" in rep["our_files_likely_to_change"]
    assert rep["checks"] == {}
    assert all(c.startswith("repos/") for c in gh.calls)


def test_run_watch_no_change(tmp_path):
    repo = make_repo(tmp_path / "repo")
    gh = FakeGh({"agent-core/commits/main": {"sha": PIN}, "pulls?state=open": [],
                 "llm-gateway/commits/main": {"sha": LLM}})
    rep = pw.run_watch(repo, gh, run_checks=False, state_path=None, scratch_root=tmp_path / "s")
    assert rep["verdict"] == "no-change" and rep["agent_core"]["commits"] == []


def test_since_last_state_roundtrip(tmp_path):
    repo = make_repo(tmp_path / "repo")
    st = tmp_path / "state.json"
    r1 = pw.run_watch(repo, FakeGh(routes()), run_checks=False, state_path=st, since_last=True,
                      scratch_root=tmp_path / "s")
    assert r1["verdict"] == "breaking" and r1["since_last"]["skipped"] is False
    saved = json.loads(st.read_text())
    assert saved["agent_core_main"] == NEW and saved["llm_gateway_main"] == LLM
    assert saved["last_verdict"] == "breaking" and saved["pin_sha"] == PIN
    r2 = pw.run_watch(repo, FakeGh(routes()), run_checks=False, state_path=st, since_last=True,
                      scratch_root=tmp_path / "s")
    assert r2["since_last"]["skipped"] is True and r2["verdict"] == "no-change"
    assert r2["since_last"]["previous_verdict"] == "breaking"
    moved = {**routes(), "agent-core/commits/main": {"sha": "d" * 40}}
    moved = {"agent-core/commits/main": {"sha": "d" * 40}, **{k: v for k, v in routes().items()
                                                              if k != "agent-core/commits/main"}}
    r3 = pw.run_watch(repo, FakeGh(moved), run_checks=False, state_path=st, since_last=True,
                      scratch_root=tmp_path / "s")
    assert r3["since_last"]["skipped"] is False


def test_llm_gateway_baseline_and_change(tmp_path):
    repo = make_repo(tmp_path / "repo")
    st = tmp_path / "state.json"
    st.write_text(json.dumps({"llm_gateway_main": "1" * 40}))
    cmp_ = {"commits": [{"sha": "2" * 40, "commit": {"message": "x", "author": {"name": "a"}}}],
            "files": [{"filename": "openapi.json", "status": "modified", "patch": "+x"}]}
    gh = FakeGh({"agent-core/commits/main": {"sha": PIN}, "pulls?state=open": [],
                 "llm-gateway/commits/main": {"sha": "2" * 40}, "llm-gateway/compare/" + "1" * 40: cmp_})
    rep = pw.run_watch(repo, gh, run_checks=False, state_path=st, scratch_root=tmp_path / "s")
    lg = rep["llm_gateway"]
    assert lg["changed"] is True and lg["baseline"] == "1" * 40 and lg["files"] == ["openapi.json"]
    assert rep["verdict"] == "needs-bump-work"


def test_llm_gateway_first_run_records_baseline_only(tmp_path):
    repo = make_repo(tmp_path / "repo")
    gh = FakeGh({"agent-core/commits/main": {"sha": PIN}, "pulls?state=open": [],
                 "llm-gateway/commits/main": {"sha": LLM}})
    rep = pw.run_watch(repo, gh, run_checks=False, state_path=tmp_path / "st.json", scratch_root=tmp_path / "s")
    assert rep["llm_gateway"]["changed"] is False and rep["llm_gateway"]["baseline"] is None
    assert rep["verdict"] == "no-change"


def test_markdown_summary(tmp_path):
    repo = make_repo(tmp_path / "repo")
    rep = pw.run_watch(repo, FakeGh(routes()), run_checks=False, state_path=None, scratch_root=tmp_path / "s")
    md = pw.render_markdown(rep)
    assert "# pin-watch" in md and "breaking" in md and "contracts_version" in md and "#31" in md


# ---------------------------------------------------------------- local fixture git repo + checks
def git(cwd, *a):
    env = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t",
           "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
    return subprocess.run(["git", "-C", str(cwd), *a], check=True, capture_output=True, text=True,
                          env=env).stdout.strip()


@pytest.fixture()
def upstream(tmp_path):
    up = tmp_path / "upstream"
    (up / "agent_core").mkdir(parents=True)
    (up / "agent_core" / "__init__.py").write_text("VALUE = 1\n")
    git(up, "init", "-q", "-b", "main")
    git(up, "add", "-A")
    git(up, "commit", "-qm", "base")
    base = git(up, "rev-parse", "HEAD")
    (up / "agent_core" / "__init__.py").write_text("VALUE = 2\n")
    git(up, "commit", "-qam", "bump")
    return up, base, git(up, "rev-parse", "HEAD")


def test_scratch_checkout_never_touches_source(upstream, tmp_path):
    up, base, head = upstream
    before = git(up, "status", "--porcelain")
    co = pw.make_scratch_checkout(str(up), head, tmp_path / "scratch")
    assert (co / "agent_core" / "__init__.py").read_text() == "VALUE = 2\n"
    assert git(co, "rev-parse", "HEAD") == head
    assert git(up, "status", "--porcelain") == before and git(up, "rev-parse", "HEAD") == head
    assert pw.make_scratch_checkout(str(up), base, tmp_path / "scratch") == co
    assert git(co, "rev-parse", "HEAD") == base


def test_run_compat_uses_scratch_checkout(upstream, tmp_path):
    up, base, head = upstream
    co = pw.make_scratch_checkout(str(up), head, tmp_path / "scratch")
    ok = pw.run_compat(co, python=sys.executable, repo_root=tmp_path,
                       code="import agent_core; assert agent_core.VALUE == 2")
    assert ok["status"] == "pass"
    bad = pw.run_compat(co, python=sys.executable, repo_root=tmp_path, code="raise SystemExit(2)")
    assert bad["status"] == "fail"


def test_shadow_repo_pins_new_sha(tmp_path):
    repo = tmp_path / "repo"
    (repo / "core-bridge" / "scripts").mkdir(parents=True)
    (repo / "core-bridge" / "scripts" / "gen_wire.py").write_text("# gen")
    (repo / "platform-sim").mkdir()
    (repo / "platform-sim" / "m.py").write_text("x=1")
    sh = pw.make_shadow_repo(repo, tmp_path / "shadow", NEW, "1.3.0")
    assert (sh / "core-bridge" / "scripts" / "gen_wire.py").exists() and (sh / "platform-sim" / "m.py").exists()
    assert json.loads((sh / "contracts" / "agent_core" / "pin.json").read_text())["sha"] == NEW


def test_cli_end_to_end_json_and_exit_code(tmp_path):
    repo = make_repo(tmp_path / "repo")
    fx = tmp_path / "fx"
    fx.mkdir()
    data = {"repos_pulso-factored_agent-core_commits_main": {"sha": PIN},
            "repos_pulso-factored_agent-core_pulls_state=open&per_page=100": [],
            "repos_pulso-factored_llm-gateway_commits_main": {"sha": LLM}}
    for name, d in data.items():
        (fx / (name.replace("?", "_") + ".json")).write_text(json.dumps(d))
    out = tmp_path / "out"
    p = subprocess.run([sys.executable, str(HERE.parent / "pin_watch.py"), "--repo-root", str(repo), "--json",
                        "--gh-fixtures", str(fx), "--no-checks", "--out-dir", str(out),
                        "--state-file", str(tmp_path / "st.json")], capture_output=True, text=True)
    assert p.returncode == 0, p.stderr
    rep = json.loads(p.stdout)
    assert rep["verdict"] == "no-change"
    assert json.loads((out / "pin-watch-report.json").read_text())["verdict"] == "no-change"
    assert (out / "pin-watch-report.md").exists()


def test_cli_error_exit_code(tmp_path):
    p = subprocess.run([sys.executable, str(HERE.parent / "pin_watch.py"), "--repo-root", str(tmp_path), "--json",
                        "--no-checks"], capture_output=True, text=True)
    assert p.returncode == 1 and "pin" in p.stderr.lower()
