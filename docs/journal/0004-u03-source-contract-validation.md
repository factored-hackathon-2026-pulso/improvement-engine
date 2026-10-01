# U03 — validación inmutable de contratos y snapshots de fuente

## Alcance entregado

`improvement-engine-core::source_validation` carga los `SourceContract` JSON canónicos en orden determinista de nombre de archivo. Antes de validar bytes rechaza contratos o snapshots parciales: campos desconocidos, versión mayor distinta de 1, contrato no `original`/`read_only`, taxonomías inválidas, URI/digest/referencia mal formados y tablas duplicadas. También exige que la referencia sellada tenga el `id` y la versión mayor del contrato recibido. Para una fuente que un adaptador ya autorizó, compara:

1. el SHA-256 de la representación JSON exacta del contrato contra la referencia fijada en `SourceSnapshot`;
2. los nombres y el orden del header CSV contra las columnas del contrato;
3. el SHA-256 del header CSV contra el snapshot;
4. el SHA-256 de los bytes completos contra el snapshot; y
5. cada clasificación de columna contra `access_policy.permitted_classifications`.

El resultado conserva `tenant_id`, `world_ref` y `observed_cutoff` del snapshot. Las findings se emiten siempre en este orden: contrato, estructura de header, digest de header, archivo y luego columnas en el orden del contrato. El header de esta capa es intencionalmente simple: UTF-8, sin BOM ni quoting y separado por comas; los identificadores de columnas del contrato hacen suficiente esa regla. No se interpreta ni se transmite contenido a un modelo o a un servicio externo.

El fixture `contracts/fixtures/sources-v1/validation/` es completamente sintético y su CSV se marca `-text` en `.gitattributes`: el digest prueba los bytes exactos en Windows y Linux sin conversión automática de fin de línea.

## Evidencia TDD

1. Se añadieron primero las pruebas de contrato para coincidencia golden, deriva de header, deriva de archivo, clasificación no permitida, deriva de contrato y carga determinista.
2. La ejecución RED falló porque no existían el módulo público ni los fixtures.
3. Se añadieron fixture sintético, snapshot sellado, cargador y verificador mínimo.
4. Una revisión adversarial detectó que el parser aceptaba contratos/snapshots parciales y no vinculaba `id`/versión de la referencia; se añadieron pruebas RED para contrato writable/campos desconocidos, snapshot duplicado/desconocido y referencia incorrecta.
5. `cargo +1.98.1 test -p improvement-engine-core --test source_validation` pasó con nueve pruebas en Windows.

## Decisiones y límites

- El contrato canónico sigue el ADR 0002: JSON es la única representación ejecutable y el digest se calcula sobre sus bytes exactos.
- La entrada es `&[u8]`; abrir una ruta, credenciales, PII, dataset histórico, escritura de estado, Agent Core y gateway de modelos están fuera de este slice.
- Un mismatch genera una finding de calidad, no una excepción ni una falsa aceptación. Errores estructurales (contrato/snapshot inválido, namespace distinto, referencia incoherente, snapshot sin tabla o fuente sin header) devuelven error explícito.

## E01 — trabajo futuro explícito

E01 implementará un adaptador de datos real aprobado: autenticación y política de acceso, selección de tenant/world/cutoff, streaming con límites, creación y persistencia inmutable de snapshots, y pruebas de integración contra un entorno sandbox. E01 no podrá usar este fixture como evidencia de acceso al dataset real.
