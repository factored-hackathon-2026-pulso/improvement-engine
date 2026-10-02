# U20 — contrato de evaluación sellado

## Propósito y límite

U20 prepara una comparación reproducible desde un `WorkflowBridgeContract` de
U16. No ejecuta candidatos, no llama el runtime de Agent Core, no publica
artefactos, no decide elegibilidad y no autoriza release. Aunque todos los
inputs estén presentes, U16 sigue siendo únicamente `MechanismProxy`:
`allows_same_outcome_claim()` y `eligible_for_proposal()` devuelven siempre
`false`.

## Contrato implementado

`EvaluationPlan::seal_from_bridge` recibe cuatro
`EvaluationArtifactRef` tipados: baseline, oracle, suite de desarrollo y suite
final. Cada uno contiene un `ArtifactReference` completo; un digest de
contenido nunca es identidad ni autoridad suficiente.

El almacenamiento actual conserva `ArtifactKind::ScenarioSet`. Para evitar
que ese tipo genérico debilite el contrato, U20 re-lee cada revisión desde el
`ArtifactRepository` y exige el objeto sellado `evaluation_contract`:

- `artifact_type` coincide con el slot tipado y `partition` es exactamente
  `shared`, `development` o `final` según corresponda.
- El tenant, job, grant y `authority_ref` son idénticos al scope U14/U16.
- La referencia de `SourceSnapshot` completa (tenant, id, revisión y digest)
  es idéntica a la del bridge.
- Los cuatro contratos contienen y coinciden con `target_outcome`,
  `unit_of_analysis` y `oracle_measure` del bridge. Esto liga la métrica al
  outcome y a la unidad, y rechaza mismo snapshot/scope con semántica distinta.
- Ninguna referencia puede reutilizarse entre slots, incluyendo las suites de
  desarrollo/final.

Después de verificar todo, el commitment cubre bridge, snapshot, identidad y
digest de cada revisión, y los tres campos semánticos. Cambiar una revisión
aunque su contenido sea idéntico cambia el commitment. La operación sólo lee
el repositorio; no tiene puerto de append ni efectos sobre candidatos, runtime
o releases.

## TDD y pruebas

La prueba pública `crates/core/tests/evaluation_plan.rs` se escribió primero y
falló por ausencia de los refs tipados, el contrato compartido y el fixture
U14/U16. Se habilita exclusivamente bajo `test-support`: ese fixture ensambla
la composición real U14/U16 con recibos sintéticos y no existe en el build por
defecto.

La cobertura pública verifica:

1. sellado correcto con inputs re-leídos, sin append (adaptador read-only), y
   ambos predicados de outcome/release en falso;
2. gate `MechanismProxy` antes de tocar inputs;
3. rechazo de outcome, unidad, medida y partición development/final erróneos;
4. commitment distinto al cambiar la identidad sellada de los inputs.

Validación ejecutada al cierre:

```powershell
cargo +1.98.1 fmt --all --check
cargo +1.98.1 clippy --workspace --all-targets --features test-support -- -D warnings
cargo +1.98.1 test --workspace --features test-support
git diff --check
```

Los tests PostgreSQL siguen ignorados sin URL explícita y consentimiento
destructivo; no representan un éxito simulado de integración externa.
