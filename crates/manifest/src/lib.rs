//! Registro de proyectos, perfiles y manifests (`agent_rules.toml`,
//! `requirements.toml`) de `micli`.
//!
//! Este crate solo modela, parsea y valida; no impone reglas (eso lo hacen
//! los hooks y gates generados por el binario) ni conoce rutas globales.

mod registry;
mod requirements;
mod rules;

pub use registry::{RegisteredProject, Registry};
pub use requirements::{RequirementSpec, RequirementsFile};
pub use rules::{
    AgentRules, Budget, Gate, Perfil, ProjectInfo, Protected, SandboxConfig, SandboxNetwork,
};

/// Errores de carga y validación de manifests.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("error de E/S leyendo o escribiendo un manifest: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML inválido: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("no se pudo serializar el manifest: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error(
        "versión de esquema no soportada: {encontrada} (esta versión de micli soporta {soportada})"
    )]
    VersionNoSoportada { encontrada: u32, soportada: u32 },
    #[error("el nombre del proyecto está vacío")]
    NombreProyectoVacio,
    #[error("gate `{gate}` inválido: {motivo}")]
    GateInvalido { gate: String, motivo: String },
    #[error("gate `{gate}` duplicado")]
    GateDuplicado { gate: String },
    #[error("requerimiento `{id}` inválido: {motivo}")]
    RequirementInvalido { id: String, motivo: String },
    #[error("requerimiento `{id}` duplicado")]
    RequirementDuplicado { id: String },
    #[error("el proyecto `{proyecto}` ya está registrado")]
    ProyectoDuplicado { proyecto: String },
    #[error("el archivo `{path}` ya existe; no se sobreescribe")]
    ArchivoYaExiste { path: String },
}

/// Escribe `contenido` en `path` solo si el archivo no existe todavía,
/// creando el directorio padre si falta.
pub(crate) fn escribir_nuevo(path: &std::path::Path, contenido: &str) -> Result<(), ManifestError> {
    if let Some(padre) = path.parent()
        && !padre.as_os_str().is_empty()
    {
        std::fs::create_dir_all(padre)?;
    }
    if path.exists() {
        return Err(ManifestError::ArchivoYaExiste {
            path: path.display().to_string(),
        });
    }
    std::fs::write(path, contenido)?;
    Ok(())
}
