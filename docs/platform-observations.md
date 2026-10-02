# U29: observaciones de la plataforma de atención

Este contrato incorpora evidencia de la plataforma de atención a Pulso sin
confundirla con eventos internos del motor (`EngineEvent`). La fuente de
atención no es el motor de evolución; cada evento declara `target_system`.
El slice es una frontera de ingestión y lectura Rust con persistencia
PostgreSQL; no implementa un collector HTTP, dashboard ni el cómputo U30.

## Sobre y autoridad

`PlatformObservationBatch` declara tenant, fuente, partición, versión de
contrato, cursor de transporte, cobertura, eventos tratados, reloj de
observación, retención, `batch_digest` y `event_blob_ref`. El cursor de fuente
es distinto de la secuencia de `EngineEvent`. El productor puede usar una
secuencia contigua o un cursor opaco; su modo se fija en un
`ObservationSourceContract` de configuración confiable, nunca en el propio
batch. Los contratos se inyectan como `ObservationSourceRegistry` inmutable
al construir el repositorio; no existe mutador de registro en la ruta de
ingestión. Quien controle el composition root aún podría sustituir el
registro o el puerto de autorización: ese código y su configuración son
parte explícita de la base confiable del despliegue, no están certificados
criptográficamente por U29. Fuente o versión desconocida, modo de cursor equivocado, dominio
incorrecto y declaraciones de cobertura incompatibles fallan cerrados.

Cada evento conserva `source_event_id`, `source_run_ref`, `episode_ref`
opcional, `occurred_at_ms`, `received_at_ms`, tipo de evidencia, tipo de
interacción y referencias de traza. La unión cubre intentos por capa,
handoffs, resultados de tools, decisiones de modelo, acciones humanas y
estados de respuesta/entrega/caso. Evidencia OTel exige carga diagnóstica
tipada: nombre/unidad/valor de métrica, código/severidad de log o referencia
y duración de span. Los campos textuales admiten únicamente referencias
allowlisted; contenido libre de clientes no puede entrar por esta API.

Un `CoreAudit` requiere digest de evento de origen y
`CoreChainVerifierPort`. Sólo `Verified` lo admite; el receipt conserva el
digest de verificación. `Unavailable` e `Invalid` rechazan el batch.
El puerto NO presume que una firma upstream sea válida: el adaptador de
verificación real y su raíz de confianza aún deben implementarse.

`ObservationAccess` contiene tenant, grant y propósito. Un
`ObservationAuthorizationPort` confiable debe validarlo al crear el
repositorio y en cada operación; construir el struct no concede permiso. El
adaptador PG aplica además `FORCE ROW LEVEL SECURITY` a sus cuatro tablas.
La policy consulta una tabla de entitlements administrada por el control
plane y exige rol efectivo + tenant + grant + propósito. El rol de runtime
no puede modificar esa tabla. Grant y propósito se establecen con
`SET LOCAL` dentro de cada transacción, por lo que no quedan pegados a una
conexión reutilizada. El despliegue debe proveer un rol PG restringido, sus
permisos DML U29 y sus entitlements; conectarse como superuser o dueño de
tablas invalida esta barrera. La política no afirma impedir DML directo
dentro del tenant autorizado: la integridad de ese camino depende del
servicio y de los contratos de ingestión, no de una API SQL cerrada.

## Persistencia e idempotencia

`ContentAddressedInlinePostgresBlobStore` guarda únicamente bytes JSON
tratados, con digest y retención, en la misma transacción de observaciones.
El límite por blob es 256 KiB; un batch mayor se rechaza hasta habilitar
otro blob store autorizado. `event_blob_ref` refiere a este blob PostgreSQL,
no a S3. Eventos y batches son inmutables; mismo contenido reentregado
devuelve el mismo receipt, mientras un ID/cursor reutilizado con contenido
distinto produce conflicto. El cursor contiguo detecta huecos. La lectura
queda acotada al tenant del repositorio y a una ventana `received_as_of_ms`.

## Denominadores, huecos y correcciones tardías

Cobertura tiene estado `complete`, `partial`, `unknown` o `degraded`,
referencia de población, ventana, población esperada y razón de falta cuando
no es completa. Un batch degradado sin eventos sigue siendo evidencia de
collector ausente, no evidencia de cero incidencias. La población declarada
de un batch sólo es candidata a denominador para un `PlatformAudit` de
atención con cobertura completa. Incluso un `CoreAudit` verificado con target
`Attention` sigue siendo evidencia del motor, no población bancaria.
Métricas/logs/spans muestreados y fuentes parciales tampoco son denominador. U30 tendrá que
reconciliar ventanas/fuentes superpuestas; no debe sumar poblaciones de
batches sin probar unicidad y cobertura. Una observación tardía agrega una
nueva revisión de lectura determinista: consultas con el `as_of` anterior
conservan el mismo resultado; no se reescribe una conclusión histórica.
`CoverageEvidence::denominator_for` sólo entrega un `BoundDenominator` si la
evidencia y el evento observado coinciden exactamente en tenant, fuente,
versión de contrato y batch digest, la capacidad de cobertura completa fue
validada durante ingestión y el digest de evidencia recomputa. La evidencia
no es una firma criptográfica de un tercero: se apoya en la integridad de
la persistencia y el contrato de fuente confiable. Ningún evento se vuelve
visible antes de la aceptación de su batch, aunque su reloj de origen sea
anterior.

## Límites pendientes

El registro de contratos y el puerto de autorización están inyectados en
proceso y deben provenir de una
configuración confiable del despliegue. No hay adaptador de transporte real,
verificador Core real, S3 ni estrategia de purga/retención: los triggers de
inmutabilidad actualmente impiden borrar filas. Los tests PostgreSQL usan
una base efímera y un rol restringido creado sólo en ella; demuestran la
policy RLS pero no el provisioning real de producción. Tampoco hay métricas
agregadas ni UI. La purga está bloqueada por falta de capacidad aprobada;
no se debe prometer eliminación automática al vencimiento hasta diseñarla.
El provisioning de producción debe garantizar que el propietario de tablas
y funciones no sea el rol de runtime, que ese rol sea `NOSUPERUSER` y
`NOBYPASSRLS`, que no herede ni pueda asumir un rol privilegiado u otro
con entitlements ajenos (`NOINHERIT` y membresías controladas), y que sus
grants SQL no incluyan mutar entitlements. Esto es
una precondición de despliegue documentada, no algo instalado por U29.
