# CLAUDE.md — micli

## Estado actual del proyecto

**Fases 0–3 completadas.** `core` (máquina de estados + requirement),
`trace` (JSONL append-only con hash encadenado y `verificar()`), `manifest`
(agent_rules.toml con plantillas por perfil, requirements.toml, registro
global) y `store` (proyección SQLite reconstruible) están implementados y
testeados. El binario `micli` ya tiene `init --profile`, `status` (estado
derivado del trace) y `trace verify`; el ámbito global respeta `MICLI_HOME`.
**Fase 4 completada.** `agent` implementa el bucle de intentos (escalado
1 Directo, 2–3 ConFeedback, 4 ContextoLimpio+Hipótesis, 5 DiagnósticoPrevio;
prompts versionados con hash; presupuestos duros; `ClaudeCodeCli` sobre
`claude -p --output-format json`; traits `Ejecutor`/`EjecutorGates`
inyectables). El binario tiene el pipeline completo: `req add` / `req
avanzar` (transiciones humanas), `run` (bucle del agente → trace con
payloads Intento/GateResultado), `db rebuild` (trace → SQLite) y `init`
genera `prompts/sistema.md`. **Las 6 fases del plan están completadas.** Resumen por crate:
`core` (máquina de estados con `transition()` + proptest), `trace` (JSONL
append-only con hash encadenado; verificado también contra manipulación
real), `manifest` (agent_rules.toml con plantillas por perfil,
requirements.toml con módulos, registro global), `store` (proyección SQLite
reconstruible), `agent` (bucle de intentos con escalado, presupuestos duros,
prompts versionados con hash, `ClaudeCodeCli`), `sandbox` (Docker vía
bollard; regla #4 verificada inspeccionando `cuerpo_contenedor()`; tests
reales `#[ignore]`, corren con `cargo test -p micli-sandbox -- --ignored`).
El binario expone: `init`, `status` (avance con barras y por módulo),
`dashboard` (multi-proyecto), `req add`/`req avanzar`, `run [--sandbox]`,
`trace verify` y `db rebuild`.

