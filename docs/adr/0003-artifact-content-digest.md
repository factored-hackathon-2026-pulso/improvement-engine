# ADR 0003: digest de contenido de artefacto en U02

## Decisión

U02 calcula `artifact.digest` como `sha256:<hex>` del JSON compacto y
determinista de `{kind, payload, source_snapshot_ref}`. No incluye `tenant_id`,
`id`, `revision` ni el digest mismo. Se verifica antes de guardar y otra vez al
leer. Una referencia a fuente debe tener el mismo tenant, resolver exactamente
por `(tenant, id, revision, digest)` y apuntar a `source_snapshot`.

## Razón

El digest necesita representar contenido inmutable sin tener una dependencia
circular sobre sí mismo. Excluir identidad y revisión permite detectar el mismo
contenido en revisiones distintas; el índice de persistencia conserva aun así
su historia por tenant, identidad y revisión.

## Consecuencias y límite

La serialización es la de `serde_json` para estos tipos y no pretende ser JCS
interoperable todavía. Cambiarla exige una nueva versión de contrato y una ruta
de migración: nunca se recalculan revisiones existentes. La implementación U02
es un adaptador en memoria determinista; la migración PostgreSQL queda incluida,
pero no se ejecutó localmente porque el runtime de contenedores tiene el bloqueo
de cgroups documentado en la bitácora del proyecto.

PostgreSQL valida forma, CAS, inmutabilidad y referencias; no replica la
serialización de Rust para recalcular el digest. El límite de confianza actual
es `PostgresArtifactRepository`: valida digest antes de llamar al puerto SQL y
vuelve a validarlo al leer. El rol runtime sólo recibe `EXECUTE` sobre los
puertos `SECURITY DEFINER`, por lo que no puede insertar una revisión por DML.
Exponer las funciones a una frontera no confiable requerirá una versión futura
con canonicalización verificable en base de datos o una firma de servidor.
