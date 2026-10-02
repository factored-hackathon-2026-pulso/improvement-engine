# U07 — actividad de run con cursor opaco

## Comportamiento entregado

`improvement_engine_core::run_activity` implementa una proyección en memoria
de hechos de job y una frontera de API sin framework HTTP. Un `ActivityEvent`
está ligado a tenant, `job:sha256:`, identidad de evento, instante, clase de
actividad y digest de evidencia. `RunActivityProjection` deduplica
exactamente `(tenant_id, job_id, event_id)`: un replay idéntico es idempotente;
reutilizar identidad con contenido distinto es conflicto.

El contrato público exige `AuthenticatedTenant`: el transporte autentica al
caller y construye ese DTO antes de crear `ListRunActivityRequest`; nunca se
pasa un tenant raw a la frontera. `RunActivityReadModel` recibe una
`ActivityReadQuery` con tenant, job, posición y límite, y devuelve un
`ActivityTimeline` público y validado. Esto permite que adaptadores externos
usen una consulta indexada por `(tenant, job, position)` sin materializar un
run entero. La proyección de referencia conserva ese índice y ordena por
`(occurred_at_unix_seconds, event_id)`.

El handler entrega páginas de 1 a 100 entradas y cursores opacos de servidor.
El token es sólo una clave HMAC derivada de un nonce; tenant, job, posición y
revisión viven en un registro privado del handler. `Debug` del signer redacta
el secreto. Emitir un cursor modifica sólo ese registro efímero de transporte,
nunca la proyección ni el reducer U06.

`CursorRegistry` es ahora un puerto explícito para que un adaptador durable y
compartido pueda reemplazar el registro local al desplegar varios procesos. La
implementación local `InMemoryCursorRegistry` tiene capacidad y TTL en ticks:
evicta el cursor menos recientemente usado y conserva un marcador acotado
para retornar `CursorExpired`, en vez de crecer sin límite o reiniciar una
continuación silenciosamente. No reclama semántica multi-proceso.

El registry local y su TTL por ticks son exclusivamente una dependencia de
prueba/local. Un adapter de producción usa un reloj autoritativo y expiración
persistida/compartida; el DTO `CursorRecord` es público, validado y serializable
para que ese adapter reconstruya tenant, job, revisión y posición sin depender
de internals del crate. Su deserialización es manual y vuelve a ejecutar las
validaciones: JSON con tenant, job, revisión o posición inválidos no puede
eludir el constructor.

Un snapshot cambiado retorna `CursorExpired`; una revisión retenida retorna
`CursorPurged`; un cursor desconocido/manipulado o presentado a otro scope
retorna `InvalidCursor`. La retención es por `(tenant, job)` y elimina
físicamente eventos vencidos de listas nuevas. Los errores exponen un
`ActivityApiStatus` tipado: `NotFound` evita revelar el run de otro tenant y
cursores expirados/purgados son `Gone`.

Si una continuación coincide con una revisión retenida y además el head cambió,
`CursorPurged` tiene precedencia: el contenido ya no puede recuperarse. El
handler también rechaza un adapter que devuelva una entrada no estrictamente
posterior a la posición de cursor, o metadatos `purged_through_revision` mayores
que la revisión del timeline.

`ActivityTimeline` rechaza una respuesta imposible (`has_more` sin entradas),
y el handler rechaza cualquier adapter que entregue más entradas que el page
size solicitado. `purge_before_revision` rechaza una revisión posterior al
head, de modo que un purge inválido no puede bloquear eventos o cursores
futuros.

`ProjectionActivityStream` implementa el puerto concreto
`RunActivityStreamPort`: genera batches resumibles `run.activity` usando los
mismos request, scope, cursor y read model. Un runtime futuro puede
serializarlos como SSE sin duplicar reglas. Esta slice no simula sockets
persistentes ni desconexiones.

## Límites explícitos

No implementa endpoint HTTP/SSE, autenticación real, proyección durable de
hechos U06, UI U24, scheduler ni observabilidad de producción. Un adaptador
durable alimentará `ActivityEvent` e implementará el read port; debe preservar
el índice acotado, tenant binding, expiración y retención de esta frontera.

## Evidencia TDD

El primer RED pidió primera página, cursor y continuación de un módulo
inexistente. Las 13 regresiones cubren orden/dedupe, alcance tenant, cursor
opaco y replay ajeno, expiración/purge/status, retención física aislada,
ausencia de mutación de la proyección, secreto redactado, adapter externo,
batch de stream concreto, respuesta imposible, page sobredimensionada, purge
posterior al head, eviction acotada de cursor, progreso estricto y round-trip
serializable de cursor durable, incluso sus formas JSON inválidas.

Comandos intermedios ejecutados en Windows:

```powershell
cargo +1.98.1 fmt --all
cargo +1.98.1 test -p improvement-engine-core --test run_activity
cargo +1.98.1 clippy -p improvement-engine-core --tests -- -D warnings
```

Resultado intermedio: 13 pruebas U07 y Clippy verdes. Falta la segunda
revisión independiente, rebase y verificación completa antes de integrar.
