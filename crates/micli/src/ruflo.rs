//! Integración con ruflo (orquestador MCP de ruvnet para Claude Code).
//!
//! micli y ruflo son capas distintas: ruflo trabaja *dentro* de la sesión
//! del agente (MCP, memoria, swarms); micli decide *desde fuera* qué se
//! acepta. El puente de configuración es el `CLAUDE.md` del proyecto: micli
//! mantiene ahí un bloque delimitado con sus reglas (generado desde
//! `agent_rules.toml`), que toda sesión de Claude —con o sin ruflo— lee
//! desde el arranque.

use crate::comandos::cargar_proyecto;
use anyhow::{Context, bail};
use micli_manifest::AgentRules;
use std::fmt::Write as _;
use std::path::Path;

const MARCA_INICIO: &str = "<!-- micli:inicio -->";
const MARCA_FIN: &str = "<!-- micli:fin -->";

/// `micli ruflo init`: inicializa ruflo en el proyecto (vía npx) y deja las
/// reglas de micli comunicadas en el CLAUDE.md del proyecto.
pub fn init(proyecto: &Path) -> anyhow::Result<String> {
    // micli primero: las reglas a comunicar salen de agent_rules.toml.
    cargar_proyecto(proyecto)?;

    let salida_ruflo = correr_ruflo(proyecto, &["init", "--no-global"])?;
    let salida_sync = sync(proyecto)?;
    Ok(format!(
        "{salida_ruflo}\n{salida_sync}\n\
         Listo: las sesiones que micli lance con `micli run` cargarán el MCP \
         de ruflo y las reglas de micli automáticamente."
    ))
}

