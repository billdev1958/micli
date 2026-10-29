//! Implementación de los subcomandos. Cada función devuelve el texto a
//! imprimir; `main` es el único punto de salida a terminal.

use crate::avance::{Avance, barra, por_modulo};
use crate::rutas;
use anyhow::{Context, bail};
use micli_agent::{
    ClaudeCodeCli, Desenlace, Ejecutor, EjecutorGates, GateSpec, GatesEnHost, PlanEjecucion,
    Presupuesto, PromptVersionado, ejecutar_requerimiento,
};
use micli_core::{Event, RequirementId, State, transition};
use micli_manifest::{AgentRules, Perfil, Registry, RequirementsFile};
use micli_store::{AttemptRow, GateResultRow, Store, TransitionRow};
use micli_trace::{Payload, Trace, TraceEntry};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

/// Prompt de sistema por defecto. `init` lo materializa en
/// `.micli/prompts/sistema.md` para que cada proyecto lo versione y ajuste;
/// su hash SHA-256 queda en el trace con cada intento.
const PROMPT_SISTEMA_DEFAULT: &str = "\
Eres un agente de código operando dentro del arnés micli.

Reglas no negociables:
- Trabaja únicamente en el requerimiento indicado; no toques nada más.
- No debilites, borres ni marques con ignore tests existentes. Si un test \
parece incorrecto, detente y explícalo en tu respuesta final.
- No toques fixtures de tests ni archivos de dependencias.
- El cambio debe ser el mínimo que satisfaga la spec y ponga los tests en \
verde.

La aceptación la deciden gates deterministas (tests, lint) que corren \
después de tu intento; tu autoevaluación no cuenta.
";

/// `micli init`: genera `.micli/` y registra el proyecto en el ámbito global.
pub fn init(proyecto: &Path, name: Option<String>, profile: Perfil) -> anyhow::Result<String> {
    let proyecto = rutas::ruta_legible(
        proyecto
            .canonicalize()
            .with_context(|| format!("no existe el directorio {}", proyecto.display()))?,
    );
    let name = match name {
        Some(n) => n,
        None => proyecto
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .context("no se pudo derivar el nombre del proyecto; usa --name")?,
    };

    // Primero se valida el alta global en memoria (rechaza duplicados)…
    let ruta_registro = rutas::registro_global()?;
    let mut registro = Registry::cargar(&ruta_registro)?;
    registro.registrar(name.clone(), proyecto.clone())?;

    // …luego se generan los archivos locales (fallan si ya existen)…
    let dir = rutas::dir_micli(&proyecto);
    let reglas = AgentRules::plantilla(name.clone(), profile);
    reglas.guardar(&dir.join(rutas::ARCHIVO_REGLAS))?;
    RequirementsFile::vacio().guardar(&dir.join(rutas::ARCHIVO_REQUIREMENTS))?;
    let ruta_prompt = dir.join(rutas::ARCHIVO_PROMPT_SISTEMA);
    if let Some(padre) = ruta_prompt.parent() {
        std::fs::create_dir_all(padre)?;
    }
    std::fs::write(&ruta_prompt, PROMPT_SISTEMA_DEFAULT)?;
    // El trace nace vacío; se crea el archivo (solo-append) desde ya para
    // que quede visible y `trace verify` funcione sin eventos.
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(dir.join(rutas::ARCHIVO_TRACE))?;

    // …y solo al final se persiste el registro global.
    registro.guardar(&ruta_registro)?;

    Ok(format!(
        "Proyecto `{name}` registrado en {dir}.\n\
         Revisa y ajusta agent_rules.toml (gates, protected, budget) y \
         prompts/sistema.md antes de usar el agente.",
        dir = dir.display(),
    ))
}

pub(crate) fn cargar_proyecto(proyecto: &Path) -> anyhow::Result<AgentRules> {
    let ruta = rutas::dir_micli(proyecto).join(rutas::ARCHIVO_REGLAS);
    if !ruta.exists() {
        bail!(
            "este directorio no está inicializado como proyecto micli (falta {}); corre `micli init --profile <perfil>`",
            ruta.display()
        );
    }
    Ok(AgentRules::cargar(&ruta)?)
}

