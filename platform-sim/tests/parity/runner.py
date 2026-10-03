"""registry-wire-contract runner. Independent of the Pulso adapter and of agent_core: plain HTTP + normalisation.

A case is data (YAML). Steps are executed against REGISTRY_BASE_URL; every response is reduced to a normalised
structure (method, path template, status, content-type, type, code, body key shape, trace_id presence, a small
whitelist of stable scalars) that is compared against a recorded fixture (see `record.py`)."""

from __future__ import annotations

import copy
import json
import re
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import httpx
import yaml

from registry_mock import jws
from registry_mock.sim_common import AGENT_ID, BASE_RELEASE_ID, PIN_SHA

HERE = Path(__file__).resolve().parent
CASES_DIR = HERE / "cases"
FIXTURES = Path(__file__).resolve().parents[2] / "fixtures" / "agent_core_wire" / PIN_SHA[:7]
WIRE = Path(__file__).resolve().parents[3] / "core-bridge" / "wire" / f"agent_core@{PIN_SHA[:7]}"
GOLDEN_DRAFT = WIRE / "golden" / "golden_draft_disputa.json"

STABLE_SCALARS = {"state", "rev", "verdict", "status", "origin", "decision", "alias", "changed_vs_base", "title",
                  "agent_id", "created_by", "actor", "reason", "base_release_id", "released_by", "published_by"}
CHECKED_HEADERS = ("www-authenticate", "retry-after", "cache-control", "allow", "location")
OPAQUE_KEYS = {"content", "args", "locales", "seed", "scenarios", "steps"}  # user content: not part of the shape
_REL = re.compile(r"rel-[0-9a-f]{16}")
_VAR = re.compile(r"\{([a-z_]+[0-9]*)\}")
ACTORS = ("bot", "human", "admin", "customer", "advisor", "human_session", "approver_only", "bot_aprobador")


@dataclass
class Case:
    id: str
    rule: str
    applies_to: str
    steps: list[dict[str, Any]]
    requires_sim: bool = False
    justification: str = ""
    file: str = ""


def _step(method: str, path: str, **kw: Any) -> dict[str, Any]:
    return {"method": method, "path": path, **kw}


_DRAFT = [
    _step("POST", "/v1/registry/proposals", json={"agent_id": "atencion", "origin": "manual", "title": "golden"},
          save={"pid": "proposal_id"}),
    _step("PUT", "/v1/registry/proposals/{pid}/draft", json={"$golden_draft": {"expected_rev": 0}}),
]
_FROZEN = _DRAFT + [
    _step("POST", "/v1/registry/proposals/{pid}/validate", save={"ch": "candidate_hash"}),
    _step("POST", "/v1/registry/proposals/{pid}/freeze", save={"rel": "release_id_preview"}),
]
_EVALUATED = _FROZEN + [
    _step("POST", "/v1/registry/proposals/{pid}/evaluate", json={"suite_id": "disputas-suite"}),
]
_APPROVED = _EVALUATED + [
    _step("POST", "/v1/registry/proposals/{pid}/approve", json={"candidate_hash": "{ch}"}, **{"as": "human"}),
]
_PUBLISHED = _APPROVED + [
    _step("POST", "/v1/registry/proposals/{pid}/publish", headers={"Idempotency-Key": "k-{pid}"},
          save={"rel2": "release_id"}, **{"as": "human"}),
]
PRESETS: dict[str, list[dict[str, Any]]] = {
    "golden_draft": _DRAFT, "golden_frozen": _FROZEN, "golden_evaluated": _EVALUATED,
    "golden_approved": _APPROVED, "golden_published": _PUBLISHED,
}


