# U14-EQ — calificación pura de oportunidad Frozen E0

## Propósito y límite

U14-EQ transporta un hallazgo E0 sólo hasta una calificación conservadora y
no persistente. Sus únicos inputs son `VerifiedScoutCandidate` de U13-A y el
`FrozenE0VerificationReport` opaco de U14-E. Antes de emitir la salida, el
composer crate-private rehidrata el candidato E0 y recomputa/verifica el
reporte exacto: scope, candidate, proveniencia U13/E0, policy, input,
evidencia, snapshot, status y report commitment deben coincidir.

La salida es `FrozenE0OpportunityQualification`, también opaca, con los únicos
valores permitidos:

- `frozen_provenance_consistent`
- impacto comercial: `not_assessed`
- esfuerzo operativo: `not_assessed`
- siguiente paso: `requires_mechanism_and_evaluation`
- ruta: `none`

No es una oportunidad comercial cuantificada, una afirmación causal, una
propuesta, una recomendación de ruta ni autorización para construir o liberar.
No existe port configurable, LLM/Jev, Core, storage, registry, runtime o
efecto externo.

## Invariantes

- El reporte no puede reutilizarse entre candidatos, tenant/scope, snapshot,
  política o evidencia: se recalcula desde la capability U13-A y debe igualar
  en todas sus bindings.
- La calificación es determinista e idempotente: repetir inputs exactos produce
  el mismo commitment y no escribe estado.
- No hay conversión a U16 `WorkflowBridge`, `ChangeSpec`, proposal, value
  model, evaluation/release o U35; un doctest compile-fail protege la frontera
  de tipos.
- No expone filas, PII, SQL, receipts mutables ni el contenido de proveniencia.

## RED → GREEN

El RED inicial fue el módulo y contratos inexistentes; el GREEN añade la
composición mínima y la prueba E2E real U02 raw JSON → U04-B V2 → U08 → U12-E
→ U13-E → U13-A → U14-E → U14-EQ. La matriz altera por separado candidate,
proveniencia E0, policy, input, evidencia, snapshot y report commitment y
demuestra denegación antes de producir una calificación.

## Fuera de alcance

U16 debe evaluar mecanismo y alternativas por su propia entrada/contrato;
U20/U35 siguen siendo los únicos caminos hacia evaluación y elegibilidad. Esta
slice no sustituye al verificador U14 genérico ni emite su `VerificationReport`.