/// Reproduce el trace y devuelve el estado actual de cada requerimiento.
fn estados_desde_trace(
    entradas: &[TraceEntry],
) -> (HashMap<String, State>, HashMap<String, String>) {
    let mut estados: HashMap<String, State> = HashMap::new();
    let mut titulos: HashMap<String, String> = HashMap::new();
    for entrada in entradas {
        match &entrada.payload {
            Payload::RequirementCreado { id, titulo } => {
                estados
                    .entry(id.as_str().to_owned())
                    .or_insert(State::Draft);
                titulos.insert(id.as_str().to_owned(), titulo.clone());
            }
            Payload::Transicion {
                requirement_id,
                hacia,
                ..
            } => {
                estados.insert(requirement_id.as_str().to_owned(), *hacia);
            }
            Payload::Intento { .. } | Payload::GateResultado { .. } => {}
        }
    }
    (estados, titulos)
}

/// `micli status`: perfil, eventos del trace y estado actual de cada
/// requerimiento, derivado reproduciendo el trace (la fuente de verdad).
pub fn status(proyecto: &Path) -> anyhow::Result<String> {
    let reglas = cargar_proyecto(proyecto)?;
    let dir = rutas::dir_micli(proyecto);
    let declarados = RequirementsFile::cargar(&dir.join(rutas::ARCHIVO_REQUIREMENTS))?;
    let entradas = Trace::abrir(dir.join(rutas::ARCHIVO_TRACE))?.entradas()?;
    let (mut estados, mut titulos) = estados_desde_trace(&entradas);

    let mut filas: Vec<(String, String, State, String)> = Vec::new();
    for req in &declarados.requirements {
        let id = req.id.as_str().to_owned();
        let estado = estados.get(&id).copied().unwrap_or(State::Draft);
        filas.push((id.clone(), req.modulo.clone(), estado, req.titulo.clone()));
        estados.remove(&id);
        titulos.remove(&id);
    }
    // Requerimientos que aparecen en el trace pero no están declarados.
    let mut extra: Vec<(String, String, State, String)> = estados
        .into_iter()
        .map(|(id, estado)| {
            let titulo = titulos.remove(&id).unwrap_or_default();
            (
                id,
                String::new(),
                estado,
                format!("{titulo} [no declarado]"),
            )
        })
        .collect();
    extra.sort_by(|a, b| a.0.cmp(&b.0));
    filas.extend(extra);

    let avance = Avance::desde(filas.iter().map(|f| f.2));
    let mut salida = format!(
        "Proyecto: {} (perfil {:?})\nTrace: {} eventos\nAvance: {} {}% ({}/{} aceptados, {} bloqueados)\n",
        reglas.project.name,
        reglas.project.profile,
        entradas.len(),
        barra(&avance, 20),
        avance.porcentaje(),
        avance.aceptados,
        avance.total,
        avance.bloqueados,
    );

    if filas.is_empty() {
        salida.push_str("Sin requerimientos declarados.\n");
    } else {
        salida.push_str("\nPor módulo:\n");
        let grupos = por_modulo(filas.iter().map(|f| (f.1.as_str(), f.2)));
        for (modulo, avance) in &grupos {
            let _ = writeln!(
                salida,
                "  {modulo:<14} {} {:>3}% ({}/{})",
                barra(avance, 20),
                avance.porcentaje(),
                avance.aceptados,
                avance.total,
            );
        }
        salida.push_str("\nRequerimientos:\n");
        for (id, modulo, estado, titulo) in filas {
            let modulo = if modulo.trim().is_empty() {
                crate::avance::MODULO_GENERAL
            } else {
                &modulo
            };
            let _ = writeln!(
                salida,
                "  {id:<12} {modulo:<14} {:<24} {titulo}",
                estado.to_string()
            );
        }
    }
    Ok(salida)
}

