# U34-F — fork de una corrida original

## Decisión

El fork de depuración es un reducer aislado que recibe un `ForkAuthorization`
con actor, grant, revisión de grant, motivo, estado y versión esperados, además de
`(replay_of,idempotency_key)`. No acepta snapshot, configuración, memoria ni
cutoff del operador. El padre sólo se registra tras una atestación contra el
repositorio de artifacts: kind, digest, tenant, cutoff, vida y final-lock se
verifican antes de conservarlo. Los cuatro valores se copian al hijo y quedan
fijados junto a `replay_of`; por tanto un fork no reescribe evidencia histórica.

La condición dinámica `final_locked` y el control state/version no entran por
el request ni por la atestación de padre: `ForkRunLifecycle` los resuelve en
cada operación, incluido retry. `ForkGrantAuthority` resuelve grant y su
revisión en el mismo borde. `ForkCommitPort` es el contrato durable: captura
un `CommitFence` inmutable con idempotency digest, grant/revisión, lifecycle
(state/version/final-lock) y las tres referencias exactas con su liveness
versionada. El adaptador durable compara *ese mismo fence* y persiste hija,
receipt y auditoría en una sola transacción/conditional write; no recompone
checks secuenciales. `InMemoryForkCommitPort` lo implementa sobre interfaces
genéricas de artifacts, policy, grants y lifecycle. Para la simulación local,
`InMemoryForkCommitPort` posee *todo* el estado mutable (artifacts, policy,
grants, lifecycle, hija, audit e idempotencia) y el `RunForkStore` interno no
expone operación pública de commit. Así `&mut self` es su lock transaccional:
no hay callbacks de una autoridad externa entre captura, compare y escrituras.
Un retry revalida liveness y devuelve
`ReferenceUnavailable` si el padre se revocó desde la primera respuesta; no
devuelve una hija que ya no sería ejecutable. Esta unidad no
afirma que ya existe el endpoint `/fork-replay`, autorización humana, PG ni un
replay E0: corresponden a U24/U34-FE y sus dependencias.

## Invariantes implementados

- La clave de idempotencia se ata al payload completo, incluido actor/grant,
  revisión de grant, motivo y control-state/version; conserva el digest SHA-256 completo, nunca
  un prefijo truncado.
- La inserción sólo ocurre tras autorización y atestación de padre en el mismo
  tenant. Un padre ajeno se ve como inexistente para no filtrar tenancy.
- Snapshot/config/memoria usan `ArtifactReference` y kind/digest exactos;
  referencia revocada, final-locked, futura o con cutoff divergente falla
  cerrado sin dejar una hija.
- Un re-registro se rechaza antes de validar o insertar el nuevo binding, de
  modo que no puede sustituir la primera atestación. Grant revocado, lifecycle
  ausente, versión/cambio concurrente o final-lock también invalidan un retry.
- Un cambio entre captura y comparación del fence falla cerrado sin hijo ni
  evento; el mismo request exitoso devuelve el receipt idéntico y no crea
  segundo hijo ni audit.
- El hijo conserva cutoff y referencias exactas del padre, incluye
  `replay_of`, y no ofrece API de mutación del padre.

## Evidencia RED → GREEN

1. El test inicial falló con `E0432` al no existir el módulo `run_fork`.
2. Se añadió el reducer mínimo y pasó la creación de hijo inmutable.
3. La revisión adversarial P1 reemplazó referencias string fabricables con
   attestation por `ArtifactRepository`, `ForkReferencePolicy` y autorización
   explícita; se agregaron receipt/event, final-lock y digest completo.
4. Se añadieron regresiones para retry/crash idempotente con revocación,
   payload alterado, control stale, autorización, padre cross-tenant, cutoff y
   final-lock.

Comando verificado:

```text
cargo +1.98.1 test -p improvement-engine-core --test run_fork
```

5. Una segunda revisión adversarial eliminó `final_locked` forjable de la
   atestación, añadió lifecycle/grant revalidables y comprobó que un duplicate
   registration no altera el primer padre.
6. La tercera revisión P1 convirtió la atomicidad declarada en `ForkCommitPort`:
   grant versionado, lifecycle y referencias se revalidan en la condición final;
   se añadieron regresiones de grant/policy/lifecycle mutables entre lectura y
   commit, y de retry con un único receipt/hijo/audit.
7. La cuarta revisión P1 reemplazó dos validaciones secuenciales por un
   `CommitFence` versionado. La regresión adversarial revoca el snapshot al
   revisar config/memory y confirma que el compare final no deja hija, receipt
   ni audit.
8. La quinta revisión P1 eliminó autoridades prestadas durante el commit. El
   puerto in-memory pasó a poseer su estado y simula cambios adversariales como
   mutaciones internas programadas; un epoch de policy hace visible incluso una
   revocación que ocurre durante la comparación final de otra referencia.
9. La revisión final P2 corrigió la documentación del puerto concreto, añadió
   un compile-fail que prueba que `RunForkStore` no es superficie pública y
   aisló una regresión de final-lock programado justo en el predicado final.

Resultado: 6 pruebas de contrato U34 verdes. Antes de integración acumulativa, un revisor
independiente debe comprobar el contrato contra U03/U15/U33 y que el adaptador
durable conserva la atomicidad declarada.
