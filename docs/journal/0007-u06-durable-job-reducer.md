# U06 — admisión, lease y recuperación de jobs durables

## Comportamiento entregado

`improvement_engine_core::durable_jobs` define el contrato ejecutable de un
job de mejora autónoma. Una `JobAdmissionRequest` une exactamente el
`tenant_id`, la clave idempotente del trigger, el digest del trigger, la
identidad inmutable de `RunConfig` y el `QuotaReservation` de U05. El
constructor rechaza antes de mutar si tenant, configuración o clave no
coinciden entre las dos fronteras. La identidad de job se deriva de
`(tenant, RunConfigIdentity, trigger_key)`; el digest de admisión incluye
además el digest completo de la reserva, por lo que reutilizar una clave con
un payload distinto es un conflicto, no una repetición silenciosa.

`DurableJobStore::admit` serializa en la misma frontera la reserva U05, la
idempotencia y la creación del job. Un receipt `Reserved` crea un único
`JobRef`; un receipt diferido por grant o cuota se guarda idempotentemente,
pero contiene `job: None` y no existe en el conjunto leasable. No hay un job
pendiente que pueda ejecutar un efecto sin presupuesto.

Un lease contiene `worker_id`, attempt, fence token y expiración. Reemplazar
un lease expirado incrementa tanto attempt como fence. `begin_effect_dispatch`
y `acknowledge_effect` exigen el tenant, owner, fence y lease vigente; un
worker anterior o un caller que sólo conoce el fence no puede hacer la
transición. Los estados públicos distinguen cola, lease, `UnknownPendingReconciliation`,
`CompletedNoEffect` y `AppliedAcknowledged`.

Antes de despachar una dependencia mutable, el reducer cambia a
`UnknownPendingReconciliation`: después de un crash no hay retry ni re-lease
implícito. `recover_after_restart` sólo libera un lease expirado que aún
prueba `NoEffect`. Para salir de `Unknown`, `reconcile_unknown` exige una
`ReconciliationEvidence` content-addressed que observa `NoEffect` o un
`effect:sha256:` aplicado. La evidencia une tenant, `JobRef`, fence exacto de
dispatch y `authority_ref`; el store exige que la autoridad esté confiada para
ese tenant antes de mutar. Así un digest sintácticamente válido para otro job,
otro dispatch o una autoridad no confiada no puede cerrar el job. El store
conserva la referencia verificada y sólo entonces lleva el job a un estado
terminal.

## Persistencia y límite explícito

`DurableJobRepository` es el puerto de persistencia. Un adaptador durable
debe implementar `admit_atomically` como una transacción que abarque reserva
U05, índice de idempotencia y row de job, y las demás operaciones como
`UPDATE ... WHERE tenant_id AND job_id AND worker_id AND fence AND expiry`
condicionales. Esta slice sólo implementa el adaptador en memoria como modelo
de referencia: no acredita recuperación real de proceso, locks distribuidos,
outbox, PostgreSQL, scheduler, Agent Core ni un efecto externo.

## Evidencia TDD

1. El primer RED importó `durable_jobs` inexistente para demostrar que dos
   triggers equivalentes no devolvían el mismo job/receipt.
2. Los siguientes RED declararon el fence obsoleto, crash tras dispatch,
   aislamiento cross-tenant, grant inválido, key conflictiva, quota diferida,
   owner distinto con fence actual, mismatch de fronteras, payload de cuota
   distinto y el puerto durable inexistente.
3. `crates/core/tests/durable_jobs.rs` cubre admisión exacta, reserva única,
   diferimiento no-ejecutable, lifecycle/attempt, stale fence, owner,
   estados de efecto, recovery conservador, reconciliación ligada a
   tenant/job/fence/autoridad, y el consumo a través del puerto.

Comandos ejecutados en Windows tras rebase sobre `main`:

```powershell
cargo +1.98.1 fmt --all --check
cargo +1.98.1 clippy --workspace --all-targets -- -D warnings
cargo +1.98.1 test --workspace
python -m unittest discover -s tests -p "test_*_contract.py" -v
python contracts/validate_fixtures.py
```

Resultado: formato y Clippy verdes; 67 tests Rust pasaron y una integración
PostgreSQL destructiva existente quedó `ignored` por no contar localmente con
su URL aislada; 7 contratos Python y el validador de fixtures pasaron.