/// `micli dashboard`: avance de todos los proyectos registrados en esta
/// instalación. Cada proyecto es autocontenido: se lee su propio trace.
pub fn dashboard() -> anyhow::Result<String> {
    let registro = Registry::cargar(&rutas::registro_global()?)?;
    if registro.projects.is_empty() {
        return Ok(
            "No hay proyectos registrados. Corre `micli init --profile <perfil>` en cada proyecto."
                .to_owned(),
        );
    }
    let mut salida = format!("Proyectos registrados: {}\n\n", registro.projects.len());
    for proyecto in &registro.projects {
        match avance_de_proyecto(&proyecto.path) {
            Ok(avance) => {
                let _ = writeln!(
                    salida,
                    "  {:<16} {} {:>3}% ({}/{} aceptados, {} bloqueados)  {}",
                    proyecto.name,
                    barra(&avance, 20),
                    avance.porcentaje(),
                    avance.aceptados,
                    avance.total,
                    avance.bloqueados,
                    proyecto.path.display(),
                );
            }
            Err(error) => {
                let _ = writeln!(salida, "  {:<16} (no accesible: {error})", proyecto.name);
            }
        }
    }
    Ok(salida)
}

/// Avance de un proyecto: requerimientos declarados (Draft si no tienen
/// eventos) más los que solo existen en el trace.
fn avance_de_proyecto(proyecto: &Path) -> anyhow::Result<Avance> {
    let dir = rutas::dir_micli(proyecto);
    if !dir.join(rutas::ARCHIVO_REGLAS).exists() {
        bail!("no existe {}", dir.display());
    }
    let declarados = RequirementsFile::cargar(&dir.join(rutas::ARCHIVO_REQUIREMENTS))?;
    let entradas = Trace::abrir(dir.join(rutas::ARCHIVO_TRACE))?.entradas()?;
    let (mut estados, _) = estados_desde_trace(&entradas);
    let mut todos: Vec<State> = Vec::new();
    for req in &declarados.requirements {
        todos.push(estados.remove(req.id.as_str()).unwrap_or(State::Draft));
    }
    todos.extend(estados.into_values());
    Ok(Avance::desde(todos))
}

/// `micli trace verify`: valida la cadena completa de eventos.
pub fn trace_verify(proyecto: &Path) -> anyhow::Result<String> {
    cargar_proyecto(proyecto)?;
    let trace = Trace::abrir(rutas::dir_micli(proyecto).join(rutas::ARCHIVO_TRACE))?;
    let verificadas = trace.verificar().context("la cadena del trace está rota")?;
    Ok(format!("OK: cadena íntegra ({verificadas} eventos)"))
}

/// `micli req add`: registra en el trace (nace en `Draft`) un requerimiento
/// ya declarado en `requirements.toml`.
pub fn req_add(proyecto: &Path, id: &str) -> anyhow::Result<String> {
    cargar_proyecto(proyecto)?;
    let dir = rutas::dir_micli(proyecto);
    let declarados = RequirementsFile::cargar(&dir.join(rutas::ARCHIVO_REQUIREMENTS))?;
    let spec = declarados
        .requirements
        .iter()
        .find(|r| r.id.as_str() == id)
        .with_context(|| {
            format!("`{id}` no está declarado en requirements.toml; agrégalo ahí primero")
        })?;

    let mut trace = Trace::abrir(dir.join(rutas::ARCHIVO_TRACE))?;
    let (estados, _) = estados_desde_trace(&trace.entradas()?);
    if estados.contains_key(id) {
        bail!("`{id}` ya está registrado en el trace");
    }
    trace.append(Payload::RequirementCreado {
        id: spec.id.clone(),
        titulo: spec.titulo.clone(),
    })?;
    Ok(format!("`{id}` registrado en Draft."))
}

/// `micli req avanzar`: aplica una transición humana (aprobar spec, confirmar
/// test rojo, bloquear, desbloquear) y la persiste en el trace.
pub fn req_avanzar(proyecto: &Path, id: &str, evento: Event) -> anyhow::Result<String> {
    cargar_proyecto(proyecto)?;
    let dir = rutas::dir_micli(proyecto);
    let mut trace = Trace::abrir(dir.join(rutas::ARCHIVO_TRACE))?;
    let (estados, _) = estados_desde_trace(&trace.entradas()?);
    let desde = *estados
        .get(id)
        .with_context(|| format!("`{id}` no está en el trace; corre `micli req add {id}`"))?;
    let hacia = transition(desde, evento)
        .with_context(|| format!("transición rechazada desde `{desde}`"))?;
    trace.append(Payload::Transicion {
        requirement_id: RequirementId::new(id),
        evento,
        desde,
        hacia,
    })?;
    Ok(format!("`{id}`: {desde} -> {hacia}"))
}

