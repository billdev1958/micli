//! Resolución de rutas: el ámbito global de micli y el directorio
//! autocontenido `.micli/` de cada proyecto.

use anyhow::Context;
use std::path::{Path, PathBuf};

/// Nombre del directorio por proyecto.
pub const DIR_PROYECTO: &str = ".micli";
pub const ARCHIVO_REGLAS: &str = "agent_rules.toml";
pub const ARCHIVO_REQUIREMENTS: &str = "requirements.toml";
pub const ARCHIVO_TRACE: &str = "trace.jsonl";
const ARCHIVO_REGISTRO: &str = "projects.toml";

/// Directorio global de micli. `MICLI_HOME` tiene prioridad (útil en tests y
/// cuando el antivirus interfiere con `%APPDATA%`); si no, `%APPDATA%\micli`
/// en Windows y `~/.micli` en el resto.
pub fn dir_global() -> anyhow::Result<PathBuf> {
    if let Ok(valor) = std::env::var("MICLI_HOME")
        && !valor.trim().is_empty()
    {
        return Ok(PathBuf::from(valor));
    }
    if cfg!(windows) {
        let appdata = std::env::var("APPDATA").context(
            "no se pudo resolver el directorio global: ni MICLI_HOME ni APPDATA están definidos",
        )?;
        Ok(PathBuf::from(appdata).join("micli"))
    } else {
        let home = std::env::var("HOME").context(
            "no se pudo resolver el directorio global: ni MICLI_HOME ni HOME están definidos",
        )?;
        Ok(PathBuf::from(home).join(".micli"))
    }
}

/// Ruta del registro global de proyectos.
pub fn registro_global() -> anyhow::Result<PathBuf> {
    Ok(dir_global()?.join(ARCHIVO_REGISTRO))
}

/// El directorio `.micli/` de un proyecto.
pub fn dir_micli(proyecto: &Path) -> PathBuf {
    proyecto.join(DIR_PROYECTO)
}
