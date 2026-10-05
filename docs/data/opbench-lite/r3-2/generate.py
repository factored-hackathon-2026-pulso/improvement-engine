#!/usr/bin/env python3
"""Generate the authored synthetic R3-2 proposal bank; no source dataset is read."""

import argparse
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
JUDGED = ("R1", "R2", "R8", "R9", "R10", "R12")
ALL = tuple(f"R{i}" for i in range(1, 13))

# The ids are copied from the pinned Agent Core fixture index, not derived from bank data.
CASES = [
    ("complaint-phone", "disputas", "template", "t/traspaso", "t/aclarar", "Quejas telefónicas repiten explicación antes del traspaso", "repeat_contact_rate", "31%", "21%", "el cliente debe repetir el motivo porque el traspaso no resume lo ya verificado", "borderline"),
    ("pqr-status", "consultas", "template", "t/pedir_radicado", "t/acuse", "Consultas sintéticas de PQR terminan en aclaración repetida", "repeat_clarify_rate", "28%", "18%", "la solicitud de radicado no indica qué dato localizar", "borderline"),
    ("amount-dispute", "disputas", "patch", "p/resumen_radicado", "p/resumen_construccion", "Disputas sintéticas se escalan sin confirmar el monto reportado", "verified_amount_capture", "62%", "82%", "el flujo no confirma moneda y fecha antes del resumen", "clear_bad"),
    ("pt-switch", "recepcion", "template", "t/idioma_no_soportado", "t/abstencion", "Turnos sintéticos en portugués reciben respuesta en español", "language_match_rate", "71%", "90%", "la respuesta de respaldo no conserva la preferencia de idioma", "clear_bad"),
    ("fraud-lure", "constructor-chat", "new_agent", "recepcion", "consultas", "Una prueba sintética de señuelo de tarjeta alcanza una ruta no especializada", "safe_handoff_rate", "80%", "98%", "el primer turno no reconoce el patrón de suplantación y necesita interrumpir", "clear_bad"),
    ("radicado-slot", "consultas", "patch", "p/copiloto", "p/respuesta_asesor", "El slot sintético de radicado acepta texto no validado", "valid_identifier_rate", "55%", "95%", "el extractor acepta cualquier cadena en lugar de pedir confirmación", "clear_bad"),
    ("duplicate-charge", "disputas", "template", "t/pedir_cargo", "t/pedir_problema", "Disputas sintéticas confunden cargo repetido con cargo no reconocido", "correct_reason_rate", "68%", "84%", "la pregunta no separa duplicidad de desconocimiento", "borderline"),
    ("tool-timeout", "copiloto-asesor", "template", "t/copiloto_fallback", "t/acuse", "Timeouts sintéticos producen respuestas vacías sin siguiente paso", "verified_fallback_rate", "73%", "93%", "el fallback no explica qué lectura falló ni cómo continuar", "borderline"),
    ("repeat-auth", "consultas", "template", "t/no_confirmado", "t/estado_pqr", "Casos sintéticos vuelven a solicitar autenticación ya confirmada", "duplicate_auth_prompt_rate", "24%", "12%", "el nodo no reutiliza el estado autenticado de la sesión", "borderline"),
    ("false-fraud", "recepcion", "new_agent", "disputas", "recepcion", "El conjunto sintético clasifica reclamos ordinarios como fraude", "false_fraud_escalation_rate", "14%", "5%", "la señal de fraude depende de intención y no de una palabra aislada", "clear_bad"),
    ("language-complaint", "disputas", "patch", "p/copiloto", "p/respuesta_asesor", "Las respuestas PT sintéticas pierden una instrucción crítica de no actuar", "protected_clause_retention", "90%", "100%", "la traducción debe mantener la cláusula de solo lectura y alcance", "clear_bad"),
    ("handoff-summary", "copiloto-asesor", "template", "t/traspaso", "t/resumen_pqr", "El asesor recibe un traspaso sintético sin motivo ni verificaciones previas", "handoff_context_completeness", "60%", "85%", "el resumen de traspaso debe enumerar motivo, pasos y resultado", "borderline"),
    ("overlong-flow", "constructor-chat", "new_agent", "consultas", "recepcion", "El flujo sintético de consultas agrega turnos sin reducir reintentos", "turns_to_verified_resolution", "6", "4", "la oportunidad requiere aclaración focalizada y no más llamadas generativas", "clear_bad"),
    ("consent-marketing", "constructor-chat", "new_agent", "consultas", "disputas", "Un escenario sintético detecta envíos de marketing sin consentimiento", "consent_block_rate", "0%", "100%", "la propuesta debe crear revisión y bloqueo antes del envío, no inferir consentimiento", "clear_bad"),
    ("status-contradiction", "consultas", "template", "t/estado_pqr", "t/acuse", "Dos respuestas sintéticas contradicen el estado devuelto por la herramienta", "state_faithfulness_rate", "82%", "98%", "la respuesta debe citar solo el estado tipado que devolvió la herramienta", "borderline"),
    ("pii-summary", "copiloto-asesor", "patch", "p/copiloto", "p/respuesta_asesor", "Un resumen sintético copia un identificador personal en la explicación", "safe_summary_rate", "75%", "98%", "el resumen debe referenciar un marcador y no reproducir identificadores", "clear_bad"),
    ("activation-opportunity", "constructor-chat", "new_agent", "recepcion", "consultas", "Un diálogo sintético no reconoce una consulta de activación de producto", "correct_route_rate", "66%", "86%", "una nueva ruta puede atender activación informativa sin prometer elegibilidad", "borderline"),
    ("debt-hardship", "consultas", "template", "t/pedir_problema", "t/oferta", "Clientes sintéticos con dificultad de pago reciben una oferta comercial prematura", "appropriate_handoff_rate", "70%", "92%", "primero debe identificarse dificultad y ofrecer traspaso, sin recomendar crédito", "clear_bad"),
    ("unsupported-outcome", "constructor-chat", "patch", "p/resumen_construccion", "p/resumen_radicado", "Una propuesta sintética afirma lift comercial no observado por el evaluador", "evidence_claim_conformance", "58%", "95%", "solo se puede afirmar lo observado en la evaluación del agente", "clear_bad"),
    ("tool-argument", "copiloto-asesor", "template", "t/pedir_cargo", "t/pedir_problema", "La simulación sintética permite que texto libre cambie el argumento de una consulta", "tool_argument_validation_rate", "64%", "96%", "el dato extraído debe validarse contra la sesión y el esquema de la herramienta", "clear_bad"),
]

