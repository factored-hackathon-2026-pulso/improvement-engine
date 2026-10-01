# 0004 — U02: revisiones inmutables de artefactos

## Comportamiento entregado

Se implementó el puerto `ArtifactRepository`, su adaptador determinista en
memoria y `PostgresArtifactRepository`. `append` sólo acepta la siguiente
revisión cuando el `expected_head` coincide (CAS), nunca actualiza una fila
previa, comprueba el digest de contenido y valida `source_snapshot_ref`. `get`
es tenant-scoped y vuelve a comprobar digest y tipo de referencia antes de
devolver un artefacto.

La migración `migrations/0001_pulso_artifact_revisions.sql` crea únicamente
tablas Pulso; no modifica el dataset fuente. Conserva la clave compuesta
`(tenant_id, artifact_id, revision)`, una tabla de cabezas bloqueables y la
función `pulso_append_artifact_revision`. La función materializa una cabeza
sentinela y hace `SELECT ... FOR UPDATE` antes de validar CAS, secuencia y la
referencia a snapshot. Un trigger rechaza `UPDATE`/`DELETE`; el rol runtime
debe recibir únicamente `EXECUTE` sobre la función (el grant se configura en
infraestructura). `PostgresArtifactRepository` invoca esa operación y el
puerto de lectura `pulso_get_artifact_revision`, ambos `SECURITY DEFINER`; hace
readback verificado sin `SELECT` directo ni una secuencia ad-hoc de queries.

## Evidencia ejecutada

En Windows, con Rust `1.98.1`:

```text
cargo +1.98.1 fmt --all
cargo +1.98.1 test --workspace
```

Resultado: 9 pruebas Rust aprobadas (8 U02 y 1 tracer de Capa 0). Existe además
una prueba de integración PostgreSQL marcada `#[ignore]`: sólo puede ejecutarse
contra una base aislada con `PULSO_TEST_POSTGRES_URL` y
`PULSO_ALLOW_DESTRUCTIVE_TEST_DB=1`. El usuario de esa base debe poder crear un
rol efímero: la prueba configura `pulso_u02_runtime_test`, le concede sólo
`EXECUTE` sobre los dos puertos y comprueba que no puede insertar ni actualizar
tablas directamente.

## Cobertura y límites

Las pruebas cubren append/readback, CAS obsoleto, digest alterado, lectura
cross-tenant, tipo incorrecto, digest incorrecto y tenant cruzado de
`source_snapshot_ref`. La prueba PostgreSQL verifica la función CAS, la guarda
de tipo de snapshot y la inmutabilidad, pero queda explícitamente pendiente de
ejecución en la ruta de contenedores/CI. No se afirma todavía cobertura de una
carrera multi-proceso. `run_config_ref` y `parent_refs` existen en el envelope
de Capa 0 pero no son campos persistidos por U02: sus reglas de resolución son
alcance de sus slices de lineage. Agent Core, gateway de modelos y acciones
bancarias siguen fuera del slice.
