# U24 — consola técnica read-only: primer corte vertical

## Comportamiento entregado

`improvement_engine_core::debug_console` aporta la frontera de lectura que
usará el `control-api` interno y una UI técnica posterior. `DebugConsole`
consume exclusivamente el contrato U07 (`RunActivityReadModel` y
`RunActivityHandler`) y devuelve un `ConsoleTimeline` acotado. Cada evento
incluye sólo el identificador validado, clase, instante y digest de evidencia,
además de un resumen textual accesible. No devuelve payload de evidencia,
fuente, SQL, prompts, artefactos ni datos de clientes.

`DebugConsoleApi` es un adaptador de aplicación sin framework HTTP. No acepta
un `ListRunActivityRequest` ni un tenant del caller: recibe una request de
sesión y una request de timeline sin tenant, y pide a `DebugIdentityPort` una
principal autenticada. El puerto recibe por llamada un `DebugViewerIssuer`
opaco que sólo crea la API; sin esa capability no se puede fabricar un
`DebugViewer`. El adaptador deriva entonces el `AuthenticatedTenant` U07
internamente. Así se conserva U07 para otros transportes, pero la frontera de
debug no permite inyectar `tenant_b` en una request.

La composición (`DebugConsoleApi`, `DebugIdentityPort`, issuer y principal)
permanece `pub(crate)` hasta que exista el `control-api` real que la componga
con SSO/OIDC. Esta decisión evita que un consumidor externo seleccione un
`DebugIdentityPort` propio y reciba el issuer; no presentamos un mock de
identidad como endpoint utilizable. El futuro adapter deberá aterrizar en una
slice explícita y preservar esta capability de composición.

Los doctests `compile_fail` bloquean explícitamente desde un crate consumidor
el import/implementación de `DebugIdentityPort`, la construcción de
`DebugViewer`, `DebugAuthenticationRequest` y `DebugTimelineRequest`, y el
import/construcción del issuer o de `DebugConsoleApi`. No son tipos de request
de una API externa: sólo el futuro adapter autenticado podrá traducir el
transporte a esta composición interna.

Conserva el cursor opaco de U07 y mapea los fallos a estados de transporte
seguros. En particular, un cursor vencido o purgado es `Gone`, sin reconstruir
una línea de tiempo ni afirmar que el run terminó; un run inexistente es
`NotFound`, sin proyección adjunta. `accessible_status_summary()` ofrece texto
estable para éxito (count/continuation), `Gone`, `NotFound` y `BadRequest`,
sin serializar errores crudos, tenant, run o evidencia.

La frontera no expone operaciones para proyectar eventos, mutar runs, ejecutar
herramientas, leer PG/S3 directamente ni crear exportaciones. Emitir un cursor
es el único estado local y efímero, propiedad de U07; una lectura no cambia la
revisión de la proyección.

## RED → GREEN ejecutado

1. El primer test importó `debug_console::DebugConsole` inexistente y falló con
   `E0432`.
2. La implementación mínima añadió la proyección segura de una timeline
   tenant-bound y el test pasó.
3. Se añadieron regresiones para reanudación por cursor opaco/cross-tenant,
   gap al cambiar la revisión y respuesta `NotFound` sin mutar la proyección.
4. Una revisión adversarial detectó que el primer adaptador aceptaba un DTO U07
   con constructor público. El RED siguiente exigió autenticación mediante
   principal/capability y resúmenes accesibles; la regresión ahora prueba que
   no existe un parámetro para inyectar tenant, que un cursor cross-tenant se
   bloquea y que los mensajes de estado no revelan el error interno.

Comando ejecutado en Windows:

```powershell
cargo +1.98.1 test -p improvement-engine-core --test debug_console
```

Resultado actual: 6 pruebas verdes.

## Límites y trabajo posterior

Este corte no reclama una UI browser, HTTP/SSO reales, SSE persistente,
proyección PG, grafo de jobs, OpenTelemetry, consultas/model calls, evaluación,
memoria, exportaciones ni comandos de operador. Esos detalles exigen los
contratos/productores correspondientes y no se inventan a partir de la
actividad U07. El browser futuro consumirá este adaptador desde `control-api`;
nunca obtendrá acceso directo al dataset, PG/S3 o SQL arbitrario.