/// Adaptador: corre los gates dentro del sandbox de Docker. Vive aquí porque
/// `sandbox` solo depende de `core` y no conoce el trait de `agent`.
struct GatesEnSandbox {
    sandbox: micli_sandbox::Sandbox,
}

impl EjecutorGates for GatesEnSandbox {
    fn ejecutar(
        &mut self,
        gate: &GateSpec,
        _dir: &Path,
    ) -> Result<micli_agent::ResultadoGate, micli_agent::AgentError> {
        let resultado = self.sandbox.ejecutar(&gate.cmd).map_err(|e| {
            micli_agent::AgentError::GateNoEjecutable {
                gate: gate.name.clone(),
                fuente: std::io::Error::other(e.to_string()),
            }
        })?;
        let mut salida = resultado.salida.clone();
        if resultado.timeout_agotado {
            salida.push_str("\n[micli] el gate excedió el timeout y el contenedor fue matado");
        }
        Ok(micli_agent::ResultadoGate {
            gate: gate.name.clone(),
            paso: resultado.paso(),
            exit_code: resultado.exit_code.and_then(|c| i32::try_from(c).ok()),
            salida,
        })
    }
}

/// `micli run`: ejecuta el bucle del agente para un requerimiento en
/// `TestRojo` y persiste todo lo ocurrido en el trace. Con `usar_sandbox`,
/// los gates corren aislados en Docker según `[sandbox]` de agent_rules.toml.
pub fn run(
    proyecto: &Path,
    id: &str,
    agente: &str,
    usar_sandbox: bool,
    permission_mode: Option<String>,
) -> anyhow::Result<String> {
    let args_extra = permission_mode
        .map(|modo| vec!["--permission-mode".to_owned(), modo])
        .unwrap_or_default();
    let mut ejecutor = ClaudeCodeCli {
        binario: agente.into(),
        args_extra,
    };
    if !usar_sandbox {
        return run_con(proyecto, id, &mut ejecutor, &mut GatesEnHost);
    }

    let reglas = cargar_proyecto(proyecto)?;
    let sb = reglas
        .sandbox
        .context("agent_rules.toml no tiene sección [sandbox]; agrégala o corre sin --sandbox")?;
    let timeout_seg = reglas.budget.map_or(30 * 60, |b| b.max_minutes * 60);
    let config = micli_sandbox::ConfigSandbox {
        imagen: sb.image,
        red: match sb.network {
            micli_manifest::SandboxNetwork::None => micli_sandbox::Red::None,
            micli_manifest::SandboxNetwork::Bridge => micli_sandbox::Red::Bridge,
        },
        dir_proyecto: proyecto.canonicalize()?,
        timeout: Duration::from_secs(timeout_seg),
    };
    let mut gates = GatesEnSandbox {
        sandbox: micli_sandbox::Sandbox::nuevo(config)?,
    };
    run_con(proyecto, id, &mut ejecutor, &mut gates)
}

