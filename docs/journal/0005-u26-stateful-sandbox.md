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

## Paired-comparison extension

`paired_scenario` adds a pure fixture-only comparison of two caller-supplied
observations. They are not authenticated as outputs of executed sandbox
arms. The implementation status uses the same caller-supplied-observation
wording to avoid implying authenticated execution. `PairPlan` fixes
evaluation/scenario/fixture IDs, the seed digest,
baseline and candidate digests, and the exact oracle; both observations must
repeat those bindings and identify different arm IDs. An arm with a different
seed/scenario/fixture or artifact digest is rejected
before comparison (`BindingMismatch` / `ArtifactDigestMismatch`). The result
seals plan and observation digests in a receipt without exposing state values.
These digests commit to values supplied by the caller; they do not authenticate
the runner or prove fresh sandbox execution. Projection keys and values are
closed enums for minimized synthetic outcomes, not caller-provided strings.
Plan and binding IDs must use canonical SHA-256-shaped opaque-reference syntax;
the receipt and input containers deliberately do not implement `Debug`.
This rejects common raw email/phone/free-text IDs at the contract boundary,
but is not proof of anonymization: never create these references by hashing
source/customer identifiers, and do not pass source/customer IDs or payloads.
Upstream sanitization remains a caller responsibility. Receipt fields are
read-only outside the module and `validate_integrity()` recomputes a digest
over every claim. Optional fixture-evidence digest references may be empty;
they are not authenticated and do not prove an arm executed. The receipt is
marked `FixtureOnlyUnverified` so it cannot be treated as U27/Core gate evidence.

If the baseline satisfies the oracle and the candidate does not, the verdict
is `candidate_regression`; infrastructure outcomes are classified as
`failed_infra`, never as a functional regression or a pass. A baseline that
misses its oracle makes the pair `not_comparable`, even if the candidate passes.
Infrastructure failures are retained per arm, including when both arms fail.
`no_regression_observed` is only a fixture scenario result. The receipt explicitly sets
`business_lift_measured=false`: this does not estimate business improvement or
replace the full Core/U27 gate. This layer consumes evidence produced by a
runner; it does not provision a sandbox or prove execution isolation itself.
