# 0036 — U36: identidad y policy en el banco sandbox

## Objetivo y límite

U36 fortalece el adaptador sintético de U26, no construye autenticación bancaria
ni un servicio de atención. Un fixture que declara `SandboxIdentityPolicy` exige
`IdentityEvidence` actual para ejecutar una acción, leer su resultado o resetear
el brazo. La frontera valida tenant, caso, canal, principal permitido, digest de
policy, digest del conjunto de preguntas y vigencia **antes** de consultar o
mutar el estado del brazo.

`IdentityEvidence` contiene únicamente IDs y compromisos (`*_digest`): no hay
campo para respuestas de preguntas, texto de reto, tokens de cliente ni output
de modelo, y deliberadamente no implementa `Debug`. No tiene constructor
público: la composición confiable usa `start_protected_arm` para recibir el
`SandboxIdentityIssuer` opaco, ligado al brazo con un nonce privado del
`StatefulSandbox`. Un `SandboxArmRef` público por sí solo no puede derivar ese
issuer ni llamar a `issue_identity`; el issuer determina internamente el único
brazo para el cual puede emitir. El sandbox crea otro nonce opaco y conserva el
proof canónico en estado privado del brazo. Un issuer de `candidate` no puede
emitir para `baseline`. La frontera rechaza proof no registrado, alterado o
emitido para otro brazo, aunque el caller conozca todos los IDs y digests de la
policy.
Los `ActionReceipt` y `Readback` existentes continúan registrando solamente la
evidencia sintética de efecto (evaluación/brazo/fixture/revisión), no evidencia
de autenticación ni respuestas.

Los fixtures heredados sin `SandboxIdentityPolicy` se mantienen como fixtures
genéricos U26; no representan una operación sensible E0. Las suites que
evalúen identidad deben usar explícitamente `with_identity_policy`, evitando
que el modo de compatibilidad sea una afirmación de seguridad.

## Comportamiento entregado

- `SandboxIdentityPolicy` sella caso, canal, digests, principals permitidos y
  vencimiento con el fixture.
- `ActionRequest::with_identity` y `ReadRequest::with_identity` presentan
  evidencia; el instante no procede de requests. `StatefulSandbox` consulta un
  `SandboxClock` interno (inyectable sólo al componer el sandbox),
  de modo que action/read/reset no pueden falsificar un timestamp. Una petición
  sin evidencia sobre fixture protegido devuelve `IdentityEvidenceMissing`.
- Un tenant/caso/canal/policy/preguntas/principal distinto devuelve
  `IdentityEvidenceMismatch`; el vencimiento de la evidencia o de la policy
  devuelve `IdentityEvidenceExpired`. Ninguno cambia estado.
- `revoke_identity` registra el commitment de identidad sólo en el brazo
  indicado. Tras revocación, la misma evidencia no puede ejecutar ni hacer
  readback en ese brazo (`IdentityRevoked`), mientras un brazo baseline del
  mismo fixture continúa aislado y utilizable.

`revoke_identity` es una semántica local, en memoria y **no autorizada** del
fixture para inyectar un evento de revocación durante la evaluación. No es una
API de revocación bancaria ni acredita quién podría emitirla. U20-E y el broker
de sandbox posterior deberán decidir la autoridad administrativa, clock sellado
y persistencia de auditoría. Esta unidad no declara revocación de credenciales
de producción.

## Evidencia TDD

El test RED inicial no compilaba porque todavía no existían policy, evidence,
constructores autenticados ni errores U36. Luego se incorporó la frontera
mínima y en Windows/Rust 1.98.1 pasaron:

```text
cargo +1.98.1 fmt --all -- --check
cargo +1.98.1 test -p improvement-engine-core --test sandbox_identity
cargo +1.98.1 test -p improvement-engine-core --test sandbox
cargo +1.98.1 test -p improvement-engine-core --doc sandbox
cargo +1.98.1 clippy --workspace --all-targets -- -D warnings
```

Las pruebas nuevas cubren ausencia de evidencia, cada componente de binding
equivocado, que un `SandboxArmRef` sin capability no puede cruzar la frontera,
reemisión con issuer cruzado (imposible también por firma), proof cruzado de
brazo para action/read/reset,
vencimiento de evidencia, acción y readback válidos, reloj interno para
action/read/reset, revocación en vuelo y aislamiento candidate/baseline. Un
doctest `compile_fail` comprueba que el proof no puede construirse ni
formatearse con `Debug`. `StatefulSandbox` ya no tiene `Default`: cada
composición debe declarar un clock; `FixedSandboxClock` se limita a fixtures
locales explícitas.
