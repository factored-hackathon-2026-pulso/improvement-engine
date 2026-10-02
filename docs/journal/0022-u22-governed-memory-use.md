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
llama `MemoryPublisher::record_allowed_use` y luego el puerto U33
`MemoryUseReceiptAttestationPort`: recalcula la identidad canónica del receipt
contra el head vigente, exige que exista exactamente en el ledger y revalida
snapshot vivo y autorización. U22 además liga el receipt al request completo
(scope, snapshot, run, grant, propósito e instante permitido). Un id de receipt
fabricado o un head positivo pero distinto fallan cerrados; un adapter no puede
convertir un receipt para otra corrida en capability U22.

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
   incorrecto; incluso desde composición confiable, un publisher de prueba que
   devuelve un receipt con otro run produce `ReceiptMismatch`.
4. Tres doctests `compile_fail` bloquean construction literal de
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