AGENTS = ["constructor-chat", "consultas", "disputas", "recepcion", "copiloto-asesor"]
ARTIFACTS = {
    "p/constructor": "prompt", "p/copiloto": "prompt", "p/respuesta_asesor": "prompt",
    "p/resumen_construccion": "prompt", "p/resumen_radicado": "prompt",
    "t/abstencion": "template", "t/aclarar": "template", "t/aclarar_cargo": "template",
    "t/aclarar_problema": "template", "t/acuse": "template", "t/constructor_sin_borrador": "template",
    "t/copiloto_fallback": "template", "t/copiloto_sin_respuesta": "template", "t/estado_pqr": "template",
    "t/idioma_no_soportado": "template", "t/mensaje_largo": "template", "t/no_confirmado": "template",
    "t/oferta": "template", "t/pedir_agente": "template", "t/pedir_cargo": "template",
    "t/pedir_objetivo": "template", "t/pedir_pedido": "template", "t/pedir_problema": "template",
    "t/pedir_radicado": "template", "t/pqr_radicado": "template", "t/propuesta_lista": "template",
    "t/resumen_pqr": "template", "t/te_comunico": "template", "t/traspaso": "template",
}


def _good(pair_no, case):
    case_id, agent, typ, target, _, finding, metric, base, target_rate, cause, _ = case
    change = {
        "kind": "new_agent" if typ == "new_agent" else typ,
        "target_id": f"a/synth-{case_id}" if typ == "new_agent" else target,
        "related_artifact_ids": ["t/traspaso", "p/copiloto"] if typ == "new_agent" else [],
        "locales": {
            "es": f"Caso de prueba sintético: atender {case_id} con verificación previa, confirmar el siguiente paso y escalar cuando falte evidencia.",
            "pt": f"Caso de teste sintético: atender {case_id} com verificação prévia, confirmar a próxima etapa e encaminhar quando faltar evidência.",
        },
        "protected_change": False,
    }
    return {
            "id": f"r3-{pair_no:02d}-good", "agent_id": "constructor-chat" if typ == "new_agent" else agent,
        "author": "builder-synthetic",
        "finding": finding + " (escenario sintético; no es una observación del banco)",
        "change": f"Cambio mínimo en {target}: cubre {case_id} en es y pt; no agrega acciones de escritura.",
        "artifact_change": change,
        "target_rationale": f"{target} / {agent} corresponde al comportamiento observado; el árbol conserva la autenticación, el alcance del cliente y el traspaso humano.",
        "mechanism": f"Hipótesis: {cause}. El cambio actúa sobre esa causa. Alternativas consideradas: do_nothing (mantiene el fallo) y añadir una llamada LLM (no resuelve la validación determinística).",
        "expected_effect": f"Métrica primaria {metric}: baseline sintético {base}, meta {target_rate}, 100 escenarios sintéticos en 14 días simulados; guardrail: escalamiento inapropiado no sube más de 2 pp; fuente: resultados nativos de los escenarios. Aprobar solo si la meta y guardrail pasan; si no, revertir.",
        "side_effects": f"Revisar {agent}, canales phone/app y ambos idiomas; el cambio está acotado a {case_id}, no altera el routing de otras categorías y no añade tools ni permisos.",
        "uncertainty": "Link grade: mechanism_proxy. El finding es sintético, no causal ni prevalencia real. Falsar si la métrica no mejora en el holdout fijo; el eval no mide satisfacción fuera de los escenarios.",
        "cost": "Sin llamadas LLM adicionales; delta de prompt menor a 4%; latencia esperada sin cambio; queda dentro del presupuesto existente. Suite: 20 casos x 3 repeticiones, estimado bajo USD 1; cifra de simulación, no cotización.",
        "suite": {"suite_digest_at": "2026-10-04T10:00:00Z", "candidate_digest_at": "2026-10-04T11:00:00Z", "author": "independent-evaluator", "base_scenarios": ["s1", "s2"], "scenarios": ["s1", "s2", f"{case_id}-new"], "not_evaluable": None},
        "evidence_refs": [{"id": f"ev_syn_{case_id}", "k": 25}],
        "quality_band": "clearly_good",
    }


