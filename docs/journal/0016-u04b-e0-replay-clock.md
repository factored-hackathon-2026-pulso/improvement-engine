# U04-B — reloj explícito para replay E0 sin `ingested_at`

## Decisión y alcance

El histórico enriquecido E0 no trae una columna física `ingested_at`. Forzarla
en el adapter mezclaba una convención de replay con evidencia operativa y
bloqueaba el paquete correcto. `AvailabilityClockMode` y el
`AvailabilityProfile` V2 hacen la distinción parte del manifest sellado y de la
proveniencia que recibe cada consumidor:

- `observed_ingested_at` exige `event_time` e `ingested_at` en cada fila. Es el
  contrato para fuentes que sí demuestran disponibilidad observada.
- `replay_at_event_time` exige sólo `event_time` y una etiqueta de supuesto
  versionable. Por definición, asume disponibilidad en el instante del evento
  con `ingestion_lag=0`; no demuestra la latencia ni el orden de ingestión de
  producción.

No existe un constructor implícito entre ambos modos para replay. El modo de
replay rechaza tanto un `ingested_at` declarado en `field_availability` como
una fila que lo introduzca. La etiqueta se valida como identificador versionable
(minúsculas, dígitos y `_`). De esa forma, un paquete no puede afirmar a la vez
que usa el supuesto E0 y que cuenta con reloj físico.

`AvailabilityProfile` tiene `profile_id`, versión, modo, etiqueta y el digest
del byte-stream exacto de `SourceSnapshot`; su propio digest cubre todos esos
campos. El adapter V2 de replay sólo se crea con `from_snapshot`: rechaza la
lectura vía `from_manifest` porque aún no tendría un snapshot con el cual
comparar el enlace. Cambiar modo, supuesto o snapshot después de sellar el
perfil invalida el contrato. La proveniencia de cada tabla expone el perfil que
se usó.

La migración es deliberadamente estrecha: manifest unversioned/N-1 se trata
como V1 `observed_ingested_at`, para no romper el histórico que sí trae ese
reloj cuando se abre vía `from_manifest`. V1 no puede expresar replay ni
lleva un compromiso con el byte-stream de `SourceSnapshot`; por ello
`from_snapshot` lo rechaza cerrado. Sin ese compromiso no puede demostrar que
un snapshot con igual namespace, mundo, corte y archivos pertenece al mismo
tenant. Todo adapter unido a snapshot requiere V2 y un `AvailabilityProfile`
válido; su `source_snapshot_digest` cubre los bytes del snapshot, incluido
`tenant_id`. No hay fallback de E0 a V1 ni una migración implícita de V1 al
camino snapshot-bound.

## Controles que permanecen intactos

El modo E0 no relaja ninguna frontera de descubrimiento. El `event_time` debe
seguir siendo UTC válido y no posterior al `observed_cutoff`; la disponibilidad
sellada de archivo y de todos los campos sigue limitada por el mismo corte; los
digests, namespace/mundo/snapshot y calidad continúan siendo obligatorios. Las
tablas `labels`/`signal` y cualquier campo final, expected, signal o label
anidado continúan bloqueados antes de exponer filas. Un snapshot de bytes
posterior no habilita eventos, campos o resultados futuros.

Además, cada fila E0 lleva metadatos de disponibilidad por campo fuera de la
proyección visible. Su digest canónico, junto con las filas, debe coincidir con
el digest sellado del archivo. Todos sus `available_at` deben ser UTC válidos y
no posteriores al `event_time` de esa misma fila; no basta con que sean
anteriores al cutoff global. Esto impide que un atributo aparecido después del
contacto entre retrospectivamente al contexto de esa decisión.

## Evidencia TDD y validación

1. Se añadió primero la prueba de un manifest `replay_at_event_time` con fila
   E0 sin `ingested_at`; falló en rojo por no existir tipo, constructor ni
   proveniencia del modo.
2. Se implementó el tipo sellado, el constructor explícito y la validación por
   modo; la prueba quedó verde.
3. Una segunda prueba roja mostró que el manifest de replay todavía aceptaba
   declarar un reloj físico. Se añadió el rechazo tipado
   `UnexpectedAvailabilityClock` antes de abrir filas.
4. La revisión adversarial detectó dos P1: atributos posteriores al evento y
   modo mutable sin compromiso con snapshot. Se añadieron el digest de
   proyección fila/campo y el perfil V2 ligado a bytes de snapshot; una lectura
   replay sin `from_snapshot`, una mutación de modo o un snapshot con idéntica
   proveniencia pero bytes diferentes son rechazados.
5. Las regresiones cubren corte futuro, disponibilidad posterior por campo,
   fuga final, N/N-1, modo/etiqueta inválidos, rechazo de `ingested_at` físico
   en manifest/fila y las garantías U04 previas.
6. Una segunda revisión encontró que el perfil aún no probaba que cada archivo
   del paquete perteneciera al snapshot. `SourceSnapshot::source_file_seal`
   expone ahora una vista read-only, sin constructor público, que compromete
   tabla, URI, `file_digest`, `header_digest` y referencia/digest del contrato.
   `from_snapshot` exige un seal idéntico para **cada** `PackageFile`, además
   de igualdad del `file_digest` de la proyección. Tabla no listada, seal
   ausente y cualquier divergencia fallan cerrados antes de abrir filas.
7. Una nueva auditoría señaló que V1 aún podía unirse a dos snapshots de
   tenants distintos si sus demás metadatos y archivos coincidían. Se eligió
   compatibilidad fail-closed: V1 sigue disponible sólo por `from_manifest`;
   `from_snapshot` devuelve `SnapshotBindingUnavailable`. Las regresiones
   incluyen tenants A/B indistinguibles fuera de `tenant_id`, URI distinto con
   los mismos digests y un seal de otra tabla. El perfil V2 ya fija el digest
   de bytes completos del snapshot, por lo que su enlace incorpora tenant y
   los seals siguen fijando tabla/URI/contrato por archivo.
8. La revisión final observó que los campos de identidad de `SourceSnapshot`
   todavía eran mutables públicamente después de parsear sus bytes. Ahora son
   privados y sólo se exponen por getters; un doctest `compile_fail` cubre los
   cuatro intentos de mutación (`tenant_id`, namespace, mundo y cutoff). El
   perfil V2 también carga `source_tenant_id`, incluido en su digest, y
   `from_snapshot` compara explícitamente ese valor además del digest de bytes.
   Un manifest V2 de tenant A contra un JSON idéntico de tenant B falla cerrado
   antes de que se abra cualquier proyección.
9. Dos regresiones menores cerraron los seams de serialización: un
   `SourceSnapshot` obtenido por deserialización directa no tiene digest de
   bytes canónico y `from_snapshot` lo rechaza antes de evaluar el manifest;
   y `source_file_seal` se omite de JSON, por lo que un `PackageFile`
   round-trip queda sin capability y devuelve `MissingSourceFileSeal`. Un
   intento de inyectar ese campo en JSON es desconocido bajo
   `deny_unknown_fields` y falla al deserializar, nunca se interpreta como un
   seal válido.

Comandos verdes:

```powershell
cargo +1.98.1 fmt --all
cargo +1.98.1 test -p improvement-engine-core --test enriched_history
cargo +1.98.1 clippy -p improvement-engine-core --all-targets -- -D warnings
```

El modo no implementa aún el protocolo frozen/continuous (U23), ni permite
reclasificar resultados posteriores como inputs disponibles. Es únicamente el
contrato de disponibilidad E0 que esos consumidores deberán respetar.
