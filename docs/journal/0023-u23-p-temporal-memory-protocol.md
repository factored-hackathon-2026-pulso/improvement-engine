# U23-P — protocolo temporal Frozen/Continuous de memoria gobernada

## Propósito y límite

Esta slice implementa únicamente el borde temporal que faltaba entre U04-B y
la admisión gobernada U22/U33. No es el U23 completo: no hay runner E0,
`CampaignManifest`, escenarios, puntuación, resultado de evaluación, publicación,
runtime, Scout, caché ni páginas de wiki.

`TemporalMemoryEvidence` sustituye los instantes construibles por el llamador.
Su único camino no-test es `EnrichedHistoryAdapter::verified_replay_availability`
de U04-B V2: vuelve a comprobar el `SourceSnapshot` parseado, tenant, world,
cutoff, digest exacto del snapshot y `AvailabilityProfile` sellado antes de que
el composition root interno cree el issuer opaco. El commitment temporal fija
scope completo, snapshot de memoria, run, grant y su revisión, propósito, instante
autorizado, cutoff, protocolo, digest de snapshot fuente y digest de profile;
para Continuous también fija provenance y disponibilidad del outcome. El head
no es un dato del issuer: U33 lo vuelve a leer dentro del commit condicional y
lo incorpora al `receipt_id` canónico. El protocolo nunca abre archivos de
fuente ni interpreta labels/outcomes.

## Contrato ejecutable

`MemoryTemporalProtocol` tiene exactamente dos semánticas y exige que
`MemoryScope.protocol` coincida con la elegida:

- **Frozen** admite sólo un uso cuyo instante autorizado no supere el cutoff y
  rechaza cualquier feedback de outcome. Así preserva memoria de entrenamiento
  durante el replay en vez de aprender a mitad de él.
- **Continuous** admite sólo cuando el outcome previo ya era observable a más
  tardar en el instante del nuevo uso, y ambos instantes están dentro del
  cutoff. No recibe el valor del outcome ni lo publica: sólo permite que una
  futura integración autorizada decida si produce una revisión nueva.

`MemoryTemporalAdmission::admit` permanece `pub(crate)`. Antes de delegar a
U22 usa exclusivamente `AtomicMemoryUseCommitPort`, también `pub(crate)`: un
`AtomicMemoryUseRequest` privado lleva scope, snapshot, request y commitment;
no contiene una observación de autorización. La implementación durable U33
debe resolver y comparar grant/revisión viva dentro de la misma transacción
condicional que head identidad/versión actual, tombstone/linaje y solicitud
exacta antes de escribir (o devolver idempotentemente) el receipt. Un fence
fallido no deja receipt. El puerto público `MemoryPublisher` no puede emitir la
capability. U33 recalcula la identidad incluyendo el commitment además de
scope, snapshot, head, run, grant, revisión, purpose y reloj autorizado. Un
cutoff/outcome/timestamp fabricado, feedback futuro o protocolo cruzado falla
antes de registrar un receipt; no se emite `VerifiedMemoryUse`.

La fábrica no-test `from_u04b_replay` sólo puede terminar en una admisión
**Frozen** hoy: U04-B demuestra cutoff, snapshot y profile, pero aún no existe
el adaptador sellado de outcome de U20-E/U27 que Continuous requiere. Intentar
Continuous con dicha proyección queda denegado por outcome ausente antes de
U33; ningún caller puede aportar ese outcome por fuera del emisor confiable.

La capability resultante sigue siendo la opaca U22: no se añade un handle a
wiki, páginas, cache, publicación o autoridad de aprendizaje.

## RED → GREEN

El primer RED añadió `crates/core/tests/memory_temporal_protocol.rs` y falló
con `E0432` porque el módulo no existía. Las iteraciones posteriores verifican:

1. Frozen acepta memoria de entrenamiento disponible al cutoff y rechaza una
   posterior.
2. Frozen rechaza feedback de outcome incluso si el timestamp parece válido.
3. Continuous exige un outcome ya observable; uno ausente o posterior al uso
   falla.
4. El trusted composition path corta antes de U22/U33 cuando el outcome es
   futuro o el timestamp no coincide con el reloj que U33 sellará; ambos dejan
   el ledger sin receipts.
5. Una admisión Frozen válida emite únicamente la provenance opaca existente
   de U22/U33, conservando el head/run sellados por el receipt.
6. Una proyección real U04-B V2 (no el fixture test-only) debe enlazar tenant,
   world, cutoff, snapshot y profile antes de poder emitir evidencia; un world
   cruzado falla sin receipt.
7. Revocación y cambio de head que ganan dentro del predicado condicional
   niegan la admisión y no dejan receipt ni capability; una revisión de grant
   reemplazada o revocada tampoco cruza el fence.
8. Tenant cruzado y snapshot/profile con el mismo significado aparente pero
   distinto digest producen evidencia distinta o fallan antes del receipt.
9. La revisión del grant forma parte de evidencia y commitment: reemitir el
   mismo id de grant con revisión distinta no puede reutilizar la evidencia
   anterior. Revocación o reemplazo que gana tras la observación inicial del
   adapter también falla el predicado final sin receipt.

## Dependencias y continuación

U23-P depende de U04-B, U22 y U33 ya presentes en la rama acumulativa. El U23
completo sigue necesitando U09, U20-E y U27 para correr replay E0 con versiones
preaprobadas, cobertura, campaña y resultado sin doble puntuación. Es trabajo
de implementación posterior, no un bloqueo que requiera decisión humana, por
lo que no se registra en `OPEN_GAPS.md`.

Validación local prevista antes de revisión independiente:

```powershell
cargo +1.98.1 fmt --all -- --check
cargo +1.98.1 clippy --workspace --all-targets -- -D warnings
cargo +1.98.1 test --workspace
```