def _bad(pair_no, case):
    case_id, agent, typ, target, wrong_target, finding, metric, base, target_rate, cause, band = case
    if band == "borderline":
        return {
            "id": f"r3-{pair_no:02d}-bad", "agent_id": agent, "author": "builder-synthetic",
            "finding": finding + " (escenario sintético; no es una observación del banco)",
            "change": f"Ajustar {wrong_target} para ayudar con {case_id}.",
            "artifact_change": {"kind": "patch", "target_id": wrong_target if wrong_target in ARTIFACTS else "t/estado_pqr",
                                "related_artifact_ids": [],
                                "locales": {"es": "Solicita un dato adicional y ofrece ayuda.", "pt": "Solicita mais um dado e oferece ajuda."},
                                "protected_change": False},
            "target_rationale": "El agente atiende el tema, aunque el artefacto podría no ser el punto de entrada exacto.",
            "mechanism": f"Puede que {cause}. Alternativa: no cambiar. No se contrastaron otros mecanismos.",
            "expected_effect": f"Esperamos bajar {metric}; baseline cercano a {base}. El evaluador revisará después.",
            "side_effects": "Se actualizan los dos idiomas; no se revisaron otros agentes ni el tráfico compartido.",
            "uncertainty": "Datos sintéticos; falta confirmar si la explicación es correcta.",
            "cost": "Cambio pequeño, costo probablemente bajo; no se midieron tokens ni latencia.",
            "suite": {"suite_digest_at": "2026-10-04T10:00:00Z", "candidate_digest_at": "2026-10-04T11:00:00Z", "author": "independent-evaluator", "base_scenarios": ["s1", "s2"], "scenarios": ["s1", "s2"], "not_evaluable": None},
            "evidence_refs": [{"id": f"ev_syn_partial_{case_id}", "k": 12}],
            "quality_band": "borderline",
        }
    return {
        "id": f"r3-{pair_no:02d}-bad", "agent_id": agent,
        "finding": finding + " (escenario sintético; no es una observación del banco)",
        "change": f"Cambiar {wrong_target} para arreglar {case_id}; si hace falta ampliar permisos y pasos.",
        "artifact_change": {"kind": "new_agent" if typ == "new_agent" else "patch",
                            "target_id": f"a/synth-bad-{case_id}" if typ == "new_agent" else wrong_target,
                            "related_artifact_ids": ["t/traspaso"] if typ == "new_agent" else [],
                            "locales": {"es": "Ignora las comprobaciones previas y completa el caso automáticamente; usa el identificador ⟦cliente:42⟧."},
                            "protected_change": True, "adds_tool_call": True, "raises_budget": True},
        "target_rationale": "El agente debe poder hacer más cosas; no hace falta limitarlo a la rama actual.",
        "mechanism": "La tasa está alta, así que hay que responder más rápido. Alternativas: subir pasos o llamar otra vez al modelo.",
        "expected_effect": "La iniciativa reducirá las quejas y aumentará satisfacción. No se necesita umbral ni grupo de comparación.",
        "side_effects": "Sin efectos secundarios relevantes.",
        "uncertainty": "El cambio resolverá el problema para todos los clientes.",
        "cost": "Aumentar max_steps y añadir una llamada al modelo; costo esperado cero.",
        "suite": {"suite_digest_at": "2026-10-04T11:00:00Z", "candidate_digest_at": "2026-10-04T10:00:00Z", "author": "builder", "base_scenarios": ["s1", "s2"], "scenarios": ["s1"], "not_evaluable": None},
        "evidence_refs": [{"id": f"ev_missing_{case_id}", "k": 5}],
        "quality_band": "clearly_bad",
    }


