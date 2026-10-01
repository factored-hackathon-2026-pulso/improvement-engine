# U08/U12 — laboratorio local y señal determinista

## Comportamiento entregado

`local_lab` construye sesiones efímeras y estrictamente acotadas por `run_id`, tenant, propósito, grant exacto, `SourceSnapshot` exacto y TTL. La fuente llega como una representación ya aprobada por un adaptador: no contiene rutas, URI, credenciales ni capacidad para importar el dataset original. Un `ApprovedLabSource` opaco sólo lo emite el seam externo de aprobación si el tenant coincide con el snapshot, la clasificación es tratada (no PII) y U03/U04 la declararon segura para discovery; el lab no posee ni puede emitir esa capability. El laboratorio carga esa representación en SQLite embebido en memoria y fija `PRAGMA query_only`; no expone la conexión ni acepta SQL crudo.

La única consulta ejecutable es un `SELECT` construido a partir de un AST pequeño: tabla y columnas allowlisted, filtro de igualdad o dependencia de un receipt anterior, y máximo de 100 filas. `source_write` y `external_io` son formas explícitas que se rechazan antes de SQLite. Un segundo `SELECT` puede usar valores de una primera salida sellada; no puede apuntar a otra sesión ni a un digest de recibo inexistente. La sesión nunca abre filesystem, red ni fuente externa.

Un `QueryReceipt` ordenado cita sesión, run, snapshot, contrato, fuente tratada, transformación, capability de aprobación, cutoff, digest de consulta, dependencia, número de filas y digest canónico de las filas. El resultado/receipt sólo entra al estado de la sesión después de que SQLite devuelve todas las filas; fallo o dependencia inválida no publica estado parcial. Un receipt dependiente se consume una sola vez y se valida contra sesión/run/snapshot/capability antes de ejecutarse. El constructor `QueryResult::untrusted` existe para cruzar la frontera de lectura, por lo que cualquier consumidor debe verificar digest del receipt y de filas antes de usarlo.

Las lecturas de receipts reciben `now`; al vencer TTL se niegan. `close` y `purge_expired` eliminan la sesión completa, incluida conexión SQLite, resultados y receipts. Cada lab recibe un nonce de instancia, por lo que dos labs con igual scope no comparten `session_id` ni receipts. El constructor agrupa su entrada en `LabSourceManifest`, encapsula el source después de validarlo, normaliza orden de tablas/filas y aplica caps explícitos de tablas, filas, valores y bytes; cada SELECT tiene orden canónico y límite de salida. El valor de filtro también tiene cap, antes de llegar a SQLite.

`deterministic_sensor` implementa el primer consumidor: una tasa booleana descriptiva. Valida secuencia y provenance iguales, cuenta numerador, denominador y faltantes sin convertir valores desconocidos en éxito/fallo, y emite cobertura en basis points con los receipts exactos. No infiere causa, prioridad ni oportunidad; U13/U14 son dueños de investigación y verificación.

## Decisiones y límites

- SQLite embebido (`rusqlite` con `bundled`) es el motor local real: elimina dependencia de una DB del host y soporta Windows/CI. U28 reemplazará este proceso por sandbox remoto; esta slice no afirma aislamiento de contenedor.
- El AST limitado es deliberado. Libertad de exploración futura no equivale a ejecutar SQL arbitrario contra fuente bancaria: U08 sólo materializa la frontera mínima reproducible y deniega egress/escritura explícitamente.
- U05 sigue siendo autoridad durable de grants/cuota y U06 de jobs/leasing. `InMemoryLabGrantAuthority` prueba únicamente el seam local exacto.
- No incluye importación del dataset, E0 (`U08-E/U12-E`), sensores de plataforma, LLM/Jev, UI, AWS, modelos, descubrimiento o propuestas.

## Evidencia TDD y verificación

El primer RED de U08 importó el módulo inexistente y describió dos consultas dependientes. Los siguientes RED cubrieron tenant/snapshot/grant ajeno, fuente PII/no aprobada, proyección vacía, escritura, URL externa, TTL/purga, digest de dependencia inválido/replay y ausencia de estado parcial. U12 arrancó con una tasa repetible con numerador/denominador/faltantes; agregó evidencia alterada (filas/receipt), receipt duplicado y columna ausente.

Comandos ejecutados en Windows antes de revisión final:

```powershell
cargo +1.98.1 test -p improvement-engine-core --test local_lab
cargo +1.98.1 test -p improvement-engine-core --test deterministic_sensor
```

Al cierre técnico de la slice, `cargo +1.98.1 fmt --all --check` y `cargo +1.98.1 clippy --workspace --all-targets -- -D warnings` pasaron. `cargo +1.98.1 test --workspace` ejecutó 81 tests Rust verdes; la única integración PostgreSQL destructiva ya existente quedó `ignored` sin su URL aislada local. Los 8 contratos Python, el validador de fixtures y `git diff --check` también pasaron. Queda la aprobación de la revisión independiente final antes de crear commit/PR.
