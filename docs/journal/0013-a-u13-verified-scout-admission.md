# U13-A — admisión verificada de candidatos Scout

## Decisión

`ScoutCandidateDraft` continúa siendo una salida descriptiva pública de U13,
pero deja de ser una entrada aceptable para etapas posteriores. La nueva
frontera crea `VerifiedScoutCandidate`, una capability opaca sin constructor,
`From`, serialización ni campos mutables públicos.

La composición interna `record_scout_discovery` es la única ruta que registra
un draft: vuelve a ejecutar la validación normal de `AutonomousScout` usando
un `ScoutInvocationExpectation` opaco y escribe mediante el puerto durable
interno `ScoutCandidateRepository`. El contrato exige
`record_batch_if_absent` atómico, canónico e idempotente: antes de calcular o
persistir el SHA-256, ordena los miembros por
`(candidate_id, candidate_digest, provenance_commitment)`. Así el mismo
conjunto en otro orden es el mismo replay y retorna `AlreadyRecorded`; una
identidad repetida o miembro distinto falla sin escribir ningún miembro del
batch.

`TrustedScoutCandidateAdmissionAuthority<R>` hace lookup contra ese puerto en
cada admisión; no mantiene un `BTreeMap` de proceso. Compara scope completo,
digest canónico y el registro canónico antes de emitir la capability. Repetir
la admisión es read-only y devuelve una capability equivalente para el mismo
registro durable. Las construcciones de record, batch y repositorio son
`pub(crate)`: un consumidor U14 sólo recibe `VerifiedScoutCandidate` y el
trait de admisión público, no una ruta para inyectar drafts. No publica,
promueve ni escribe un artefacto mutable.

La capability conserva de forma privada el draft canónico y scope, por lo que
liga tenant/job/grant/authority, snapshot, signal, query receipts, bindings e
intentos/salidas Core y modelo, policy y capability del modelo. Sólo expone los
accesores de lectura que U14 necesita inicialmente: scope, digest de candidato,
commitment de provenance y snapshot.

## TDD y verificación local

RED inicial, contra el head exacto de PR #42 `014fdf7`, fue el error E0432 por
los símbolos de admisión inexistentes. Después, el slice se reubicó sobre
`ba0d159` (U29, U34 y U13 ya fusionados): sólo se reaplicaron los cambios de
admisión y esta bitácora; no se reemplazaron módulos ya incorporados. GREEN
posterior:

```text
cargo +1.98.1 test --workspace
140 passed; 0 failed; 2 PostgreSQL integration tests ignored without an isolated URL

La revisión de frontera detectó además que `#[derive(Debug)]` en la capability
opaca recursaba hacia el record privado y podía publicar el draft canónico en
logs. Se añadió un segundo `compile_fail` público que intenta
`format!("{candidate:?}")`: RED con el derive (`Test compiled successfully,
but it's marked compile_fail`), GREEN al retirar `Debug`. Ningún accessor de
lectura autorizado fue retirado.

Doc-tests
2 compile_fail passed

cargo +1.98.1 fmt --all -- --check
passed

cargo +1.98.1 clippy --workspace --all-targets -- -D warnings
passed

git diff --check
passed
```

Las regresiones internas rehashan provenance público, prueban replay
read-only, rehidratación por una segunda instancia desde bytes serializados,
scope cruzado y conflicto de identidad. La rehidratación vuelve a validar cada
digest de candidato y el commitment SHA-256 de todo el batch; la corrupción de
cualquiera de los dos se rechaza. El caso de conflicto intenta escribir un
batch con un miembro nuevo seguido de otro incompatible y prueba que el
miembro nuevo no quedó persistido; otro cubre miembros duplicados. El
`compile_fail` de `VerifiedScoutCandidate` prueba que un consumidor externo no
puede importar el record constructor. El adapter serializado es sólo una
prueba del contrato durable; un adapter de producción debe satisfacer el mismo
puerto y sus invariantes transaccionales.

## Dependencia

El trabajo partió de PR #42, que ya se fusionó. Tras la revisión adversarial
final, su commit se integrará en el PR acumulativo #44; este slice no abre un
PR propio.
