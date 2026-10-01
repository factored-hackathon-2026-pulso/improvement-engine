# U05 — cuota global y grants gobernados

## Alcance entregado

`improvement_engine_core::quota_grant` entrega una frontera determinista de
admisión para una corrida autónoma. `QuotaWindow` identifica la bolsa global
por `(tenant_id, resource, start)` y guarda `end` como metadata que debe
coincidir: cambiar el cierre para el mismo inicio devuelve
`QuotaWindowEndMismatch`, nunca abre otra bolsa. Deliberadamente no contiene
`RunConfigIdentity`. Un `QuotaLimit` se configura explícitamente antes de
reservar; no hay un límite global implícito derivado de una configuración ni
de variables de entorno.

Una `QuotaReservation` fija la identidad inmutable de `RunConfig`, un grant
preautorizado, unidades, momento y clave de idempotencia. La configuración se
conserva en el `QuotaReceipt` como procedencia, pero una nueva identidad no
reinicia `reserved_units` de la misma ventana. Cada receipt tiene un digest
estable sobre una codificación length-prefixed de su entrada/salida. Repetir
la misma solicitud devuelve exactamente el mismo receipt; reutilizar la clave
para otro payload se rechaza y no modifica saldo.

`AuthorizedGrant` es un dato que ya trae `authority_ref`, tenant, recurso,
límite y expiración. El uso y la revocación se indexan por el scope completo
`(tenant_id, authority_ref, grant_id)`, no por `grant_id` aislado. La
idempotencia se indexa por `(tenant_id, idempotency_key)`: tenants diferentes
pueden reutilizar una misma key sin ver la receipt o el saldo del otro. Esta
crate no simula ni inventa una autoridad de política:
la emisión real seguirá siendo un puerto externo. Antes de reservar se exige
tenant/recurso coincidentes, vigencia estricta y ausencia de revocación. Una
revocación es inmutable e idempotente para el mismo evento; una revocación
conflictiva se rechaza. Una reserva que excede la bolsa global retorna
`DeferredQuotaExhausted`; una que excede su grant retorna
`DeferredGrantExhausted`. Grant expirado o revocado devuelve error explícito
y no consume unidades.

`RunConfig` complementa esa frontera como valor tipado e inmutable: versión
soportada, cadencia event-driven o periódica acotada, límites de escaneo y
fuentes explícitamente elegibles. Su identidad SHA-256 es independiente del
orden con que llegan las fuentes. No posee un global mutable y no realiza
lectura de datos, persistencia, llamadas a modelos, Agent Core ni gateway.

## Evidencia TDD

1. El primer RED importó `run_config` inexistente; el constructor mínimo
   validado hizo verde una corrida periódica acotada.
2. El RED principal de U05 declaró dos `RunConfig` distintos y dos grants
   contra una sola ventana. El primer diseño que usaba sólo el máximo del
   grant produjo `DeferredGrantExhausted`, revelando que no modelaba la bolsa
   global.
3. Se añadió `QuotaLimit` explícito y se hizo obligatorio configurar la
   ventana antes de admisión. El mismo test pasó: 7 unidades quedan reservadas
   y una segunda solicitud de 4 se difiere aunque cambie la config y el grant.
4. Se añadieron de forma incremental regresiones de grant expirado/revocado,
   retry idempotente y colisión de key; todas están verdes.
5. Una revisión adversarial encontró que `grant_id` e `idempotency_key` se
   indexaban sin scope. Los tests RED demuestran ahora que dos tenants pueden
   reutilizar key y grant ID, y que dos autoridades con el mismo `grant_id` no
   comparten consumo ni revocación. Se cambió la indexación antes del segundo
   ciclo verde.
6. La re-revisión detectó que `QuotaWindow` incluía `end` en su clave, lo que
   permitía reiniciar la bolsa modificando sólo ese campo. Un RED con el mismo
   tenant/recurso/inicio y otro cierre confirmó el hueco; la clave ahora omite
   el cierre y una discrepancia retorna el error tipado, conservando el saldo
   original.

Comandos ejecutados en Windows:

```powershell
cargo +1.98.1 test -p improvement-engine-core --test run_config
cargo +1.98.1 test -p improvement-engine-core --test quota_grant
```

## Límites y siguiente integración

El ledger de esta unidad vive en memoria para hacer verificable la semántica
antes del adaptador durable. No acredita atomicidad entre procesos, recovery,
leases, outbox, migración `pulso_quotas` ni una firma/consulta de política
real. U02/U06 deben llevar exactamente esta clave y receipt a PostgreSQL con
un lock transaccional de `(tenant_id, resource, window_start)`; el cambio de
config debe tomar ese mismo lock. La política duradera debe definir además
cómo bajar un límite por debajo de consumo existente; esta frontera lo rechaza
si la ventana ya fue configurada para evitar esconder esa decisión.