/// Cuerpo de `run` con ejecutor y gates inyectables (testeable sin procesos).
pub fn run_con(
    proyecto: &Path,
    id: &str,
    ejecutor: &mut dyn Ejecutor,
    gates: &mut dyn EjecutorGates,
) -> anyhow::Result<String> {
    let reglas = cargar_proyecto(proyecto)?;
    let dir = rutas::dir_micli(proyecto);
    let declarados = RequirementsFile::cargar(&dir.join(rutas::ARCHIVO_REQUIREMENTS))?;
    let spec = declarados
        .requirements
        .iter()
        .find(|r| r.id.as_str() == id)
        .with_context(|| format!("`{id}` no está declarado en requirements.toml"))?;

    let mut trace = Trace::abrir(dir.join(rutas::ARCHIVO_TRACE))?;
    let (estados, _) = estados_desde_trace(&trace.entradas()?);
    let estado = *estados
        .get(id)
        .with_context(|| format!("`{id}` no está en el trace; corre `micli req add {id}`"))?;
    if estado != State::TestRojo {
        bail!(
            "`{id}` está en `{estado}`; para correr el agente debe estar en `TestRojo` \
             (spec aprobada y test rojo confirmado)"
        );
    }

    let presupuesto = reglas.budget.map_or(
        Presupuesto {
            max_intentos: 5,
            max_duracion: Duration::from_secs(30 * 60),
            max_tokens: None,
        },
        |b| Presupuesto {
            max_intentos: b.max_attempts,
            max_duracion: Duration::from_secs(b.max_minutes * 60),
            max_tokens: None,
        },
    );
    let gates_spec: Vec<GateSpec> = reglas
        .gates
        .iter()
        .map(|g| GateSpec {
            name: g.name.clone(),
            cmd: g.cmd.clone(),
            required: g.required,
        })
        .collect();
    let ruta_prompt = dir.join(rutas::ARCHIVO_PROMPT_SISTEMA);
    let prompt_sistema = if ruta_prompt.exists() {
        PromptVersionado::cargar(&ruta_prompt)?
    } else {
        PromptVersionado::desde_texto(PROMPT_SISTEMA_DEFAULT.to_owned())
    };
    let spec_texto = format!("{}\n\n{}", spec.titulo, spec.spec);

    let plan = PlanEjecucion {
        spec: &spec_texto,
        prompt_sistema: &prompt_sistema,
        gates: &gates_spec,
        presupuesto,
        dir_trabajo: proyecto,
    };
    let informe = ejecutar_requerimiento(&plan, ejecutor, gates)?;

    // Persistencia: primero las transiciones (definen el estado), luego el
    // detalle de intentos y gates, todo anclado a la cadena.
    for t in &informe.transiciones {
        trace.append(Payload::Transicion {
            requirement_id: RequirementId::new(id),
            evento: t.evento,
            desde: t.desde,
            hacia: t.hacia,
        })?;
    }
    for intento in &informe.intentos {
        let resumen = if intento.gates_pasaron {
            "gates ok".to_owned()
        } else if intento.gates.is_empty() {
            "el agente no terminó su ejecución".to_owned()
        } else {
            intento.gates.iter().find(|g| !g.paso).map_or_else(
                || "fallo".to_owned(),
                |g| format!("falló gate `{}`", g.gate),
            )
        };
        trace.append(Payload::Intento {
            requirement_id: RequirementId::new(id),
            numero: intento.numero,
            estrategia: format!("{:?}", intento.estrategia),
            tokens: intento.tokens,
            duracion_ms: u64::try_from(intento.duracion.as_millis()).unwrap_or(u64::MAX),
            gates_pasaron: intento.gates_pasaron,
            resumen,
            hash_prompt_sistema: informe.hash_prompt_sistema.clone(),
        })?;
        for gate in &intento.gates {
            trace.append(Payload::GateResultado {
                requirement_id: RequirementId::new(id),
                intento: intento.numero,
                gate: gate.gate.clone(),
                paso: gate.paso,
                exit_code: gate.exit_code,
            })?;
        }
    }

    let tokens_totales: u64 = informe.intentos.iter().map(|i| i.tokens).sum();
    let mut salida = format!(
        "`{id}`: {} tras {} intento(s), {tokens_totales} tokens.\n",
        match &informe.desenlace {
            Desenlace::Aceptado => "ACEPTADO".to_owned(),
            Desenlace::Bloqueado { motivo } => format!("BLOQUEADO ({motivo})"),
        },
        informe.intentos.len()
    );
    for intento in &informe.intentos {
        let _ = writeln!(
            salida,
            "  intento {} [{:?}]: {}",
            intento.numero,
            intento.estrategia,
            if intento.gates_pasaron {
                "gates ok"
            } else {
                "fallo"
            }
        );
    }
    Ok(salida)
}

