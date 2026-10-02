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
- Antes de leer el payload de cada slot, `EvaluationArtifactAuthorityPort`
  debe atestar la revisión exacta bajo el scope U14/U16. El grant/capability
  vive fuera del `ScenarioSet`; por tanto cuatro payloads coherentes auto
  publicados no crean autoridad. El doble local soporta revocación y el puerto
  real queda como dependencia de política/capabilities.

Después de verificar todo, el commitment cubre bridge, snapshot, identidad y
digest de cada revisión, y los tres campos semánticos. Cambiar una revisión
aunque su contenido sea idéntico cambia el commitment. La operación sólo lee
el repositorio; no tiene puerto de append ni efectos sobre candidatos, runtime
o releases.

## TDD y pruebas

Las pruebas unitarias privadas se escribieron primero y fallaron por ausencia
de los refs tipados, el contrato compartido, la atestación de autoridad y el
fixture U14/U16. El fixture permanece `cfg(test)` y `pub(crate)`: no existe
una superficie pública ni siquiera bajo `test-support` que permita a un
consumidor fabricar un bridge `MechanismProxy`. Un doctest `compile_fail`
vigila ese límite de compilación.

La cobertura del módulo verifica:

1. sellado correcto con inputs re-leídos, sin append (adaptador read-only), y
   ambos predicados de outcome/release en falso;
2. gate `MechanismProxy` antes de tocar inputs;
3. rechazo de outcome, unidad, medida y partición development/final erróneos;
4. commitment distinto al cambiar la identidad sellada de los inputs.
5. cuatro `ScenarioSet` semánticamente coherentes pero sin atestaciones no
   sellan; tras atestar las cuatro revisiones el plan es válido, y revocar una
   vuelve a bloquearlo.

`crates/core/tests/evaluation_plan.rs` conserva cobertura de superficie pública
para los refs tipados y el puerto de autoridad independiente. No fabrica un
bridge: el doctest anterior verifica que dicha fábrica no es importable.

Validación ejecutada al cierre:

```powershell
cargo +1.98.1 fmt --all --check
cargo +1.98.1 clippy --workspace --all-targets -- -D warnings
cargo +1.98.1 test --workspace
git diff --check
```

Los tests PostgreSQL siguen ignorados sin URL explícita y consentimiento
destructivo; no representan un éxito simulado de integración externa.
