# U19-0 — admisión sellada de evaluación nativa

## Comportamiento entregado

U19-0 agrega sólo la frontera previa a una evaluación nativa de Agent Core.
`NativeEvaluationRequest` es opaco y sólo puede ser sellado por composición
confiable cuando coinciden: scope U09, receipt de candidata **registrada** en
Agent Core, `EvaluationPlan` U20, binding de suite development pública y una
capacidad de evaluador pinneada por SHA/schema/contrato. El commitment incluye
tenant, job, grant, autoridad, intento, propuesta, candidata, base, agente,
suite, plan/snapshot y los tres digests del evaluador.

La proyección de U20 que recibe este corte excluye deliberadamente oracle y
suite final. `FrozenCandidateReceipt` de U18 no entra al API y no puede hacerse
pasar por candidata Core. Un readback es una proyección opaca distinta del
HTTP: exige un `eval_id` observado y propuesta/candidata/base/suite exactos.
Por ello HTTP 200 nunca se interpreta como pass.

## Límites y dependencias

No hay HTTP, runtime, dispatch, sandbox bridge, `EvalRun`, verdict, aprobación,
release ni efecto bancario. Los únicos estados de dispatch declarados son
`not_dispatched` y `unknown_after_dispatch`; una dependencia ausente retorna
`dependency_unavailable`. Esta slice no simula una evaluación exitosa.

U19 real sigue bloqueada por U18-E (registro/freeze de candidata real) y por
el contrato Unit 6 de Agent Core para `EvalPort`, harness y sandbox. U26 sigue
siendo un sandbox Pulso aislado, no una implementación del puerto upstream.

## TDD y validación

RED: el contrato público importó `native_evaluation` inexistente y falló con
E0432. GREEN: tests verifican sellado con candidata registrada/capacidad
pinneada, rechazo de suite o capacidad divergente y que un readback vacío no
convierte un éxito HTTP en evaluación atribuida.

Comandos ejecutados en Windows:

```powershell
cargo +1.98.1 fmt --all --check
cargo +1.98.1 test --locked -p improvement-engine-core --lib native_evaluation
cargo +1.98.1 test --locked -p improvement-engine-core --test native_evaluation_admission
cargo +1.98.1 clippy -p improvement-engine-core --all-targets -- -D warnings
```

La suite completa y revisión adversarial quedan requeridas antes de integrar.
