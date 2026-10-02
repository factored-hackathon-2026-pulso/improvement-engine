# U13 — Scout autónomo de candidatos

## Corte GREEN de invocación con proveniencia, no de evidencia E0 autenticada

El Scout es una frontera pura de descubrimiento. Consume una señal descriptiva
de U12, receipts del laboratorio U08/U12, un receipt de tarea Core U09 y un
receipt gobernado U10. Sólo emite borradores candidatos versionables
de `Signal`, `Claim` y `Opportunity`; no publica detector/propuesta, no llama
al proveedor, no ejecuta acciones y no modifica datos fuente.

La publicación exige receipts de query íntegros, outcome exitoso U09 y outcome
exitoso U10 ligado al mismo scope. Un `ScoutInvocationAuthority` del plano de
control sella la expectativa opaca: scope completo, digest de señal, binding,
attempt, run y output Core, y policy/capability/input commitment/attempt del modelo. El
Scout exige coincidencia exacta; no admite strings esperados aportados por el
llamador. `NonProductionScoutInvocationAuthority` es composición explícitamente
no productiva y sólo existe bajo `test-support`; producción implementa el puerto
desde el plano de control.

La ruta `DeterministicSensor` disponible en este corte consume el
`QueryResult` público del laboratorio para pruebas descriptivas. Sus receipts
pueden aportar trazabilidad local, pero no son una capability de evidencia E0
autenticada y no deben presentarse como evidencia de una mejora autónoma. U13
no puede usar un `QueryResult` público para sostener una mejora, aunque los
campos aparenten coincidir. U12-E, todavía no integrado, será el único camino
E0 autenticado: deberá emitir una capability opaca desde sus propios controles
de source/snapshot/receipt; recién esa capability podrá alimentar la ruta de
mejora posterior.

Cualquier tarea/modelo no resuelto se hace visible como `DependencyBlocked`;
el output del LLM no se conserva ni se toma como prueba causal. Cada candidato
retiene snapshot, scope, receipts de query, señal, binding/attempt/run/output
Core y policy/capability/input/evidence/output/attempt de modelo. U08 sella
tenant/grant/authority en cada receipt y U12 lo propaga hasta la señal. Su ID
usa un SHA-256 canónico completo de la procedencia, no un prefijo truncado.

## TDD

RED: `autonomous_scout` no existía y el test falló con `E0432`.
GREEN local: la cadena de prueba U08 → `DeterministicSensor` público → U09
(CoreTaskSimulator) → U10 (ModelProviderSimulator) emite exactamente tres
drafts con provenance local, no evidencia E0. Las regresiones cubren scope distinto, receipt de lab
manipulado, Core `Unknown`, binding o attempt Core equivocado, policy,
run/output Core alterado, capability, input, evidence/output o attempt del modelo equivocado, replay de
una señal válida y mismo tenant con grant/authority distintos; cada caso niega
la evidencia o bloquea sin drafts según corresponda.

```powershell
cargo +1.98.1 test -p improvement-engine-core --features test-support --test autonomous_scout
cargo +1.98.1 clippy -p improvement-engine-core --all-targets --features test-support -- -D warnings
```

Antes de commit/PR falta ejecutar las compuertas completas actualizadas y la
revisión independiente AI/data/security.