/// `micli ruflo sync`: (re)escribe el bloque de micli en el CLAUDE.md del
/// proyecto a partir de `agent_rules.toml`. Idempotente: correrlo dos veces
/// deja un solo bloque. Ejecútalo cada vez que cambien las reglas.
pub fn sync(proyecto: &Path) -> anyhow::Result<String> {
    let reglas = cargar_proyecto(proyecto)?;
    let bloque = bloque_micli(&reglas);
    let ruta = proyecto.join("CLAUDE.md");
    let actual = match std::fs::read_to_string(&ruta) {
        Ok(contenido) => contenido,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    std::fs::write(&ruta, insertar_bloque(&actual, &bloque))?;

    let mut salida = format!(
        "Reglas de micli sincronizadas en {} (gates, protected y budget de agent_rules.toml).",
        ruta.display()
    );
    if !proyecto.join(".mcp.json").exists() {
        salida.push_str(
            "\nAviso: no hay .mcp.json en el proyecto; ruflo no parece inicializado aquí. \
             Corre `micli ruflo init`.",
        );
    }
    Ok(salida)
}

/// Ejecuta `npx --yes ruflo@latest <args>` en el directorio del proyecto y
/// devuelve la cola de su salida.
fn correr_ruflo(proyecto: &Path, args: &[&str]) -> anyhow::Result<String> {
    let mut comando = if cfg!(windows) {
        // npx es un shim .cmd en Windows: hay que pasar por cmd.
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg("npx");
        c
    } else {
        std::process::Command::new("npx")
    };
    comando
        .arg("--yes")
        .arg("ruflo@latest")
        .args(args)
        .current_dir(proyecto);
    let salida = comando
        .output()
        .context("no se pudo ejecutar npx; ¿está instalado Node.js?")?;

    let texto = format!(
        "{}{}",
        String::from_utf8_lossy(&salida.stdout),
        String::from_utf8_lossy(&salida.stderr)
    );
    if !salida.status.success() {
        bail!(
            "ruflo terminó con error (exit {:?}):\n{}",
            salida.status.code(),
            cola(&texto, 15)
        );
    }
    Ok(format!("ruflo:\n{}", cola(&texto, 12)))
}

/// Últimas `n` líneas no vacías de un texto.
fn cola(texto: &str, n: usize) -> String {
    let lineas: Vec<&str> = texto.lines().filter(|l| !l.trim().is_empty()).collect();
    let desde = lineas.len().saturating_sub(n);
    lineas[desde..].join("\n")
}

/// Inserta o reemplaza el bloque delimitado por las marcas de micli.
fn insertar_bloque(contenido: &str, bloque: &str) -> String {
    match (contenido.find(MARCA_INICIO), contenido.find(MARCA_FIN)) {
        (Some(inicio), Some(fin)) if fin >= inicio => {
            let despues = fin + MARCA_FIN.len();
            format!("{}{bloque}{}", &contenido[..inicio], &contenido[despues..])
        }
        _ if contenido.trim().is_empty() => format!("{bloque}\n"),
        _ => format!("{}\n\n{bloque}\n", contenido.trim_end()),
    }
}

/// El bloque de reglas que ve el agente, generado desde `agent_rules.toml`.
/// Es comunicación, no enforcement: la aceptación la deciden los gates de
/// micli aunque el agente ignore este texto.
fn bloque_micli(reglas: &AgentRules) -> String {
    let mut b = String::new();
    let _ = writeln!(b, "{MARCA_INICIO}");
    let _ = writeln!(
        b,
        "# Arnés micli (generado por `micli ruflo sync`; no editar a mano)\n"
    );
    let _ = writeln!(
        b,
        "Este proyecto está orquestado por micli: el agente escribe código, \
         micli decide qué se acepta. La aceptación la determinan únicamente \
         los gates deterministas de abajo; ninguna autoevaluación del agente \
         ni review de IA cuenta como aprobación.\n"
    );

    let _ = writeln!(b, "## Gates de aceptación\n");
    if reglas.gates.is_empty() {
        let _ = writeln!(b, "(sin gates declarados todavía)\n");
    } else {
        let _ = writeln!(b, "| gate | comando | requerido |");
        let _ = writeln!(b, "|------|---------|-----------|");
        for gate in &reglas.gates {
            let _ = writeln!(
                b,
                "| {} | `{}` | {} |",
                gate.name,
                gate.cmd,
                if gate.required { "sí" } else { "advisory" }
            );
        }
        let _ = writeln!(b);
    }

    if !reglas.protected.paths.is_empty() || !reglas.protected.dependency_files.is_empty() {
        let _ = writeln!(b, "## Rutas protegidas\n");
        for ruta in &reglas.protected.paths {
            let _ = writeln!(b, "- NO tocar: `{ruta}`");
        }
        for archivo in &reglas.protected.dependency_files {
            let _ = writeln!(
                b,
                "- `{archivo}`: editar dependencias requiere aprobación humana explícita"
            );
        }
        let _ = writeln!(b);
    }

    if let Some(budget) = reglas.budget {
        let _ = writeln!(
            b,
            "## Presupuesto por requerimiento\n\n\
             Máximo {} intentos y {} minutos; lo impone micli en Rust, no es \
             negociable desde la sesión.\n",
            budget.max_attempts, budget.max_minutes
        );
    }

    let _ = writeln!(
        b,
        "## Flujo de trabajo\n\n\
         - Los requerimientos viven en `.micli/requirements.toml` y avanzan \
         por la máquina de estados de micli (`Draft → SpecAprobada → TestRojo \
         → Implementando → Verificando → Aceptado`).\n\
         - El agente solo trabaja dentro de `micli run <id>`: recibe la spec \
         y, si hubo un intento fallido, un resumen estructurado del fallo.\n\
         - No debilites, borres ni marques con ignore tests existentes; si un \
         test parece incorrecto, detente y explícalo.\n\
         - El trace de micli (`.micli/trace.jsonl`) es append-only y auditado: \
         no lo edites."
    );
    let _ = write!(b, "{MARCA_FIN}");
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use micli_manifest::Perfil;

    fn bloque_de_prueba() -> String {
        bloque_micli(&AgentRules::plantilla("demo".to_owned(), Perfil::Rust))
    }

    #[test]
    fn el_bloque_comunica_gates_budget_y_flujo() {
        let bloque = bloque_de_prueba();
        assert!(bloque.starts_with(MARCA_INICIO));
        assert!(bloque.ends_with(MARCA_FIN));
        assert!(bloque.contains("cargo test --workspace"));
        assert!(bloque.contains("Máximo 5 intentos y 30 minutos"));
        assert!(bloque.contains("micli run"));
    }

    #[test]
    fn insertar_agrega_al_final_cuando_no_hay_bloque() {
        let resultado = insertar_bloque("# Mi proyecto\n\nNotas.\n", &bloque_de_prueba());
        assert!(resultado.starts_with("# Mi proyecto"));
        assert!(resultado.contains(MARCA_INICIO));
        assert_eq!(resultado.matches(MARCA_INICIO).count(), 1);
    }

    #[test]
    fn insertar_es_idempotente_y_reemplaza_el_bloque_viejo() {
        let bloque = bloque_de_prueba();
        let una_vez = insertar_bloque("contenido previo\n", &bloque);
        let dos_veces = insertar_bloque(&una_vez, &bloque);
        assert_eq!(una_vez, dos_veces);
        assert_eq!(dos_veces.matches(MARCA_INICIO).count(), 1);
        assert_eq!(dos_veces.matches(MARCA_FIN).count(), 1);

        // Si el bloque viejo tenía otro contenido, se reemplaza completo.
        let viejo = format!("intro\n\n{MARCA_INICIO}\nreglas viejas\n{MARCA_FIN}\ncoda\n");
        let actualizado = insertar_bloque(&viejo, &bloque);
        assert!(!actualizado.contains("reglas viejas"));
        assert!(actualizado.contains("intro"));
        assert!(actualizado.contains("coda"));
        assert!(actualizado.contains("cargo test --workspace"));
    }

    #[test]
    fn archivo_vacio_recibe_solo_el_bloque() {
        let resultado = insertar_bloque("", &bloque_de_prueba());
        assert!(resultado.starts_with(MARCA_INICIO));
        assert_eq!(resultado.matches(MARCA_INICIO).count(), 1);
    }
}
