# Capa 0 — workspace Rust

## Comportamiento entregado

El repositorio tiene ahora un workspace Cargo con una librería `improvement-engine-core`. Su tracer público `service_name()` prueba que el workspace compila y expone una identidad de core estable. La crate no contiene contratos de dominio, detección, persistencia, ejecución de Agent Core, routing de proveedores ni llamadas externas.

## Evidencia TDD

1. El test de integración `crates/core/tests/tracer.rs` se añadió primero.
2. `cargo test --workspace` falló porque no existía un manifest de workspace.
3. Se añadió el manifest mínimo, manifest de crate y función pública.
4. `cargo +1.98.1 test --workspace` pasó en Windows: un test de integración pasó.

## Reproducibilidad y CI

- `rust-toolchain.toml` fija exactamente Rust `1.98.1`, perfil mínimo, `rustfmt` y `clippy`.
- `Cargo.lock` se versiona aunque esta capa no tenga dependencias externas.
- `.github/workflows/ci.yml` valida formato, lint, harness Rust y el harness Python de fixtures de contratos en Windows y Linux. Usa checkout fijado por SHA y permiso de contenido de sólo lectura.
- El workflow no despliega, accede a secretos, invoca modelos ni contacta sistemas bancarios.

## Comandos ejecutados

```powershell
cargo +1.98.1 fmt --all --check
cargo +1.98.1 clippy --workspace --all-targets -- -D warnings
cargo +1.98.1 test --workspace
python -m unittest discover -s tests -p "test_*_contract.py" -v
python contracts/validate_fixtures.py
```

Todos pasaron localmente en Windows. CI se validará contra el SHA del PR; el resultado local no es evidencia de una ejecución remota.

## Límites y siguiente frontera

La capa 0 no implementa U02 ni persiste artefactos. Las interfaces compartidas viven en `contracts/`; cada slice futuro introduce lógica de dominio mediante su propio comportamiento observable y pruebas. Agent Core, gateway de modelos, datos fuente y contenedores siguen fuera de esta crate.
