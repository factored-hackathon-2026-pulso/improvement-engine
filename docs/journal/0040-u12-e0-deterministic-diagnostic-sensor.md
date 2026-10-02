# U12-E — sensor diagnóstico determinista sobre evidencia E0

## Propósito y límite

U12-E calcula una tasa booleana descriptiva únicamente a partir de uno o más
`VerifiedE0QueryResult` opacos. Esa capability sólo puede existir después de
que U08 recupera su candidate desde el ledger vivo y U04-B verifica la
proyección replay. Por ello este sensor no acepta `QueryResult`, filas,
receipts ni digests aportados por el caller.

La señal inmutable conserva numerador, denominador, missingness, ventana,
cutoff y los compromisos exactos de tenant, snapshot U04, perfil de
disponibilidad, tabla, contrato/fuente/transform, campos, proyección replay,
evidencia fuente y receipts. Es una descripción de evidencia observada: no
es label, outcome, explicación causal, claim, oportunidad ni decisión de
release.

## Invariantes

- La ventana virtual es inclusiva, termina en o antes del cutoff sellado y
  cada `event_time` debe estar dentro de ella.
- Todas las evidencias agregadas deben preservar el mismo commitment U04 y el
  mismo tenant, sesión/run/grant/authority/source approval y snapshot U08.
- El receipt debe seguir ligando exactamente sus filas y haber leído tanto el
  campo métrico como el reloj; campos no leídos —incluido `label`— fallan
  cerrados.
- La salida expone sólo digests de receipts, no filas ni receipts mutables, y
  nunca autoriza ejecución o release.

## Fuera de alcance

No abre fuentes, ejecuta SQL, llama LLM/Jev/Agent Core, infiere labels u
outcomes, crea Scout/claim/oportunidad, evalúa candidatos, persiste señales,
ni escribe/publica/libera cambios. El `DeterministicSensor` previo permanece
sin cambios: consume `QueryResult` público y no es evidencia E0 autenticada.

## RED → GREEN

El RED inicial fue el import de `e0_deterministic_sensor` inexistente desde
un test público de contrato (`E0432`). El GREEN agrega una ruta E0 separada y
regresiones internas que construyen la cadena U04 fixture → U08 ledger
governed candidate → capability E0 antes de medir. Cubren cálculo reproducible
con commitments, ventana/cutoff, drift de proyección, campo no leído y
denegación de labels; un doctest `compile_fail` impide entregar un
`QueryResult` público al sensor E0.

## Validación prevista

```powershell
cargo test -p improvement-engine-core e0_deterministic_sensor
cargo test -p improvement-engine-core --test e0_deterministic_sensor
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```
