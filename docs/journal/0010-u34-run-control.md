# U34 — control de runs en el límite durable U06

U34 no introduce un store o reducer paralelo. La única superficie es
`DurableJobRepository::control_job_atomically`: una implementación durable debe
evaluar en la misma transacción `(tenant_id, job_id, expected_version,
expected_fence)`, aplicar la transición y persistir el receipt/auditoría.
`DurableJobStore` es el modelo ejecutable de ese contrato, no una afirmación de
durabilidad multi-proceso.

`control_version` representa los hechos autoritativos de lifecycle y efecto.
Toda mutación U06 que puede cambiar la seguridad de un control (lease,
pre-dispatch, acknowledgement, reconciliación o recuperación) incrementa esa
versión. Un comando que sólo observa `ReconciliationRequired` o
`AlreadyTerminal` no cambia esa versión: su receipt confirma que no hubo
mutación. Así, una lectura anterior pierde frente a una carrera y no puede
cancelar un run que entró en dispatch. La identidad de idempotencia es
`(tenant, job, idempotency_key)` y su huella incluye tenant, job, operador,
key, versión esperada, fence y acción; una reutilización semánticamente distinta
falla sin mutar.

El modelo conserva separación entre `JobStatus` (lifecycle) y
`JobEffectState` (hecho externo). `Pause` sólo pausa `Queued` sin efecto;
`Cancel` sólo confirma `CancelledBeforeEffect` desde `Queued`/`Paused` sin
efecto. `Leased`, `UnknownPendingReconciliation` o cualquier efecto incierto
devuelven `ReconciliationRequired`; nunca se presenta una cancelación como
hecha. Un job terminal devuelve `AlreadyTerminal`. El tenant equivocado recibe
`JobNotFound` en esta superficie para no inferir propiedad de otro tenant.

El receipt incluye tenant, job, operador, idempotency key, fence esperado, solicitud,
estado/efecto observado y confirmado, versión objetivo/confirmada e instante
de registro. No hay cancelación de una plataforma
externa, scheduler, UI, HTTP ni Agent Core en U34. El futuro adapter de
Postgres debe llevar la condición, cambio de job y audit receipt a una sola
transacción; no puede sustituirla por read-then-write. Ese adapter recibe el
operador desde la identidad autenticada y obtiene `recorded_at` de su reloj
autoritativo/persistente; el reducer de prueba sólo acepta ambos como datos de
entrada para hacer el contrato reproducible.

Las regresiones cubren pause/replay vía el puerto, Unknown pre-efecto,
carrera de lease contra versión leída, no-inferencia cross-tenant,
idempotencia semántica y cancelación terminal seguida de pause. Pendiente de
este slice: adaptación SQL concreta y entrega HTTP/UI, que pertenecen a los
adapters/plataforma y no se simulan como si existieran.
