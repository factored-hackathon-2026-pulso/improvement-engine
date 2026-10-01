# Capa 0 — contrato de envelope de artefactos

## Comportamiento entregado

Se agregó el contrato JSON `artifact-envelope` v1 y fixtures sintéticos para
una revisión Pulso inmutable. El envelope tiene una versión mayor explícita,
tenant opaco, identidad UUIDv7, revisión positiva, `kind`, digest SHA-256,
payload y lineage opcional. Los refs llevan tenant y el verificador rechaza
cruces de tenant, una revisión cero y una versión mayor desconocida.

También se añadieron `ArtifactRef`, `SourceRef`, `SourceSnapshot` y un único
SourceContract original de ejemplo para `call_center_interactions`. Esto
permite que otros slices citen una fila/archivo/snapshot sin copiar datasets ni
crear FKs hacia la fuente. Las instantáneas y referencias llevan `tenant_id`;
una referencia de snapshot debe pertenecer al mismo tenant. Cada fuente de una
instantánea apunta a un contrato inmutable por `id`, versión y SHA-256, no por
una ruta mutable. El contrato declara explícitamente que la fuente es readonly
y clasifica cada columna permitida como `internal`, `pseudonymized` o
`aggregated`; su fixture no contiene datos bancarios.

El contrato fuente ejecutable es JSON, no un espejo YAML: se hashea el mismo
archivo que describe el schema y se comprueba contra un golden header CSV
sintético. ADR 0002 registra el trade-off y actualiza el spec para eliminar la
doble autoridad.

El envelope es deliberadamente pequeño: no es un body de Agent Core, no
reimplementa sus primitives y no predetermina schemas de payload por familia.
Es una frontera estable para que U02/persistencia, adaptadores y equipos de
dominio puedan evolucionar en paralelo.

## Evidencia esperada del slice

1. RED: `python -m unittest discover -s tests -v` falla inicialmente porque
   `contracts.validate_fixtures` no existe.
2. GREEN: el mismo comando valida fixture positivo y los tres negativos.
3. `python contracts/validate_fixtures.py` recorre todos los fixtures sin
   requerir paquetes externos.
4. El validador semántico protege también las fronteras que los schemas v1
   exponen: propiedades inesperadas, refs de lineage duplicadas, aislamiento de
   tenant, nombre de tabla, política de clasificación aplicada a columnas y
   forma inmutable de `source_contract_ref`.

## Límites y siguiente trabajo

Esto no persiste artefactos, no calcula bytes canónicos/JCS ni valida schemas
por `kind`; esos comportamientos pertenecen a U02 y deberán añadir tests de
CAS, digest y aislamiento cross-tenant sobre PostgreSQL. Los datos fuente
siguen fuera del repositorio y no se usan en este slice. Los SHA-256 de
fixtures validan patrón, no que el contenido haya sido canonicalizado. La
clasificación es una frontera de contrato, no una autorización: el adaptador de
datos posterior deberá imponer la política y nunca cargar PII no permitida.
