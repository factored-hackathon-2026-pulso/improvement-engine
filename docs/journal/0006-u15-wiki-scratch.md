# 0006 — U15: wiki autorizada en scratch efímero

## Comportamiento entregado

`improvement_engine_core::wiki_scratch` implementa el contrato de U15. Un
`memory_wiki` es una revisión inmutable U02 con payload cerrado:
`available_at_unix_seconds`, `purpose` y `pages`. El montaje recibe un
`WikiAccess` que fija run, tenant, propósito, grant, referencia exacta
`(tenant, id, revision, digest)` y reloj máximo del run. Primero pasa por
`WikiAuthorizationPort`; después resuelve la revisión desde el repositorio,
vuelve a comparar la referencia, exige `ArtifactKind::MemoryWiki`, valida el
payload y corta antes de leer páginas si `available_at` está en el futuro.

El `workspace_id` es determinista sobre run y contexto de montaje: dos runs
con la misma wiki no comparten scratch ni receipt. Tanto `read` como
`transform` revalidan también el mismo corte temporal, así que no puede
reutilizarse un workspace ya montado para leer desde un reloj anterior.

El adaptador local `InMemoryWikiGrantAuthority` sólo admite grants previamente
emitidos que coincidan de forma exacta en run, tenant, propósito y snapshot. Es una
costura ejecutable y estrecha para U05, **no** un proveedor de identidad ni una
política de producción. `read` y `transform` vuelven a llamar al puerto de
autorización; por tanto, montar una vez no convierte el workspace en una
capacidad libre.

`WikiWorkspace` mantiene un mapa de páginas sólo en memoria. No acepta rutas
absolutas, barras invertidas, componentes vacíos, `.` ni `..`. `transform`
aplica `create`, `replace` y `remove` tipados sobre una copia y sólo sustituye
el scratch al terminar toda la lista: un fallo no deja un resultado parcial.
Los receipts y digests deterministas citan workspace y snapshot exactos. Este
slice no tiene API de publicación, CAS, head, revocación, filesystem host,
red, runtime de agentes ni efectos externos.

## Evidencia TDD ejecutada

1. Se creó primero `crates/core/tests/wiki_scratch.rs` contra el módulo público
   inexistente.
2. El RED ejecutado con Rust 1.98.1 falló como correspondía:
   `could not find wiki_scratch in improvement_engine_core`.
3. Se implementó el puerto, el adaptador determinista y el decoder de payload
   cerrado.
4. Al hacer GREEN, la primera fixture llevaba un UUID cuya versión estaba en el
   segmento equivocado; la corrección se hizo en el test para respetar la
   validación U02. Otra prueba reveló que el scope se comprobaba antes del
   grant: se ordenó la revalidación para negar primero una autorización no
   válida en cada read/transform.
5. Pasaron siete pruebas específicas en Windows:

```text
cargo +1.98.1 fmt --all --check
cargo +1.98.1 test -p improvement-engine-core --test wiki_scratch
cargo +1.98.1 clippy -p improvement-engine-core --all-targets -- -D warnings
```

Cubren montaje/transformación y fuente inmutable, tenant/purpose/grant/run
incorrectos, snapshot futuro, digest/referencia no autorizados, traversal POSIX
y Windows (`C:/...`) y fallo atómico, y determinismo de los digests de
resultado/receipt.

## Revisión adversarial independiente

La primera ronda detectó un fallo de compilación introducido al agregar
`run_id`, grants no ligados al run, rutas Windows de drive aceptadas y la falta
de una comparación de `run_id` del workspace. Se añadieron regresiones para
montaje/read/transform cross-run con una segunda autoridad válida, rutas
`C:/...`, `x:...` y UNC; `WikiWorkspace` ahora conserva el run y la frontera
lo compara tras validar grant. La re-revisión independiente confirmó que una
autoridad válida para `run-2` no accede al scratch de `run-1` y no encontró
P0/P1. La verificación completa posterior pasó: formato, Clippy con warnings
denegados, workspace Rust (32 tests ejecutados y una integración PG preexistente
ignored), siete contratos Python, fixtures y `git diff --check`.

## Integración y límites explícitos

U05 es dueño del lifecycle, revocación, cuota y autoridad real de grants. Al
integrarse reemplazará `InMemoryWikiGrantAuthority` por un adaptador de
`WikiAuthorizationPort`; no se cambiará el hecho de que todas las operaciones
reciben `WikiAccess` y vuelven a validarlo. U33 recibirá el diff/manifest de
scratch y será el único dueño de crear una nueva revisión durable, mover heads
por CAS o propagar revocación. Ninguna transformación U15 constituye memoria
publicada ni conocimiento de runtime.
