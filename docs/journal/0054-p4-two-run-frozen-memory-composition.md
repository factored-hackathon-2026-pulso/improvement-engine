# P4 — composición local de memoria Frozen entre dos runs

## Slice

En la rama `feat/p4-two-run-memory-composition`, apilada sobre la cabeza
remota de #65 `origin/feat/e0-frozen-memory-cycle` (`d154c55`), se conectó la
admisión U23-E con una lectura de memoria para el run posterior. El primer run
publica el resumen estático U15-EQ mediante la cabeza/sidecar U33-E existente;
el segundo obtiene su `VerifiedFrozenE0MemoryUse` y solo entonces puede pedir
una página con el binding exacto de run, tenant, purpose, grant/revisión,
scope, snapshot y reloj. La capability no expone bytes ni se puede construir
fuera de U23-E. Cada lectura revalida las revocaciones explícitas/tombstones
del snapshot o ancestro y la autorización vigente del grant; un nuevo head por
sí solo no invalida una publicación inmutable anterior.

Esto es una composición semántica local sobre artefactos in-memory; no se
conectó al CLI, no demuestra persistencia entre procesos y no habilita memoria
durable. `DurableFrozenE0SummaryPublicationUnavailable` continúa devolviendo
`DependencyUnavailable` también para la admisión U23-E. No se crearon tablas,
ledgers, autoridad ni recibos ficticios.

## TDD

La regresión inicial `u23e_two_run_composition_reads_the_published_summary_after_exact_admission`
falló antes de implementar la ruta con `E0599`: `read_admitted_page` no
existía. La implementación mínima guarda en el recibo interno la identidad de
grant/revisión y el reloj/cutoff ya atestados por U23-E; la nueva lectura
comprueba el binding completo y delega el mount/read al puerto scratch
autorizado.

Las regresiones cubren:

- run posterior lee el resumen publicado en la referencia exacta tras U23-E;
- drift de run, tenant, purpose, world/campaign/protocol/partition, grant
  revision, hora o snapshot no da acceso;
- revocar el grant o tombstonear el snapshot/ancestro después de admitir impide
  el mount;
- reemplazar la revisión del mismo grant entre admisión y lectura impide el
  mount con la revisión anterior;
- avanzar el head sin revocar el snapshot preserva la admisión/lectura de la
  referencia histórica publicada, si la autoridad actual aún la autoriza;
- la ruta durable de admisión mantiene `DependencyUnavailable`;
- los negativos U23-E preexistentes para cutoff, tenant/world/scope, revisión,
  revocación de snapshot/grant y reintentos siguen en la suite. El compare-and-
  swap del head sigue protegiendo nuevas publicaciones, pero head advancement
  no sustituye la revocación de una referencia anterior.

## Verificación

RED inicial:

```text
cargo +1.98.1 test --locked --offline --target-dir target-p4-two-run -p improvement-engine-core --lib --features test-support e0_frozen_memory_publication::tests::u23e_two_run_composition_reads_the_published_summary_only_through_admission
```

Falló por el comportamiento aún inexistente: `E0599`, no había método
`FrozenE0MemoryCycle::read_admitted_page`.

La revisión adversarial posterior encontró que la primera versión no consultaba
estado U33-E al leer después de la admisión. Se añadieron REDs de revocación
post-admission y de avance del head sin revocación: este último falló en ambos
bordes (admission y read) con `MemoryUseHeadConflict`, revelando una
invalidación histórica demasiado amplia. GREEN revalida publication record y
revocaciones explícitas/tombstones, vuelve a autorizar el grant y conserva
publicaciones históricas no revocadas aunque el head haya avanzado.

GREEN/verificación del código actual:

```text
cargo +1.98.1 test --locked --offline --target-dir target-p4-two-run -p improvement-engine-core --lib --features test-support e0_frozen_memory_publication::tests::u23e_
cargo +1.98.1 test --locked --offline --target-dir target-p4-two-run -p improvement-engine-core --features test-support
cargo +1.98.1 test --locked --offline --target-dir target-p4-two-run -p improvement-engine-core --features local-simulation --test local_simulation
cargo +1.98.1 clippy --locked --offline --target-dir target-p4-two-run -p improvement-engine-core --all-targets --features test-support -- -D warnings
cargo +1.98.1 fmt --all --check
git diff --check
```

Resultados anteriores al último ajuste: U23-E `15 passed`; suite
`improvement-engine-core --features test-support` pasó (126 tests unitarios,
suites de integración y 50 doctests); el target explícito `local-simulation`
pasó `10/10`; Clippy, rustfmt y diff-check pasaron. Tras el último ajuste,
U23-E pasó `19/19` sobre el diff final; la suite completa
`improvement-engine-core --features test-support` pasó (129 tests unitarios,
suites de integración y 50 doctests) antes de añadir la última regresión
específica de grant-revision replacement; el target explícito local-simulation
pasó `10/10`; Clippy con `-D warnings` pasó tras esa regresión, además de
rustfmt --check y diff-check.
La invocación más amplia `--all-features` se canceló previamente
tras más de tres minutos sin output durante contención de Cargo; no se declara
CI ni persistencia durable.

## Frontera abierta

El CLI/runtime todavía no selecciona esta composición. La lectura local es
válida solo para el contrato semántico/in-memory. Para ejecución durable,
U05 debe ofrecer la decisión de autoridad transaction-bound y U33-E debe
verificar el publication record exacto, el grant/revocación vigente y escribir
el recibo en la misma transacción PostgreSQL; avanzar el head no puede
sustituir una revocación explícita. La lectura in-memory hace una comprobación
inmediata, pero no promete atomicidad ante una revocación concurrente entre
revalidación y materialización de bytes; el adaptador durable debe cerrar esa
race dentro de la misma transacción/protocolo. El journal `0053` detalla el gap.
