# U22 — consumo gobernado de memoria publicada

## Propósito y frontera

U22 permite que una segunda corrida reciba evidencia de que usó una revisión
publicada de `memory_wiki`; no recibe una caché ni las páginas de la wiki. El
resultado es `VerifiedMemoryUse`, una capability opaca que expone únicamente
provenance tratada: receipt, scope, referencia exacta, versión del head, run,
grant y propósito. No entrega payloads, `WikiWorkspace`, paths ni handle de
repositorio.

U33 sigue siendo dueño del estado mutable: head, tombstones, autorización
exacta, lectura de revisión viva e idempotencia del receipt. La admisión U22
usa únicamente el puerto interno `AtomicMemoryUseCommitPort` con un
`AtomicMemoryUseRequest` sellado. U22 no lee ni transporta un fence de grant:
el único adapter local U33 posee conjuntamente autoridad de grants y ledger de
receipts; el adapter durable debe hacerlo en la misma transacción. Así resuelve
grant y revisión desde su estado durable y los verifica en **la misma**
operación condicional que scope, snapshot, request completo, head
identidad/versión y tombstone/linaje; sólo entonces inserta (o devuelve
idempotentemente) el receipt. U22 liga por ello scope, snapshot, run, grant,
revisión de grant, propósito, instante permitido y commitment temporal. Un
fence que pierde no deja receipt ni capability.

`MemoryUseAdmission::admit` es `pub(crate)`. Por ello un consumidor externo no
puede sustituir un `MemoryPublisher`/attestation port permisivo, ni construir
una `VerifiedMemoryUse` o el carrier `MemoryUseAdmission` por literal. El
composition root futuro conectará ahí el adapter durable U33 y la autoridad
U05. No se expone una vía temporal de test ni un endpoint de control-api.

## RED → GREEN

El primer RED agregó `crates/core/tests/governed_memory_use.rs` y falló con
`E0432`: el módulo `governed_memory_use` no existía. El GREEN mínimo expuso la
frontera. Los siguientes ciclos verticales agregaron la capability opaca,
admisión ligada a U33 y las regresiones de autorización/revocación.

Cobertura ejecutable:

1. Una segunda corrida autorizada recibe provenance del receipt y el replay
   exacto conserva un único receipt idempotente U33.
2. Una revisión revocada, un scope distinto o un acceso de otro tenant no emite
   capability.
3. U33 rechaza por atestación un id de receipt fabricado y un head positivo
   incorrecto.
4. Interleavings deterministas de revocación y cambio de head ganan entre la
   primera lectura y el predicado final: ambos dejan el ledger sin receipt ni
   `VerifiedMemoryUse`; una revisión de grant reemplazada o revocada también
   falla cerrada exactamente en el límite final de la operación poseída por
   U33, sin un callback de autoridad de caller entre predicate e insert.
5. Tres doctests `compile_fail` bloquean construction literal de
   `VerifiedMemoryUse` y `MemoryUseAdmission`, además de la invocación externa
   de `MemoryUseAdmission::admit`.

Validación local:

```powershell
cargo +1.98.1 test -p improvement-engine-core --test governed_memory_use
cargo +1.98.1 test -p improvement-engine-core --lib governed_memory_use
cargo +1.98.1 test -p improvement-engine-core --doc
cargo +1.98.1 fmt --all --check
cargo +1.98.1 clippy -p improvement-engine-core --all-targets -- -D warnings
git diff --check
```

## No incluido

U22 no publica ni transforma memoria; no modifica U13 Scout, U20/U35
evaluation, Agent Core, Jev, LLM, runtime, release ni control-api. Tampoco
implementa el protocolo temporal frozen/continuous de U23. Que una corrida use
memoria es provenance, no evidencia causal de que la memoria mejoró un outcome.