def _expand(steps: list[dict[str, Any]]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for st in steps:
        out.extend(PRESETS[st["use"]] if "use" in st else [st])
    return out


def load_cases(directory: Path = CASES_DIR) -> list[Case]:
    cases: list[Case] = []
    for path in sorted(directory.glob("*.yaml")):
        for raw in yaml.safe_load(path.read_text(encoding="utf-8")):
            c = Case(id=raw["id"], rule=raw["rule"], applies_to=raw.get("applies_to", "both"), steps=_expand(raw["steps"]),
                     requires_sim=bool(raw.get("requires_sim", False)), justification=raw.get("justification", ""),
                     file=path.name)
            if c.applies_to not in ("both", "mock_only"):
                raise ValueError(f"{c.id}: applies_to must be both|mock_only")
            if c.applies_to == "mock_only" and not c.justification:
                raise ValueError(f"{c.id}: a mock_only case needs a written justification")
            cases.append(c)
    ids = [c.id for c in cases]
    if len(ids) != len(set(ids)):
        raise ValueError("duplicate case ids")
    return cases


# --- request building --------------------------------------------------------------------------------------

def _token(actor: Any) -> str | None:
    if actor in (None, "none"):
        return None
    if actor in ACTORS:
        return jws.issue(actor)
    head, body, _sig = jws.issue("bot").split(".")
    ok_payload = jws.principal_payload("constructor-bot", "builder", ["constructor"])
    match actor:
        case "bad_signature":
            return f"{head}.{body}.{jws.b64url_encode(bytes(64))}"
        case "wrong_typ":
            return jws.issue("bot", typ="delegation+jws")
        case "unknown_kid":
            return jws.issue("bot", kid="sim-unknown-9")
        case "extra_header_key":
            return jws.sign({"alg": "EdDSA", "kid": jws.SIM_KID, "typ": jws.PRINCIPAL_TYP, "cty": "x"}, ok_payload)
        case "expired":
            return jws.issue("bot", exp=datetime(2020, 1, 1, tzinfo=timezone.utc))
        case "garbage":
            return "not-a-jws"
        case "alg_none":
            return jws.sign({"alg": "none", "kid": jws.SIM_KID, "typ": jws.PRINCIPAL_TYP}, ok_payload)
    raise ValueError(f"unknown actor {actor!r}")


def _subst(value: Any, env: dict[str, Any]) -> Any:
    if isinstance(value, str):
        return _VAR.sub(lambda m: str(env.get(m.group(1), m.group(0))), value)
    if isinstance(value, list):
        return [_subst(v, env) for v in value]
    if isinstance(value, dict):
        return {k: _subst(v, env) for k, v in value.items()}
    return value


def _golden_draft(expected_rev: int = 0) -> dict[str, Any]:
    return {**json.loads(GOLDEN_DRAFT.read_text(encoding="utf-8")), "expected_rev": expected_rev}


def _docs(n: str) -> dict[str, str]:
    return {"description": f"parity {n}", "rationale": "parity", "changelog": "parity"}


def _template(i: int | str, locale_text: str = "hola", version: str = "1.0.0") -> dict[str, Any]:
    return {"kind": "template", "docs": _docs(str(i)),
            "content": {"id": f"t/par{i}", "version": version, "locales": {"es": locale_text}, "reads": []}}


def _canonical_len(content: dict[str, Any]) -> int:
    return len(json.dumps(content, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8"))


def _body(spec: Any, env: dict[str, Any]) -> Any:
    if isinstance(spec, dict) and len(spec) == 1:
        ((key, arg),) = spec.items()
        if key == "$golden_draft":
            return _golden_draft(int(arg.get("expected_rev", 0)))
        if key == "$bulk_templates":
            return {"expected_rev": int(arg.get("expected_rev", 0)),
                    "changes": [_template(i) for i in range(int(arg["count"]))]}
        if key == "$entity_bytes":
            change = _template(0, "")
            base = _canonical_len(change["content"])
            change["content"]["locales"]["es"] = "a" * (int(arg["size"]) - base)
            assert _canonical_len(change["content"]) == int(arg["size"])
            return {"expected_rev": int(arg.get("expected_rev", 0)), "changes": [change]}
        if key == "$flow_nodes":  # golden flow padded to exactly `count` nodes (clones with fresh ids)
            draft = _golden_draft(int(arg.get("expected_rev", 0)))
            flow = next(c for c in draft["changes"] if c["kind"] == "flow")
            nodes = flow["content"]["nodes"]
            first = nodes[0]  # entry `collect` node: padding is a reachable chain of its clones
            pads = [{**copy.deepcopy(first), "id": f"pad{i}"} for i in range(int(arg["count"]) - len(nodes))]
            for i, node in enumerate(pads):
                node["next"]["ok"] = pads[i + 1]["id"] if i + 1 < len(pads) else first["next"]["ok"]
            if pads:
                first["next"]["ok"] = pads[0]["id"]
            flow["content"]["nodes"] = nodes + pads
            return draft
        if key == "$suite_scenarios":  # golden suite padded to exactly `count` scenarios (clones with fresh ids)
            draft = _golden_draft(int(arg.get("expected_rev", 0)))
            suite = next(c for c in draft["changes"] if c["kind"] == "eval_suite")
            scen = suite["content"]["scenarios"]
            suite["content"]["scenarios"] = [{**copy.deepcopy(scen[i % len(scen)]), "id": f"sc{i}"}
                                             for i in range(int(arg["count"]))]
            return draft
        if key == "$title":
            return {"agent_id": AGENT_ID, "origin": arg.get("origin", "manual"), "title": "t" * int(arg["length"])}
        if key == "$one_template":
            return {"expected_rev": int(arg.get("expected_rev", 0)), "changes": [_template(0, version=arg.get("version", "1.0.0"))]}
    return _subst(spec, env)


# --- normalisation -----------------------------------------------------------------------------------------

def _shape(value: Any, depth: int) -> Any:
    if isinstance(value, dict):
        if depth == 0:
            return sorted(value)
        return {k: ("<opaque>" if k in OPAQUE_KEYS else _shape(v, depth - 1)) for k, v in sorted(value.items())}
    if isinstance(value, list):
        if not value:
            return ["[]"]
        shapes = [json.dumps(_shape(v, depth - 1), sort_keys=True) for v in value]
        return ["[]" + s for s in sorted(set(shapes))]
    return "null" if value is None else "scalar"


def _scalars(body: Any, only_rules: list[str] | None = None) -> dict[str, Any]:
    found: dict[str, Any] = {}
    if isinstance(body, dict):
        for k in STABLE_SCALARS:
            if k in body and isinstance(body[k], (str, int, bool)):
                v = body[k]
                # Release ids are hashes of Core's normalised entities (not reproducible by a mock): only the seeded
                # base release is stable; any other `rel-<hex16>` is masked.
                found[k] = "<release_id>" if isinstance(v, str) and _REL.fullmatch(v) and v != BASE_RELEASE_ID else v
        if isinstance(body.get("proposal"), dict):
            for k, v in _scalars(body["proposal"]).items():
                found[f"proposal.{k}"] = v
        if isinstance(body.get("violations"), list):
            rules = {str(v.get("rule")) for v in body["violations"] if isinstance(v, dict)}
            # `only_rules` narrows a limits case to the rule under test (the mock validates a documented subset)
            found["violation_rules"] = sorted(rules & set(only_rules) if only_rules else rules)
            found["violation_paths"] = sorted({f"{v.get('rule')}@{v.get('path')}" for v in body["violations"]
                                               if isinstance(v, dict) and (not only_rules or v.get("rule") in only_rules)})
    return found


def normalise(method: str, path_tpl: str, resp: httpx.Response, only_rules: list[str] | None = None) -> dict[str, Any]:
    ctype = resp.headers.get("content-type", "").split(";")[0].strip()
    try:
        body = resp.json()
    except ValueError:
        body = None
    out: dict[str, Any] = {"method": method, "path": path_tpl, "status": resp.status_code, "content_type": ctype}
    out["headers"] = {h: resp.headers[h] for h in CHECKED_HEADERS if h in resp.headers}
    if isinstance(body, dict):
        out["type"] = body.get("type")
        out["title"] = body.get("title") if "type" in body else None
        out["code"] = body.get("code")
        out["keys"] = sorted(body)
        out["shape"] = _shape(body, 2)
        out["trace_id_present"] = bool(body.get("trace_id")) if "trace_id" in body else None
        out["scalars"] = _scalars(body, only_rules)
    else:
        out["keys"] = None
    return out


# --- execution ---------------------------------------------------------------------------------------------

@dataclass
class CaseResult:
    case: str
    steps: list[dict[str, Any]] = field(default_factory=list)


def _dig(body: Any, dotted: str) -> Any:
    for part in dotted.split("."):
        body = body[int(part)] if isinstance(body, list) else body[part]
    return body


def run_case(client: httpx.Client, case: Case, *, sim: bool, reset: Callable[[], None] | None = None,
             control: Any = None) -> CaseResult:
    """`reset` isolates cases on a target without `/_sim` (the real server): the harness, not the served app, resets.
    `control` (real_pg_scripted) programs the eval/clock doubles in-process (`program_eval`, `advance`); no /_sim."""
    if sim:
        client.post("/_sim/reset").raise_for_status()
    elif reset is not None:
        reset()
    env: dict[str, Any] = {"base_release": BASE_RELEASE_ID, "agent": AGENT_ID}
    result = CaseResult(case.id)
    for step in case.steps:
        if "sim" in step:
            if not sim and control is None:
                raise RuntimeError(f"{case.id}: requires the /_sim channel")
            s = step["sim"]
            if "eval" in s:
                if control is not None:
                    control.program_eval(s["eval"])
                else:
                    client.post("/_sim/eval", json={"script": s["eval"]}).raise_for_status()
            if "advance" in s:
                if control is not None:
                    control.advance(float(s["advance"]))
                else:
                    client.post("/_sim/clock/advance", json={"seconds": s["advance"]}).raise_for_status()
            if "method" not in step:
                continue
        for i in range(int(step.get("repeat", 1))):
            env["i"] = i
            env["n"] = i + 1
            path = _subst(step["path"], env)
            headers: dict[str, str] = {k: (v["$repeat"][0] * int(v["$repeat"][1]) if isinstance(v, dict) else str(_subst(v, env)))
                                       for k, v in (step.get("headers") or {}).items()}
            token = _token(step.get("as", "bot"))
            if token is not None:
                headers["Authorization"] = f"Bearer {token}"
            kwargs: dict[str, Any] = {"headers": headers}
            if "json" in step:
                kwargs["json"] = _body(step["json"], env)
            resp = client.request(step["method"], path, **kwargs)
            result.steps.append(normalise(step["method"], step["path"], resp, step.get("only_rules")))
            for name, dotted in (step.get("save") or {}).items():
                try:
                    env[name] = _dig(resp.json(), dotted)
                except (KeyError, IndexError, ValueError, TypeError):
                    env[name] = "UNSAVED"
    return result


def fixtures_dir(target: str = "a2") -> Path:
    """a2 (and mock-only) fixtures live in the pin dir; real_local recordings in `<pin>/real/`."""
    return {"real": FIXTURES / "real", "real_scripted": FIXTURES / "real_pg_scripted"}.get(target, FIXTURES)


def fixture_path(case_id: str) -> Path:
    return FIXTURES / f"{case_id}.json"


def dump_fixture(result: CaseResult) -> str:
    return json.dumps({"case": result.case, "steps": result.steps}, indent=2, sort_keys=True) + "\n"


def diff_steps(expected: list[dict[str, Any]], actual: list[dict[str, Any]]) -> list[str]:
    problems: list[str] = []
    if len(expected) != len(actual):
        problems.append(f"step count {len(actual)} != {len(expected)}")
    for i, (e, a) in enumerate(zip(expected, actual)):
        for key in sorted(set(e) | set(a)):
            if e.get(key) != a.get(key):
                problems.append(f"step {i} {e.get('method')} {e.get('path')}: {key}: got {a.get(key)!r} expected {e.get(key)!r}")
    return problems
