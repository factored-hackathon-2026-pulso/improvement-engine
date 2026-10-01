# 0005 — U26: sandbox stateful para evaluación

## Comportamiento entregado

`improvement_engine_core::sandbox` entrega `SandboxPort` y
`StatefulSandbox`, un adaptador determinista de memoria para **fixtures
sintéticos**. Un `SandboxFixture` fija tenant, namespace, recursos semilla y
acciones permitidas. La primera llamada de `start_arm` sella el fixture exacto
para `evaluation_id`; los demás brazos de esa evaluación deben recibir el mismo
fixture o fallan con `FixtureMismatch`. Cada brazo identificado por
`(evaluation_id, arm_id)` clona esa semilla: baseline y candidate no comparten
estado ni pueden partir de universos distintos.

Un `ActionRequest` lleva una `SandboxScope` tenant/namespace y se valida antes
de intentar un efecto. `reset_arm` exige la misma scope. Estas son aserciones
de scope del evaluador, **no una identidad/autoridad bancaria**: U36 será dueño
de la policy e identidad real. Su `Action::replace` sólo cambia una clave que
exista en el fixture y sólo si su nombre está permitido. Exige
`expected_revision`, de modo que una acción obsoleta no cambia el estado. Un `ReadRequest` vuelve a comprobar tenant y
namespace antes de devolver `Readback { value, state_revision }`; el readback
es la evidencia de efecto, no un HTTP 200 ni una afirmación del ejecutor.
`reset_arm` vuelve a la semilla del brazo, elimina sus receipts de acciones y
devuelve `ResetReceipt` con evaluación, brazo, fixture, revisión anterior y
`state_revision` posterior, y digest determinista del estado sellado tras el
reset. `ActionReceipt`,
`Readback` y `ResetReceipt` incluyen evaluación/brazo/fixture para que la
evidencia pueda mostrar qué fixture sintético se ejecutó.

Los fallos son tipados: brazo desconocido, fixture inválido, tenant/namespace
denegado, recurso desconocido, acción prohibida, revisión obsoleta y conflicto
de `action_id`. Un retry idéntico devuelve el receipt ya emitido sin aplicar un
segundo efecto; reutilizar su id con otro payload falla explícitamente.

## Evidencia TDD ejecutada

Primero se añadió `crates/core/tests/sandbox.rs`. El RED falló con
`could not find sandbox in improvement_engine_core`. Luego se implementó el
puerto y el adaptador mínimo. En Windows, con Rust 1.98.1, pasaron:

```text
cargo +1.98.1 fmt --all
cargo +1.98.1 test -p improvement-engine-core --test sandbox
cargo +1.98.1 clippy -p improvement-engine-core --all-targets -- -D warnings
```

Las ocho pruebas cubren efecto/readback autorizado, fixture trazable,
aislamiento y reset de
brazos, aislamiento tenant/namespace, rechazo sin efecto de acciones
forbidden/unknown/stale y retry idempotente.

## Límites deliberados

No hay servicio bancario, Agent Core, red, filesystem, credenciales, PII ni
persistencia en esta unidad. La referencia de brazo es opaca y el adaptador
vive sólo durante la evaluación local. U19/U20/U27 deberán montar este puerto
por brazo/repetición desde un manifest sellado y distinguir sus errores de
infraestructura de un resultado funcional; esta prueba unitaria no acredita
una evaluación Core ni una mejora de negocio.