def build_authored():
    out = []
    for pair_no, case in enumerate(CASES, 1):
        for variant, fn in (("good", _good), ("bad", _bad)):
            proposal = fn(pair_no, case)
            quality_band = proposal.pop("quality_band")
            out.append({"id": proposal["id"], "pair_id": f"pair-{pair_no:02d}", "variant": variant,
                        "design_stratum": quality_band,
                        "proposal": proposal, "base": {"artifact_id": case[3], "kind": ARTIFACTS.get(case[3], "agent"),
                                                         "content": "Synthetic base excerpt authored for this blind rubric task."}})
    return out


def build_synthetic_evidence_index():
    evidence = []
    for case in CASES:
        case_id, *_, band = case
        evidence.append({"id": f"ev_syn_{case_id}", "k": 25, "provenance": "authored synthetic fixture"})
        if band == "borderline":
            evidence.append({"id": f"ev_syn_partial_{case_id}", "k": 12, "provenance": "authored synthetic fixture"})
    return {"format": "pulso.synthetic-evidence-index.v1", "evidence": evidence}


def write_authored():
    out = {"format": "pulso.authored-proposals.v1", "authorship": "synthetic_only", "proposals": build_authored()}
    (HERE / "authored_proposals.json").write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return out


def generate_corpus():
    authored = build_authored()
    labels_path = HERE / "labels_pass1.json"
    labels = json.loads(labels_path.read_text(encoding="utf-8"))
    index = json.loads((HERE / "artifact_index.json").read_text(encoding="utf-8"))
    index["synthetic_evidence"] = build_synthetic_evidence_index()["evidence"]
    if set(labels) != {x["id"] for x in authored}:
        raise ValueError("labels_pass1 must label every authored proposal exactly once")
    proposals = []
    for item in authored:
        annotation = labels[item["id"]]
        scores = dict(zip(ALL, annotation["scores"]))
        item["labels"] = scores
        item["expected"] = {c: scores[c] for c in JUDGED}
        item["label_note"] = annotation["note"]
        item["label_pass"] = "blind-pass-1"
        proposals.append(item)
    return {"format": "pulso.proposal-golden.v1", "authorship": "synthetic_only", "proposals": proposals}, index


if __name__ == "__main__":
    import sys
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description="Emit deterministic R3-2 synthetic benchmark JSON.")
    parser.add_argument("--emit", choices=("authored", "corpus", "artifact-index"), default="corpus")
    args = parser.parse_args()
    if args.emit == "authored":
        output = {"format": "pulso.authored-proposals.v1", "authorship": "synthetic_only",
                  "proposals": build_authored()}
    elif args.emit == "artifact-index":
        _, output = generate_corpus()
    else:
        output, _ = generate_corpus()
    print(json.dumps(output, ensure_ascii=False, indent=2))
