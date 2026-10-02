# U33-E — publicación sellada del resumen Frozen E0

## Decisión

U33-E no reutiliza `MemoryPublisher::publish` como autoridad E0. Ese puerto
genérico acepta un `MemoryPublishRequest` y un resultado U15 que no contienen
la cadena U13-A/U14-E/U14-EQ/U04-B. La nueva composición es crate-private y
acepta únicamente el `PreparedFrozenE0MemorySummary` opaco junto con los
objetos sellados que debe volver a comprobar.

Antes de escribir, vuelve a validar candidate, report, qualification, replay,
scope y access. Además vuelve a construir el resultado desde el `MemoryWiki`
de origen y el único transform permitido; por eso un receipt/hash coincidente
no basta para publicar páginas modificadas por otro actor.

El resultado público es `PublishedFrozenE0MemorySummary`: expone sólo el
commitment y la referencia/version de la revisión de memoria. No implementa
`Debug`, no expone páginas/workspace/source, y declara explícitamente que no
autoriza MemoryUse, promoción, propuesta ni release.

## Persistencia y límite durable

El modelo versionado `FrozenE0PublicationRecord` es el sidecar que un adapter
durable debe insertar atómicamente con el append U02 y el CAS del head. Liga
scope completo, access/grant revision, base/nueva revisión, commitments U15,
candidate/report/qualification, replay cutoff/source-binding/profile y el
source snapshot. No se modifica la migración U33 ya aplicada ni se añaden
campos al payload `memory_wiki`: U15 exige exactamente sus tres campos.

La rama no finge que PostgreSQL puede validar la liveness/revisión de un grant
U05: la migración actual sólo recibe un `grant_ref` string. Por tanto
`DurableFrozenE0SummaryPublicationUnavailable` falla cerrada con
`DependencyUnavailable`. El fixture in-memory prueba CAS/idempotencia, pero
no reclama semántica durable ni de procesos concurrentes. Una migración
forward y un projection/fence U05 transaccional son condición para habilitar
el adapter durable.

## Verificación

El tracer recorre la cadena real de fixtures autenticados U02/U04-B/U08/U12/
U13/U14/U14EQ/U15 y publica exactamente una capability opaca. Regresiones
verifican drift de access/scope, drift de provenance candidate, idempotencia y
un CAS obsoleto después de un advance externo (sin sidecar nuevo). La carrera
de revocación entre el precheck y el port no se atribuye al fixture: el adapter
durable sigue fallando cerrado hasta disponer del fence U05 co-transaccional.
Los doctests
`compile_fail` impiden importar el composer privado o sustituirlo por el
`MemoryPublishRequest` genérico, y probar que la capability publicada no se
puede formatear con `Debug`.

## Fuera de alcance

No hay `MemoryUse` U22/U23, reattestation temporal, proposal/U16, evaluation,
Agent Core, LLM, release ni claim causal/comercial. U23-E deberá consumir el
sidecar sellado bajo su propio commit condicional.
