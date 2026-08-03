//! Implementación de los subcomandos. Cada función devuelve el texto a
//! imprimir; `main` es el único punto de salida a terminal.

use crate::rutas;
use anyhow::{Context, bail};
use micli_core::State;
use micli_manifest::{AgentRules, Perfil, Registry, RequirementsFile};
use micli_trace::{Payload, Trace};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

/// `micli init`: genera `.micli/` y registra el proyecto en el ámbito global.
pub fn init(proyecto: &Path, name: Option<String>, profile: Perfil) -> anyhow::Result<String> {
    let proyecto = proyecto
        .canonicalize()
        .with_context(|| format!("no existe el directorio {}", proyecto.display()))?;
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
    // El trace nace vacío; se crea el archivo (solo-append) desde ya para
    // que quede visible y `trace verify` funcione sin eventos.
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(dir.join(rutas::ARCHIVO_TRACE))?;

    // …y solo al final se persiste el registro global.
    registro.guardar(&ruta_registro)?;

    Ok(format!(
        "Proyecto `{name}` registrado.\n\
         - Reglas:          {dir}\\{reglas}\n\
         - Requerimientos:  {dir}\\{reqs}\n\
         - Trace:           {dir}\\{trace}\n\
         Revisa y ajusta agent_rules.toml (gates, protected, budget) antes de usar el agente.",
        dir = dir.display(),
        reglas = rutas::ARCHIVO_REGLAS,
        reqs = rutas::ARCHIVO_REQUIREMENTS,
        trace = rutas::ARCHIVO_TRACE,
    ))
}

fn cargar_proyecto(proyecto: &Path) -> anyhow::Result<AgentRules> {
    let ruta = rutas::dir_micli(proyecto).join(rutas::ARCHIVO_REGLAS);
    if !ruta.exists() {
        bail!(
            "este directorio no está inicializado como proyecto micli (falta {}); corre `micli init --profile <perfil>`",
            ruta.display()
        );
    }
    Ok(AgentRules::cargar(&ruta)?)
}

/// `micli status`: perfil, eventos del trace y estado actual de cada
/// requerimiento, derivado reproduciendo el trace (la fuente de verdad).
pub fn status(proyecto: &Path) -> anyhow::Result<String> {
    let reglas = cargar_proyecto(proyecto)?;
    let dir = rutas::dir_micli(proyecto);
    let declarados = RequirementsFile::cargar(&dir.join(rutas::ARCHIVO_REQUIREMENTS))?;
    let entradas = Trace::abrir(dir.join(rutas::ARCHIVO_TRACE))?.entradas()?;

    let mut estados: HashMap<String, State> = HashMap::new();
    let mut titulos: HashMap<String, String> = HashMap::new();
    for entrada in &entradas {
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
        }
    }

    let mut salida = format!(
        "Proyecto: {} (perfil {:?})\nTrace: {} eventos\n",
        reglas.project.name,
        reglas.project.profile,
        entradas.len()
    );
    let mut filas: Vec<(String, State, String)> = Vec::new();
    for req in &declarados.requirements {
        let id = req.id.as_str().to_owned();
        let estado = estados.get(&id).copied().unwrap_or(State::Draft);
        filas.push((id.clone(), estado, req.titulo.clone()));
        estados.remove(&id);
        titulos.remove(&id);
    }
    // Requerimientos que aparecen en el trace pero no están declarados.
    let mut extra: Vec<(String, State, String)> = estados
        .into_iter()
        .map(|(id, estado)| {
            let titulo = titulos.remove(&id).unwrap_or_default();
            (id, estado, format!("{titulo} [no declarado]"))
        })
        .collect();
    extra.sort_by(|a, b| a.0.cmp(&b.0));
    filas.extend(extra);

    if filas.is_empty() {
        salida.push_str("Sin requerimientos declarados.\n");
    } else {
        salida.push_str("Requerimientos:\n");
        for (id, estado, titulo) in filas {
            let _ = writeln!(salida, "  {id:<12} {:<24} {titulo}", estado.to_string());
        }
    }
    Ok(salida)
}

/// `micli trace verify`: valida la cadena completa de eventos.
pub fn trace_verify(proyecto: &Path) -> anyhow::Result<String> {
    cargar_proyecto(proyecto)?;
    let trace = Trace::abrir(rutas::dir_micli(proyecto).join(rutas::ARCHIVO_TRACE))?;
    let verificadas = trace.verificar().context("la cadena del trace está rota")?;
    Ok(format!("OK: cadena íntegra ({verificadas} eventos)"))
}
