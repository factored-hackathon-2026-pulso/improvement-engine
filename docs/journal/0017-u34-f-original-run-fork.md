# U34-F — fork de una corrida original

## Decisión

El fork de depuración es un reducer aislado que recibe un `ForkAuthorization`
con actor, grant, motivo, estado y versión esperados, además de
`(replay_of,idempotency_key)`. No acepta snapshot, configuración, memoria ni
cutoff del operador. El padre sólo se registra tras una atestación contra el
repositorio de artifacts: kind, digest, tenant, cutoff, vida y final-lock se
verifican antes de conservarlo. Los cuatro valores se copian al hijo y quedan
fijados junto a `replay_of`; por tanto un fork no reescribe evidencia histórica.

La condición dinámica `final_locked` y el control state/version no entran por
el request ni por la atestación de padre: `ForkRunLifecycle` los resuelve en
cada operación, incluido retry. La autoridad de grant también se vuelve a
consultar antes de cada resultado. `RunForkStore` es el contrato ejecutable en memoria. El adaptador durable debe
hacer en una sola transacción el lookup de idempotencia, revalidación de
disponibilidad/revocación/final-lock, control-version, inserción de hija y
receipt/event de auditoría. Un retry revalida liveness y devuelve
`ReferenceUnavailable` si el padre se revocó desde la primera respuesta; no
devuelve una hija que ya no sería ejecutable. Esta unidad no
afirma que ya existe el endpoint `/fork-replay`, autorización humana, PG ni un
replay E0: corresponden a U24/U34-FE y sus dependencias.

## Invariantes implementados

- La clave de idempotencia se ata al payload completo, incluido actor/grant,
  motivo y control-state/version; conserva el digest SHA-256 completo, nunca
  un prefijo truncado.
- La inserción sólo ocurre tras autorización y atestación de padre en el mismo
  tenant. Un padre ajeno se ve como inexistente para no filtrar tenancy.
- Snapshot/config/memoria usan `ArtifactReference` y kind/digest exactos;
  referencia revocada, final-locked, futura o con cutoff divergente falla
  cerrado sin dejar una hija.
- Un re-registro se rechaza antes de validar o insertar el nuevo binding, de
  modo que no puede sustituir la primera atestación. Grant revocado, lifecycle
  ausente, versión/cambio concurrente o final-lock también invalidan un retry.
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

Resultado: 7 pruebas verdes. Antes de integración acumulativa, un revisor
independiente debe comprobar el contrato contra U03/U15/U33 y que el adaptador
durable conserva la atomicidad declarada.
