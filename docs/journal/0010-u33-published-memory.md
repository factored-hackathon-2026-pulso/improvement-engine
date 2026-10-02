# 0010 — U33: memoria publicada, CAS y revocación

## Comportamiento

`memory_store` es la frontera que convierte un resultado de scratch U15 en una
nueva revisión inmutable `memory_wiki`. Una `MemoryScope` fija tenant,
propósito, mundo, campaña, protocolo y partición; una misma revisión no se
puede sembrar en dos scopes. El primer head se siembra contra una referencia
U02 exacta y viva. Publicar exige el `head_version` observado, un `WikiAccess`
autorizado para la referencia base, el receipt U15 que cita la misma base y el
digest determinista de las páginas resultantes.

El nuevo snapshot conserva el mismo artifact id y avanza su revisión U02;
la fuente scratch no se muta. Después de insertar la revisión, el head del
scope avanza de forma explícita. Un escritor que llega con versión vieja recibe
`HeadConflict`: debe montar el head actual, rebasear su cambio en scratch y
publicar una propuesta nueva. No hay merge textual silencioso.

`MemoryUseReceipt` fija run, grant, propósito, reloj autorizado, scope, head y
snapshot exactos. Es prueba de una lectura permitida, no prueba de que esa
lectura causó una mejora. La revocación agrega un tombstone; no modifica ni
borra la revisión histórica. Antes de publicar o emitir un receipt se consulta
el overlay; al clonar/restaurar el registry, el tombstone continúa denegando el
uso. En PostgreSQL tombstones y receipts son append-only. El control-api
verifica primero el grant U05 y registra el receipt U15 sellado (workspace,
run, grant, scope, base y páginas); la publicación consume ese receipt durable,
rechaza payload/scope/base distintos y mueve el head junto con el append U02.
El digest del resultado U15 se verifica en Rust sobre sus páginas tipadas; SQL
no intenta reimplementar RFC8785/JCS. Por eso el digest/páginas sólo cruzan la
frontera SQL dentro del receipt registrado por el boundary privilegiado, nunca
como input ejecutable del runtime genérico.

## Estado durable

`0002_pulso_memory_control.sql` agrega solamente tablas Pulso propias:

- `pulso_memory_heads`: un head CAS por scope, no un head de atención;
- `pulso_memory_tombstones`: overlay inmutable de revocación;
- `pulso_memory_lineage`: relación padre exacta para cierre transitivo de una
  revocación;
- `pulso_memory_transform_receipts`: resultado U15 ya verificado contra grant,
  scope y base antes de permitir publicación;
- `pulso_memory_use_receipts`: ledger tratado de uso permitido.

La migración no toca el dataset, no actualiza `pulso_artifact_revisions`, no
incluye PII ni implanta Agent Core, modelo, gateway o política de plataforma.
El control de grant real sigue siendo U05: la implementación local conserva el
puerto `WikiAuthorizationPort` para que la frontera no dependa de una autoridad
ficticia. Hasta que exista ese adaptador durable, las funciones U33
`SECURITY DEFINER` no se conceden al runtime genérico: el test real-PG prueba
que ese rol no puede sembrar una memoria ni fabricar receipts. El boundary
privilegiado será responsable de validar la autoridad U05 antes de invocarlas.

## Evidencia TDD

El primer RED fue `published_memory.rs` importando el módulo inexistente
`memory_store`. El compilador falló con `could not find memory_store in
improvement_engine_core`. Tras GREEN, los tests específicos verifican:

1. publicación autorizada, revisión inmutable anterior y receipt permitido;
2. CAS viejo no sobreescribe aprendizaje posterior;
3. tombstone bloquea uso también tras una restauración del registry;
4. resultado transformado/tampered y reutilización cross-scope se rechazan.

El test PostgreSQL ignored amplía el gate existente: aplica U02+U33, usa el rol
runtime de privilegio mínimo y verifica receipt→publicación por CAS, rechazo
del CAS viejo y que un tombstone de ancestro bloquea la siguiente publicación.
Además abre dos clientes sincronizados por `Barrier` para el primer `INSERT`
de un receipt de transform y de uso (keys nunca persistidas antes): ambos
llamados idénticos deben pasar y quedar en una sola fila; una repetición
divergente con esa key debe fallar.
En esta máquina todavía no se ejecuta localmente por
la indisponibilidad ya documentada de Podman rootful; el workflow del repo usa
PostgreSQL efímero real y es la evidencia de integración pendiente de CI.

## Límites y siguiente integración

No hay publicación automática desde un LLM ni resolución de conflictos
automática. U22 será el primer consumidor que demuestre una segunda corrida con
un `MemoryUseReceipt`; U23 aplica además el protocolo temporal frozen/
continuous. U33 no afirma que un backup físico ya fue restaurado: establece que
los tombstones son facts durables que un restore debe cargar antes de servir
memoria. El adaptador PostgreSQL de servicio se añade cuando el composition root
del control-api comparta la transacción U02; no se abre una segunda conexión ni
se simula atomicidad entre procesos aquí.
