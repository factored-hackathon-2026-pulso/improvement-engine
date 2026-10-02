# U19-0 — admisión sellada de evaluación nativa

## Comportamiento entregado

U19-0 agrega sólo la frontera previa a una evaluación nativa de Agent Core.
`NativeEvaluationRequest` es opaco y sólo puede ser sellado por composición
confiable después de un readback de registro nativo. El readback debe atestar
la identidad Core exacta de candidata y suite (tenant/id/revisión/digest/ruta),
scope U09, plan U20, propuesta/base/hash, capability de evaluador
(SHA/schema/contrato), receipt/revisión del registro y vigencia. El commitment
incluye todos esos campos y el intento.

La proyección de U20 que recibe este corte excluye deliberadamente oracle y
suite final. `FrozenCandidateReceipt` de U18 no entra al API y no puede hacerse
pasar por candidata Core. Un readback es una proyección opaca distinta del
HTTP: exige un `eval_id` observado y propuesta/candidata/base/suite exactos.
Por ello HTTP 200 nunca se interpreta como pass. El readback posterior sólo se
acepta si coincide con el intento/commitment completo y el registro vuelve a
atestar el mismo receipt vivo; revocación, expiración o cambio se rechazan.

## Límites y dependencias

No hay HTTP, runtime, dispatch, sandbox bridge, `EvalRun`, verdict, aprobación,
release ni efecto bancario. Los únicos estados de dispatch declarados son
`not_dispatched` y `unknown_after_dispatch`; un adapter ausente retorna
`dependency_unavailable`, mientras un intento vacío retorna `invalid_attempt`.
Esta slice no simula una evaluación exitosa.

U19 real sigue bloqueada por U18-E (registro/freeze de candidata real) y por
el contrato Unit 6 de Agent Core para `EvalPort`, harness y sandbox. U26 sigue
siendo un sandbox Pulso aislado, no una implementación del puerto upstream.

## TDD y validación

RED: el contrato público importó `native_evaluation` inexistente y falló con
E0432. GREEN: tests verifican adapter ausente, intento inválido, receipt emitido
por registro, binding exacto de intento/capability/plan/scope en readback y
rechazo por revocación/expiración. No existe fixture pública ni adaptador U18
local que pueda emitir evidencia Core.

Comandos ejecutados en Windows:

```powershell
cargo +1.98.1 fmt --all --check
cargo +1.98.1 test --locked -p improvement-engine-core --lib native_evaluation
cargo +1.98.1 test --locked -p improvement-engine-core --test native_evaluation_admission
cargo +1.98.1 clippy -p improvement-engine-core --all-targets -- -D warnings
```

La suite completa y revisión adversarial quedan requeridas antes de integrar.