/// `micli db rebuild`: borra la proyección SQLite y la reconstruye completa
/// desde el trace. La DB nunca es fuente de verdad.
pub fn db_rebuild(proyecto: &Path) -> anyhow::Result<String> {
    cargar_proyecto(proyecto)?;
    let dir = rutas::dir_micli(proyecto);
    let entradas = Trace::abrir(dir.join(rutas::ARCHIVO_TRACE))?.entradas()?;
    let mut store = Store::abrir(&dir.join(rutas::ARCHIVO_DB))?;
    store.reset()?;
    for entrada in &entradas {
        match &entrada.payload {
            Payload::RequirementCreado { id, titulo } => {
                store.proyectar_requirement_creado(id.as_str(), titulo, &entrada.hash)?;
            }
            Payload::Transicion {
                requirement_id,
                evento,
                desde,
                hacia,
            } => {
                store.proyectar_transicion(&TransitionRow {
                    seq: entrada.seq,
                    requirement_id: requirement_id.as_str().to_owned(),
                    evento: *evento,
                    desde: *desde,
                    hacia: *hacia,
                    hash_evento: entrada.hash.clone(),
                })?;
            }
            Payload::Intento {
                requirement_id,
                numero,
                tokens,
                duracion_ms,
                resumen,
                ..
            } => {
                store.proyectar_attempt(&AttemptRow {
                    requirement_id: requirement_id.as_str().to_owned(),
                    numero: *numero,
                    resultado: resumen.clone(),
                    tokens: *tokens,
                    duracion_ms: *duracion_ms,
                    hash_evento: entrada.hash.clone(),
                })?;
            }
            Payload::GateResultado {
                requirement_id,
                gate,
                paso,
                ..
            } => {
                store.proyectar_gate_result(&GateResultRow {
                    requirement_id: requirement_id.as_str().to_owned(),
                    gate: gate.clone(),
                    paso: *paso,
                    hash_evento: entrada.hash.clone(),
                })?;
            }
        }
    }
    Ok(format!(
        "DB reconstruida desde el trace: {} eventos proyectados.",
        entradas.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use micli_agent::{AgentError, ResultadoGate, ResultadoIntento, SolicitudIntento};
    use std::path::PathBuf;

    struct EjecutorMock {
        exito: bool,
    }

    impl Ejecutor for EjecutorMock {
        fn ejecutar(
            &mut self,
            _solicitud: SolicitudIntento<'_>,
        ) -> Result<ResultadoIntento, AgentError> {
            Ok(ResultadoIntento {
                exito: self.exito,
                tokens: 500,
                duracion: Duration::from_secs(2),
            })
        }
    }

    struct GatesMock {
        pasan: bool,
    }

    impl EjecutorGates for GatesMock {
        fn ejecutar(&mut self, gate: &GateSpec, _dir: &Path) -> Result<ResultadoGate, AgentError> {
            Ok(ResultadoGate {
                gate: gate.name.clone(),
                paso: self.pasan,
                exit_code: Some(i32::from(!self.pasan)),
                salida: "salida del gate".to_owned(),
            })
        }
    }

    /// Proyecto con `.micli/` armado a mano (sin tocar el registro global) y
    /// un requerimiento llevado hasta `TestRojo`.
    fn proyecto_en_test_rojo() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let proyecto = temp.path().canonicalize().unwrap();
        let dir = proyecto.join(rutas::DIR_PROYECTO);
        AgentRules::plantilla("demo".to_owned(), Perfil::Rust)
            .guardar(&dir.join(rutas::ARCHIVO_REGLAS))
            .unwrap();
        std::fs::write(
            dir.join(rutas::ARCHIVO_REQUIREMENTS),
            r#"
                version = 1
                [[requirements]]
                id = "REQ-001"
                titulo = "Alta de usuarios"
                spec = "Dado un correo valido, crear el usuario."
            "#,
        )
        .unwrap();
        req_add(&proyecto, "REQ-001").unwrap();
        req_avanzar(&proyecto, "REQ-001", Event::AprobarSpec).unwrap();
        req_avanzar(&proyecto, "REQ-001", Event::ConfirmarTestRojo).unwrap();
        (temp, proyecto)
    }

    fn estado_actual(proyecto: &Path, id: &str) -> State {
        let entradas = Trace::abrir(rutas::dir_micli(proyecto).join(rutas::ARCHIVO_TRACE))
            .unwrap()
            .entradas()
            .unwrap();
        estados_desde_trace(&entradas).0[id]
    }

    #[test]
    fn el_ciclo_humano_avanza_hasta_test_rojo() {
        let (_temp, proyecto) = proyecto_en_test_rojo();
        assert_eq!(estado_actual(&proyecto, "REQ-001"), State::TestRojo);
        // Transición inválida rechazada y sin efecto.
        assert!(req_avanzar(&proyecto, "REQ-001", Event::AprobarSpec).is_err());
        assert_eq!(estado_actual(&proyecto, "REQ-001"), State::TestRojo);
        // req add duplicado rechazado.
        assert!(req_add(&proyecto, "REQ-001").is_err());
    }

    #[test]
    fn run_persiste_el_ciclo_completo_y_la_cadena_queda_integra() {
        let (_temp, proyecto) = proyecto_en_test_rojo();
        let salida = run_con(
            &proyecto,
            "REQ-001",
            &mut EjecutorMock { exito: true },
            &mut GatesMock { pasan: true },
        )
        .unwrap();
        assert!(salida.contains("ACEPTADO"), "{salida}");

        assert_eq!(estado_actual(&proyecto, "REQ-001"), State::Aceptado);
        let trace = Trace::abrir(rutas::dir_micli(&proyecto).join(rutas::ARCHIVO_TRACE)).unwrap();
        trace.verificar().unwrap();
        let entradas = trace.entradas().unwrap();
        assert!(entradas.iter().any(|e| matches!(
            &e.payload,
            Payload::Intento {
                numero: 1,
                gates_pasaron: true,
                ..
            }
        )));
        // Dos gates del perfil rust: tests y lint.
        let gates: Vec<_> = entradas
            .iter()
            .filter(|e| matches!(&e.payload, Payload::GateResultado { .. }))
            .collect();
        assert_eq!(gates.len(), 2);
    }

    #[test]
    fn run_exige_test_rojo() {
        let (_temp, proyecto) = proyecto_en_test_rojo();
        req_avanzar(&proyecto, "REQ-001", Event::Bloquear).unwrap();
        let error = run_con(
            &proyecto,
            "REQ-001",
            &mut EjecutorMock { exito: true },
            &mut GatesMock { pasan: true },
        )
        .unwrap_err();
        assert!(error.to_string().contains("TestRojo"), "{error}");
    }

    /// El proceso del agente no arranca (sin sesión, flag inválido, API caída).
    struct EjecutorQueNoArranca;

    impl Ejecutor for EjecutorQueNoArranca {
        fn ejecutar(
            &mut self,
            _solicitud: SolicitudIntento<'_>,
        ) -> Result<ResultadoIntento, AgentError> {
            Err(AgentError::AgenteNoArranco {
                exit_code: Some(1),
                stderr: "Not logged in. Run `claude login`.".to_owned(),
            })
        }
    }

    #[test]
    fn run_falla_sin_tocar_el_trace_si_el_agente_no_arranca() {
        let (_temp, proyecto) = proyecto_en_test_rojo();
        let ruta_trace = rutas::dir_micli(&proyecto).join(rutas::ARCHIVO_TRACE);
        let eventos_antes = Trace::abrir(&ruta_trace).unwrap().entradas().unwrap().len();

        let error = run_con(
            &proyecto,
            "REQ-001",
            &mut EjecutorQueNoArranca,
            &mut GatesMock { pasan: true },
        )
        .unwrap_err();

        // El operador ve la causa real, no "el agente no terminó" cinco veces.
        assert!(error.to_string().contains("Not logged in"), "{error}");
        // Ni intentos ni transiciones: el requerimiento sigue en TestRojo y
        // `micli run` puede relanzarse cuando el entorno esté arreglado.
        let trace = Trace::abrir(&ruta_trace).unwrap();
        trace.verificar().unwrap();
        assert_eq!(trace.entradas().unwrap().len(), eventos_antes);
        assert_eq!(estado_actual(&proyecto, "REQ-001"), State::TestRojo);
    }

    #[test]
    fn run_bloquea_cuando_los_gates_nunca_pasan_y_rebuild_lo_proyecta() {
        let (_temp, proyecto) = proyecto_en_test_rojo();
        let salida = run_con(
            &proyecto,
            "REQ-001",
            &mut EjecutorMock { exito: true },
            &mut GatesMock { pasan: false },
        )
        .unwrap();
        assert!(salida.contains("BLOQUEADO"), "{salida}");
        assert!(matches!(
            estado_actual(&proyecto, "REQ-001"),
            State::Bloqueado { .. }
        ));

        let resumen = db_rebuild(&proyecto).unwrap();
        assert!(resumen.contains("proyectados"), "{resumen}");
        let store = Store::abrir(&rutas::dir_micli(&proyecto).join(rutas::ARCHIVO_DB)).unwrap();
        let fila = store.requirement("REQ-001").unwrap().unwrap();
        assert!(matches!(fila.estado, State::Bloqueado { .. }));
        assert!(!store.transiciones("REQ-001").unwrap().is_empty());
    }
}