Pendientes conocidos (mejoras, no fases): checkpoints de git por transición,
hooks de Claude Code generados por `init` (regla `protected` en proyectos
objetivo, hoy solo mecánica en este repo), el agente mismo dentro del
sandbox (hoy solo los gates), gate de baseline de tests (regla #2) y DB
índice global como caché del dashboard (hoy lee cada trace directo).

Plan de fases: 0 bootstrap → 1 `core` → 2 `manifest` + `trace` + `store` →
3 CLI mínimo (`init`, `status`, `trace verify`) → 4 `agent` → 5 `sandbox` →
6 visualización de avance.

## Qué es este proyecto

`micli` es un arnés (harness) de orquestación en Rust para agentes de código,
**multi-proyecto**: se instala una vez y se registra en cada desarrollo
(Angular, FastAPI, Rust, Node, …). Envuelve a Claude Code: gestiona una
máquina de estados por requerimiento, impone compuertas de calidad
declarativas por proyecto, registra trazas inmutables y muestra el avance por
módulo y por proyecto. El agente escribe código; `micli` decide qué se acepta.

Principio rector: **el cumplimiento no depende del LLM**. Toda regla crítica
se implementa como código Rust determinista fuera del alcance del agente,
nunca solo como instrucción en un prompt.

## Dos niveles de reglas (no confundir)

1. **Reglas de este repo** (cómo se desarrolla micli): todo lo que sigue en
   este documento — Rust, convenciones, reglas duras.
2. **Reglas que micli impone a los proyectos objetivo**: son configurables,
   viven en el `agent_rules.toml` de cada proyecto registrado, con perfiles
   predefinidos (`rust`, `angular`, `python-fastapi`, `node`). micli es
   agnóstico al lenguaje de los proyectos que orquesta.

## Comandos

```bash
cargo build --workspace            # compilar todo
cargo test --workspace             # correr toda la suite
cargo test -p <crate>              # tests de un crate
cargo clippy --workspace --all-targets -- -D warnings   # lint (cero warnings)
cargo fmt --all                    # formatear antes de cada commit
cargo deny check                   # licencias, advisories, deps prohibidas
cargo run -p micli -- <subcomando> # probar el CLI localmente
```

Un cambio no está terminado si `cargo test`, `clippy`, `fmt --check` o
`cargo deny check` fallan.

## Estructura del workspace

```
crates/
  micli/        # binario: parsing de args (clap), wiring, salida a terminal
  core/         # máquina de estados, modelo de requerimientos, transiciones
  manifest/     # registro de proyectos, perfiles, agent_rules.toml, requirements.toml
  trace/        # log de eventos append-only con hash encadenado (fuente de verdad)
  store/        # proyección SQLite derivada del trace, siempre reconstruible
  sandbox/      # ejecución aislada en Docker vía bollard, imagen por proyecto
  agent/        # integración con Claude Code (modo no interactivo, hooks)
```

Reglas de dependencia entre crates: `core` no depende de nadie del workspace.
`micli` es el único que conoce a todos. `sandbox`, `trace`, `manifest`,
`store` y `agent` solo dependen de `core`. No introducir dependencias
circulares.

## Máquina de estados (corazón del sistema)

Estados de un requerimiento y transiciones válidas:

```
Draft → SpecAprobada → TestRojo → Implementando → Verificando → Aceptado
                                        ↑               │
                                        └─ (gate falla) ─┘
Bloqueado: alcanzable desde cualquier estado activo; requiere intervención humana.
```

- Las transiciones viven ÚNICAMENTE en `core::state`, vía `transition()`, que
  valida o devuelve error. Ningún otro módulo muta estado directamente.
- Usar typestate donde aporte: estados como tipos para que transiciones
  inválidas no compilen.
- Invariantes verificadas con property-based testing (`proptest`), p. ej.:
  ninguna secuencia de eventos llega a `Aceptado` sin pasar por `TestRojo`.

## Persistencia y auditoría (event sourcing)

- **Fuente de verdad**: el trace JSONL append-only por proyecto, con hash
  encadenado: `hash(n) = H(hash(n-1) || evento(n))`. Alterar un evento pasado
  invalida toda la cadena posterior; `micli trace verify` lo comprueba.
- **SQLite (`store`) es proyección derivada**: se construye leyendo el trace y
  se puede regenerar desde cero (`micli db rebuild`). Nunca es fuente de
  verdad. Modo WAL, escrituras transaccionales.
- Esquema base: `requirements`, `transitions` (ancladas al hash del evento),
  `attempts` (intento, resultado, tokens, duración), `gate_results`.
- Ámbitos de datos:
  - Global (`%APPDATA%\micli\` / `~/.micli/`): registro de proyectos, DB
    índice para el dashboard multi-proyecto (también proyección).
  - Por proyecto (`.micli/` en cada repo): su trace, su SQLite, sus manifests.
    Cada proyecto es autocontenido.

## Gates declarativos por proyecto

Ejemplo de `agent_rules.toml` de un proyecto objetivo:

```toml
version = 1

[project]
name = "sync-backend"
profile = "python-fastapi"

[[gates]]
name = "tests"
cmd = "pytest -x --tb=short"
required = true

[[gates]]
name = "lint"
cmd = "ruff check ."
required = true

[protected]
paths = ["tests/fixtures/**"]            # el agente no puede tocarlas
dependency_files = ["requirements.txt"]  # editar requiere aprobación

[sandbox]
image = "python:3.12-slim"
network = "none"

[budget]
max_attempts = 5
max_minutes = 30
```

Al registrar un proyecto (`micli init --profile <perfil>`), micli genera en
ese repo los hooks de Claude Code que hacen cumplir `protected` de forma
mecánica.

## Estrategia de agente (Fase 4)

- Contexto fresco por intento: el intento N recibe la spec + resumen
  estructurado del fallo anterior, nunca la transcripción completa.
- Escalado entre intentos: directo → con feedback del fallo → contexto limpio
  + hipótesis → diagnóstico antes de tocar código. Máximo 5 intentos; después
  el requerimiento pasa a `Bloqueado` con resumen de lo intentado.
- Prompts de sistema versionados en el repo; su hash queda en el trace para
  poder comparar efectividad entre versiones.
- El LLM nunca es compuerta: reviews de IA son advisory y se registran en el
  trace, pero la aceptación la deciden solo los gates deterministas.
- Presupuestos duros (tokens, tiempo, intentos) impuestos por micli en Rust.
- Checkpoint de git por transición de estado para rollback determinista.

## Convenciones de código

- Errores: `thiserror` en librerías, `anyhow` solo en el binario `micli`.
- Prohibido `unwrap()` y `expect()` fuera de tests (clippy lo impone; en
  tests está permitido vía `clippy.toml`).
- Prohibido `unsafe` en todo el workspace (`unsafe_code = "forbid"`).
- Todo struct serializado a disco (`serde`) lleva `#[serde(deny_unknown_fields)]`
  y un campo `version` para migraciones futuras.
- Rutas: usar `std::path::PathBuf`, nunca concatenar strings.
- Nada de `println!` fuera del crate `micli` (clippy `print_stdout`); los
  demás crates devuelven datos o usan `tracing`.
- Tests de lógica central acompañados de propiedades `proptest` cuando haya
  invariantes que declarar.

## Reglas duras y su enforcement

Cada regla dura tiene (o tendrá) un mecanismo determinista. El texto de este
documento es la intención; el mecanismo es la garantía.

| # | Regla | Mecanismo | Estado |
|---|-------|-----------|--------|
| 1 | No agregar dependencias sin proponerlo y esperar aprobación explícita | Hook `PreToolUse` fuerza aprobación al editar `Cargo.toml`; `cargo deny check` en CI | **Activo** |
| 2 | No debilitar, borrar ni `#[ignore]` tests existentes para hacer pasar un cambio; si un test parece incorrecto, señalarlo y detenerse | Gate en CI comparará conteo de tests e `#[ignore]` nuevos contra baseline | Pendiente (Fase 3) |
| 3 | No tocar `fixtures/` de tests de integración salvo pedido explícito | Hook `PreToolUse` niega Write/Edit sobre `fixtures/` | **Activo** |
| 4 | `sandbox` nunca monta credenciales, env del host ni el socket de Docker | Tests inspeccionan `cuerpo_contenedor()` (la spec exacta que recibe Docker) | **Activo** |
| 5 | Eventos de `trace` son append-only: nunca editar ni borrar entradas | Archivo solo en modo append + `micli trace verify` valida la cadena | **Activo** |

Los hooks viven en `.claude/hooks/` y se configuran en `.claude/settings.json`.

## Flujo de trabajo esperado

1. Antes de escribir código, presentar un plan corto: qué archivos se tocan,
   qué tests se agregan, qué riesgos hay. Esperar aprobación.
2. Trabajar en tareas atómicas: un requerimiento = un diff pequeño y revisable.
   Si la tarea es más grande, proponer cómo partirla en lugar de empezar.
3. Para bugs: primero escribir un test que reproduzca el fallo (debe estar en
   rojo), después arreglar.
4. Para features: el cambio debe incluir tests nuevos que cubran el
   comportamiento agregado. Sin tests no hay feature completa.
5. Si después de 5 intentos los tests no pasan, detenerse, resumir qué se
   intentó y qué se sospecha, y pedir ayuda. No seguir iterando a ciegas.

## Definición de terminado

- [ ] `cargo test --workspace` en verde
- [ ] `cargo clippy` sin warnings
- [ ] `cargo fmt --check` limpio
- [ ] `cargo deny check` limpio
- [ ] Tests nuevos o modificados cubren el cambio
- [ ] Diff pequeño, sin archivos ni dependencias no relacionados con la tarea
- [ ] Sin `unwrap`, `expect`, `unsafe`, `todo!()` ni código muerto nuevo
