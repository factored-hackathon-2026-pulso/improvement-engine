# 0005 — I02: caller CI PostgreSQL reutilizable

## Comportamiento entregado

El workflow `rust-ci` conserva su matriz portátil de formato, lint, pruebas
Rust y contratos. Además ejecuta el job
`postgres-artifact-migration`, que llama exclusivamente al workflow reusable
de `pulso-factored/infra` fijado al commit
`8f6df2e1916480b4dcacc5e60094d0f2c76d69d2`.

El workflow llamado hace checkout de este repositorio y ejecuta la única
prueba de migración PostgreSQL ignorada de U02 contra su contenedor efímero.
El caller no transmite secretos, no construye un segundo servicio PostgreSQL,
no define URL/consentimiento destructivo y no convierte el gate en opcional.
El job se activa con la misma política de PR y `main` que el CI existente.

## Evidencia de TDD

1. Se añadió primero `tests/test_postgres_ci_caller_contract.py`. El primer
   ciclo falló porque `postgres-artifact-migration` no existía en `ci.yml`.
2. El caller mínimo con SHA y permiso `contents: read` dejó el contrato en
   verde. La prueba también bloquea secretos, una URL/consentimiento local,
   una imagen PostgreSQL duplicada y bypasses de error en el caller.
3. La ejecución remota del PR es la evidencia necesaria de que GitHub permite
   el reusable workflow privado y de que la migración se ejecuta de verdad;
   una comprobación local de YAML no equivale a ese gate.

## Límite y operación

Si el SHA debe actualizarse, el cambio se revisa como actualización explícita
de dependencia de infraestructura, con su nuevo commit y su nueva CI. No se
usa una rama, tag mutable ni `secrets: inherit`. El acceso de Actions entre los
dos repositorios continúa siendo un prerrequisito administrado por GitHub.
