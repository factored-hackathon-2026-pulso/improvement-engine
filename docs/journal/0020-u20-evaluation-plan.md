# U20 — plan de evaluación sellado

## Alcance del corte

U20 congela el contrato de comparación **antes** de que exista o se ejecute un
candidato. Recibe únicamente el `WorkflowBridgeContract` de U16, una selección
de baseline por digest, un `OracleSpec`, una métrica y digests de suite de
desarrollo/final. Produce `EvaluationPlan` inmutable con un commitment
determinista que cubre todos esos valores.

El corte no ejecuta un brazo de `SandboxPort`, no crea artefactos Agent Core,
no materializa casos, no publica artifacts y no autoriza release. Es una
frontera de preparación: la ejecución comparada será U19/U27 y el predicado
de elegibilidad final pertenece a U35.

## Gates fail-closed

- Sólo acepta `LinkGrade::MechanismProxy` y una alternativa de ruta existente;
  `Unlinked` y `NotEvaluable` no se convierten en plan.
- Oracle debe tener el tenant, outcome y digest de snapshot exactos del bridge.
  Una diferencia de tenant falla antes de que exista un plan.
- Baseline, autoridad de oracle y suites son SHA-256 explícitos. Las suites de
  desarrollo y holdout final deben ser distintas; la API recibe sus digests,
  nunca contenido de casos finales.
- La métrica conserva la unidad de análisis del bridge. Cambiar baseline u
  oracle cambia el commitment; no hay setter ni mutación post-freeze.
- `allows_same_outcome_claim` y `eligible_for_proposal` devuelven siempre
  `false`: sellar un plan no eleva el límite `MechanismProxy` de U16.

## Evidencia TDD

1. El primer test RED importó el módulo inexistente `evaluation_plan` y falló
   por contrato ausente.
2. Se añadieron constructores tipados que rechazan oracle, métrica o suite
   inválidos antes de sellar.
3. Las pruebas unitarias construyen evidencia U14/U16 interna y verifican el
   camino permitido, bridge incierto, oracle cross-tenant y cambios de
   baseline/oracle. Las pruebas de integración cubren los inputs públicos.

Validación prevista para este corte:

```powershell
cargo +1.98.1 fmt --all --check
cargo +1.98.1 clippy --workspace --all-targets -- -D warnings
cargo +1.98.1 test -p improvement-engine-core --test evaluation_plan
cargo +1.98.1 test -p improvement-engine-core --lib evaluation_plan
```
