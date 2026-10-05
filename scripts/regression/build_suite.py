"""Build a REGRESSION eval_suite from a detected finding (REG1 / plan W1-1).

    python scripts/regression/build_suite.py --finding finding.json --target template:t/estado_pqr --out <dir>

Input: a finding (cell: dims reason x channel x locale, metric, evidence counts) and the target artifact ref the proposer
chose (`template:<id>`, `prompt:<id>`, `new_agent:<donor>`). Output: an agent-core `eval_suite` (JSON, attachable with
scripts/dev-stack/attach_eval_suite.py) plus a sidecar `meta` that tags every case with the finding key and the behaviour it
protects, and the wording `probes` agent-core's native scorer cannot express (it never reads response text).

Rules (checked by the tests): deterministic (same input, byte-identical output, no clock, no randomness); 6-10 finding cases
plus exactly 3 guard cases copied verbatim from `agent-core-assets/eval-suites/pulso-min`; ES and PT in the finding cases;
templated utterances and fake slots only; no digit run of 6 or more, no emails, no urls; the finding evidence must clear the
k-anonymity floor (K_MIN) or no suite is built. A suite is a regression suite only AFTER prove_fails_on_base.py shows a
finding case failing on the base; this module only builds.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
PULSO_MIN = REPO / "agent-core-assets" / "eval-suites" / "pulso-min"
K_MIN = 10  # the repo suppresses cells under k=10 (floor 5 for the sensor); a suite from thinner evidence is refused
MIN_CASES, MAX_CASES = 6, 10

# Guard cases: protected behaviours every candidate must keep (copied, never edited, from pulso-min).
GUARDS = {
    "consultas": ("es-fraude-interrumpe", "es-inyeccion-ignora-instrucciones", "es-falla-herramienta-escala"),
    "disputas": ("es-monto-alto-escala", "es-fraude-interrumpe", "es-cancela-confirmacion"),
}
GUARD_FILES = {"consultas": PULSO_MIN / "consultas" / "consultas-min@1.0.0.yaml",
               "disputas": PULSO_MIN / "disputas" / "disputas-min@1.0.0.yaml"}
GUARD_BEHAVIOUR = {
    "es-fraude-interrumpe": "fraud interrupt (queue fraude, nothing written)",
    "es-inyeccion-ignora-instrucciones": "injection ruleset flags the turn, no tool call",
    "es-falla-herramienta-escala": "tool failure escalates (tool_failure)",
    "es-monto-alto-escala": "amount escalation policy (> 500 USD escalates, nothing written)",
    "es-cancela-confirmacion": "customer says no: cancelled, no write",
}

# Which mechanism a target ref defaults to (the reasoning crate names it in its mapping row).
DEFAULT_MECHANISM = {"template:t/estado_pqr": "status_message_gap", "prompt:p/resumen_radicado": "closing_followup",
                     "new_agent:consultas": "uncovered_topic"}
TARGET_AGENT = {"status_message_gap": "consultas", "closing_followup": "disputas", "uncovered_topic": "recepcion"}

PII_PATTERNS = (re.compile(r"\d{6,}"), re.compile(r"[\w.+-]+@[\w-]+\.[\w.]+"), re.compile(r"https?://", re.I),
                re.compile(r"\b\d{3}[ -]\d{3}[ -]\d{4}\b"))

# status value the seeded read-back returns -> words a state sentence may use to reflect it (lower case, per locale)
STATUS_LABELS = {
    "Open": {"es": ("open", "abiert", "en curso", "radicad", "recibid"),
             "pt": ("open", "abert", "em andamento", "registrad", "recebid")},
    "Resolved": {"es": ("resolved", "resuelt", "solucionad", "cerrad"),
                 "pt": ("resolved", "resolvid", "solucionad", "encerrad")},
    "Rejected": {"es": ("rejected", "rechazad", "no procede"), "pt": ("rejected", "rejeitad", "recusad", "negad")},
    "InReview": {"es": ("inreview", "en revisi", "revisando"), "pt": ("inreview", "em an", "em revis", "analisand")},
}


class SuiteRefused(Exception):
    def __init__(self, code: str, why: str) -> None:
        super().__init__(f"{code}: {why}")
        self.code, self.why = code, why


# ---------------------------------------------------------------------------------------------------------------- findings
def finding_key(f: dict) -> str:
    dims = ",".join(f"{k}={v}" for k, v in sorted((f.get("dims") or {}).items()))
    d, h = f.get("discovery") or {}, f.get("holdout") or {}
    return f"{f['metric']}|{dims}|{d.get('numerator')}/{d.get('denominator')}|{h.get('numerator')}/{h.get('denominator')}"


def finding_hash(f: dict) -> str:
    return hashlib.sha256(finding_key(f).encode()).hexdigest()[:8]


def check_evidence(f: dict) -> list[str]:
    """k-anonymity: every stage must clear K_MIN on its denominator and have at least one positive case."""
    problems = []
    for name in ("discovery", "holdout"):
        st = f.get(name)
        if not st:
            problems.append(f"{name}: missing")
            continue
        if int(st.get("denominator", 0)) < K_MIN:
            problems.append(f"{name}: denominator {st.get('denominator')} < k={K_MIN}")
        if int(st.get("numerator", 0)) < 1:
            problems.append(f"{name}: no positive case")
    return problems


def pii_problems(text: str) -> list[str]:
    return [p.pattern for p in PII_PATTERNS if p.search(text)]


# --------------------------------------------------------------------------------------------------------------- guards
def load_guards(agent: str) -> list[dict]:
    if agent not in GUARDS:
        raise SuiteRefused("no_guards_for_agent", f"pulso-min has no guard cases for {agent}")
    import yaml
    suite = yaml.safe_load(GUARD_FILES[agent].read_text(encoding="utf-8"))
    by_id = {s["id"]: s for s in suite["scenarios"]}
    out = []
    for gid in GUARDS[agent]:
        sc = copy.deepcopy(by_id[gid])
        sc["id"] = "guard-" + gid
        out.append(sc)
    return out


# ------------------------------------------------------------------------------------------------------------ mechanisms
def _principal(h: str, i: int, country: str) -> dict:
    return {"id": f"cust-reg-{h}-{i}", "attrs": {"country": country}}


def _start(lang: str) -> dict:
    return {"op": "start", "auth": "session", **({"lang": lang} if lang != "es" else {})}


def status_message_gap(f: dict, h: str) -> tuple[list[dict], dict, list[dict]]:
    """consultas / t/estado_pqr: the response is one static sentence that says nothing about the PQR state.
    Native part: the read-back is answered and the run closes resolved with a template response (what the patch must not
    break: its placeholder has to resolve in the real engine). Probe part: the rendered sentence must reflect the seeded
    state (agent-core's scorer cannot read wording)."""
    cases, meta, probes = [], {}, []
    statuses = ("Open", "Resolved", "Rejected", "InReview")
    for lang, country, text in (("es", "CO", "quiero saber como va mi reclamo, es la solicitud {tag}"),
                                ("pt", "BR", "quero saber como esta minha solicitacao, e a {tag}")):
        for i, status in enumerate(statuses):
            tag = f"pqr-reg-{i + 1}"
            cid = f"reg{h}-{lang}-estado-{status.lower()}"
            cases.append({
                "id": cid, "principal": _principal(h, i, country),
                "steps": [_start(lang), {"op": "turn", "text": text.format(tag=tag), "auth": "session"}],
                "seed": {"tools": {"obtener_pqr": [{"result": {"status": status, "id": tag}}]}},
                "expect": {"outcome": "resolved", "escalated": False},
                "assertions": [{"event": "engine.response_emitted", "where": [{"field": "kind", "op": "eq", "value": "template"}]},
                               {"event": "engine.response_failed", "expect": "none"}]})
            meta[cid] = {"behaviour": f"the state sentence reflects a {status} PQR ({lang}) and its placeholder renders in the engine",
                         "check": "native+wording_probe"}
            probes.append({"case_id": cid, "kind": "state_reflected", "template_id": "t/estado_pqr", "locale": lang,
                           "values": {"facts.pqr.value.status": status},
                           "must_contain_any": list(STATUS_LABELS[status][lang])})
    return cases, meta, probes


FOLLOWUP_WORDS = {"es": ("seguimiento", "especialista"), "pt": ("acompanhamento", "especialista")}


def closing_followup(f: dict, h: str) -> tuple[list[dict], dict, list[dict]]:
    """disputas / p/resumen_radicado: an unresolved-complaint finding asks that the closing message of a verified dispute say
    who follows the case up. Native part (agent-core scorer): the flow still verifies the write, resolves and emits a response
    without `response_failed`. It deliberately does NOT assert `fallback_used`: agent-core's `evaluate` generates through a
    gateway bound to the LIVE registry, so a candidate prompt is never exercised (REG1 live finding: even a text-identical
    version bump falls back to the template, 0 tokens, no gateway request). Probe part: real model samples of the
    prompt text under test must name the follow-up (`seguimiento`/`especialista`, `acompanhamento`/`especialista`) and carry
    no digit (the prompt forbids numbers); this is the only check that discriminates."""
    cases, meta, probes = [], {}, []
    variants = (("es", "CO", "no reconozco un cargo de {amt} dolares en una tienda en linea"),
                ("es", "CO", "me hicieron un cobro indebido, pague dos veces una compra de {amt} dolares"),
                ("es", "CO", "hay un cargo de {amt} dolares que no hice en una tienda"),
                ("pt", "BR", "nao reconheco uma cobranca de {amt} dolares em uma loja online"),
                ("pt", "BR", "tive uma cobranca indevida, paguei duas vezes uma compra de {amt} dolares"),
                ("pt", "BR", "ha uma cobranca de {amt} dolares que eu nao fiz em uma loja"))
    amounts = (120, 80, 45, 120, 80, 45)
    for i, ((lang, country, text), amt) in enumerate(zip(variants, amounts)):
        cid = f"reg{h}-{lang}-cierre-seguimiento-{i % 3 + 1}"
        tx = f"tx-reg-{i + 1}"
        seed_tx = {"transaction_id": tx, "amount": f"{amt}.00", "currency": "USD", "merchant": "Tienda Aurora"}
        pqr = {"status": "Open", "id": f"pqr-reg-{i + 1}"}
        cases.append({
            "id": cid, "principal": _principal(h, i, country),
            "steps": [_start(lang), {"op": "turn", "text": text.format(amt=amt), "auth": "session"},
                      {"op": "confirm", "answer": "yes", "auth": "step_up"}],
            "seed": {"tools": {"buscar_transacciones": [{"result": [seed_tx]}], "seleccionar": [{"result": seed_tx}],
                               "convertir_moneda": [{"result": amt}], "radicar_pqr": [{"result": pqr}],
                               "obtener_pqr": [{"result": pqr}]}},
            "expect": {"outcome": "resolved", "actions_verified": ["radicar_pqr"], "escalated": False},
            "assertions": [{"event": "engine.response_emitted"},
                           {"event": "engine.response_failed", "expect": "none"}]})
        meta[cid] = {"behaviour": f"the closing message of a verified dispute ({lang}) names who follows the case up "
                                  "(real model samples of the prompt under test)", "check": "native+generated_probe"}
        probes.append({"case_id": cid, "kind": "generated_contains", "prompt_id": "p/resumen_radicado", "locale": lang,
                       "inputs": {"facts.pqr_verificada": {"value": {"status": "Open"}}}, "samples": 3,
                       "must_contain_any": list(FOLLOWUP_WORDS[lang]), "must_not_match": r"\d"})
    return cases, meta, probes


def uncovered_topic(f: dict, h: str) -> tuple[list[dict], dict, list[dict]]:
    """recepcion: a topic of the finding cell with no specialist must end `transferred`, not escalate or stall. Needs the
    candidate to add a new agent to the directory (generator only; not exercised live in REG1)."""
    cases, meta = [], {}
    reason = (f.get("dims") or {}).get("reason_category", "Tecnico")
    topics = {"Tecnico": ("mi aplicacion se cierra sola al abrirla", "meu aplicativo fecha sozinho ao abrir"),
              "Queja": ("quiero presentar una queja por la atencion recibida", "quero fazer uma reclamacao sobre o atendimento"),
              "Comercial": ("quiero cambiar las condiciones de mi plan", "quero mudar as condicoes do meu plano"),
              "Retencion": ("estoy pensando en cancelar mi cuenta", "estou pensando em cancelar minha conta")}
    es_t, pt_t = topics.get(reason, topics["Tecnico"])
    for lang, country, base, sfx in (("es", "CO", es_t, ("desde ayer", "desde esta manana", "otra vez hoy")),
                                     ("pt", "BR", pt_t, ("desde ontem", "desde esta manha", "de novo hoje"))):
        for i, s in enumerate(sfx):
            cid = f"reg{h}-{lang}-ruta-{reason.lower()}-{i + 1}"
            cases.append({
                "id": cid, "principal": _principal(h, i, country),
                "steps": [_start(lang), {"op": "turn", "text": f"{base} {s}", "auth": "session"}],
                "seed": {"tools": {}},
                "expect": {"outcome": "transferred", "escalated": False},
                "assertions": [{"event": "engine.escalated", "expect": "none"}]})
            meta[cid] = {"behaviour": f"a {reason} topic ({lang}) is routed to a specialist instead of staying unresolved",
                         "check": "native"}
    return cases, meta, []


MECHANISMS = {"status_message_gap": status_message_gap, "closing_followup": closing_followup,
              "uncovered_topic": uncovered_topic}


# --------------------------------------------------------------------------------------------------------------- builder
def build_suite(finding: dict, target: str, mechanism: str | None = None, max_finding_cases: int = MAX_CASES) -> dict:
    bad = check_evidence(finding)
    if bad:
        raise SuiteRefused("k_below_minimum", "; ".join(bad))
    mechanism = mechanism or DEFAULT_MECHANISM.get(target)
    if mechanism not in MECHANISMS:
        raise SuiteRefused("no_mechanism", f"no case generator for target {target!r} (known: {sorted(DEFAULT_MECHANISM)})")
    agent = TARGET_AGENT[mechanism]
    h = finding_hash(finding)
    cases, meta, probes = MECHANISMS[mechanism](finding, h)
    # deterministic truncation that keeps both locales: interleave es/pt, then cut
    es = [c for c in cases if "-es-" in c["id"]]
    pt = [c for c in cases if "-pt-" in c["id"]]
    cases = ([c for pair in zip(es, pt) for c in pair] + es[len(pt):] + pt[len(es):])[:max_finding_cases]
    keep = {c["id"] for c in cases}
    meta = {k: v for k, v in meta.items() if k in keep}
    probes = [p for p in probes if p["case_id"] in keep]
    if not (MIN_CASES <= len(cases) <= MAX_CASES):
        raise SuiteRefused("case_count", f"{len(cases)} finding cases, need {MIN_CASES}-{MAX_CASES}")
    leaks = pii_problems(json.dumps(cases, ensure_ascii=False))
    if leaks:
        raise SuiteRefused("pii_pattern", f"generated text matches {leaks}")
    key = finding_key(finding)
    for m in meta.values():
        m.update({"kind": "finding", "finding_key": key, "mechanism": mechanism})
    guards = load_guards(agent)
    for g in guards:
        meta[g["id"]] = {"kind": "guard", "finding_key": key, "mechanism": mechanism, "check": "native", "source": "pulso-min",
                         "behaviour": GUARD_BEHAVIOUR[g["id"][len("guard-"):]]}
    suite = {"id": f"reg-{agent}-{h}", "version": "1.0.0", "agent_id": agent, "repetitions": 3, "thresholds": {},
             "scenarios": cases + guards}
    return {"finding_key": key, "finding_id": finding.get("finding_id"), "target": target, "mechanism": mechanism,
            "agent": agent, "suite": suite, "meta": meta, "probes": probes,
            "finding_case_ids": [c["id"] for c in cases], "guard_case_ids": [g["id"] for g in guards]}


def dumps(obj: dict) -> str:
    return json.dumps(obj, ensure_ascii=False, indent=2, sort_keys=True)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--finding", type=Path, required=True)
    ap.add_argument("--target", required=True)
    ap.add_argument("--mechanism")
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    finding = json.loads(a.finding.read_text(encoding="utf-8"))
    try:
        bundle = build_suite(finding, a.target, a.mechanism)
    except SuiteRefused as e:
        print(json.dumps({"refused": e.code, "why": e.why}, ensure_ascii=False))
        return 2
    a.out.mkdir(parents=True, exist_ok=True)
    sid = bundle["suite"]["id"]
    (a.out / f"{sid}.suite.json").write_text(dumps(bundle["suite"]), encoding="utf-8")
    (a.out / f"{sid}.bundle.json").write_text(dumps(bundle), encoding="utf-8")
    print(json.dumps({"suite": sid, "cases": len(bundle["suite"]["scenarios"]), "finding_cases": len(bundle["finding_case_ids"]),
                      "guards": len(bundle["guard_case_ids"]), "out": str(a.out)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
