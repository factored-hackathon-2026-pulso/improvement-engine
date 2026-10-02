# U08-E — adapter de consulta E0 de sólo lectura

## Propósito y límite

U08-E une dos fronteras ya existentes: U04-B prueba que un paquete replay está
ligado a un `SourceSnapshot`, perfil de disponibilidad y cutoff; U08 prueba que
una consulta local finalizó bajo su grant. Esta slice no abre archivos, no monta
una base adicional, no acepta SQL, no escribe fuente ni scratch, no llama
Scout/modelos/Jev/Agent Core, y no calcula U12-E.

`EnrichedHistoryAdapter::verified_e0_query_projection` es crate-private y
recibe sólo un `VerifiedReplayAvailability` producido por el mismo adapter
U04-B V2, snapshot y `TableInput`. Revalida tenant, world, cutoff, digest de
snapshot y profile antes de reutilizar `discovery_table`: labels, señales,
campos futuros y anotaciones de disponibilidad/proyección alteradas fallan en
U04-B. El resultado opaco conserva tabla, digests de contrato/fuente/transform,
commitment canónico de disponibilidad de campos y digest de proyección replay.

`E0QueryLab::admit` no acepta un `QueryResult` público: acepta únicamente un
candidate opaco recuperado por U08 desde su ledger efímero, aún autorizado, con
el receipt y las filas exactas que U08 almacenó. Un digest canónico de
`QueryReceipt` sólo detecta alteración; **no autentica un emisor** y no puede
crear una capability E0. La capability de éxito es un marcador opaco no
constructible, separado del receipt público y de las filas; no afirma ser una
firma ni una atestación criptográfica.

Antes de emitirla, compara el `SourceSnapshot.binding_digest` sellado por U04
con un mapeo explícito y privado emitido por el registro de snapshots
inmutables tras volver a parsear los bytes crudos del snapshot, y almacenado
en la capability aprobada del lab,
los tres digests, cutoff, tabla y la tabla fuente
exacta (schema y filas) contra la evidencia `TableInput` validada por U04. La
lista de campos permitidos se deriva de las columnas presentes en ese
`TableInput`, no del schema más amplio del manifest. Todas las columnas leídas,
incluida la columna de `Equals`, deben pertenecer a esa lista. Labels, señales,
campos futuros/desconocidos, joins dependientes, reuso cross-tenant, mapeo de
snapshot distinto o evidencia divergente fallan antes de capability. El
`ArtifactReference.digest` de U08 nombra contenido y no se compara con el
binding digest de U04: ambos dominios pueden diferir. Nada añade
escritura, ejecución o release.

## RED → GREEN

El RED inicial fue `crates/core/tests/e0_query_lab.rs`, cuyo import del módulo
inexistente falló con `E0432`. El primer GREEN expuso sólo el marcador público.
Los ciclos siguientes añadieron el projection verifier y la revalidación del
receipt con pruebas para:

1. una proyección real U04-B V2 que preserva tabla, campo, replay y cutoff;
2. un receipt emitido y recuperado desde el ledger U08, ligado al snapshot y a
   las filas selladas U04, obtiene una capability E0 opaca;
3. cutoff futuro, labels/campo no permitido (incluido filtro), tenant/mapeo de snapshot
   cruzado, evidencia divergente y join dependiente son bloqueados antes de
   crear capability;
4. `SourceWrite` denegado por U08 sin receipt E0.

## Fuera de alcance

No implementa U12-E, señal, Scout, modelo, Agent Core, ejecución, evaluación,
release, HTTP, almacenamiento durable ni un sandbox nuevo. El lab U08 existente
continúa siendo el único ejecutor local de su AST de lectura.

## Validación

```powershell
cargo +1.98.1 test -p improvement-engine-core --lib e0_query_lab --quiet
cargo +1.98.1 test -p improvement-engine-core --test e0_query_lab --quiet
cargo +1.98.1 fmt --all -- --check
cargo +1.98.1 clippy --workspace --all-targets -- -D warnings
cargo +1.98.1 test --workspace
```
