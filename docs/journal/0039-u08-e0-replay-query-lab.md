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

`E0QueryLab::admit` recibe únicamente un `QueryResult` ya terminado por U08.
Antes de crear la capability vuelve a verificar digest, filas, conteo, tenant,
snapshot tenant, cutoff, tabla, columnas, tres digests de fuente y ausencia de
dependencia. Por ello una consulta a labels/campo no permitido, un join posterior,
reuso cross-tenant, un cutoff futuro o filas/receipt alterados no emite un
resultado E0. Al éxito vuelve a firmar el `QueryReceipt` con los cuatro
commitments E0; no cambia las filas ni añade capacidad de escritura o release.

## RED → GREEN

El RED inicial fue `crates/core/tests/e0_query_lab.rs`, cuyo import del módulo
inexistente falló con `E0432`. El primer GREEN expuso sólo el marcador público.
Los ciclos siguientes añadieron el projection verifier y la revalidación del
receipt con pruebas para:

1. una proyección real U04-B V2 que preserva tabla, campo, replay y cutoff;
2. receipt U08 válido que recibe commitments E0 y continúa validando su digest;
3. filas alteradas, cutoff futuro, labels/campo no permitido, tenant cruzado y
   join dependiente bloqueados antes de crear capability;
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
