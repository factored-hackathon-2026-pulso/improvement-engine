# U18 — writer gobernado del registry

U18 recibe únicamente el `CompiledChange` opaco que produjo U17; ninguna API
pública acepta un JSON, un draft, una autorización o un commitment como
sustituto. Justo antes de escribir, el límite crate-private vuelve a reconstruir
el material de escritura y verifica el compromiso completo de autorización:
tenant/scope, snapshot, bridge, plan, ruta, mecanismo, operación, entidad,
versión, cuerpo canónico y precondición tipada. Así, un `CompiledChange`
coherente en apariencia pero alterado falla antes de crear entidad o receipt.

El corte soporta solamente el predicado que U17 puede emitir hoy:
`EntityAbsent { kind, entity_id }` para un `Flow add`. Se evalúa dentro de la
misma sección condicional que congela el payload y añade el receipt. El modelo
en memoria representa el contrato que un adaptador durable debe conservar en
una sola transacción/CAS: llave de entidad con tenant, cabeza de entidad,
precondición, candidate/idempotency key y receipt. Un intento competidor que
ocupa la entidad en el límite final deja cero receipts de U18. `RevisionEquals`
queda como predicado tipado para un corte futuro de reemplazo, no como una
capacidad que U18 afirme soportar ahora.

La identidad idempotente incluye tenant y el commitment compilado; el request
digest además liga scope completo, snapshot, autorización, versión y digests
del payload. Un retry idéntico devuelve el mismo receipt; material distinto
con la misma identidad falla cerrado. Para una caída antes de commit no se
afirma éxito ni se deja estado; para una respuesta perdida después de un commit
la relectura idempotente devuelve el receipt persistido.

Las regresiones cubren replay idéntico, conflicto idempotente, aislamiento de
tenant, tenant de snapshot alterado, stale `EntityAbsent` al límite final y
los dos resultados de crash/unknown. La prueba RED inicial fue el import de
`governed_registry`, que no existía. U18 no integra HTTP de Agent Core, no
llama runtime, no ejecuta/evalúa/aprueba/publica/libera candidatos ni afirma
durabilidad de producción. El siguiente adaptador durable debe implementar el
mismo conditional write sin ampliar este contrato.
