# U09 — tarea determinista pinneada de Agent Core

## Comportamiento entregado

`improvement_engine_core::core_task` implementa el contrato consumidor
`CoreTaskPort` y el doble `CoreTaskSimulator`. El binding conserva el agente,
release, versión de contrato y SHA inmutable de Agent Core. El corte admite
únicamente el snapshot revisado `53e729d624c8284e906249df84c1a1df84cc8d40`
con contrato `0.5.0`. Además, `CoreTaskBindingRegistry` debe aprobar el digest
exacto del binding: un SHA sintácticamente válido no autoriza una release
arbitraria y un snapshot distinto no entra al registro.

Una invocación une `tenant_id`, `job_id`, `grant_id`, `authority_ref`, binding,
input digest y `attempt_id`. La clave de attempts es `(tenant_id, job_id,
attempt_id)`: la misma identidad/payload retorna exactamente el receipt
anterior; cambiar grant, autoridad, binding o input dentro de ese scope es un
conflicto. El mismo `attempt_id` en otro tenant o job es independiente. En el
adaptador durable posterior este índice debe ser una escritura condicional
transaccional antes del dispatch, no un map en memoria.

El receipt conserva el digest exacto del binding y distingue `Succeeded` de
`Unknown`. Timeout o crash después del dispatch dejan `Unknown`; un retry del
mismo attempt devuelve ese mismo receipt y no emite una segunda ejecución.
Por tanto el doble no simula éxito, cancelación ni reintento ciego cuando la
dependencia externa puede haber recibido la tarea.

## Límite explícito

No hay Agent OS, HTTP, runtime Python, Jev, LLM, lectura de datos, creación de
agentes, scheduler, credenciales ni llamada a Agent Core. U09 consume un
contrato y entrega una costura determinista para que un bridge real haga el
transporte. El grant proviene de la autoridad/política de U05 y el job de U06;
este slice no emite ni verifica políticas externas.

## Evidencia TDD

El primer RED importó el módulo público inexistente desde
`crates/core/tests/core_task.rs`. Las pruebas posteriores cubren binding
pinneado/configurado, digest de receipt, repetición y concurrencia serializada
de attempt, mutación de scope/payload y timeout/crash post-dispatch.

Comandos ejecutados en Windows:

```powershell
cargo +1.98.1 fmt --all
cargo +1.98.1 test -p improvement-engine-core --test core_task
```

Resultado inicial: el RED falló porque `core_task` no existía. Resultado GREEN:
las cuatro pruebas U09 pasaron. El cierre de la slice exige además el suite
completo, contratos/fixtures, Clippy y una revisión independiente de runtime
AI antes del commit.
