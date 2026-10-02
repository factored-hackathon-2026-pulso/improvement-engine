# U13-E — admisión Scout autenticada desde señal E0

## Propósito y límite

U13-E convierte una señal diagnóstica U12-E ya sellada en borradores Scout
durables, sin tratar una señal pública genérica como evidencia de mejora. La
composición de confianza es crate-private: `TrustedE0ScoutComposer` recibe el
`E0DiagnosticSignal` inmutable, el receipt U09 (`CoreTaskReceipt`) y el
receipt U10 (`ModelReceipt`) y emite `AuthenticatedE0ScoutSignal`, una
capability opaca sin constructor, serialización ni `Debug` público.

La composición revalida tenant, grant, autoridad, snapshot U04, binding crudo,
perfil de disponibilidad, cutoff, proyección/evidencia fuente y receipts de
consulta E0. También conserva el `run_id` de U08-E, que no se confunde con el
`job_id` de U13. El consumo posterior exige que los receipts U09/U10 continúen
ligados a la capability y al signal commitment exacto.

`AutonomousScout::discover_e0` produce únicamente los tres borradores
descriptivos (`Signal`, `Claim`, `Opportunity`). Cada uno persiste una
`E0ScoutCandidateProvenance` tipada y canónica: scope completo, run E0,
métrica/policy/semántica, agregados y ventana, snapshot/binding/profile,
cutoff, fuente/transform/proyección/evidencia y receipts. El digest del
borrador cubre esa proveniencia; la rehidratación la vuelve a validar antes de
aceptar el registro.

`record_e0_scout_discovery` usa el batch y el repositorio atómico existentes de
U13-A. Por tanto U14 recibe únicamente `VerifiedScoutCandidate` después de un
registro canónico: ni un caller ni una deserialización de un borrador pueden
saltar la admisión.

## Invariantes

- La ruta no acepta `QueryResult`, `DeterministicSignal` ni strings de scope
  aportados por el caller. Tampoco invoca `ScoutInvocationAuthority` ni
  `AutonomousScout::discover` genérico.
- Drift de scope, receipt, snapshot, binding, cutoff, profile, fuente o
  commitment E0 falla cerrado antes de persistir.
- El signal E0 y la proveniencia contienen sólo agregados y commitments; no
  filas, PII, SQL ni receipts mutables.
- La persistencia es el boundary U13-A existente: batch completo, canónico,
  idempotente y read-only en admisión. El capability resultante no implementa
  `Debug`.
- La slice no registra detectores, promueve/libera cambios, ejecuta runtime o
  Agent Core, ni afirma causalidad, outcome o éxito de negocio.

## RED → GREEN

El RED final intentó comparar el resultado de admisión con `assert_eq!`; falló
porque `VerifiedScoutCandidate` no implementa `Debug`, que es una frontera de
no exposición intencional. El GREEN verifica el rechazo con `matches!` sin
ampliar la superficie pública. Las regresiones cubren la cadena real
U12-E → capability opaca → receipts U09/U10 → batch U13-A → capability
verificada U14, drift de scope/receipt y adulteración de proveniencia E0
rehashada.

## Validación ejecutada

```powershell
cargo +1.98.1 fmt --all -- --check
cargo +1.98.1 test --locked -p improvement-engine-core --lib autonomous_scout --features test-support
cargo +1.98.1 test --locked -p improvement-engine-core --doc autonomous_scout
cargo +1.98.1 clippy --locked -p improvement-engine-core --all-targets --features test-support -- -D warnings
git diff --check
```

Los cuatro comandos Rust anteriores quedaron verdes en esta rama. La suite
workspace completa se deja como verificación posterior de integración, no como
una afirmación de runtime o release.

## Dependencias y fuera de alcance

Issue: #53. U12-E provee la señal descriptiva; U09/U10 son receipts existentes;
U13-A conserva el único boundary durable de admisión. Un adapter de control
plane/Agent Core futuro deberá preservar este contrato, no sustituirlo por
señales o receipts caller-provided.
