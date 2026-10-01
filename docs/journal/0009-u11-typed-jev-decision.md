# U11 — decisión Jev tipada y calibrada

## Comportamiento entregado

`improvement_engine_core::jev_decision` añade `JevDecisionPort` y un doble
determinista consumidor del contrato Agent Core M5. Una `DecisionPolicy` fija
`DecisionModelRef` (id/version/digest), versiones Jev/modelo, referencia de
calibración y enum ordenado. Las filas selladas de calibración/threshold se
indexan por `(locale, choice)` y validan igualdad exacta de `p_raw`; el receipt conserva `p_raw`, `p_cal`,
threshold, digests del modelo/task/policy/model-view y output estructurado.

La salida sólo puede elegir una alternativa declarada. Un enum ajeno no se
normaliza ni se convierte en una alternativa de negocio. Calibración ausente,
locale no cubierto, `p_raw` sin fila exacta o `p_cal` bajo threshold retornan
`DecisionRoute::LowConfidence`; ningún caller recibe una selección implícita
por incertidumbre. La clave durable es `(tenant, job, attempt)` y la mutación
de policy/binding/locale/model-view dentro de ella es un conflicto.

## Límite explícito

No hay llamada Jev real, proveedor, gateway, prompt, modelo general, scout,
verifier, publicación ni UI. El simulador sólo prueba la frontera de tipado,
calibración, receipt e idempotencia que un bridge Agent Core real deberá
preservar. La validación de la autoridad de grant sigue siendo responsabilidad
de U05/política externa; aquí se conserva y compara el scope entregado.

## Evidencia TDD

El primer RED importó `jev_decision` inexistente. Las pruebas cubren receipt
con task/Jev/policy/output ligados, enum inválido, policy no soportada,
low-confidence explícito y cambios de tenant/job/grant para el mismo attempt.

Comandos focalizados ejecutados en Windows:

```powershell
cargo +1.98.1 fmt --all
cargo +1.98.1 test -p improvement-engine-core --test core_task --test jev_decision
```

Resultado: los 8 tests focalizados pasaron. Falta la validación completa y la
revisión independiente AI/runtime para cerrar el bundle U09/U11.
