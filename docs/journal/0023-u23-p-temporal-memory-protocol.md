# U23-P — protocolo temporal Frozen/Continuous de memoria gobernada

## Propósito y límite

Esta slice implementa únicamente el borde temporal que faltaba entre U04-B y
la admisión gobernada U22/U33. No es el U23 completo: no hay runner E0,
`CampaignManifest`, escenarios, puntuación, resultado de evaluación, publicación,
runtime, Scout, caché ni páginas de wiki.

`TemporalMemoryEvidence` sustituye los instantes construibles por el llamador.
La emite únicamente el composition root confiable de U04-B o del futuro runner:
lleva un commitment con nonce que fija tenant, scope completo, snapshot, head,
run, grant, propósito, instante autorizado, cutoff, protocolo y, para
Continuous, provenance y disponibilidad del outcome. El protocolo nunca abre
archivos de fuente ni interpreta labels/outcomes.

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
U22, valida el evidence opaco y lo incorpora al receipt canónico U33. U33
recalcula la identidad incluyendo ese commitment además de scope, snapshot,
head, run, grant, purpose y reloj autorizado. Un cutoff/outcome/timestamp
fabricado, feedback futuro o protocolo cruzado falla antes de registrar un
receipt; no se emite `VerifiedMemoryUse`.

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
