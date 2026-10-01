# U04 — proyección temporal y sellada del histórico enriquecido E0

## Comportamiento implementado

`improvement_engine_core::enriched_history::EnrichedHistoryAdapter` recibe un
`EnrichedHistoryManifest` y filas que un llamador ya autorizado suministra. No
abre rutas, Parquet ni el paquete E0; tampoco transmite sus filas, modifica
fuentes o llama a Agent Core/modelos. Esto conserva el límite de acceso de
U03/E01: el adapter ejecuta controles sobre una proyección en memoria, no una
conexión de datos.

Cada tabla sellada preserva `source_namespace`, `world_ref`,
`observed_cutoff` y cuatro digests independientes: archivo, esquema,
transformación y política. `from_snapshot` además vincula `source_namespace`,
`world_ref` y corte al `SourceSnapshot` inmutable existente; una sustitución de
namespace, mundo o corte falla antes de exponer filas.

Como corrección mínima de integración U03/U04, `SourceProvenance` ahora expone
el `source_namespace` que ya validaba `SourceSnapshot`. No se cambió el formato
de snapshot ni se creó una fuente paralela: se hace visible un componente de su
identidad para que U04 pueda comparar el sello completo.

Para descubrimiento se bloquean las tablas precomputadas `labels` y `signal`.
Una normalización case/underscore-insensible bloquea claves que contienen
`label` o `signal`, que comienzan con `final` o `expected`, y
`human_errors`/`stress_test`, incluso dentro de objetos o arrays anidados. Cada
tabla declara la disponibilidad sellada de cada campo/grupo superior: una fila
con un campo que no fue declarado o cuya disponibilidad es posterior al corte
falla antes de exponerse. Cada fila también debe tener `event_time` e
`ingested_at` UTC válidos, ambos no posteriores al corte; así un evento ocurrido
antes pero disponible después tampoco aparece. La disponibilidad del archivo y
de cada grupo se valida al construir el adapter.

Los desacuerdos de los cuatro digests son `QualityFinding` deterministas en
orden archivo, esquema, transformación y política, dentro de un error tipado
`QualityBlocked`. No se devuelve una tabla ni filas en ese caso: un consumidor
no puede ignorar accidentalmente una bandera de elegibilidad. No se convierten
en una aceptación silenciosa ni en una señal precomputada. Errores estructurales,
tabla sin sello, fuga, reloj ausente/inválido/futuro y proveniencia incompatible
son errores explícitos sin filas parciales.

## Evidencia TDD

1. Se escribió `crates/core/tests/enriched_history.rs` antes del módulo; la
   ejecución RED falló con `could not find enriched_history`.
2. Se implementó el adapter mínimo y luego el enlace explícito a
   `SourceSnapshot`.
3. Las pruebas cubren manifest/proveniencia válida, corte de archivo y filas,
   señales/labels/finales anidados (incluidos arrays) prohibidos, drift como
   finding que bloquea exposición de filas, relojes imposibles, disponibilidad de cada grupo,
   errores deterministas, orden estable y sustitución de snapshot.
4. Comandos ejecutados tras la implementación:

   ```powershell
   cargo +1.98.1 fmt --all
   cargo +1.98.1 clippy -p improvement-engine-core --all-targets -- -D warnings
   cargo +1.98.1 test -p improvement-engine-core --test enriched_history
   ```

   La prueba específica terminó verde con 9 pruebas. La verificación completa
   pasó: 35 pruebas Rust, 7 pruebas Python y el validador de fixtures; una
   prueba PostgreSQL destructiva existente quedó `ignored` sin su URL aislada.
   Formato, Clippy y `git diff --check` también pasaron. Dos revisiones
   adversariales independientes cerraron los hallazgos de corte por campo,
   fuga anidada, proveniencia, namespace y clocks imposibles.

## Límites explícitos

- U04 no autoriza acceso al paquete `pulso_muestra_e0`, a datasets históricos,
  PII, credenciales ni almacenamiento de filas.
- No implementa U08-E/U12-E, replay, evaluación completa ni detección. El
  `signal.parquet` del paquete es expresamente una entrada prohibida para
  descubrimiento.
- El formato actual de reloj exige UTC fijo (`YYYY-MM-DDTHH:MM:SSZ`) para que
  la comparación sea determinista y sin parser permisivo. Si se requieren
  offsets o fracciones, se deberá versionar el contrato y usar un parser de
  tiempo validado antes de ampliar la aceptación.
